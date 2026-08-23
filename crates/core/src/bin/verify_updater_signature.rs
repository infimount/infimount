use base64::Engine;
use minisign_verify::{PublicKey, Signature};
use std::fs::File;
use std::io::Read;
use std::path::Path;

/// Accept both signature encodings produced in this repo:
/// - classic minisign text files (`untrusted comment:` + base64 line), and
/// - Tauri bundler output, which is the whole classic file wrapped in one
///   outer base64 layer. The release workflow verifies Tauri's native
///   `.sig` files directly, so the wrapper must be decoded here.
fn load_signature(path: &Path) -> Result<Signature, String> {
    if let Ok(signature) = Signature::from_file(path) {
        return Ok(signature);
    }
    let mut wrapped = String::new();
    File::open(path)
        .and_then(|mut file| file.read_to_string(&mut wrapped))
        .map_err(|error| error.to_string())?;
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(wrapped.trim())
        .ok()
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .ok_or_else(|| "not decodable as minisign or base64-wrapped minisign".to_string())?;
    Signature::decode(&decoded).map_err(|error| error.to_string())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("updater signature verification failed: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let public_key = args.next().ok_or_else(|| {
        "usage: verify_updater_signature <public-key> <artifact> <signature>".to_string()
    })?;
    let artifact = args
        .next()
        .ok_or_else(|| "missing updater artifact path".to_string())?;
    let signature = args
        .next()
        .ok_or_else(|| "missing updater signature path".to_string())?;
    if args.next().is_some() {
        return Err("unexpected extra arguments".to_string());
    }

    let decoded_key = base64::engine::general_purpose::STANDARD
        .decode(public_key.trim())
        .ok()
        .and_then(|bytes| String::from_utf8(bytes).ok());
    let key_line = decoded_key
        .as_deref()
        .unwrap_or(public_key.trim())
        .lines()
        .find(|line| !line.trim().is_empty() && !line.starts_with("untrusted comment:"))
        .ok_or_else(|| "updater public key is empty".to_string())?;
    let public_key = PublicKey::from_base64(key_line.trim())
        .map_err(|error| format!("invalid updater public key: {error}"))?;
    let signature = load_signature(Path::new(&signature))
        .map_err(|error| format!("invalid updater signature: {error}"))?;
    let mut artifact =
        File::open(&artifact).map_err(|error| format!("cannot open updater artifact: {error}"))?;
    let mut verifier = public_key
        .verify_stream(&signature)
        .map_err(|error| format!("cannot initialize verifier: {error}"))?;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = artifact
            .read(&mut buffer)
            .map_err(|error| format!("cannot read updater artifact: {error}"))?;
        if read == 0 {
            break;
        }
        verifier.update(&buffer[..read]);
    }
    verifier
        .finalize()
        .map_err(|error| format!("signature mismatch: {error}"))?;
    println!("Updater signature verified.");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::engine::general_purpose::STANDARD;

    /// Build a structurally valid classic minisign text signature. The
    /// cryptographic values are dummies: load_signature only parses.
    fn classic_text() -> String {
        let mut sig = vec![0x45_u8, 0x64];
        sig.extend_from_slice(&[0_u8; 8]);
        sig.extend_from_slice(&[0_u8; 64]);
        let global = [7_u8; 64];
        format!(
            "untrusted comment: fixture\n{}\ntrusted comment: fixture\n{}\n",
            STANDARD.encode(sig),
            STANDARD.encode(global)
        )
    }

    #[test]
    fn load_signature_accepts_classic_and_tauri_wrapped_formats() {
        let dir = tempfile::tempdir().unwrap();
        let text = classic_text();

        let classic_path = dir.path().join("classic.sig");
        std::fs::write(&classic_path, &text).unwrap();
        assert!(load_signature(&classic_path).is_ok());

        // Tauri bundler output: the whole classic file wrapped in outer base64.
        let wrapped_path = dir.path().join("wrapped.sig");
        std::fs::write(&wrapped_path, STANDARD.encode(text.as_bytes())).unwrap();
        assert!(load_signature(&wrapped_path).is_ok());

        let garbage_path = dir.path().join("garbage.sig");
        std::fs::write(&garbage_path, b"not a signature").unwrap();
        assert!(load_signature(&garbage_path).is_err());
    }
}
