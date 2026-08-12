mod app_state;
mod daemon_log;
#[cfg(unix)]
mod daemon_singleton;
mod heuristics;
#[cfg(unix)]
mod isolated_desktop;
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
    // The MCP client is spawned by arbitrary hosts (Codex Desktop, app
    // server); drop their leaked descriptors before opening any of our own
    // so they cannot propagate into the long-lived service daemon either.
    sky_cua_platform::fd_hygiene::close_inherited_fds();
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
    let args: Vec<String> = std::env::args().skip(1).collect();

    // Hidden development subcommand exercising the isolated xpra desktop
    // lifecycle module. It is intentionally kept out of the operator CLI's
    // advertised modes; it brings up, inspects, or tears down the private
    // desktop from the command line for development and the VM smoke profile.
    #[cfg(unix)]
    if args.first().map(String::as_str) == Some("isolated-desktop") {
        return run_isolated_desktop(&args[1..]);
    }

    // Hidden development/CI subcommand: dumps every `SKY_CUA_*` key
    // `sky-cua-platform` declares as its canonical source of truth, one per
    // line. Used by `scripts/test_env_key_contract.py` and by operators
    // auditing the runtime env-key contract; not part of the advertised
    // operator CLI surface.
    if args.first().map(String::as_str) == Some("env-keys") {
        return run_env_keys(&args[1..]);
    }

    let mode = operator_cli::parse_cli_mode(args)?;
    match mode {
        operator_cli::CliMode::Mcp => {
            let runtime = tokio::runtime::Runtime::new()?;
            runtime.block_on(async {
                let service = service_launcher::ServiceClient::connect_or_spawn()?;
                let heuristics = HeuristicsRegistry::load_from_repo()?;
                // `ServiceClient` is `Clone` over shared `Arc` state (including
                // the isolated-desktop handle), so this clone is cheap and the
                // shutdown teardown below operates on the same handle `serve`
                // used.
                let shutdown_service = service.clone();
                let serve_result = mcp_server::serve(service, heuristics).await;
                // Honor an ephemeral isolated-desktop lifecycle by tearing the
                // private xpra session down on shutdown. This must run on BOTH
                // the normal stdio-EOF exit AND the error exit: a host closing
                // the MCP pipe makes `serve` return Err, and that is exactly the
                // path on which an ephemeral xpra server would otherwise leak.
                // Best-effort inside the call (a teardown failure only warns), so
                // it never changes the exit code. Persistent (the default) and
                // the non-isolated path are no-ops.
                shutdown_service.shutdown_isolated_if_ephemeral();
                serve_result?;
                Ok::<ExitCode, anyhow::Error>(ExitCode::SUCCESS)
            })
        }
        operator_cli::CliMode::ClearPortalTokens => operator_cli::run_clear_portal_tokens(),
        operator_cli::CliMode::Operator(command) => operator_cli::run_operator_command(command),
    }
}

/// Print every `SKY_CUA_*` key `sky-cua-platform` declares, one per line.
fn run_env_keys(args: &[String]) -> Result<ExitCode> {
    use anyhow::bail;

    if let Some(extra) = args.first() {
        bail!("unexpected argument for env-keys: {extra}");
    }

    for key in sky_cua_platform::config::all_env_keys() {
        println!("{key}");
    }
    Ok(ExitCode::SUCCESS)
}

/// Dispatch the hidden `isolated-desktop {ensure|status|stop}` development
/// subcommand. `ensure` brings the private desktop up (idempotently) and prints
/// its display, settled geometry, and viewer mode; `status` reports whether the
/// configured display is up and its geometry; `stop` tears it down.
#[cfg(unix)]
fn run_isolated_desktop(args: &[String]) -> Result<ExitCode> {
    use anyhow::bail;
    use isolated_desktop::IsolatedDesktopHandle;
    use sky_cua_platform::config::resolve_isolated_desktop_selection;

    let action = args.first().map(String::as_str);
    if let Some(extra) = args.get(1) {
        bail!(
            "unexpected argument for isolated-desktop {}: {extra}",
            action.unwrap_or("")
        );
    }

    let cfg = resolve_isolated_desktop_selection().map_err(|error| anyhow::anyhow!(error))?;

    match action {
        Some("ensure") => {
            let handle = IsolatedDesktopHandle::ensure(&cfg)?;
            println!("display={}", handle.display());
            println!("geometry={}", handle.geometry());
            println!("socket={}", handle.socket_path().display());
            println!("owns_bus={}", handle.owns_bus());
            println!("viewer={:?}", cfg.viewer);
            Ok(ExitCode::SUCCESS)
        }
        Some("status") => {
            let status = isolated_desktop::status(&cfg)?;
            println!("enabled={}", status.enabled);
            println!("display={}", status.display);
            println!("up={}", status.up);
            if let Some(geometry) = status.geometry {
                println!("geometry={geometry}");
            }
            println!("viewer={:?}", status.viewer);
            println!("lifecycle={:?}", status.lifecycle);
            println!("dep_xpra={}", status.dependencies.xpra);
            println!("dep_openbox={}", status.dependencies.openbox);
            println!("dep_xdotool={}", status.dependencies.xdotool);
            println!(
                "dep_at_spi_bus_launcher={}",
                status.dependencies.at_spi_bus_launcher
            );
            println!(
                "dep_at_spi_registry={}",
                status.dependencies.at_spi_registry
            );
            Ok(ExitCode::SUCCESS)
        }
        Some("stop") => {
            let display = isolated_desktop::stop(&cfg)?;
            println!("display={display}");
            println!("stopped=true");
            Ok(ExitCode::SUCCESS)
        }
        Some(other) => bail!("unsupported isolated-desktop action: {other}"),
        None => bail!("isolated-desktop requires one of ensure, status, or stop"),
    }
}
