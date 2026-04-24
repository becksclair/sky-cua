mod action_router;
mod approval_store;
mod daemon;
mod diagnostics;
mod ipc_server;
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
        other => anyhow::bail!("unsupported sky-cua-service mode: {other}"),
    }
}
