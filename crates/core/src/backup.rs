use age::secrecy::SecretString;
use serde::{Deserialize, Serialize};

#[derive(Debug)]
pub enum BackupError {
    Encryption(String),
    Decryption(String),
    Serialization(String),
    Io(String),
}

impl std::fmt::Display for BackupError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BackupError::Encryption(msg) => write!(f, "encryption failed: {msg}"),
            BackupError::Decryption(msg) => write!(f, "decryption failed: {msg}"),
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

const BACKUP_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Serialize, Deserialize)]
pub struct BackupPayload {
    pub schema_version: u32,
    pub created_at: String,
    pub storages: Vec<serde_json::Value>,
    pub mcp_settings: Option<serde_json::Value>,
    pub app_settings: Option<serde_json::Value>,
    pub checksum: String,
}

impl BackupPayload {
    fn compute_checksum(payload: &[u8]) -> String {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        payload.hash(&mut hasher);
        format!("{:x}", hasher.finish())
    }

    pub fn new(
        storages: Vec<serde_json::Value>,
        mcp_settings: Option<serde_json::Value>,
        app_settings: Option<serde_json::Value>,
    ) -> Result<Self, BackupError> {
        let created_at = chrono::Utc::now().to_rfc3339();
        let mut payload = BackupPayload {
            schema_version: BACKUP_SCHEMA_VERSION,
            created_at,
            storages,
            mcp_settings,
            app_settings,
            checksum: String::new(),
        };
        let encoded = serde_json::to_vec(&payload)?;
        payload.checksum = Self::compute_checksum(&encoded);
        Ok(payload)
    }

    pub fn verify(&self) -> bool {
        let for_checksum = serde_json::json!({
            "schema_version": self.schema_version,
            "created_at": self.created_at,
            "storages": self.storages,
            "mcp_settings": self.mcp_settings,
            "app_settings": self.app_settings,
        });
        let encoded = serde_json::to_vec(&for_checksum).ok();
        encoded
            .as_deref()
            .map(|bytes| Self::compute_checksum(bytes) == self.checksum)
            .unwrap_or(false)
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
    let plaintext = age::decrypt(&identity, armored.as_bytes())?;
    let payload: BackupPayload = serde_json::from_slice(&plaintext)?;
    if !payload.verify() {
        return Err(BackupError::Decryption(
            "backup payload checksum mismatch; data may be corrupted".into(),
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
