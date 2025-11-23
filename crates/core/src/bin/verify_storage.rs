use opendal::{services::{Azblob, Gcs, S3, Webdav}, Operator};
use std::error::Error;

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
    match op_gcs.list("/").await {
        Ok(_) => println!("✅ GCS: Connection successful (Bucket 'test-bucket' found)"),
        Err(e) => println!("❌ GCS: Failed - {}", e),
    }

    // 2. Verify S3
    println!("\n--- Verifying S3 ---");
    let s3 = S3::default()
        .bucket("test-bucket")
        .endpoint("http://localhost:8333")
        .region("us-east-1")
        .access_key_id("admin")
        .secret_access_key("password123");
    
    let op_s3 = Operator::new(s3)?.finish();
    
    match op_s3.write("test_file.txt", "Hello S3").await {
        Ok(_) => println!("✅ S3: Write successful (Bucket 'test-bucket' accessible)"),
        Err(e) => println!("❌ S3: Write Failed - {}", e),
    }
    
    // 3. Verify Azure
    println!("\n--- Verifying Azure ---");
    let az = Azblob::default()
        .account_name("devstoreaccount1")
        .container("test-container")
        .endpoint("http://127.0.0.1:10000/devstoreaccount1")
        .account_key("Eby8vdM02xNOcqFlqUwJPLlmEtlCDXJ1OUzFT50uSRZ6IFsuFq2UVErCz4I6tq/K1SZFPTOtr/KBHBeksoGMGw==");
    
    let op_az = Operator::new(az)?.finish();
    
    match op_az.write("test_file.txt", "Hello Azure").await {
        Ok(_) => println!("✅ Azure: Write successful"),
        Err(e) => {
            println!("⚠️ Azure: Write failed - {}", e);
            println!("Note: You may need to create the container 'test-container' manually if Azurite doesn't auto-create it.");
        }
    }

    // 4. Verify WebDAV
    println!("\n--- Verifying WebDAV ---");
    let webdav = Webdav::default()
        .endpoint("http://localhost:7333")
        .root("/");
    
    let op_webdav = Operator::new(webdav)?.finish();
    match op_webdav.list("/").await {
        Ok(_) => println!("✅ WebDAV: Connection successful"),
        Err(e) => println!("❌ WebDAV: Failed - {}", e),
    }

    Ok(())
}
