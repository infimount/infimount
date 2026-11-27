#![cfg(target_os = "linux")]
//! Requires the disposable loopback Moto server started by the qualification script.
use std::collections::BTreeSet;
use std::sync::Arc;

use axum::body::{to_bytes, Body, Bytes};
use base64::{engine::general_purpose::STANDARD, Engine};
use infimount_core::{Source, SourceKind};
use infimount_hosted::auth::Principal;
use infimount_hosted::config::{HostedConfig, SecretRef};
use infimount_hosted::error::HostedError;
use infimount_hosted::storage::StoragePlane;
use infimount_hosted::transfers::TransferManager;
use serde_json::json;
use sha2::{Digest, Sha256};

fn principal(subject: &str, tenant: &str) -> Principal {
    Principal {
        issuer: "https://identity.example.com".into(),
        subject: subject.into(),
        tenant: tenant.into(),
        client_id: "artifact-agent".into(),
        scopes: BTreeSet::from([
            "infimount:read".into(),
            "infimount:create".into(),
            "infimount:download".into(),
        ]),
        expires_at: infimount_hosted::now_epoch_seconds() + 3600,
    }
}

#[tokio::test]
#[ignore = "run scripts/qualify-hosted-data-plane.py for a disposable loopback S3 service"]
async fn synthetic_s3_qualification() {
    let endpoint = std::env::var("INFIMOUNT_HOSTED_TEST_S3_ENDPOINT")
        .expect("the qualification script must supply its disposable endpoint");
    let url = reqwest::Url::parse(&endpoint).unwrap();
    assert_eq!(url.scheme(), "http");
    assert_eq!(url.host_str(), Some("127.0.0.1"));
    let bucket = std::env::var("INFIMOUNT_HOSTED_TEST_S3_BUCKET").unwrap();
    assert!(bucket.starts_with("infimount-test-"));
    let temporary = tempfile::tempdir().unwrap();
    let access = temporary.path().join("access-key");
    let secret = temporary.path().join("secret-key");
    std::fs::write(&access, b"synthetic-test-key").unwrap();
    std::fs::write(&secret, b"synthetic-test-secret").unwrap();
    let mut config: HostedConfig =
        serde_json::from_str(include_str!("../../../deploy/hosted/example-s3.json")).unwrap();
    config.storages[0].record.config = json!({
        "bucket": bucket,
        "region": "us-east-1",
        "endpoint": endpoint
    });
    config.storages[0]
        .credentials
        .insert("accessKeyId".into(), SecretRef::File { path: access });
    config.storages[0]
        .credentials
        .insert("secretAccessKey".into(), SecretRef::File { path: secret });
    let record = config.storages[0].resolved_record().unwrap();
    let raw = infimount_core::registry::build_hosted_s3_operator(&Source {
        id: record.id,
        name: record.name,
        kind: SourceKind::S3,
        root: String::new(),
        config: record.config,
    })
    .unwrap();
    let storage = Arc::new(StoragePlane::new(Arc::new(config)).unwrap());
    let transfers = TransferManager::new(storage.clone());
    storage.ready().await.unwrap();
    let alice = principal("alice", "alpha");
    let bob = principal("bob", "beta");
    let alice_root = "/Artifacts/tenants/alpha/users/alice";
    let path = format!("{alice_root}/binary.bin");
    let bytes = (0..4097)
        .map(|index| (index % 251) as u8)
        .collect::<Vec<_>>();
    let written = storage
        .write(
            &alice,
            &path,
            &STANDARD.encode(&bytes),
            "base64",
            Some("application/pdf"),
        )
        .await
        .unwrap();
    assert_eq!(written["written_bytes"], bytes.len());
    assert_eq!(written["sha256"], format!("{:x}", Sha256::digest(&bytes)));
    let metadata = raw
        .stat("tenants/alpha/users/alice/binary.bin")
        .await
        .unwrap();
    assert_eq!(metadata.content_type(), Some("application/pdf"));
    let read = storage.read(&alice, &path, 0, 8192).await.unwrap();
    assert_eq!(
        STANDARD.decode(read["content"].as_str().unwrap()).unwrap(),
        bytes
    );
    assert_eq!(
        storage.read(&bob, &path, 0, 8192).await.unwrap_err(),
        HostedError::NotFound
    );
    assert_eq!(
        storage
            .write(&alice, &path, "overwrite", "utf-8", None)
            .await
            .unwrap_err(),
        HostedError::Conflict
    );
    assert_eq!(
        raw.read("tenants/alpha/users/alice/binary.bin")
            .await
            .unwrap()
            .to_vec(),
        bytes
    );

    for index in 0..3 {
        storage
            .write(
                &alice,
                &format!("{alice_root}/page-{index}.txt"),
                "page",
                "utf-8",
                None,
            )
            .await
            .unwrap();
    }
    let page = storage.list(&alice, alice_root, 1, None).await.unwrap();
    let cursor = page["next_cursor"]
        .as_str()
        .expect("more than one object")
        .to_owned();
    assert!(storage
        .list(
            &bob,
            "/Artifacts/tenants/beta/users/bob",
            1,
            Some(cursor.clone())
        )
        .await
        .is_err());
    let mut other_client = alice.clone();
    other_client.client_id = "untrusted-client".into();
    assert!(storage
        .list(&other_client, alice_root, 1, Some(cursor.clone()))
        .await
        .is_err());
    assert!(storage
        .list(&alice, alice_root, 1, Some(cursor))
        .await
        .is_ok());

    let download = transfers
        .create_download(&alice, &path, Some(60))
        .await
        .unwrap();
    let download_id = download["download_id"].as_str().unwrap();
    assert!(!download["url"].as_str().unwrap().contains("127.0.0.1"));
    assert!(transfers.download(&bob, download_id, None).await.is_err());
    let response = transfers.download(&alice, download_id, None).await.unwrap();
    assert_eq!(
        response.headers()["content-type"],
        "application/octet-stream"
    );
    assert_eq!(response.headers()["cache-control"], "private, no-store");
    assert_eq!(response.headers()["x-content-type-options"], "nosniff");
    assert_eq!(
        to_bytes(response.into_body(), bytes.len() + 1)
            .await
            .unwrap()
            .as_ref(),
        bytes
    );
    assert!(transfers.download(&alice, download_id, None).await.is_err());

    let stale = transfers
        .create_download(&alice, &path, Some(60))
        .await
        .unwrap();
    raw.write("tenants/alpha/users/alice/binary.bin", "replacement")
        .await
        .unwrap();
    assert_eq!(
        transfers
            .download(&alice, stale["download_id"].as_str().unwrap(), None)
            .await
            .unwrap_err(),
        HostedError::Conflict
    );

    // Exceed the inline limit and exercise a real S3 multipart writer.
    let large = (0..(9 * 1024 * 1024 + 37))
        .map(|index| (index % 251) as u8)
        .collect::<Vec<_>>();
    let digest = format!("{:x}", Sha256::digest(&large));
    let large_path = format!("{alice_root}/large.bin");
    let upload = transfers
        .create_upload(
            &alice,
            &large_path,
            large.len() as u64,
            &digest,
            Some("application/zip"),
            Some(120),
        )
        .await
        .unwrap();
    let upload_id = upload["upload_id"].as_str().unwrap();
    assert!(transfers
        .upload(&bob, upload_id, Body::from("wrong"))
        .await
        .is_err());
    let frames = large
        .chunks(31_337)
        .map(|chunk| Ok::<_, std::io::Error>(Bytes::copy_from_slice(chunk)))
        .collect::<Vec<_>>();
    let uploaded = transfers
        .upload(
            &alice,
            upload_id,
            Body::from_stream(futures::stream::iter(frames)),
        )
        .await
        .unwrap();
    assert_eq!(uploaded["received_bytes"], large.len());
    assert!(raw
        .stat("tenants/alpha/users/alice/large.bin")
        .await
        .is_err());
    let committed = transfers.complete_upload(&alice, upload_id).await.unwrap();
    assert_eq!(committed["written_bytes"], large.len());
    assert_eq!(
        raw.stat("tenants/alpha/users/alice/large.bin")
            .await
            .unwrap()
            .content_type(),
        Some("application/zip")
    );
    assert_eq!(
        raw.read("tenants/alpha/users/alice/large.bin")
            .await
            .unwrap()
            .to_vec(),
        large
    );
    assert!(transfers.complete_upload(&alice, upload_id).await.is_err());
    let large_download = transfers
        .create_download(&alice, &large_path, Some(120))
        .await
        .unwrap();
    let response = transfers
        .download(
            &alice,
            large_download["download_id"].as_str().unwrap(),
            Some("bytes=1048576-2097151"),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), axum::http::StatusCode::PARTIAL_CONTENT);
    assert_eq!(
        to_bytes(response.into_body(), 1024 * 1024)
            .await
            .unwrap()
            .as_ref(),
        &large[1024 * 1024..2 * 1024 * 1024]
    );

    let failed_path = format!("{alice_root}/bad-checksum.bin");
    let failed = transfers
        .create_upload(&alice, &failed_path, 3, &"0".repeat(64), None, None)
        .await
        .unwrap();
    let failed_id = failed["upload_id"].as_str().unwrap();
    assert!(transfers
        .upload(&alice, failed_id, Body::from("bad"))
        .await
        .is_err());
    assert!(transfers.complete_upload(&alice, failed_id).await.is_err());
    assert!(raw
        .stat("tenants/alpha/users/alice/bad-checksum.bin")
        .await
        .is_err());
    let aborted = transfers
        .create_upload(
            &alice,
            &format!("{alice_root}/aborted.bin"),
            0,
            &format!("{:x}", Sha256::digest([])),
            None,
            None,
        )
        .await
        .unwrap();
    let aborted_id = aborted["upload_id"].as_str().unwrap();
    transfers.abort_upload(&alice, aborted_id).await.unwrap();
    assert!(transfers.complete_upload(&alice, aborted_id).await.is_err());
    let restarted = TransferManager::new(storage);
    assert!(restarted
        .download(&alice, stale["download_id"].as_str().unwrap(), None)
        .await
        .is_err());
}
