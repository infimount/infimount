use infimount_core::operations::{transfer_entries, TransferConflictPolicy, TransferOperation};
use opendal::{
    services::{Gdrive, Onedrive},
    Operator,
};
use std::{
    env,
    error::Error,
    io,
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Clone)]
struct OAuthBackend {
    name: &'static str,
    op: Operator,
}

fn env_var(name: &str) -> Option<String> {
    env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn unique_prefix(backend: &str) -> String {
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default();
    format!("infimount-oauth-smoke/{backend}-{ts}/")
}

fn build_gdrive() -> Result<Option<OAuthBackend>, Box<dyn Error>> {
    let mut builder = Gdrive::default();
    if let Some(root) = env_var("INFIMOUNT_GDRIVE_ROOT") {
        builder = builder.root(&root);
    }

    if let Some(access_token) = env_var("INFIMOUNT_GDRIVE_ACCESS_TOKEN") {
        builder = builder.access_token(&access_token);
    } else if let Some(refresh_token) = env_var("INFIMOUNT_GDRIVE_REFRESH_TOKEN") {
        let client_id = env_var("INFIMOUNT_GDRIVE_CLIENT_ID").ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "INFIMOUNT_GDRIVE_CLIENT_ID is required with INFIMOUNT_GDRIVE_REFRESH_TOKEN",
            )
        })?;
        let client_secret = env_var("INFIMOUNT_GDRIVE_CLIENT_SECRET").ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "INFIMOUNT_GDRIVE_CLIENT_SECRET is required with INFIMOUNT_GDRIVE_REFRESH_TOKEN",
            )
        })?;
        builder = builder
            .refresh_token(&refresh_token)
            .client_id(&client_id)
            .client_secret(&client_secret);
    } else {
        return Ok(None);
    }

    Ok(Some(OAuthBackend {
        name: "Google Drive",
        op: Operator::new(builder)?,
    }))
}

fn build_onedrive() -> Result<Option<OAuthBackend>, Box<dyn Error>> {
    let mut builder = Onedrive::default();
    if let Some(root) = env_var("INFIMOUNT_ONEDRIVE_ROOT") {
        builder = builder.root(&root);
    }
    if let Some(access_token) = env_var("INFIMOUNT_ONEDRIVE_ACCESS_TOKEN") {
        builder = builder.access_token(&access_token);
    } else if let Some(refresh_token) = env_var("INFIMOUNT_ONEDRIVE_REFRESH_TOKEN") {
        let client_id = env_var("INFIMOUNT_ONEDRIVE_CLIENT_ID").ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "INFIMOUNT_ONEDRIVE_CLIENT_ID is required with INFIMOUNT_ONEDRIVE_REFRESH_TOKEN",
            )
        })?;
        builder = builder.refresh_token(&refresh_token).client_id(&client_id);
        if let Some(client_secret) = env_var("INFIMOUNT_ONEDRIVE_CLIENT_SECRET") {
            builder = builder.client_secret(&client_secret);
        }
    } else {
        return Ok(None);
    }

    Ok(Some(OAuthBackend {
        name: "Microsoft OneDrive",
        op: Operator::new(builder)?,
    }))
}

async fn verify_round_trip(backend: &OAuthBackend) -> Result<(), Box<dyn Error>> {
    let prefix = unique_prefix(backend.name.replace(' ', "-").to_ascii_lowercase().as_str());
    let nested_dir = format!("{prefix}nested/");
    let file_path = format!("{nested_dir}verify.txt");
    let content = format!("Infimount OAuth smoke for {}", backend.name);

    backend.op.create_dir(&prefix).await.map_err(|e| {
        io::Error::other(format!(
            "{}: create root smoke folder failed: {e}",
            backend.name
        ))
    })?;
    backend.op.create_dir(&nested_dir).await.map_err(|e| {
        io::Error::other(format!(
            "{}: create nested smoke folder failed: {e}",
            backend.name
        ))
    })?;
    backend
        .op
        .write(&file_path, content.clone())
        .await
        .map_err(|e| io::Error::other(format!("{}: write smoke file failed: {e}", backend.name)))?;

    let meta =
        backend.op.stat(&file_path).await.map_err(|e| {
            io::Error::other(format!("{}: stat smoke file failed: {e}", backend.name))
        })?;
    if meta.content_length() != content.len() as u64 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("{}: stat length mismatch", backend.name),
        )
        .into());
    }

    let actual =
        backend.op.read(&file_path).await.map_err(|e| {
            io::Error::other(format!("{}: read smoke file failed: {e}", backend.name))
        })?;
    if String::from_utf8(actual.to_vec())? != content {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("{}: read content mismatch", backend.name),
        )
        .into());
    }

    let listed = backend.op.list(&nested_dir).await.map_err(|e| {
        io::Error::other(format!("{}: list smoke folder failed: {e}", backend.name))
    })?;
    if !listed.iter().any(|entry| entry.path() == file_path) {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("{}: list did not include smoke file", backend.name),
        )
        .into());
    }

    backend.op.delete(&file_path).await.map_err(|e| {
        io::Error::other(format!("{}: delete smoke file failed: {e}", backend.name))
    })?;
    backend.op.delete(&nested_dir).await.ok();
    backend.op.delete(&prefix).await.ok();
    Ok(())
}

async fn verify_transfer(from: &OAuthBackend, to: &OAuthBackend) -> Result<(), Box<dyn Error>> {
    let source_prefix = unique_prefix("oauth-transfer-source");
    let target_prefix = unique_prefix("oauth-transfer-target");
    let source_path = format!("{source_prefix}transfer.txt");
    let destination_path = format!("{target_prefix}{source_prefix}transfer.txt");
    let content = format!("Infimount OAuth transfer {} to {}", from.name, to.name);

    from.op.create_dir(&source_prefix).await?;
    from.op
        .write(&source_path, content.clone())
        .await
        .map_err(|e| io::Error::other(format!("OAuth transfer source write failed: {e}")))?;
    to.op.create_dir(&target_prefix).await?;

    transfer_entries(
        &from.op,
        &to.op,
        vec![source_prefix.clone()],
        &target_prefix,
        TransferOperation::Copy,
        false,
        TransferConflictPolicy::Overwrite,
    )
    .await
    .map_err(|e| {
        io::Error::other(format!(
            "OAuth transfer {} -> {} failed: {e}",
            from.name, to.name
        ))
    })?;

    let actual =
        to.op.read(&destination_path).await.map_err(|e| {
            io::Error::other(format!("OAuth transfer destination read failed: {e}"))
        })?;
    if String::from_utf8(actual.to_vec())? != content {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "OAuth transfer content mismatch",
        )
        .into());
    }

    from.op.delete(&source_path).await.ok();
    from.op.delete(&source_prefix).await.ok();
    to.op.delete(&destination_path).await.ok();
    to.op
        .delete(&format!("{target_prefix}{source_prefix}"))
        .await
        .ok();
    to.op.delete(&target_prefix).await.ok();
    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    println!("Starting optional OAuth storage verification...");

    let mut backends = Vec::new();
    if let Some(gdrive) = build_gdrive()? {
        backends.push(gdrive);
    }
    if let Some(onedrive) = build_onedrive()? {
        backends.push(onedrive);
    }

    if backends.is_empty() {
        println!("No OAuth storage credentials configured; skipping optional OAuth smoke.");
        println!(
            "Set INFIMOUNT_GDRIVE_* and/or INFIMOUNT_ONEDRIVE_* environment variables to run it."
        );
        return Ok(());
    }

    for backend in &backends {
        verify_round_trip(backend).await?;
        println!(
            "✅ {}: OAuth-backed read/write/list/stat/delete smoke successful",
            backend.name
        );
    }

    if backends.len() >= 2 {
        verify_transfer(&backends[0], &backends[1]).await?;
        println!("✅ OAuth drives: cross-backend recursive transfer smoke successful");
    }

    println!("OAuth storage verification passed.");
    Ok(())
}
