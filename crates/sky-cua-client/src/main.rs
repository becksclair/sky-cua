mod heuristics;
mod launch_environment;
mod mcp_server;
mod mcp_tools;
mod operator_cli;
mod output_shapes;
mod service_launcher;

use anyhow::Result;
use heuristics::HeuristicsRegistry;
use std::process::ExitCode;

fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .with_writer(std::io::stderr)
        .init();

    match run() {
        Ok(code) => code,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::from(1)
        }
    }
}

fn run() -> Result<ExitCode> {
    let mode = operator_cli::parse_cli_mode(std::env::args().skip(1))?;
    match mode {
        operator_cli::CliMode::Mcp => {
            let runtime = tokio::runtime::Runtime::new()?;
            runtime.block_on(async {
                let service = service_launcher::ServiceClient::connect_or_spawn()?;
                let heuristics = HeuristicsRegistry::load_from_repo()?;
                mcp_server::serve(service, heuristics).await?;
                Ok::<ExitCode, anyhow::Error>(ExitCode::SUCCESS)
            })
        }
        operator_cli::CliMode::ClearPortalTokens => operator_cli::run_clear_portal_tokens(),
        operator_cli::CliMode::Operator(command) => operator_cli::run_operator_command(command),
    }
}
