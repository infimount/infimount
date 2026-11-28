//! HTTP boundary for the explicitly configured hosted service.
//!
//! Authentication and aggregate byte limits precede the MCP parser. Permits
//! remain owned by streaming response bodies until completion or disconnect.
use std::collections::HashMap;
use std::future::Future;
use std::io;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::task::{Context, Poll};
use std::time::Duration;

use axum::body::{to_bytes, Body, Bytes};
use axum::extract::{Path, State};
use axum::http::header::{
    AUTHORIZATION, CACHE_CONTROL, CONTENT_ENCODING, CONTENT_LENGTH, HOST, ORIGIN, RANGE,
    TRANSFER_ENCODING, WWW_AUTHENTICATE,
};
use axum::http::{HeaderMap, HeaderValue, Method, Request, StatusCode};
use axum::middleware::{from_fn_with_state, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, put};
use axum::{Extension, Json, Router};
use http_body::{Body as HttpBody, Frame, SizeHint};
use rmcp::transport::streamable_http_server::{
    session::never::NeverSessionManager, StreamableHttpServerConfig, StreamableHttpService,
};
use serde_json::json;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio::time::{sleep_until, timeout, timeout_at, Instant, Sleep};
use tokio_util::sync::CancellationToken;

use crate::auth::{Authenticator, Principal};
use crate::config::HostedConfig;
use crate::error::{HostedError, Result};
use crate::mcp::{HostedMcpServer, CREATE_SCOPE, DOWNLOAD_SCOPE, READ_SCOPE};
use crate::storage::StoragePlane;
use crate::transfers::TransferManager;

const MAX_BEARER_BYTES: usize = 16 * 1024;
const READY_CACHE_TTL: Duration = Duration::from_secs(5);
const READY_PROBE_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Clone)]
struct HttpState {
    config: Arc<HostedConfig>,
    auth: Authenticator,
    storage: Arc<StoragePlane>,
    transfers: Arc<TransferManager>,
    limiter: Arc<RequestLimiter>,
    authority: String,
    hostname: String,
    port: u16,
    challenge: HeaderValue,
    readiness: Arc<ReadinessCache>,
}

struct ReadinessCache {
    state: tokio::sync::Mutex<(Instant, bool)>,
    last_result: AtomicBool,
}

struct RequestLimiter {
    global: Arc<Semaphore>,
    per_owner: Mutex<HashMap<String, Weak<Semaphore>>>,
    owner_limit: usize,
}

struct Permits {
    _global: OwnedSemaphorePermit,
    principal: Option<OwnedSemaphorePermit>,
}

/// The SDK may detach its stateless handler task. Sharing the lease with that
/// handler prevents HTTP cancellation from freeing capacity while work is
/// still running. No principal is ever stored in global transport state.
pub(crate) struct RequestLifetime {
    cancellation: CancellationToken,
    deadline: Instant,
    _permits: Permits,
}

tokio::task_local! {
    // Resource ownership only. No identity, scopes, paths, or credentials are
    // ambient. Storage cleanup can retain capacity without retaining authority.
    static RESOURCE_LEASE: Arc<RequestLifetime>;
}

pub(crate) fn current_resource_lease() -> Option<Arc<dyn Send + Sync>> {
    RESOURCE_LEASE
        .try_with(|lease| lease.clone() as Arc<dyn Send + Sync>)
        .ok()
}

impl RequestLifetime {
    pub(crate) async fn run<T>(
        self: &Arc<Self>,
        operation: impl Future<Output = Result<T>>,
    ) -> Result<T> {
        RESOURCE_LEASE
            .scope(self.clone(), async {
                tokio::select! {
                    biased;
                    _ = self.cancellation.cancelled() => Err(HostedError::Unavailable),
                    _ = sleep_until(self.deadline) => Err(HostedError::Unavailable),
                    result = operation => result,
                }
            })
            .await
    }
}

struct HttpLease {
    lifetime: Arc<RequestLifetime>,
}

impl HttpLease {
    fn new(permits: Permits, deadline: Instant) -> Self {
        Self {
            lifetime: Arc::new(RequestLifetime {
                cancellation: CancellationToken::new(),
                deadline,
                _permits: permits,
            }),
        }
    }
}

impl Drop for HttpLease {
    fn drop(&mut self) {
        self.lifetime.cancellation.cancel();
    }
}

enum RequestGuard {
    Permits(Permits),
    Lease(HttpLease),
}

impl From<Permits> for RequestGuard {
    fn from(value: Permits) -> Self {
        Self::Permits(value)
    }
}

impl From<HttpLease> for RequestGuard {
    fn from(value: HttpLease) -> Self {
        Self::Lease(value)
    }
}

impl RequestLimiter {
    fn new(global_limit: usize, owner_limit: usize) -> Self {
        Self {
            global: Arc::new(Semaphore::new(global_limit)),
            per_owner: Mutex::new(HashMap::new()),
            owner_limit,
        }
    }

    fn global(&self) -> Result<Permits> {
        Ok(Permits {
            _global: self
                .global
                .clone()
                .try_acquire_owned()
                .map_err(|_| HostedError::Limit)?,
            principal: None,
        })
    }

    fn principal(&self, owner: String, permits: &mut Permits) -> Result<()> {
        // A Weak entry cannot outlive the last in-flight request for an owner.
        // Pruning on acquisition bounds this map by global concurrency, rather
        // than by the number of identities ever seen by the server.
        let semaphore = {
            let mut owners = self.per_owner.lock().map_err(|_| HostedError::Internal)?;
            owners.retain(|_, semaphore| semaphore.strong_count() > 0);
            if let Some(semaphore) = owners.get(&owner).and_then(Weak::upgrade) {
                semaphore
            } else {
                let semaphore = Arc::new(Semaphore::new(self.owner_limit));
                owners.insert(owner, Arc::downgrade(&semaphore));
                semaphore
            }
        };
        permits.principal = Some(
            semaphore
                .try_acquire_owned()
                .map_err(|_| HostedError::Limit)?,
        );
        Ok(())
    }
}

/// Validate configuration, resolve the initial key set, and construct the
/// independent hosted router. This never reads desktop MCP settings.
pub async fn router(config: Arc<HostedConfig>) -> Result<Router> {
    config.validate()?;
    let auth = Authenticator::new(&config.auth).await?;
    let storage = Arc::new(StoragePlane::new(config.clone())?);
    let issuer = crate::delegation::DelegatedIssuer::from_config(&config.delivery)?;
    let transfers = Arc::new(TransferManager::with_issuer(storage.clone(), issuer));
    build_router(config, auth, storage, transfers)
}

fn build_router(
    config: Arc<HostedConfig>,
    auth: Authenticator,
    storage: Arc<StoragePlane>,
    transfers: Arc<TransferManager>,
) -> Result<Router> {
    let public_url = url::Url::parse(&config.public_url)
        .map_err(|_| HostedError::Config("public URL is invalid"))?;
    let authority = public_url[url::Position::BeforeHost..url::Position::AfterPort].to_owned();
    let hostname = public_url
        .host_str()
        .ok_or(HostedError::Config("public URL host is missing"))?
        .to_ascii_lowercase();
    let port = public_url
        .port_or_known_default()
        .ok_or(HostedError::Config("public URL port is missing"))?;
    let challenge = HeaderValue::from_str(&format!(
        "Bearer resource_metadata=\"{}/.well-known/oauth-protected-resource/mcp\"",
        config.public_url
    ))
    .map_err(|_| HostedError::Config("public URL cannot form an authentication challenge"))?;
    let state = HttpState {
        limiter: Arc::new(RequestLimiter::new(
            config.limits.global_concurrency,
            config.limits.principal_concurrency,
        )),
        config,
        auth,
        storage: storage.clone(),
        transfers: transfers.clone(),
        authority: authority.clone(),
        hostname,
        port,
        challenge,
        readiness: Arc::new(ReadinessCache {
            state: tokio::sync::Mutex::new((Instant::now() - READY_CACHE_TTL, false)),
            last_result: AtomicBool::new(false),
        }),
    };
    let server = HostedMcpServer::new(storage, transfers);
    let service: StreamableHttpService<HostedMcpServer, NeverSessionManager> =
        StreamableHttpService::new(
            move || Ok(server.clone()),
            Arc::new(NeverSessionManager::default()),
            StreamableHttpServerConfig::default()
                .with_stateful_mode(false)
                .with_json_response(true)
                .with_sse_keep_alive(None)
                .with_sse_retry(None)
                .with_allowed_hosts([authority]),
        );
    Ok(Router::new()
        .route_service("/mcp", service)
        .route("/downloads/{id}", get(download).head(method_not_allowed))
        .route("/uploads/{id}", put(upload))
        .route("/health/live", get(live))
        .route("/health/ready", get(ready))
        .route(
            "/.well-known/oauth-protected-resource",
            get(protected_resource),
        )
        .route(
            "/.well-known/oauth-protected-resource/mcp",
            get(protected_resource),
        )
        .fallback(|| async { HostedError::NotFound.into_response() })
        .with_state(state.clone())
        .layer(from_fn_with_state(state, boundary)))
}

/// The deployment must keep this listener reachable only through its trusted
/// TLS ingress. Host and Origin checks do not replace that network boundary.
pub async fn serve(config: HostedConfig) -> Result<()> {
    let listen = config.listen;
    let exporter = crate::telemetry::OtlpExporter::from_config(&config.telemetry)?;
    let app = router(Arc::new(config)).await?;
    let listener = tokio::net::TcpListener::bind(listen)
        .await
        .map_err(|_| HostedError::Unavailable)?;
    #[cfg(unix)]
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .map_err(|_| HostedError::Unavailable)?;
    let _reporter = crate::metrics::start_reporting(exporter);
    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            #[cfg(unix)]
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {},
                _ = terminate.recv() => {},
            }
            #[cfg(not(unix))]
            {
                let _ = tokio::signal::ctrl_c().await;
            }
        })
        .await
        .map_err(|_| HostedError::Unavailable)
}

async fn boundary(
    State(state): State<HttpState>,
    mut request: Request<Body>,
    next: Next,
) -> Response {
    if validate_target(&state, &request).is_err() {
        crate::audit::record(None, "http_target", Err(HostedError::Forbidden), None, 0);
        return security_headers(HostedError::Forbidden.into_response());
    }
    if request.uri().query().is_some()
        || request.headers().contains_key("mcp-session-id")
        || request.headers().contains_key("last-event-id")
        || request.headers().contains_key(CONTENT_ENCODING)
    {
        crate::audit::record(
            None,
            "http_headers",
            Err(HostedError::InvalidRequest),
            None,
            0,
        );
        return security_headers(HostedError::InvalidRequest.into_response());
    }
    // Normalize the already validated authority for the SDK's independent Host
    // check, including HTTP/2 requests carrying an authority without Host.
    let Ok(host) = HeaderValue::from_str(&state.authority) else {
        return security_headers(HostedError::Internal.into_response());
    };
    request.headers_mut().insert(HOST, host);

    if public_endpoint(request.uri().path()) {
        if !matches!(*request.method(), Method::GET | Method::HEAD)
            || request.headers().contains_key(TRANSFER_ENCODING)
            || request.body().size_hint().exact() != Some(0)
        {
            crate::audit::record(
                None,
                "http_public_request",
                Err(HostedError::InvalidRequest),
                None,
                0,
            );
            return security_headers(HostedError::InvalidRequest.into_response());
        }
        return security_headers(next.run(request).await);
    }

    let deadline =
        Instant::now() + Duration::from_secs(state.config.limits.request_timeout_seconds);
    let mut permits = match state.limiter.global() {
        Ok(permits) => permits,
        Err(_) => {
            crate::audit::record(None, "http_concurrency", Err(HostedError::Limit), None, 0);
            return security_headers(busy());
        }
    };
    let principal = match bearer(request.headers()) {
        Ok(token) => match timeout_at(deadline, state.auth.verify(token)).await {
            Ok(Ok(principal)) => principal,
            Ok(Err(HostedError::Unavailable)) | Err(_) => {
                crate::audit::record(
                    None,
                    "http_authentication",
                    Err(HostedError::Unavailable),
                    None,
                    0,
                );
                return guarded(
                    HostedError::Unavailable.into_response(),
                    permits,
                    deadline,
                    None,
                );
            }
            _ => return guarded(unauthorized(&state), permits, deadline, None),
        },
        Err(_) => return guarded(unauthorized(&state), permits, deadline, None),
    };
    if state
        .limiter
        .principal(principal.owner_key(), &mut permits)
        .is_err()
    {
        crate::audit::record(
            Some(&principal),
            "http_concurrency",
            Err(HostedError::Limit),
            None,
            0,
        );
        return guarded(busy(), permits, deadline, Some(principal));
    }
    let lease = HttpLease::new(permits, deadline);
    // Authentication is carried as an immutable extension, never as a mutable
    // server field or an identity asserted by an HTTP client.
    request.headers_mut().remove(AUTHORIZATION);
    request.extensions_mut().insert(principal.clone());
    request.extensions_mut().insert(lease.lifetime.clone());

    let input_limit = if request.uri().path().starts_with("/uploads/") {
        state.config.limits.transfer_bytes
    } else if request.uri().path() == "/mcp" {
        state.config.limits.json_bytes
    } else {
        0
    };
    if input_limit == 0 && request.headers().contains_key(TRANSFER_ENCODING) {
        return guarded(
            HostedError::InvalidRequest.into_response(),
            lease,
            deadline,
            Some(principal),
        );
    }
    match content_length(request.headers()) {
        Ok(Some(length)) if length > input_limit as u64 => {
            return guarded(
                HostedError::Limit.into_response(),
                lease,
                deadline,
                Some(principal),
            );
        }
        Err(error) => {
            return guarded(error.into_response(), lease, deadline, Some(principal));
        }
        _ => {}
    }
    let (parts, body) = request.into_parts();
    let body = if parts.uri.path() == "/mcp" {
        // The SDK reads Body directly, so an extractor-only DefaultBodyLimit
        // would not protect it. to_bytes applies http_body_util::Limited while
        // collecting actual frames, including requests with no Content-Length.
        match timeout_at(deadline, to_bytes(body, input_limit)).await {
            Ok(Ok(bytes)) => Body::from(bytes),
            Ok(Err(_)) => {
                return guarded(
                    HostedError::Limit.into_response(),
                    lease,
                    deadline,
                    Some(principal),
                );
            }
            Err(_) => {
                return guarded(
                    HostedError::Unavailable.into_response(),
                    lease,
                    deadline,
                    Some(principal),
                );
            }
        }
    } else {
        Body::new(BoundedBody::new(
            body,
            deadline,
            Some(principal.clone()),
            Some(input_limit),
            None,
        ))
    };
    let request = Request::from_parts(parts, body);
    let response = match timeout_at(deadline, next.run(request)).await {
        Ok(response) => response,
        Err(_) => HostedError::Unavailable.into_response(),
    };
    guarded(response, lease, deadline, Some(principal))
}

fn public_endpoint(path: &str) -> bool {
    matches!(
        path,
        "/health/live"
            | "/health/ready"
            | "/.well-known/oauth-protected-resource"
            | "/.well-known/oauth-protected-resource/mcp"
    )
}

fn validate_target(state: &HttpState, request: &Request<Body>) -> Result<()> {
    let host = single_header(request.headers(), HOST.as_str())?;
    let authority = host
        .or_else(|| request.uri().authority().map(|value| value.as_str()))
        .ok_or(HostedError::Forbidden)?;
    for value in [
        Some(authority),
        request.uri().authority().map(|value| value.as_str()),
    ]
    .into_iter()
    .flatten()
    {
        let authority: axum::http::uri::Authority =
            value.parse().map_err(|_| HostedError::Forbidden)?;
        if authority.host().to_ascii_lowercase() != state.hostname
            || authority.port_u16().unwrap_or(443) != state.port
            || authority.as_str().contains('@')
        {
            return Err(HostedError::Forbidden);
        }
    }
    if let Some(origin) = single_header(request.headers(), ORIGIN.as_str())? {
        if origin != state.config.public_url {
            return Err(HostedError::Forbidden);
        }
    }
    Ok(())
}

fn single_header<'a>(headers: &'a HeaderMap, name: &str) -> Result<Option<&'a str>> {
    let mut values = headers.get_all(name).iter();
    let value = values
        .next()
        .map(|value| value.to_str().map_err(|_| HostedError::InvalidRequest))
        .transpose()?;
    if values.next().is_some() {
        return Err(HostedError::InvalidRequest);
    }
    Ok(value)
}

fn bearer(headers: &HeaderMap) -> Result<&str> {
    let token = single_header(headers, AUTHORIZATION.as_str())?
        .and_then(|value| value.strip_prefix("Bearer "))
        .ok_or(HostedError::Unauthorized)?;
    if token.is_empty()
        || token.len() > MAX_BEARER_BYTES
        || token.bytes().any(|byte| byte.is_ascii_whitespace())
    {
        return Err(HostedError::Unauthorized);
    }
    Ok(token)
}

fn content_length(headers: &HeaderMap) -> Result<Option<u64>> {
    single_header(headers, CONTENT_LENGTH.as_str())?
        .map(|value| value.parse().map_err(|_| HostedError::InvalidRequest))
        .transpose()
}

fn unauthorized(state: &HttpState) -> Response {
    crate::audit::record(
        None,
        "http_authentication",
        Err(HostedError::Unauthorized),
        None,
        0,
    );
    let mut response = HostedError::Unauthorized.into_response();
    response
        .headers_mut()
        .insert(WWW_AUTHENTICATE, state.challenge.clone());
    response
}

fn busy() -> Response {
    (
        StatusCode::TOO_MANY_REQUESTS,
        [("retry-after", "1")],
        Json(json!({"error":"concurrency_limit"})),
    )
        .into_response()
}

fn security_headers(mut response: Response) -> Response {
    let headers = response.headers_mut();
    headers.insert(CACHE_CONTROL, HeaderValue::from_static("private, no-store"));
    headers.insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    headers.insert("referrer-policy", HeaderValue::from_static("no-referrer"));
    headers.insert(
        "content-security-policy",
        HeaderValue::from_static("default-src 'none'; frame-ancestors 'none'"),
    );
    response
}

fn guarded(
    response: Response,
    guard: impl Into<RequestGuard>,
    deadline: Instant,
    principal: Option<Principal>,
) -> Response {
    if principal.is_some()
        && (response.status().is_client_error() || response.status().is_server_error())
    {
        let error = match response.status() {
            StatusCode::UNAUTHORIZED => HostedError::Unauthorized,
            StatusCode::FORBIDDEN => HostedError::Forbidden,
            StatusCode::NOT_FOUND => HostedError::NotFound,
            StatusCode::CONFLICT => HostedError::Conflict,
            StatusCode::PAYLOAD_TOO_LARGE | StatusCode::TOO_MANY_REQUESTS => HostedError::Limit,
            StatusCode::SERVICE_UNAVAILABLE | StatusCode::GATEWAY_TIMEOUT => {
                HostedError::Unavailable
            }
            _ => HostedError::InvalidRequest,
        };
        crate::audit::record(principal.as_ref(), "http_request", Err(error), None, 0);
    }
    let lease = match guard.into() {
        RequestGuard::Permits(permits) => HttpLease::new(permits, deadline),
        RequestGuard::Lease(lease) => lease,
    };
    // Leave a bounded window for a timeout error itself to be transmitted.
    // The request lifetime retains its original execution deadline.
    let response_deadline = deadline.max(Instant::now() + Duration::from_secs(1));
    let (parts, body) = security_headers(response).into_parts();
    Response::from_parts(
        parts,
        Body::new(BoundedBody::new(
            body,
            response_deadline,
            principal,
            None,
            Some(lease),
        )),
    )
}

struct BoundedBody {
    inner: Pin<Box<Body>>,
    deadline: Pin<Box<Sleep>>,
    principal: Option<Principal>,
    remaining: Option<usize>,
    lease: Option<HttpLease>,
    finished: bool,
}

impl BoundedBody {
    fn new(
        inner: Body,
        deadline: Instant,
        principal: Option<Principal>,
        remaining: Option<usize>,
        lease: Option<HttpLease>,
    ) -> Self {
        Self {
            inner: Box::pin(inner),
            deadline: Box::pin(sleep_until(deadline)),
            principal,
            remaining,
            lease,
            finished: false,
        }
    }

    fn fail(
        &mut self,
        kind: io::ErrorKind,
    ) -> Poll<Option<std::result::Result<Frame<Bytes>, axum::Error>>> {
        self.finished = true;
        self.lease.take();
        Poll::Ready(Some(Err(axum::Error::new(io::Error::new(
            kind,
            "stream boundary rejected",
        )))))
    }
}

impl HttpBody for BoundedBody {
    type Data = Bytes;
    type Error = axum::Error;

    fn poll_frame(
        self: Pin<&mut Self>,
        context: &mut Context<'_>,
    ) -> Poll<Option<std::result::Result<Frame<Self::Data>, Self::Error>>> {
        let this = self.get_mut();
        if this.finished {
            return Poll::Ready(None);
        }
        if this.deadline.as_mut().poll(context).is_ready() {
            return this.fail(io::ErrorKind::TimedOut);
        }
        if this
            .principal
            .as_ref()
            .is_some_and(|principal| principal.ensure_live().is_err())
        {
            return this.fail(io::ErrorKind::PermissionDenied);
        }
        match this.inner.as_mut().poll_frame(context) {
            Poll::Ready(Some(Ok(frame))) => {
                if let (Some(remaining), Some(data)) = (&mut this.remaining, frame.data_ref()) {
                    let Some(next) = remaining.checked_sub(data.len()) else {
                        return this.fail(io::ErrorKind::InvalidData);
                    };
                    *remaining = next;
                }
                Poll::Ready(Some(Ok(frame)))
            }
            Poll::Ready(None) => {
                this.finished = true;
                this.lease.take();
                Poll::Ready(None)
            }
            Poll::Ready(Some(Err(error))) => {
                this.finished = true;
                this.lease.take();
                Poll::Ready(Some(Err(error)))
            }
            Poll::Pending => Poll::Pending,
        }
    }

    fn is_end_stream(&self) -> bool {
        self.finished || self.inner.is_end_stream()
    }

    fn size_hint(&self) -> SizeHint {
        self.inner.size_hint()
    }
}

async fn live() -> Json<serde_json::Value> {
    Json(json!({"status":"live"}))
}

async fn ready(State(state): State<HttpState>) -> Response {
    let ready = match state.readiness.state.try_lock() {
        Ok(mut cached) => {
            if cached.0.elapsed() >= READY_CACHE_TTL {
                let healthy = state.auth.ready()
                    && matches!(
                        timeout(READY_PROBE_TIMEOUT, state.storage.ready()).await,
                        Ok(Ok(()))
                    );
                *cached = (Instant::now(), healthy);
                state
                    .readiness
                    .last_result
                    .store(healthy, Ordering::Release);
            }
            cached.1
        }
        Err(_) => state.readiness.last_result.load(Ordering::Acquire),
    };
    if ready {
        Json(json!({"status":"ready"})).into_response()
    } else {
        HostedError::Unavailable.into_response()
    }
}

async fn protected_resource(State(state): State<HttpState>) -> Json<serde_json::Value> {
    Json(json!({
        "resource": format!("{}/mcp",state.config.public_url),
        "authorization_servers": [state.config.auth.issuer],
        "scopes_supported": [READ_SCOPE,CREATE_SCOPE,DOWNLOAD_SCOPE],
        "bearer_methods_supported": ["header"],
        "resource_name": "Infimount hosted storage"
    }))
}

async fn download(
    State(state): State<HttpState>,
    Extension(principal): Extension<Principal>,
    Extension(lifetime): Extension<Arc<RequestLifetime>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    let range = match single_header(&headers, RANGE.as_str()) {
        Ok(range) => range,
        Err(error) => return error.into_response(),
    };
    match lifetime
        .run(state.transfers.download(&principal, &id, range))
        .await
    {
        Ok(response) => response,
        Err(error) => error.into_response(),
    }
}

async fn upload(
    State(state): State<HttpState>,
    Extension(principal): Extension<Principal>,
    Extension(lifetime): Extension<Arc<RequestLifetime>>,
    Path(id): Path<String>,
    body: Body,
) -> Response {
    match lifetime
        .run(state.transfers.upload(&principal, &id, body))
        .await
    {
        Ok(result) => Json(result).into_response(),
        Err(error) => error.into_response(),
    }
}

async fn method_not_allowed() -> StatusCode {
    StatusCode::METHOD_NOT_ALLOWED
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use crate::auth::test_support;
    use crate::config::{Grant, Limits, StorageConfig};
    use futures::stream;
    use infimount_mcp::policy::{McpAccessMode, McpConfirmationRules, McpStoragePolicy};
    use infimount_mcp::registry::{StorageRecord, STORAGE_RECORD_SCHEMA_VERSION};
    use serde_json::Value;
    use std::collections::BTreeMap;
    use tempfile::TempDir;
    use tower::ServiceExt;

    const HOSTNAME: &str = "storage.example.test";
    const SCOPES: &str = "infimount:read infimount:create infimount:download";

    struct Fixture {
        app: Router,
        storage: Arc<StoragePlane>,
        transfers: Arc<TransferManager>,
        _temp: TempDir,
    }

    fn fixture(global_limit: usize, owner_limit: usize) -> Fixture {
        fixture_with_inline(global_limit, owner_limit, 256)
    }

    fn fixture_with_inline(
        global_limit: usize,
        owner_limit: usize,
        inline_bytes: usize,
    ) -> Fixture {
        let temp = tempfile::tempdir().unwrap();
        std::fs::create_dir(temp.path().join("alice")).unwrap();
        std::fs::create_dir(temp.path().join("bob")).unwrap();
        std::fs::write(temp.path().join("alice/private.bin"), [0, 255, 128, 42]).unwrap();
        std::fs::write(temp.path().join("bob/private.bin"), b"bob-only").unwrap();
        let auth = test_support::config();
        let config = Arc::new(HostedConfig {
            public_url: format!("https://{HOSTNAME}"),
            listen: "127.0.0.1:0".parse().unwrap(),
            allow_non_loopback: false,
            auth: auth.clone(),
            storages: vec![StorageConfig {
                record: StorageRecord {
                    schema_version: STORAGE_RECORD_SCHEMA_VERSION,
                    id: "files".into(),
                    name: "files".into(),
                    backend: "local".into(),
                    config: json!({"root":temp.path().to_str().unwrap()}),
                    secret_ref: None,
                    secret_fields: vec![],
                    enabled: true,
                    mcp_exposed: true,
                    read_only: false,
                    mcp_policy: McpStoragePolicy {
                        default_access: McpAccessMode::ReadWrite,
                        confirmation_rules: McpConfirmationRules {
                            require_for_write: false,
                            require_for_presign: false,
                            ..Default::default()
                        },
                        ..Default::default()
                    },
                    revision: 1,
                    created_at: "2026-11-27T00:00:00Z".into(),
                    updated_at: "2026-11-27T00:00:00Z".into(),
                },
                credentials: BTreeMap::new(),
                dedicated_local: true,
            }],
            grants: [("alice", "tenant-a"), ("bob", "tenant-b")]
                .into_iter()
                .map(|(subject, tenant)| Grant {
                    issuer: auth.issuer.clone(),
                    subject: subject.into(),
                    tenant: tenant.into(),
                    client_id: Some("agent".into()),
                    storage_id: "files".into(),
                    prefix: subject.into(),
                    read: true,
                    create: true,
                    download: true,
                })
                .collect(),
            limits: Limits {
                json_bytes: if inline_bytes > 256 {
                    6 * 1024 * 1024
                } else {
                    2048
                },
                inline_bytes,
                transfer_bytes: inline_bytes.max(4096),
                chunk_bytes: 256,
                global_concurrency: global_limit,
                principal_concurrency: owner_limit,
                ..Default::default()
            },
            delivery: Default::default(),
            telemetry: Default::default(),
        });
        config.validate().unwrap();
        let storage = Arc::new(StoragePlane::new(config.clone()).unwrap());
        let transfers = Arc::new(TransferManager::new(storage.clone()));
        let app = build_router(
            config,
            test_support::authenticator(),
            storage.clone(),
            transfers.clone(),
        )
        .unwrap();
        Fixture {
            app,
            storage,
            transfers,
            _temp: temp,
        }
    }

    fn token(subject: &str) -> String {
        let tenant = if subject == "alice" {
            "tenant-a"
        } else {
            "tenant-b"
        };
        test_support::token(subject, tenant, "agent", SCOPES)
    }

    fn request(subject: &str, value: Value) -> Request<Body> {
        Request::builder()
            .method(Method::POST)
            .uri("/mcp")
            .header(HOST, HOSTNAME)
            .header(AUTHORIZATION, format!("Bearer {}", token(subject)))
            .header("content-type", "application/json")
            .header("accept", "application/json, text/event-stream")
            .header("mcp-protocol-version", "2025-06-18")
            .body(Body::from(value.to_string()))
            .unwrap()
    }

    fn call(subject: &str, name: &str, arguments: Value) -> Request<Body> {
        request(
            subject,
            json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":name,"arguments":arguments}}),
        )
    }

    async fn json_response(response: Response) -> Value {
        let bytes = to_bytes(response.into_body(), 64 * 1024).await.unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    #[tokio::test]
    async fn authentication_precedes_body_polling_and_covers_grant_routes() {
        let fixture = fixture(16, 4);
        let unpolled = Body::from_stream(stream::poll_fn(|_| {
            panic!("unauthenticated request body was polled");
            #[allow(unreachable_code)]
            Poll::<Option<std::result::Result<Bytes, io::Error>>>::Ready(None)
        }));
        let request = Request::builder()
            .method(Method::POST)
            .uri("/mcp")
            .header(HOST, HOSTNAME)
            .body(unpolled)
            .unwrap();
        let response = fixture.app.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(response.headers()[WWW_AUTHENTICATE], "Bearer resource_metadata=\"https://storage.example.test/.well-known/oauth-protected-resource/mcp\"");
        assert_eq!(
            json_response(response).await,
            json!({"error":"unauthorized"})
        );
        for (method, path) in [
            (Method::GET, "/downloads/not-an-owned-grant"),
            (Method::PUT, "/uploads/not-an-owned-grant"),
        ] {
            let response = fixture
                .app
                .clone()
                .oneshot(
                    Request::builder()
                        .method(method)
                        .uri(path)
                        .header(HOST, HOSTNAME)
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        }
    }

    #[tokio::test]
    async fn chunked_body_limit_applies_before_the_sdk_parser() {
        let fixture = fixture(16, 4);
        let chunks: Vec<std::result::Result<Bytes, io::Error>> = vec![
            Ok(Bytes::from(vec![b' '; 1024])),
            Ok(Bytes::from(vec![b'x'; 1025])),
        ];
        let mut request = request("alice", json!({}));
        *request.body_mut() = Body::from_stream(stream::iter(chunks));
        assert!(!request.headers().contains_key(CONTENT_LENGTH));
        let response = fixture.app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
        assert_eq!(
            json_response(response).await,
            json!({"error":"limit_exceeded"})
        );
    }

    #[tokio::test]
    async fn origin_host_and_session_headers_cannot_expand_authority() {
        let fixture = fixture(16, 4);
        for (header, value, expected) in [
            ("host", "attacker.example", StatusCode::FORBIDDEN),
            ("host", "storage.example.test:8443", StatusCode::FORBIDDEN),
            ("origin", "https://attacker.example", StatusCode::FORBIDDEN),
            ("origin", "null", StatusCode::FORBIDDEN),
            (
                "mcp-session-id",
                "another-user-session",
                StatusCode::BAD_REQUEST,
            ),
            (
                "last-event-id",
                "another-user-event",
                StatusCode::BAD_REQUEST,
            ),
        ] {
            let mut request = request(
                "alice",
                json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}),
            );
            request.headers_mut().insert(
                axum::http::header::HeaderName::from_bytes(header.as_bytes()).unwrap(),
                HeaderValue::from_str(value).unwrap(),
            );
            assert_eq!(
                fixture.app.clone().oneshot(request).await.unwrap().status(),
                expected
            );
        }
        for path in [
            "/health/live",
            "/health/ready",
            "/.well-known/oauth-protected-resource/mcp",
        ] {
            let request = Request::builder()
                .uri(path)
                .header(HOST, "attacker.example")
                .body(Body::empty())
                .unwrap();
            assert_eq!(
                fixture.app.clone().oneshot(request).await.unwrap().status(),
                StatusCode::FORBIDDEN
            );
        }
        let mut duplicate = request(
            "alice",
            json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}),
        );
        duplicate.headers_mut().append(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {}", token("bob"))).unwrap(),
        );
        assert_eq!(
            fixture.app.oneshot(duplicate).await.unwrap().status(),
            StatusCode::UNAUTHORIZED
        );
    }

    #[tokio::test]
    async fn stateless_dispatch_preserves_exact_request_identity_and_rejects_extra_authority() {
        let fixture = fixture(16, 4);
        let first = fixture
            .app
            .clone()
            .oneshot(call(
                "alice",
                "read_file",
                json!({"path":"/files/alice/private.bin"}),
            ))
            .await
            .unwrap();
        assert!(!first.headers().contains_key("mcp-session-id"));
        let first = json_response(first).await;
        assert_ne!(first["result"]["isError"], true);
        assert_eq!(first["result"]["structuredContent"]["content"], "AP+AKg==");
        let mut forged = call(
            "bob",
            "read_file",
            json!({"path":"/files/alice/private.bin"}),
        );
        forged
            .headers_mut()
            .insert("x-user-id", HeaderValue::from_static("alice"));
        forged
            .headers_mut()
            .insert("x-tenant-id", HeaderValue::from_static("tenant-a"));
        let denied = json_response(fixture.app.clone().oneshot(forged).await.unwrap()).await;
        assert_eq!(denied["result"]["isError"], true);
        assert!(!denied.to_string().contains("AP+AKg=="));
        let own = json_response(
            fixture
                .app
                .clone()
                .oneshot(call(
                    "bob",
                    "read_file",
                    json!({"path":"/files/bob/private.bin"}),
                ))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(
            own["result"]["structuredContent"]["content"],
            "Ym9iLW9ubHk="
        );
        for extra in [
            json!({"overwrite":true}),
            json!({"session_id":"foreign"}),
            json!({"create_parents":true}),
        ] {
            let mut arguments = json!({"path":"/files/alice/must-not-exist","content":"malicious"});
            arguments
                .as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            let denied = json_response(
                fixture
                    .app
                    .clone()
                    .oneshot(call("alice", "write_file", arguments))
                    .await
                    .unwrap(),
            )
            .await;
            assert_eq!(
                denied["result"]["structuredContent"]["error"],
                "invalid_request"
            );
        }
        assert!(!fixture._temp.path().join("alice/must-not-exist").exists());
    }

    #[tokio::test]
    async fn permits_are_held_until_response_body_completion_or_disconnect() {
        let fixture = fixture(2, 1);
        let tools = json!({"jsonrpc":"2.0","id":1,"method":"tools/list"});
        let alice = fixture
            .app
            .clone()
            .oneshot(request("alice", tools.clone()))
            .await
            .unwrap();
        assert_eq!(alice.status(), StatusCode::OK);
        let limited = fixture
            .app
            .clone()
            .oneshot(request("alice", tools.clone()))
            .await
            .unwrap();
        assert_eq!(limited.status(), StatusCode::TOO_MANY_REQUESTS);
        drop(limited);
        let bob = fixture
            .app
            .clone()
            .oneshot(request("bob", tools.clone()))
            .await
            .unwrap();
        assert_eq!(bob.status(), StatusCode::OK);
        let globally_limited = fixture
            .app
            .clone()
            .oneshot(request("charlie", tools.clone()))
            .await
            .unwrap();
        assert_eq!(globally_limited.status(), StatusCode::TOO_MANY_REQUESTS);
        drop(globally_limited);
        drop(alice);
        let resumed = fixture
            .app
            .clone()
            .oneshot(request("alice", tools))
            .await
            .unwrap();
        assert_eq!(resumed.status(), StatusCode::OK);
        drop(resumed);
        drop(bob);
    }

    #[tokio::test]
    async fn bypassing_http_auth_does_not_create_an_ambient_mcp_principal() {
        let fixture = fixture(16, 4);
        let server = HostedMcpServer::new(fixture.storage, fixture.transfers);
        let service: StreamableHttpService<HostedMcpServer, NeverSessionManager> =
            StreamableHttpService::new(
                move || Ok(server.clone()),
                Arc::new(NeverSessionManager::default()),
                StreamableHttpServerConfig::default()
                    .with_stateful_mode(false)
                    .with_json_response(true)
                    .with_allowed_hosts([HOSTNAME]),
            );
        let app = Router::new().route_service("/mcp", service);
        let response = app
            .oneshot(request(
                "alice",
                json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}),
            ))
            .await
            .unwrap();
        let body = json_response(response).await;
        assert_eq!(body["error"]["message"], "authentication required");
        assert!(body.get("result").is_none());
    }

    #[tokio::test]
    async fn real_http_client_receives_scoped_json_tools_and_no_sse_or_session() {
        let fixture = fixture(16, 4);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let app = fixture.app.clone();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap();
        let read_token = test_support::token("alice", "tenant-a", "agent", READ_SCOPE);
        let response = client
            .post(format!("http://{address}/mcp"))
            .header(HOST, HOSTNAME)
            .header("accept", "application/json, text/event-stream")
            .header("mcp-protocol-version", "2025-06-18")
            .bearer_auth(&read_token)
            .json(&json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()["content-type"], "application/json");
        assert!(!response.headers().contains_key("mcp-session-id"));
        let result: Value = response.json().await.unwrap();
        let names: Vec<_> = result["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|tool| tool["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, ["list_dir", "stat_path", "read_file"]);
        let denied = client
            .get(format!("http://{address}/mcp"))
            .header(HOST, HOSTNAME)
            .header("accept", "text/event-stream")
            .bearer_auth(&read_token)
            .send()
            .await
            .unwrap();
        assert_eq!(denied.status(), StatusCode::METHOD_NOT_ALLOWED);
        let metadata: Value = client
            .get(format!(
                "http://{address}/.well-known/oauth-protected-resource/mcp"
            ))
            .header(HOST, HOSTNAME)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(metadata["resource"], "https://storage.example.test/mcp");
        assert_eq!(metadata["bearer_methods_supported"], json!(["header"]));
        server.abort();
    }

    #[tokio::test]
    async fn cancelling_an_http_lease_cancels_work_without_releasing_its_permit_early() {
        let limiter = RequestLimiter::new(1, 1);
        let lease = HttpLease::new(
            limiter.global().unwrap(),
            Instant::now() + Duration::from_secs(60),
        );
        let lifetime = lease.lifetime.clone();
        let running =
            tokio::spawn(async move { lifetime.run(std::future::pending::<Result<()>>()).await });
        assert!(limiter.global().is_err());
        drop(lease);
        let outcome = timeout(Duration::from_secs(1), running)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(outcome, Err(HostedError::Unavailable));
        assert!(limiter.global().is_ok());
    }

    #[tokio::test]
    async fn asynchronous_cleanup_retains_http_capacity_without_ambient_identity() {
        assert!(current_resource_lease().is_none());
        let limiter = RequestLimiter::new(1, 1);
        let lease = HttpLease::new(
            limiter.global().unwrap(),
            Instant::now() + Duration::from_secs(60),
        );
        let lifetime = lease.lifetime.clone();
        let retained = Arc::new(Mutex::new(None));
        let cleanup = retained.clone();
        let (started, ready) = tokio::sync::oneshot::channel();
        let running = tokio::spawn(async move {
            lifetime
                .run(async move {
                    *cleanup.lock().unwrap() = current_resource_lease();
                    started.send(()).unwrap();
                    std::future::pending::<Result<()>>().await
                })
                .await
        });
        ready.await.unwrap();
        drop(lease);
        assert_eq!(
            timeout(Duration::from_secs(1), running)
                .await
                .unwrap()
                .unwrap(),
            Err(HostedError::Unavailable)
        );
        assert!(limiter.global().is_err());
        assert!(current_resource_lease().is_none());
        retained.lock().unwrap().take();
        assert!(limiter.global().is_ok());
    }

    #[test]
    fn cancelled_local_read_metadata_and_listing_keep_http_capacity_until_fs_completion() {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .max_blocking_threads(2)
            .build()
            .unwrap()
            .block_on(async {
                for operation in ["read", "stat", "list"] {
                    let fixture = fixture(1, 1);
                    let principal = test_support::authenticator()
                        .verify(&token("alice"))
                        .await
                        .unwrap();
                    let limiter = RequestLimiter::new(1, 1);
                    let lease = HttpLease::new(
                        limiter.global().unwrap(),
                        Instant::now() + Duration::from_secs(60),
                    );
                    let lifetime = lease.lifetime.clone();
                    let mut releases = Vec::new();
                    let mut workers = Vec::new();
                    for _ in 0..2 {
                        let (release, wait) = std::sync::mpsc::channel();
                        let (started, ready) = tokio::sync::oneshot::channel();
                        workers.push(tokio::task::spawn_blocking(move || {
                            let _ = started.send(());
                            let _ = wait.recv();
                        }));
                        releases.push(release);
                        ready.await.unwrap();
                    }
                    let storage = fixture.storage.clone();
                    let mut pending = Box::pin(async move {
                        lifetime
                            .run(async {
                                match operation {
                                    "read" => {
                                        storage
                                            .read(&principal, "/files/alice/private.bin", 0, 4)
                                            .await
                                    }
                                    "stat" => {
                                        storage.stat(&principal, "/files/alice/private.bin").await
                                    }
                                    _ => storage.list(&principal, "/files/alice", 10, None).await,
                                }
                                .map(|_| ())
                            })
                            .await
                    });
                    assert!(futures::poll!(pending.as_mut()).is_pending());
                    drop(lease);
                    assert_eq!(pending.await, Err(HostedError::Unavailable));
                    let retained_capacity = limiter.global().is_err();
                    assert!(current_resource_lease().is_none());
                    for release in releases {
                        release.send(()).unwrap();
                    }
                    for worker in workers {
                        worker.await.unwrap();
                    }
                    timeout(Duration::from_secs(2), async {
                        while limiter.global().is_err() {
                            tokio::task::yield_now().await;
                        }
                    })
                    .await
                    .unwrap();
                    assert!(
                        retained_capacity,
                        "{operation} released capacity before queued filesystem work finished"
                    );
                }
            });
    }

    #[test]
    fn timed_out_readiness_cannot_accumulate_blocked_filesystem_probes() {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .max_blocking_threads(2)
            .build()
            .unwrap()
            .block_on(async {
                let fixture = fixture(1, 1);
                let mut releases = Vec::new();
                let mut workers = Vec::new();
                for _ in 0..2 {
                    let (release, wait) = std::sync::mpsc::channel();
                    let (started, ready) = tokio::sync::oneshot::channel();
                    workers.push(tokio::task::spawn_blocking(move || {
                        let _ = started.send(());
                        let _ = wait.recv();
                    }));
                    releases.push(release);
                    ready.await.unwrap();
                }
                let mut first = Box::pin(fixture.storage.ready());
                assert!(futures::poll!(first.as_mut()).is_pending());
                drop(first);
                let mut second = Box::pin(fixture.storage.ready());
                let denied_without_new_io = matches!(
                    futures::poll!(second.as_mut()),
                    Poll::Ready(Err(HostedError::Unavailable))
                );
                drop(second);
                for release in releases {
                    release.send(()).unwrap();
                }
                for worker in workers {
                    worker.await.unwrap();
                }
                timeout(Duration::from_secs(2), async {
                    while fixture.storage.ready().await.is_err() {
                        tokio::task::yield_now().await;
                    }
                })
                .await
                .unwrap();
                assert!(denied_without_new_io);
            });
    }

    #[tokio::test]
    async fn maximum_inline_binary_read_has_one_payload_and_bounded_json_response() {
        use base64::Engine;
        let fixture = fixture_with_inline(16, 4, 4 * 1024 * 1024);
        let bytes: Vec<u8> = (0..4 * 1024 * 1024)
            .map(|index| (index % 256) as u8)
            .collect();
        std::fs::write(fixture._temp.path().join("alice/large.bin"), &bytes).unwrap();
        let response = fixture
            .app
            .oneshot(call(
                "alice",
                "read_file",
                json!({"path":"/files/alice/large.bin"}),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), 6 * 1024 * 1024)
            .await
            .unwrap();
        let json: Value = serde_json::from_slice(&body).unwrap();
        let content = json["result"]["structuredContent"]["content"]
            .as_str()
            .unwrap();
        assert_eq!(
            base64::engine::general_purpose::STANDARD
                .decode(content)
                .unwrap(),
            bytes
        );
        assert_eq!(
            std::str::from_utf8(&body).unwrap().matches(content).count(),
            1
        );
        assert!(json["result"]["content"][0]["text"].as_str().unwrap().len() < 512);
    }

    #[tokio::test]
    async fn streamed_upload_and_download_remain_owned_across_all_http_and_mcp_steps() {
        use sha2::{Digest, Sha256};
        let fixture = fixture(16, 4);
        let bytes = vec![0_u8, 255, 128, 42, 7, 0];
        let digest = format!("{:x}", Sha256::digest(&bytes));
        let created = json_response(
            fixture
                .app
                .clone()
                .oneshot(call(
                    "alice",
                    "create_upload",
                    json!({
                        "path":"/files/alice/created.bin","size":bytes.len(),"sha256":digest
                    }),
                ))
                .await
                .unwrap(),
        )
        .await;
        let upload_id = created["result"]["structuredContent"]["upload_id"]
            .as_str()
            .unwrap();
        let upload_request = |subject: &str, body: Body| {
            Request::builder()
                .method(Method::PUT)
                .uri(format!("/uploads/{upload_id}"))
                .header(HOST, HOSTNAME)
                .header(AUTHORIZATION, format!("Bearer {}", token(subject)))
                .body(body)
                .unwrap()
        };
        let denied = fixture
            .app
            .clone()
            .oneshot(upload_request("bob", Body::from(bytes.clone())))
            .await
            .unwrap();
        assert_eq!(denied.status(), StatusCode::NOT_FOUND);
        drop(denied);
        let pieces: Vec<std::result::Result<Bytes, io::Error>> = bytes
            .chunks(2)
            .map(|piece| Ok(Bytes::copy_from_slice(piece)))
            .collect();
        let uploaded = fixture
            .app
            .clone()
            .oneshot(upload_request(
                "alice",
                Body::from_stream(stream::iter(pieces)),
            ))
            .await
            .unwrap();
        assert_eq!(uploaded.status(), StatusCode::OK);
        assert_eq!(json_response(uploaded).await["received_bytes"], bytes.len());
        assert!(!fixture._temp.path().join("alice/created.bin").exists());
        let denied = json_response(
            fixture
                .app
                .clone()
                .oneshot(call("bob", "complete_upload", json!({"id":upload_id})))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(denied["result"]["isError"], true);
        let committed = json_response(
            fixture
                .app
                .clone()
                .oneshot(call("alice", "complete_upload", json!({"id":upload_id})))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(
            committed["result"]["structuredContent"]["state"],
            "committed"
        );
        assert_eq!(
            std::fs::read(fixture._temp.path().join("alice/created.bin")).unwrap(),
            bytes
        );
        let created = json_response(
            fixture
                .app
                .clone()
                .oneshot(call(
                    "alice",
                    "create_download",
                    json!({"path":"/files/alice/created.bin"}),
                ))
                .await
                .unwrap(),
        )
        .await;
        let download_id = created["result"]["structuredContent"]["download_id"]
            .as_str()
            .unwrap();
        let download_request = |subject: &str| {
            Request::builder()
                .uri(format!("/downloads/{download_id}"))
                .header(HOST, HOSTNAME)
                .header(AUTHORIZATION, format!("Bearer {}", token(subject)))
                .body(Body::empty())
                .unwrap()
        };
        let denied = fixture
            .app
            .clone()
            .oneshot(download_request("bob"))
            .await
            .unwrap();
        assert_eq!(denied.status(), StatusCode::NOT_FOUND);
        drop(denied);
        let downloaded = fixture
            .app
            .clone()
            .oneshot(download_request("alice"))
            .await
            .unwrap();
        assert_eq!(downloaded.status(), StatusCode::OK);
        assert_eq!(
            downloaded.headers()["content-type"],
            "application/octet-stream"
        );
        assert_eq!(downloaded.headers()["x-content-type-options"], "nosniff");
        assert_eq!(downloaded.headers()["cache-control"], "private, no-store");
        assert!(downloaded.headers()["content-disposition"]
            .to_str()
            .unwrap()
            .starts_with("attachment;"));
        assert!(!downloaded.headers().contains_key("location"));
        assert_eq!(
            to_bytes(downloaded.into_body(), 4096)
                .await
                .unwrap()
                .as_ref(),
            bytes
        );
        let replay = fixture
            .app
            .oneshot(download_request("alice"))
            .await
            .unwrap();
        assert_eq!(replay.status(), StatusCode::NOT_FOUND);
    }
}
