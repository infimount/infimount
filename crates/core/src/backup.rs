use std::collections::HashMap;

use age::secrecy::SecretString;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug)]
pub enum BackupError {
    Encryption(String),
    Decryption(String),
    ChecksumMismatch(String),
    Serialization(String),
    Io(String),
}

impl std::fmt::Display for BackupError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BackupError::Encryption(msg) => write!(f, "encryption failed: {msg}"),
            BackupError::Decryption(msg) => write!(f, "decryption failed: {msg}"),
            BackupError::ChecksumMismatch(msg) => write!(f, "checksum mismatch: {msg}"),
            BackupError::Serialization(msg) => write!(f, "serialization failed: {msg}"),
            BackupError::Io(msg) => write!(f, "I/O error: {msg}"),
        }
    }
}

impl std::error::Error for BackupError {}

impl From<serde_json::Error> for BackupError {
    fn from(e: serde_json::Error) -> Self {
        BackupError::Serialization(e.to_string())
    }
}

impl From<age::EncryptError> for BackupError {
    fn from(e: age::EncryptError) -> Self {
        BackupError::Encryption(e.to_string())
    }
}

impl From<age::DecryptError> for BackupError {
    fn from(e: age::DecryptError) -> Self {
        BackupError::Decryption(e.to_string())
    }
}

impl From<std::io::Error> for BackupError {
    fn from(e: std::io::Error) -> Self {
        BackupError::Io(e.to_string())
    }
}

const BACKUP_SCHEMA_VERSION: u32 = 2;

#[derive(Debug, Serialize, Deserialize)]
pub struct BackupPayload {
    pub schema_version: u32,
    pub created_at: String,
    pub storages: Vec<serde_json::Value>,
    pub mcp_settings: Option<serde_json::Value>,
    pub app_settings: Option<serde_json::Value>,
    pub workspaces: Option<serde_json::Value>,
    pub secrets: HashMap<String, String>,
    pub checksum: String,
}

fn canonical_bytes(payload: &BackupPayload) -> Result<Vec<u8>, BackupError> {
    let for_hash = serde_json::json!({
        "schema_version": payload.schema_version,
        "created_at": payload.created_at,
        "storages": payload.storages,
        "mcp_settings": payload.mcp_settings,
        "app_settings": payload.app_settings,
        "workspaces": payload.workspaces,
        "secrets": payload.secrets,
    });
    Ok(serde_json::to_vec(&for_hash)?)
}

fn compute_checksum(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    format!("{:x}", hasher.finalize())
}

impl BackupPayload {
    pub fn new(
        storages: Vec<serde_json::Value>,
        mcp_settings: Option<serde_json::Value>,
        app_settings: Option<serde_json::Value>,
        workspaces: Option<serde_json::Value>,
        secrets: HashMap<String, String>,
    ) -> Result<Self, BackupError> {
        let created_at = chrono::Utc::now().to_rfc3339();
        let mut payload = BackupPayload {
            schema_version: BACKUP_SCHEMA_VERSION,
            created_at,
            storages,
            mcp_settings,
            app_settings,
            workspaces,
            secrets,
            checksum: String::new(),
        };
        let canonical = canonical_bytes(&payload)?;
        payload.checksum = compute_checksum(&canonical);
        Ok(payload)
    }

    pub fn verify(&self) -> bool {
        canonical_bytes(self)
            .ok()
            .map(|bytes| compute_checksum(&bytes) == self.checksum)
            .unwrap_or(false)
    }

    pub fn schema_version(&self) -> u32 {
        self.schema_version
    }
}

pub fn encrypt_backup(passphrase: &str, payload: &BackupPayload) -> Result<String, BackupError> {
    let plaintext = serde_json::to_vec(payload)?;
    let secret = SecretString::from(passphrase.to_owned());
    let recipient = age::scrypt::Recipient::new(secret);
    let armored = age::encrypt_and_armor(&recipient, &plaintext)?;
    Ok(armored)
}

pub fn decrypt_backup(passphrase: &str, armored: &str) -> Result<BackupPayload, BackupError> {
    let secret = SecretString::from(passphrase.to_owned());
    let identity = age::scrypt::Identity::new(secret);
    let plaintext = age::decrypt(&identity, armored.as_bytes()).map_err(|e| {
        BackupError::Decryption(format!("wrong passphrase or corrupted backup: {e}"))
    })?;

    let payload: BackupPayload = serde_json::from_slice(&plaintext)?;
    if payload.schema_version == 0 || payload.schema_version > BACKUP_SCHEMA_VERSION {
        return Err(BackupError::Serialization(format!(
            "unsupported backup format version: {}",
            payload.schema_version
        )));
    }
    if !payload.verify() {
        return Err(BackupError::ChecksumMismatch(
            "backup payload checksum mismatch; data may be corrupted or tampered".into(),
        ));
    }
    Ok(payload)
}

pub fn encrypt_backup_to_file(
    passphrase: &str,
    payload: &BackupPayload,
    path: &std::path::Path,
) -> Result<(), BackupError> {
    let armored = encrypt_backup(passphrase, payload)?;
    let parent = path.parent().unwrap_or(std::path::Path::new(""));
    if !parent.as_os_str().is_empty() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp_path = path.with_extension("tmp");
    std::fs::write(&tmp_path, armored.as_bytes())?;
    std::fs::rename(&tmp_path, path)?;
    Ok(())
}

pub fn decrypt_backup_from_file(
    passphrase: &str,
    path: &std::path::Path,
) -> Result<BackupPayload, BackupError> {
    let armored = std::fs::read_to_string(path)?;
    decrypt_backup(passphrase, &armored)
}

pub fn zeroize(value: &mut String) {
    unsafe {
        for byte in value.as_bytes_mut() {
            *byte = 0;
        }
    }
    value.clear();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_payload() -> BackupPayload {
        BackupPayload::new(
            vec![serde_json::json!({"id": "s1", "name": "test"})],
            None,
            None,
            None,
            HashMap::new(),
        )
        .unwrap()
    }

    #[test]
    fn test_backup_round_trip() {
        let payload = sample_payload();
        let armored = encrypt_backup("correct-passphrase", &payload).unwrap();
        let decrypted = decrypt_backup("correct-passphrase", &armored).unwrap();
        assert_eq!(decrypted.storages.len(), 1);
        assert!(decrypted.verify());
    }

    #[test]
    fn test_wrong_passphrase() {
        let payload = sample_payload();
        let armored = encrypt_backup("correct-passphrase", &payload).unwrap();
        let result = decrypt_backup("wrong-passphrase", &armored);
        assert!(matches!(result, Err(BackupError::Decryption(_))));
    }

    #[test]
    fn test_ciphertext_tampering_caught_by_decryption() {
        let payload = sample_payload();
        let armored = encrypt_backup("p", &payload).unwrap();
        let mut chars: Vec<char> = armored.chars().collect();
        if let Some(c) = chars.get_mut(100) {
            *c = (*c as u8 ^ 1) as char;
        }
        let tampered: String = chars.into_iter().collect();
        let result = decrypt_backup("p", &tampered);
        assert!(
            matches!(result, Err(BackupError::Decryption(_))),
            "expected Decryption error for ciphertext tampering, got {result:?}"
        );
    }

    #[test]
    fn test_decrypted_payload_tampering_caught_by_checksum() {
        let payload = sample_payload();
        let armored = encrypt_backup("p", &payload).unwrap();
        let mut decrypted = decrypt_backup("p", &armored).unwrap();
        decrypted.checksum = "0000000000000000000000000000000000000000000000000000000000000000".into();
        let result = serde_json::to_vec(&decrypted).unwrap();
        let re_encrypted = encrypt_backup("p", &decrypted).unwrap();
        let outcome = decrypt_backup("p", &re_encrypted);
        assert!(
            matches!(outcome, Err(BackupError::ChecksumMismatch(_))),
            "expected ChecksumMismatch for tampered checksum, got {outcome:?}"
        );
    }

    #[test]
    fn test_modified_decrypted_payload() {
        let payload = sample_payload();
        let armored = encrypt_backup("p", &payload).unwrap();
        let mut decrypted = decrypt_backup("p", &armored).unwrap();
        decrypted.storages.push(serde_json::json!({"id": "injected"}));
        assert!(!decrypted.verify());
    }

    #[test]
    fn test_verify_passes_for_unmodified_payload() {
        let p1 = sample_payload();
        let armored = encrypt_backup("p", &p1).unwrap();
        let d1 = decrypt_backup("p", &armored).unwrap();
        assert!(d1.verify());
        assert_eq!(d1.storages.len(), 1);
    }

    #[test]
    fn test_unsupported_version() {
        let mut payload = sample_payload();
        payload.schema_version = 999;
        let armored = encrypt_backup("p", &payload).unwrap();
        let result = decrypt_backup("p", &armored);
        assert!(matches!(result, Err(BackupError::Serialization(_))));
    }
}
