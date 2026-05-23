use opendal::{
    services::{Azblob, Gcs, Webdav, S3},
    Operator,
};
use std::{error::Error, io};

async fn verify_round_trip(op: &Operator, backend: &str, path: &str) -> Result<(), Box<dyn Error>> {
    let content = format!("Hello {backend}");

    op.write(path, content.clone())
        .await
        .map_err(|e| io::Error::other(format!("{backend}: write failed for {path}: {e}")))?;

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

    Ok(())
}

async fn verify_list(op: &Operator, backend: &str, path: &str) -> Result<(), Box<dyn Error>> {
    op.list(path)
        .await
        .map_err(|e| io::Error::other(format!("{backend}: list failed for {path}: {e}")))?;

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
        .allow_anonymous()
        .disable_vm_metadata()
        .disable_config_load();

    let op_gcs = Operator::new(gcs)?.finish();
    verify_round_trip(&op_gcs, "GCS", "verify-gcs.txt").await?;
    println!("✅ GCS: read/write round-trip successful");

    // 2. Verify S3
    println!("\n--- Verifying S3 ---");
    let s3 = S3::default()
        .bucket("test-bucket")
        .endpoint("http://localhost:8333")
        .region("us-east-1")
        .access_key_id("admin")
        .secret_access_key("password123");

    let op_s3 = Operator::new(s3)?.finish();
    verify_round_trip(&op_s3, "S3", "verify-s3.txt").await?;
    println!("✅ S3: read/write round-trip successful");

    // 3. Verify Azure
    println!("\n--- Verifying Azure ---");
    let az = Azblob::default()
        .account_name("devstoreaccount1")
        .container("test-container")
        .endpoint("http://127.0.0.1:10000/devstoreaccount1")
        .account_key("Eby8vdM02xNOcqFlqUwJPLlmEtlCDXJ1OUzFT50uSRZ6IFsuFq2UVErCz4I6tq/K1SZFPTOtr/KBHBeksoGMGw==");

    let op_az = Operator::new(az)?.finish();
    verify_round_trip(&op_az, "Azure", "verify-azure.txt").await?;
    println!("✅ Azure: read/write round-trip successful");

    // 4. Verify WebDAV
    println!("\n--- Verifying WebDAV ---");
    let webdav = Webdav::default()
        .endpoint("http://localhost:7333")
        .root("/");

    let op_webdav = Operator::new(webdav)?.finish();
    verify_list(&op_webdav, "WebDAV", "/").await?;
    println!("✅ WebDAV: list successful");

    println!("\nStorage verification passed.");

    Ok(())
}
