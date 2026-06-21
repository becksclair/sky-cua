use std::path::PathBuf;

use anyhow::{Result, anyhow};
use sky_cua_input_helper::server::{ServerOptions, run_server};

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("serve") => {
            let options = parse_serve_args(args.collect())?;
            run_server(options)
        }
        Some("--help") | Some("-h") | None => {
            print_help();
            Ok(())
        }
        Some(other) => Err(anyhow!("unknown command {other:?}")),
    }
}

fn parse_serve_args(args: Vec<String>) -> Result<ServerOptions> {
    let mut socket_path = PathBuf::from(
        std::env::var_os("SKY_CUA_INPUT_HELPER_SOCKET")
            .unwrap_or_else(|| "/run/sky-cua/input-helper.sock".into()),
    );
    let mut socket_mode = std::env::var("SKY_CUA_INPUT_HELPER_SOCKET_MODE")
        .ok()
        .and_then(|value| u32::from_str_radix(value.trim_start_matches('0'), 8).ok())
        .unwrap_or(0o660);
    let mut socket_group = std::env::var("SKY_CUA_INPUT_HELPER_SOCKET_GROUP")
        .ok()
        .filter(|value| !value.trim().is_empty());

    let mut iter = args.into_iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--socket" => {
                socket_path = PathBuf::from(
                    iter.next()
                        .ok_or_else(|| anyhow!("--socket requires a path"))?,
                );
            }
            "--socket-mode" => {
                let value = iter
                    .next()
                    .ok_or_else(|| anyhow!("--socket-mode requires an octal mode"))?;
                socket_mode = u32::from_str_radix(value.trim_start_matches('0'), 8)
                    .map_err(|error| anyhow!("invalid --socket-mode {value:?}: {error}"))?;
            }
            "--socket-group" => {
                socket_group = Some(
                    iter.next()
                        .ok_or_else(|| anyhow!("--socket-group requires a group name"))?,
                );
            }
            other => return Err(anyhow!("unknown serve argument {other:?}")),
        }
    }

    Ok(ServerOptions {
        socket_path,
        socket_mode,
        socket_group,
    })
}

fn print_help() {
    println!(
        "Usage:\n  sky-cua-input-helper serve [--socket PATH] [--socket-mode OCTAL] [--socket-group GROUP]"
    );
}
