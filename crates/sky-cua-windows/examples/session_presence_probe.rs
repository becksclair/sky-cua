#[cfg(target_os = "windows")]
use std::time::Duration;

#[cfg(target_os = "windows")]
use sky_cua_platform::backend::DesktopBackend;
#[cfg(target_os = "windows")]
use sky_cua_platform::model::SessionPresenceIntent;
#[cfg(target_os = "windows")]
use sky_cua_windows::WindowsDesktopBackend;

#[cfg(target_os = "windows")]
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let command = args.next().unwrap_or_else(|| "status".to_string());
    let backend = WindowsDesktopBackend::new();

    match command.as_str() {
        "status" => {
            let status = backend.session_presence_status().await;
            println!("{}", serde_json::to_string_pretty(&status)?);
        }
        "hold" => {
            let seconds = args
                .next()
                .as_deref()
                .unwrap_or("60")
                .parse::<u64>()
                .unwrap_or(60);
            let status = backend
                .ensure_session_presence(SessionPresenceIntent {
                    unlock: true,
                    inhibit_lock: true,
                    inhibit_suspend: true,
                })
                .await?;
            println!("{}", serde_json::to_string_pretty(&status)?);
            println!(
                "holding Windows power request for {seconds}s; inspect with `powercfg /requests`"
            );
            tokio::time::sleep(Duration::from_secs(seconds)).await;
            let released = backend.release_session_presence(false).await?;
            println!("{}", serde_json::to_string_pretty(&released)?);
        }
        other => {
            eprintln!("unknown command: {other}");
            eprintln!("usage: session_presence_probe [status|hold [seconds]]");
            std::process::exit(2);
        }
    }

    Ok(())
}

#[cfg(not(target_os = "windows"))]
fn main() {
    eprintln!("session_presence_probe is only available on Windows");
    std::process::exit(2);
}
