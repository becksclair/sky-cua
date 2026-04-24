mod heuristics;
mod mcp_server;
mod service_launcher;

use anyhow::Result;
use heuristics::HeuristicsRegistry;

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .with_writer(std::io::stderr)
        .init();

    let mode = std::env::args().nth(1).unwrap_or_else(|| "mcp".to_string());
    match mode.as_str() {
        "mcp" => {
            let service = service_launcher::ServiceClient::connect_or_spawn()?;
            let heuristics = HeuristicsRegistry::load_from_repo()?;
            mcp_server::serve(service, heuristics)
        }
        "clear-portal-tokens" => {
            let service = service_launcher::ServiceClient::connect_or_spawn()?;
            match service.clear_portal_tokens()? {
                sky_cua_platform::model::ServiceResponse::ResetPortalTokens {
                    cleared,
                    token_path,
                    dropped_cached_session,
                } => {
                    println!(
                        "cleared={} dropped_cached_session={} token_path={}",
                        cleared, dropped_cached_session, token_path
                    );
                    Ok(())
                }
                other => {
                    anyhow::bail!("unexpected response for clear-portal-tokens mode: {other:?}")
                }
            }
        }
        other => anyhow::bail!("unsupported sky-cua-client mode: {other}"),
    }
}
