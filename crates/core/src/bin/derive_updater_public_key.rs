//! Derive the Tauri updater public key from the signing secret.
//!
//! The release trust chain requires `tauri.conf.json` to embed the public
//! half of the private key stored in CI. This tool prints that public key
//! (classic minisign text) so it can be committed. Public material is safe
//! to display; the private key is never logged.
//!
//! Inputs:
//! - `TAURI_SIGNING_PRIVATE_KEY` (the minisign secret-key file content)
//! - optional `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`
//! or a filesystem path via argv[1] plus optional password argv[2] for
//! local use.

use base64::Engine;
use std::io::Read;

fn run() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let mut private_material = String::new();
    if let Some(path) = args.next() {
        std::fs::read_to_string(&path).map_err(|e| format!("read {path}: {e}"))?
    } else {
        std::env::var("TAURI_SIGNING_PRIVATE_KEY")
            .map_err(|_| "TAURI_SIGNING_PRIVATE_KEY is not set".to_string())?
    }
    .clone_into(&mut private_material);
    if private_material.trim().is_empty() {
        return Err("updater private key material is empty".to_string());
    }
    let password = args
        .next()
        .or_else(|| std::env::var("TAURI_SIGNING_PRIVATE_KEY_PASSWORD").ok())
        .filter(|p| !p.is_empty());

    // Tauri stores both signatures and secret keys as the classic minisign
    // text wrapped in one outer base64 layer. Unwrap when present.
    let key_text = if private_material
        .trim_start()
        .starts_with("untrusted comment:")
    {
        private_material
    } else {
        match base64::engine::general_purpose::STANDARD
            .decode(private_material.trim())
            .ok()
            .and_then(|bytes| String::from_utf8(bytes).ok())
        {
            Some(decoded) if decoded.starts_with("untrusted comment:") => decoded,
            _ => private_material,
        }
    };

    let sk_box = minisign::SecretKeyBox::from_string(&key_text)
        .map_err(|e| format!("parse private key: {e}"))?;
    let secret_key = sk_box
        .into_secret_key(password)
        .map_err(|e| "decrypt private key (wrong password?)".to_string())?;
    let public_key =
        minisign::PublicKey::from_secret_key(&secret_key).map_err(|e| format!("derive: {e}"))?;
    let pk_box = public_key
        .to_box()
        .map_err(|e| format!("encode: {e}"))?
        .into_string();
    print!("{pk_box}");
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("derive_updater_public_key failed: {error}");
        std::process::exit(1);
    }
}
