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
#[cfg(unix)]
mod phone;
#[cfg(not(unix))]
#[path = "phone/unsupported.rs"]
mod phone;
mod session_store;
mod snapshot_manager;

use anyhow::Result;
use sky_cua_platform::model::{SessionPresenceAction, SessionPresenceIntent};

fn main() -> Result<()> {
    // Must run before the tokio runtime opens its own descriptors: the
    // daemon outlives its launcher and must not keep inherited sockets
    // (e.g. an Electron DevTools listener) bound after the launcher exits.
    sky_cua_platform::fd_hygiene::close_inherited_fds();
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(async_main())
}

async fn async_main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .with_writer(std::io::stderr)
        .init();

    let mut args = std::env::args().skip(1);
    let arg = args.next().unwrap_or_else(|| "daemon".to_string());
    let rest = args.collect::<Vec<_>>();
    match arg.as_str() {
        "daemon" => ipc_server::run_service().await,
        "doctor" => run_doctor().await,
        "setup-accessibility" => run_setup_accessibility().await,
        "setup-window-targeting" => run_setup_window_targeting().await,
        "session-presence" => run_session_presence(rest).await,
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

async fn run_session_presence(args: Vec<String>) -> Result<()> {
    let action = parse_session_presence_action(&args)?;
    let backend = backend_factory::create_backend();
    let status = match action {
        SessionPresenceAction::Ensure(intent) => backend.ensure_session_presence(intent).await,
        SessionPresenceAction::Release { relock } => backend.release_session_presence(relock).await,
        SessionPresenceAction::Status => Ok(backend.session_presence_status().await),
    };
    match status {
        Ok(status) => {
            println!("{}", serde_json::to_string_pretty(&status)?);
            Ok(())
        }
        Err(error) => {
            eprintln!("Session presence failed: {}", error.message);
            std::process::exit(1);
        }
    }
}

fn parse_session_presence_action(args: &[String]) -> Result<SessionPresenceAction> {
    let Some(command) = args.first().map(String::as_str) else {
        anyhow::bail!("session-presence requires one of ensure, release, or status");
    };

    match command {
        "ensure" | "hold" => parse_session_presence_ensure(&args[1..]),
        "release" => parse_session_presence_release(&args[1..]),
        "status" => {
            ensure_no_session_presence_args(command, &args[1..])?;
            Ok(SessionPresenceAction::Status)
        }
        other => anyhow::bail!("unsupported session-presence action: {other}"),
    }
}

fn parse_session_presence_ensure(args: &[String]) -> Result<SessionPresenceAction> {
    let mut intent = SessionPresenceIntent {
        unlock: true,
        inhibit_lock: true,
        inhibit_suspend: true,
    };

    for arg in args {
        match arg.as_str() {
            "--unlock" => intent.unlock = true,
            "--no-unlock" => intent.unlock = false,
            "--inhibit-lock" => intent.inhibit_lock = true,
            "--no-inhibit-lock" => intent.inhibit_lock = false,
            "--inhibit-suspend" => intent.inhibit_suspend = true,
            "--no-inhibit-suspend" => intent.inhibit_suspend = false,
            other => anyhow::bail!("unsupported session-presence ensure flag: {other}"),
        }
    }

    Ok(SessionPresenceAction::Ensure(intent))
}

fn parse_session_presence_release(args: &[String]) -> Result<SessionPresenceAction> {
    let mut relock = false;
    for arg in args {
        match arg.as_str() {
            "--relock" => relock = true,
            "--no-relock" => relock = false,
            other => anyhow::bail!("unsupported session-presence release flag: {other}"),
        }
    }
    Ok(SessionPresenceAction::Release { relock })
}

fn ensure_no_session_presence_args(command: &str, args: &[String]) -> Result<()> {
    if let Some(arg) = args.first() {
        anyhow::bail!("unexpected argument for session-presence {command}: {arg}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_session_presence_status() {
        assert_eq!(
            parse_session_presence_action(&["status".to_string()]).unwrap(),
            SessionPresenceAction::Status
        );
    }

    #[test]
    fn parses_session_presence_ensure_flags() {
        assert_eq!(
            parse_session_presence_action(&[
                "ensure".to_string(),
                "--no-unlock".to_string(),
                "--no-inhibit-suspend".to_string(),
            ])
            .unwrap(),
            SessionPresenceAction::Ensure(SessionPresenceIntent {
                unlock: false,
                inhibit_lock: true,
                inhibit_suspend: false,
            })
        );
    }

    #[test]
    fn parses_session_presence_release_flags() {
        assert_eq!(
            parse_session_presence_action(&["release".to_string(), "--relock".to_string()])
                .unwrap(),
            SessionPresenceAction::Release { relock: true }
        );
    }
}
