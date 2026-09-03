use infimount_core::operations::{transfer_entries, TransferConflictPolicy, TransferOperation};
#[cfg(not(windows))]
use opendal::services::Sftp;
use opendal::{
    services::{Azblob, Gcs, Webdav, S3},
    Operator,
};
use std::{error::Error, io};

async fn verify_list(op: &Operator, backend: &str, path: &str) -> Result<(), Box<dyn Error>> {
    op.list(path)
        .await
        .map_err(|e| io::Error::other(format!("{backend}: list failed for {path}: {e}")))?;

    Ok(())
}

async fn verify_transfer(
    from_op: &Operator,
    to_op: &Operator,
    source_backend: &str,
    destination_backend: &str,
) -> Result<(), Box<dyn Error>> {
    let source_path = "transfer-source.txt";
    let destination_path = "sim-transfer/transfer-source.txt";
    let content = format!("Transfer {source_backend} to {destination_backend}");

    from_op
        .write(source_path, content.clone())
        .await
        .map_err(|e| {
            io::Error::other(format!(
                "{source_backend}: transfer fixture write failed for {source_path}: {e}"
            ))
        })?;

    to_op.create_dir("sim-transfer/").await.map_err(|e| {
        io::Error::other(format!(
            "{destination_backend}: transfer target dir create failed: {e}"
        ))
    })?;

    transfer_entries(
        from_op,
        to_op,
        vec![source_path.to_string()],
        "sim-transfer",
        TransferOperation::Copy,
        false,
        TransferConflictPolicy::Overwrite,
    )
    .await
    .map_err(|e| {
        io::Error::other(format!(
            "transfer {source_backend} -> {destination_backend} failed: {e}"
        ))
    })?;

    assert_transferred_content(
        to_op,
        destination_backend,
        destination_path,
        &content,
        &format!("transfer {source_backend} -> {destination_backend}"),
    )
    .await?;

    from_op.delete(source_path).await.ok();
    to_op.delete(destination_path).await.ok();
    Ok(())
}

async fn verify_recursive_transfer(
    from_op: &Operator,
    to_op: &Operator,
    source_backend: &str,
    destination_backend: &str,
    operation: TransferOperation,
) -> Result<(), Box<dyn Error>> {
    let source_root = match operation {
        TransferOperation::Copy => "recursive-copy-source/",
        TransferOperation::Move => "recursive-move-source/",
    };
    let nested_dir = format!("{source_root}nested/");
    let first_source_path = format!("{source_root}root.txt");
    let second_source_path = format!("{nested_dir}child.txt");
    let first_destination_path = format!("sim-transfer/{source_root}root.txt");
    let second_destination_path = format!("sim-transfer/{source_root}nested/child.txt");
    let first_content = format!(
        "Recursive {:?} root {source_backend} to {destination_backend}",
        operation
    );
    let second_content = format!(
        "Recursive {:?} child {source_backend} to {destination_backend}",
        operation
    );

    from_op.create_dir(source_root).await.map_err(|e| {
        io::Error::other(format!(
            "{source_backend}: recursive fixture root create failed: {e}"
        ))
    })?;
    from_op.create_dir(&nested_dir).await.map_err(|e| {
        io::Error::other(format!(
            "{source_backend}: recursive fixture nested dir create failed: {e}"
        ))
    })?;
    from_op
        .write(&first_source_path, first_content.clone())
        .await
        .map_err(|e| {
            io::Error::other(format!(
                "{source_backend}: recursive fixture write failed for {first_source_path}: {e}"
            ))
        })?;
    from_op
        .write(&second_source_path, second_content.clone())
        .await
        .map_err(|e| {
            io::Error::other(format!(
                "{source_backend}: recursive fixture write failed for {second_source_path}: {e}"
            ))
        })?;
    to_op.create_dir("sim-transfer/").await.map_err(|e| {
        io::Error::other(format!(
            "{destination_backend}: recursive transfer target dir create failed: {e}"
        ))
    })?;

    transfer_entries(
        from_op,
        to_op,
        vec![source_root.to_string()],
        "sim-transfer",
        operation,
        false,
        TransferConflictPolicy::Overwrite,
    )
    .await
    .map_err(|e| {
        io::Error::other(format!(
            "recursive {operation:?} {source_backend} -> {destination_backend} failed: {e}"
        ))
    })?;

    assert_transferred_content(
        to_op,
        destination_backend,
        &first_destination_path,
        &first_content,
        &format!("recursive {operation:?} {source_backend} -> {destination_backend}"),
    )
    .await?;
    assert_transferred_content(
        to_op,
        destination_backend,
        &second_destination_path,
        &second_content,
        &format!("recursive {operation:?} {source_backend} -> {destination_backend}"),
    )
    .await?;

    if operation == TransferOperation::Move
        && from_op.exists(&first_source_path).await.map_err(|e| {
            io::Error::other(format!(
                "{source_backend}: moved source existence check failed for {first_source_path}: {e}"
            ))
        })?
    {
        return Err(io::Error::other(format!(
            "recursive move {source_backend} -> {destination_backend} left source file behind"
        ))
        .into());
    }

    from_op.delete(&first_source_path).await.ok();
    from_op.delete(&second_source_path).await.ok();
    from_op.delete(&nested_dir).await.ok();
    from_op.delete(source_root).await.ok();
    to_op.delete(&first_destination_path).await.ok();
    to_op.delete(&second_destination_path).await.ok();
    to_op
        .delete(&format!("sim-transfer/{source_root}nested/"))
        .await
        .ok();
    to_op
        .delete(&format!("sim-transfer/{source_root}"))
        .await
        .ok();
    Ok(())
}

async fn assert_transferred_content(
    op: &Operator,
    backend: &str,
    path: &str,
    expected: &str,
    label: &str,
) -> Result<(), Box<dyn Error>> {
    let actual = op.read(path).await.map_err(|e| {
        io::Error::other(format!(
            "{backend}: transferred file read failed for {path}: {e}"
        ))
    })?;
    if String::from_utf8(actual.to_vec())? != expected {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("{label} content mismatch"),
        )
        .into());
    }

    Ok(())
}

async fn verify_round_trip(op: &Operator, backend: &str, path: &str) -> Result<(), Box<dyn Error>> {
    let content = format!("Hello {backend}");

    op.write(path, content.clone())
        .await
        .map_err(|e| io::Error::other(format!("{backend}: write failed for {path}: {e}")))?;

    let metadata = op
        .stat(path)
        .await
        .map_err(|e| io::Error::other(format!("{backend}: stat failed for {path}: {e}")))?;
    if metadata.content_length() != content.len() as u64 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "{backend}: stat size mismatch for {path}: expected {}, got {}",
                content.len(),
                metadata.content_length()
            ),
        )
        .into());
    }

    let data = op
        .read(path)
        .await
        .map_err(|e| io::Error::other(format!("{backend}: read failed for {path}: {e}")))?;

    let actual = String::from_utf8(data.to_vec()).map_err(|e| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("{backend}: read returned non-UTF-8 data for {path}: {e}"),
        )
    })?;

    if actual != content {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("{backend}: round-trip mismatch for {path}"),
        )
        .into());
    }

    let listed = op
        .list("")
        .await
        .map_err(|e| io::Error::other(format!("{backend}: list failed for root: {e}")))?;
    if !listed
        .iter()
        .any(|entry| entry.path().trim_start_matches('/') == path)
    {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("{backend}: list did not include {path}"),
        )
        .into());
    }

    op.delete(path)
        .await
        .map_err(|e| io::Error::other(format!("{backend}: delete failed for {path}: {e}")))?;

    if op
        .exists(path)
        .await
        .map_err(|e| io::Error::other(format!("{backend}: exists failed for {path}: {e}")))?
    {
        return Err(
            io::Error::other(format!("{backend}: {path} still exists after delete")).into(),
        );
    }

    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    println!("Starting storage verification...");

    // 1. Verify GCS
    println!("\n--- Verifying GCS ---");
    let gcs = Gcs::default()
        .bucket("test-bucket")
        .endpoint("http://localhost:4443")
        .skip_signature()
        .disable_vm_metadata()
        .disable_config_load();

    let op_gcs = Operator::new(gcs)?;
    verify_round_trip(&op_gcs, "GCS", "verify-gcs.txt").await?;
    println!("✅ GCS: read/write/list/stat/delete round-trip successful");

    // 2. Verify S3
    println!("\n--- Verifying S3 ---");
    let s3 = S3::default()
        .bucket("test-bucket")
        .endpoint("http://localhost:8333")
        .region("us-east-1")
        .access_key_id("admin")
        .secret_access_key("password123");

    let op_s3 = Operator::new(s3)?;
    verify_round_trip(&op_s3, "S3", "verify-s3.txt").await?;
    println!("✅ S3: read/write/list/stat/delete round-trip successful");

    // 3. Verify SFTP
    #[cfg(not(windows))]
    {
        println!("\n--- Verifying SFTP ---");
        let sftp_key = "storage-simulator/runtime/sftp/id_ed25519";
        let sftp = Sftp::default()
            .endpoint("ssh://127.0.0.1:2222")
            .user("simuser")
            .key(sftp_key)
            .root("/upload")
            .known_hosts_strategy("Accept");

        let op_sftp = Operator::new(sftp)?;
        verify_round_trip(&op_sftp, "SFTP", "verify-sftp.txt").await?;
        println!("✅ SFTP: read/write/list/stat/delete round-trip successful");
    }

    // 5. Verify Azure
    println!("\n--- Verifying Azure ---");
    let az = Azblob::default()
        .account_name("devstoreaccount1")
        .container("test-container")
        .endpoint("http://127.0.0.1:10000/devstoreaccount1")
        .account_key("Eby8vdM02xNOcqFlqUwJPLlmEtlCDXJ1OUzFT50uSRZ6IFsuFq2UVErCz4I6tq/K1SZFPTOtr/KBHBeksoGMGw==");

    let op_az = Operator::new(az)?;
    verify_round_trip(&op_az, "Azure", "verify-azure.txt").await?;
    println!("✅ Azure: read/write/list/stat/delete round-trip successful");

    // 6. Verify WebDAV
    println!("\n--- Verifying WebDAV ---");
    let webdav = Webdav::default()
        .endpoint("http://localhost:7333")
        .root("/");

    let op_webdav = Operator::new(webdav)?;
    verify_list(&op_webdav, "WebDAV", "/").await?;
    println!("✅ WebDAV: list successful");

    // 7. Verify simulator-backed transfers through Infimount core operations.
    println!("\n--- Verifying simulator-backed transfers ---");
    verify_transfer(&op_s3, &op_gcs, "S3", "GCS").await?;
    verify_transfer(&op_gcs, &op_az, "GCS", "Azure").await?;
    verify_transfer(&op_az, &op_s3, "Azure", "S3").await?;
    verify_recursive_transfer(&op_s3, &op_gcs, "S3", "GCS", TransferOperation::Copy).await?;
    verify_recursive_transfer(&op_gcs, &op_az, "GCS", "Azure", TransferOperation::Copy).await?;
    println!("✅ S3/GCS/Azure: core cross-backend file and recursive copy transfers successful");

    println!("\nStorage verification passed.");

    Ok(())
}
