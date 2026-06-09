mod action_router;
mod approval_store;
mod backend_factory;
#[cfg(unix)]
mod browser;
#[cfg(not(unix))]
#[path = "browser/unsupported.rs"]
mod browser;
mod daemon;
mod diagnostics;
mod element_resolver;
mod ipc_server;
mod overlay;
mod session_store;
mod snapshot_manager;

use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .with_writer(std::io::stderr)
        .init();

    let arg = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "daemon".to_string());
    match arg.as_str() {
        "daemon" => ipc_server::run_service().await,
        "doctor" => run_doctor().await,
        "setup-accessibility" => run_setup_accessibility().await,
        "setup-window-targeting" => run_setup_window_targeting().await,
        other => anyhow::bail!("unsupported sky-cua-service mode: {other}"),
    }
}

async fn run_doctor() -> Result<()> {
    let backend = backend_factory::create_backend();
    match backend.doctor().await {
        Ok(report) => {
            println!("{}", serde_json::to_string_pretty(&report)?);
            Ok(())
        }
        Err(error) => {
            eprintln!("Doctor failed: {}", error.message);
            std::process::exit(1);
        }
    }
}

async fn run_setup_accessibility() -> Result<()> {
    let backend = backend_factory::create_backend();
    match backend.setup_accessibility().await {
        Ok(report) => {
            println!("{}", serde_json::to_string_pretty(&report)?);
            Ok(())
        }
        Err(error) => {
            eprintln!("Setup accessibility failed: {}", error.message);
            std::process::exit(1);
        }
    }
}

async fn run_setup_window_targeting() -> Result<()> {
    let backend = backend_factory::create_backend();
    match backend.setup_window_targeting().await {
        Ok(report) => {
            println!("{}", serde_json::to_string_pretty(&report)?);
            Ok(())
        }
        Err(error) => {
            eprintln!("Setup window targeting failed: {}", error.message);
            std::process::exit(1);
        }
    }
}
