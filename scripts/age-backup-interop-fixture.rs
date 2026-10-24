//! Temporary local CI-only fixture binary for interop with released v0.8.1.
use std::collections::HashMap;
use infimount_core::backup::{encrypt_backup, decrypt_backup, BackupPayload};

const PASSPHRASE: &str = "public-ci-interop-test-only-passphrase";
fn main() {
    let args = std::env::args().collect::<Vec<_>>();
    assert_eq!(args.len(), 3, "encrypt or decrypt plus one output/input path");
    if args[1] == "encrypt" {
        let payload = BackupPayload::new(
            vec![serde_json::json!({
                "id": "cross-release-test",
                "name": "Infimount backup compatibility",
                "backend": "local"
            })],
            Some(serde_json::json!({"enabled": false})),
            None,
            None,
            HashMap::new(),
        ).unwrap();
        let armored = encrypt_backup(PASSPHRASE, &payload).unwrap();
        std::fs::write(&args[2], &armored).unwrap();
        println!("AGE_V081_ARMOR_BEGIN");
        print!("{armored}");
        println!("AGE_V081_ARMOR_END");
        println!("Fixture created using age-based passphrase backup");
    } else if args[1] == "decrypt" {
        let armored = std::fs::read_to_string(&args[2]).unwrap();
        let payload = decrypt_backup(PASSPHRASE, &armored).unwrap();
        assert!(payload.verify());
        assert_eq!(payload.schema_version(), 2);
        assert_eq!(payload.storages[0]["id"], "cross-release-test");
        assert_eq!(payload.storages[0]["name"], "Infimount backup compatibility");
        assert!(decrypt_backup("wrong-test-only-passphrase", &armored).is_err());
        println!("Encrypted backup decrypted, checksum verified and wrong passphrase rejected");
    } else {
        panic!("unknown interop operation");
    }
}
