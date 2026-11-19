//! Private Linux-only authenticated OpenAI Agents API HTTP transport.
//!
//! No desktop command, public constructor, provider key lookup, consent issuer,
//! dynamic origin or live run is wired to this module. A future trusted caller
//! must obtain independent one-shot export/spend consent first.
//! No HTTP requests are made by its unit tests.
use std::io::Read;
use std::time::{Duration, Instant};

use infimount_core::CoreError;
use reqwest::blocking::{Client, RequestBuilder, Response};
use reqwest::header::{ACCEPT, CONTENT_TYPE};
use reqwest::{Method, StatusCode};
use zeroize::Zeroizing;

use crate::hosted_execution_coordinator::{HostedRead, HostedTransport, Receive};

const SESSIONS: &str = "https://api.openai.com/v1/agents/sessions";
const MAX_JSON: usize = 2 * 1024 * 1024;
const MAX_DOWNLOAD: usize = 8 * 1024 * 1024;
const MAX_SSE: usize = 8 * 1024 * 1024;
const MAX_SSE_FRAME: usize = 2 * 1024 * 1024;
const MAX_SSE_FRAMES: usize = 10_000;
const MAX_REQUEST: usize = 12 * 1024 * 1024;
const CHUNK: usize = 16 * 1024;

fn fail() -> CoreError {
    // Never include raw HTTP errors or response bodies in user-facing errors.
    CoreError::Config("ERR_HOSTED_HTTP_TRANSPORT".into())
}

fn deadline_remaining(deadline: Instant) -> Result<Duration, CoreError> {
    let left = deadline.saturating_duration_since(Instant::now());
    if left.is_zero() {
        return Err(fail());
    }
    Ok(left)
}

fn id_valid(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

fn checked_id(value: &str) -> Result<&str, CoreError> {
    if id_valid(value) {
        Ok(value)
    } else {
        Err(fail())
    }
}

fn checked_cursor(value: &str) -> Result<&str, CoreError> {
    if !value.is_empty() && value.len() <= 1024 && value.bytes().all(|b| b.is_ascii_graphic()) {
        Ok(value)
    } else {
        Err(fail())
    }
}

struct SseDecoder {
    pending: Zeroizing<Vec<u8>>,
    event_data: Zeroizing<Vec<u8>>,
    used_bytes: usize,
    frame_count: usize,
    finished: bool,
}

impl SseDecoder {
    fn new() -> Self {
        Self {
            pending: Zeroizing::new(Vec::new()),
            event_data: Zeroizing::new(Vec::new()),
            used_bytes: 0,
            frame_count: 0,
            finished: false,
        }
    }

    fn feed(
        &mut self,
        bytes: &[u8],
        callback: &mut dyn FnMut(&[u8]) -> Result<(), CoreError>,
    ) -> Result<(), CoreError> {
        self.used_bytes = self.used_bytes.checked_add(bytes.len()).ok_or_else(fail)?;
        if self.used_bytes > MAX_SSE || self.finished {
            return Err(fail());
        }
        for &byte in bytes {
            if byte == b'\n' {
                // A carriage return is part of the line terminator, not data.
                if self.pending.last() == Some(&b'\r') {
                    self.pending.pop();
                }
                let line = std::mem::take(&mut *self.pending);
                self.process_line(&line, callback)?;
                // The line can contain sensitive provider data.
                let _line = Zeroizing::new(line);
            } else {
                if self.pending.len() >= MAX_SSE_FRAME {
                    return Err(fail());
                }
                self.pending.push(byte);
            }
        }
        Ok(())
    }

    fn process_line(
        &mut self,
        line: &[u8],
        callback: &mut dyn FnMut(&[u8]) -> Result<(), CoreError>,
    ) -> Result<(), CoreError> {
        if line.is_empty() {
            if self.event_data.is_empty() {
                return Ok(());
            }
            // Multiline SSE data joins with newlines, per the SSE framing rule.
            self.event_data.pop();
            if self.event_data.as_slice() == b"[DONE]" {
                self.finished = true;
            } else {
                self.frame_count += 1;
                if self.frame_count > MAX_SSE_FRAMES || self.event_data.len() > MAX_SSE_FRAME {
                    return Err(fail());
                }
                // Caller persists first session ID synchronously before
                // the next event or network chunk can be processed.
                callback(&self.event_data)?;
            }
            self.event_data.clear();
            return Ok(());
        }
        if self.finished {
            return Err(fail());
        }
        if let Some(value) = line.strip_prefix(b"data:") {
            let value = value.strip_prefix(b" ").unwrap_or(value);
            if value.len().saturating_add(self.event_data.len()) >= MAX_SSE_FRAME {
                return Err(fail());
            }
            self.event_data.extend_from_slice(value);
            self.event_data.push(b'\n');
        } else if line.starts_with(b":")
            || line.starts_with(b"event:")
            || line.starts_with(b"id:")
            || line.starts_with(b"retry:")
        {
            // Metadata never grants completion or approval.
        } else {
            return Err(fail());
        }
        Ok(())
    }

    fn finish(&self) -> Result<(), CoreError> {
        // Never turn an interrupted/partial event into success. The coordinator
        // independently requires a completed root turn and saved-item audit.
        if !self.pending.is_empty() || !self.event_data.is_empty() || self.frame_count == 0 {
            return Err(fail());
        }
        Ok(())
    }
}

/// Native-only. It is deliberately not `pub`, Serialize, Clone or Debug.
/// The API key is kept out of request arguments, logs and frontend results.
pub(crate) struct OpenAiAgentsHttpTransport {
    client: Client,
    api_key: Zeroizing<String>,
}

impl OpenAiAgentsHttpTransport {
    pub(crate) fn new(api_key: String) -> Result<Self, CoreError> {
        if api_key.len() < 16
            || api_key.len() > 1024
            || !api_key.bytes().all(|b| (0x21..=0x7e).contains(&b))
        {
            return Err(fail());
        }
        // Disable redirects, OS environment proxies and automatic retries.
        // A re-sent create POST may create an additional billable session.
        let client = Client::builder()
            .https_only(true)
            // A caller must hash the exact downloaded bytes. Do not silently
            // decode compressed bodies even if features are unified elsewhere.
            .no_gzip()
            .no_brotli()
            .no_deflate()
            .no_zstd()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(120))
            .build()
            .map_err(|_| fail())?;
        Ok(Self {
            client,
            api_key: Zeroizing::new(api_key),
        })
    }

    fn request(
        &self,
        method: Method,
        path: &str,
        deadline: Instant,
    ) -> Result<RequestBuilder, CoreError> {
        let timeout = deadline_remaining(deadline)?;
        // The caller only supplies prevalidated path components, never a URL,
        // scheme, authority, port, host or query-string override.
        Ok(self
            .client
            .request(method, format!("{SESSIONS}{path}"))
            .header("OpenAI-Beta", "agents=v1")
            .header("Accept-Encoding", "identity")
            .bearer_auth(self.api_key.as_str())
            .timeout(timeout))
    }

    fn execute(&self, request: RequestBuilder, deadline: Instant) -> Result<Response, CoreError> {
        deadline_remaining(deadline)?;
        let response = request.send().map_err(|_| fail())?;
        if !response.status().is_success() {
            return Err(fail());
        }
        Ok(response)
    }

    fn stream_body(
        &self,
        mut response: Response,
        limit: usize,
        deadline: Instant,
        receive: &mut Receive<'_>,
    ) -> Result<(), CoreError> {
        if response.content_length().is_some_and(|n| n > limit as u64) {
            return Err(fail());
        }
        let mut total = 0usize;
        let mut buf = Zeroizing::new([0u8; CHUNK]);
        loop {
            deadline_remaining(deadline)?;
            let n = response.read(&mut *buf).map_err(|_| fail())?;
            if n == 0 {
                return Ok(());
            }
            total = total.checked_add(n).ok_or_else(fail)?;
            if total > limit {
                return Err(fail());
            }
            receive(&buf[..n])?;
            buf[..n].fill(0);
        }
    }

    fn json(
        &self,
        request: RequestBuilder,
        deadline: Instant,
        cap: usize,
        receive: &mut Receive<'_>,
    ) -> Result<(), CoreError> {
        let response = self.execute(request.header(ACCEPT, "application/json"), deadline)?;
        if response.status() == StatusCode::NO_CONTENT {
            return Err(fail());
        }
        self.stream_body(response, cap, deadline, receive)
    }
}

impl HostedTransport for OpenAiAgentsHttpTransport {
    fn create_once(
        &mut self,
        request: &[u8],
        deadline: Instant,
        event: &mut Receive<'_>,
    ) -> Result<(), CoreError> {
        if request.is_empty() || request.len() > MAX_REQUEST {
            return Err(fail());
        }
        let response = self.execute(
            self.request(Method::POST, "", deadline)?
                .header(CONTENT_TYPE, "application/json")
                .header(ACCEPT, "text/event-stream")
                .body(request.to_vec()),
            deadline,
        )?;
        // Reject JSON/error HTML and incorrectly negotiated content types
        // before parsing any body that may contain prompt or account details.
        let content_type = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        if !content_type.starts_with("text/event-stream") {
            return Err(fail());
        }
        let mut frames = SseDecoder::new();
        self.stream_body(response, MAX_SSE, deadline, &mut |chunk| {
            frames.feed(chunk, event)
        })?;
        frames.finish()
    }

    fn read(
        &mut self,
        resource: HostedRead<'_>,
        deadline: Instant,
        receive: &mut Receive<'_>,
    ) -> Result<(), CoreError> {
        match resource {
            HostedRead::Session { session_id } => self.json(
                self.request(
                    Method::GET,
                    &format!("/{}", checked_id(session_id)?),
                    deadline,
                )?,
                deadline,
                MAX_JSON,
                receive,
            ),
            HostedRead::SavedItems {
                session_id,
                turn_id,
                after,
            } => {
                checked_id(turn_id)?;
                let mut request = self
                    .request(
                        Method::GET,
                        &format!("/{}/items", checked_id(session_id)?),
                        deadline,
                    )?
                    .query(&[("order", "asc"), ("limit", "100")]);
                if let Some(cursor) = after {
                    request = request.query(&[("after", checked_cursor(cursor)?)]);
                }
                // API returns all session items, while the audit independently
                // requires the exact root turn for each saved item.
                self.json(request, deadline, MAX_JSON, receive)
            }
            HostedRead::Artifacts { session_id, after } => {
                let mut request = self
                    .request(
                        Method::GET,
                        &format!("/{}/artifacts", checked_id(session_id)?),
                        deadline,
                    )?
                    .query(&[("order", "asc"), ("limit", "100")]);
                if let Some(cursor) = after {
                    request = request.query(&[("after", checked_cursor(cursor)?)]);
                }
                self.json(request, deadline, MAX_JSON, receive)
            }
            HostedRead::ArtifactBytes {
                session_id,
                artifact_id,
            } => {
                let path = format!(
                    "/{}/artifacts/{}/content",
                    checked_id(session_id)?,
                    checked_id(artifact_id)?
                );
                let request = self
                    .request(Method::GET, &path, deadline)?
                    .header(ACCEPT, "application/octet-stream");
                let response = self.execute(request, deadline)?;
                self.stream_body(response, MAX_DOWNLOAD, deadline, receive)
            }
        }
    }

    fn delete_session(
        &mut self,
        session_id: &str,
        deadline: Instant,
        receive: &mut Receive<'_>,
    ) -> Result<(), CoreError> {
        let request = self.request(
            Method::DELETE,
            &format!("/{}", checked_id(session_id)?),
            deadline,
        )?;
        self.json(request, deadline, MAX_JSON, receive)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn collect(input: &[&[u8]]) -> Result<Vec<Vec<u8>>, CoreError> {
        let mut frames = SseDecoder::new();
        let mut result = Vec::new();
        for bytes in input {
            frames.feed(bytes, &mut |event| {
                result.push(event.to_vec());
                Ok(())
            })?;
        }
        frames.finish()?;
        Ok(result)
    }

    #[test]
    fn fragmented_crlf_multiline_and_done_frames() {
        let frames = collect(&[
            b"event: message\r\ndata: {\"type\":\"agent.session.created\",\r\n",
            b"data: \"session\":{\"id\":\"sess_123\"}}\r\n\r\n",
            b"data: {\"type\":\"agent.session.turn.completed\",\"turn\":{\"subagent_id\":null}}\n\n",
            b"data: [DONE]\n\n",
        ]).unwrap();
        assert_eq!(frames.len(), 2);
        assert!(serde_json::from_slice::<serde_json::Value>(&frames[0]).is_ok());
        assert!(serde_json::from_slice::<serde_json::Value>(&frames[1]).is_ok());
    }

    #[test]
    fn no_false_success_on_partial_frames_or_done_only() {
        assert!(collect(&[b"data: {\"type\":\"agent.session.created\"}"]).is_err());
        assert!(collect(&[b"data: [DONE]\n\n"]).is_err());
        assert!(collect(&[b"data: {}\n\ndata: {\n"]).is_err());
        assert!(collect(&[b"data: {}\n\ndata: [DONE]\n\ndata: {}\n\n"]).is_err());
    }

    #[test]
    fn callback_error_is_not_ignored() {
        let mut frames = SseDecoder::new();
        let error = frames.feed(b"data: {}\n\n", &mut |_| Err(fail()));
        assert!(error.is_err());
    }

    #[test]
    fn request_builder_keeps_fixed_tls_host_beta_header_and_encoded_cursors() {
        let provider = OpenAiAgentsHttpTransport::new("sk-test-offline-key-valid-12345".into()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        let built = provider
            .request(Method::GET, "/sess_123/items", deadline).unwrap()
            .query(&[("after", "abc&host=attacker.test#evil"), ("limit", "100")])
            .build().unwrap();
        assert_eq!(built.url().scheme(), "https");
        assert_eq!(built.url().host_str(), Some("api.openai.com"));
        assert_eq!(built.url().path(), "/v1/agents/sessions/sess_123/items");
        assert!(built.url().query().unwrap().contains("%26host%3Dattacker.test%23evil"));
        assert_eq!(built.headers().get("OpenAI-Beta").unwrap(), "agents=v1");
        assert_eq!(built.headers().get("Accept-Encoding").unwrap(), "identity");
    }

    #[test]
    fn reject_path_tokens_and_overlong_streams_without_network() {
        for id in ["../x", "bad/id", "x?admin=1", "x#fragment", "", "a.b"] {
            assert!(!id_valid(id));
        }
        assert!(checked_cursor("cursor?x=abc&y=%2F").is_ok());
        assert!(checked_cursor("cursor\nsecret").is_err());
        let mut frames = SseDecoder::new();
        assert!(frames
            .feed(&vec![b'x'; MAX_SSE_FRAME + 1], &mut |_| Ok(()))
            .is_err());
        assert!(OpenAiAgentsHttpTransport::new("bad key".into()).is_err());
    }
}
