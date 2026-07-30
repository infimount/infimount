use base64::Engine;
use minisign_verify::{PublicKey, Signature};
use std::fs::File;
use std::io::Read;
use std::path::Path;

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
    let signature = Signature::from_file(Path::new(&signature))
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
