use infimount_hosted::{config::HostedConfig, error::HostedError};
use std::path::PathBuf;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .json()
        .with_env_filter("infimount_hosted=info")
        .with_writer(std::io::stderr)
        .init();
    if let Err(error) = run().await {
        eprintln!("{error}");
        std::process::exit(2);
    }
}

async fn run() -> Result<(), HostedError> {
    let mut args = std::env::args().skip(1);
    let command = args.next().ok_or(HostedError::Config(
        "usage: infimount-hosted <check|serve> [--config FILE]",
    ))?;
    if command != "check" && command != "serve" {
        return Err(HostedError::Config("expected check or serve command"));
    }
    let path = match args.next().as_deref() {
        Some("--config") => PathBuf::from(
            args.next()
                .ok_or(HostedError::Config("--config requires a path"))?,
        ),
        None => PathBuf::from(std::env::var_os("INFIMOUNT_HOSTED_CONFIG").ok_or(
            HostedError::Config("an explicit configuration file is required"),
        )?),
        Some(_) => return Err(HostedError::Config("unexpected command argument")),
    };
    if args.next().is_some() {
        return Err(HostedError::Config("unexpected command argument"));
    }
    let config = HostedConfig::from_file(&path)?;
    if command == "check" {
        println!("Hosted configuration validated. No listener started.");
        return Ok(());
    }
    infimount_hosted::transport::serve(config).await
}
