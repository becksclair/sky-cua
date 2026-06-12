use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use zbus::Proxy;
use zbus::zvariant::{OwnedFd, OwnedObjectPath};

const LOGIND_DEST: &str = "org.freedesktop.login1";
const LOGIND_MANAGER_PATH: &str = "/org/freedesktop/login1";
const LOGIND_MANAGER_IFACE: &str = "org.freedesktop.login1.Manager";
const LOGIND_SESSION_IFACE: &str = "org.freedesktop.login1.Session";
const SCREENSAVER_DEST: &str = "org.freedesktop.ScreenSaver";
const SCREENSAVER_PATH: &str = "/org/freedesktop/ScreenSaver";
const SCREENSAVER_IFACE: &str = "org.freedesktop.ScreenSaver";

#[tokio::main]
async fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let Some(command) = args.next() else {
        usage();
        bail!("missing subcommand");
    };

    match command.as_str() {
        "status" => {
            let logind = LogindProbe::connect().await?;
            let session = logind.resolve_session().await?;
            println!("session_id: {}", session.id);
            println!("session_path: {}", session.path);
            println!("LockedHint: {}", session.locked);
        }
        "unlock" => {
            let logind = LogindProbe::connect().await?;
            let before = logind.resolve_session().await?;
            logind.unlock_session(&before.id).await?;
            let after = logind.resolve_session().await?;
            println!(
                "requested UnlockSession({}); LockedHint before: {}; LockedHint now: {}",
                before.id, before.locked, after.locked
            );
        }
        "inhibit-suspend" => {
            let seconds = parse_seconds(args.next())?;
            let logind = LogindProbe::connect().await?;
            let _fd = logind.inhibit_suspend().await?;
            println!("holding logind sleep inhibitor for {seconds}s");
            tokio::time::sleep(Duration::from_secs(seconds)).await;
            println!("released logind sleep inhibitor");
        }
        "inhibit-lock" => {
            let seconds = parse_seconds(args.next())?;
            let screensaver = ScreensaverProbe::connect().await?;
            let cookie = screensaver.inhibit_lock().await?;
            println!("holding ScreenSaver inhibitor cookie {cookie} for {seconds}s");
            tokio::time::sleep(Duration::from_secs(seconds)).await;
            screensaver.uninhibit(cookie).await?;
            println!("released ScreenSaver inhibitor cookie {cookie}");
        }
        _ => {
            usage();
            bail!("unsupported subcommand: {command}");
        }
    }

    Ok(())
}

fn usage() {
    eprintln!(
        "usage: session_presence_probe <status|unlock|inhibit-suspend SECONDS|inhibit-lock SECONDS>"
    );
}

fn parse_seconds(value: Option<String>) -> Result<u64> {
    value
        .ok_or_else(|| anyhow!("missing duration in seconds"))?
        .parse::<u64>()
        .context("duration must be an integer number of seconds")
}

#[derive(Debug)]
struct ResolvedSession {
    id: String,
    path: OwnedObjectPath,
    locked: bool,
}

struct LogindProbe {
    connection: zbus::Connection,
}

impl LogindProbe {
    async fn connect() -> Result<Self> {
        Ok(Self {
            connection: zbus::Connection::system()
                .await
                .context("connect to system bus")?,
        })
    }

    async fn resolve_session(&self) -> Result<ResolvedSession> {
        let manager = self.manager_proxy().await?;
        let path: OwnedObjectPath = match manager.call("GetSession", &("auto",)).await {
            Ok(path) => path,
            Err(auto_error) => {
                if let Some(session_id) = std::env::var("XDG_SESSION_ID")
                    .ok()
                    .filter(|value| !value.trim().is_empty())
                {
                    manager
                        .call("GetSession", &(session_id.as_str(),))
                        .await
                        .with_context(|| {
                            format!(
                                "GetSession(auto) failed with {auto_error}; GetSession({session_id}) also failed"
                            )
                        })?
                } else {
                    let pid = std::process::id();
                    manager
                        .call("GetSessionByPID", &(pid,))
                        .await
                        .with_context(|| {
                            format!(
                                "GetSession(auto) failed with {auto_error}; GetSessionByPID({pid}) also failed"
                            )
                        })?
                }
            }
        };

        let session = self.session_proxy(&path).await?;
        let id: String = session
            .get_property("Id")
            .await
            .context("read session Id")?;
        let locked: bool = session
            .get_property("LockedHint")
            .await
            .context("read LockedHint")?;
        Ok(ResolvedSession { id, path, locked })
    }

    async fn unlock_session(&self, session_id: &str) -> Result<()> {
        let _: () = self
            .manager_proxy()
            .await?
            .call("UnlockSession", &(session_id,))
            .await
            .with_context(|| format!("UnlockSession({session_id})"))?;
        Ok(())
    }

    async fn inhibit_suspend(&self) -> Result<OwnedFd> {
        self.manager_proxy()
            .await?
            .call(
                "Inhibit",
                &("sleep", "sky-cua", "automation session active", "block"),
            )
            .await
            .context("acquire logind sleep inhibitor")
    }

    async fn manager_proxy(&self) -> Result<Proxy<'_>> {
        Proxy::new(
            &self.connection,
            LOGIND_DEST,
            LOGIND_MANAGER_PATH,
            LOGIND_MANAGER_IFACE,
        )
        .await
        .context("create logind manager proxy")
    }

    async fn session_proxy(&self, path: &OwnedObjectPath) -> Result<Proxy<'_>> {
        Proxy::new(
            &self.connection,
            LOGIND_DEST,
            path.clone(),
            LOGIND_SESSION_IFACE,
        )
        .await
        .with_context(|| format!("create logind session proxy for {path}"))
    }
}

struct ScreensaverProbe {
    connection: zbus::Connection,
}

impl ScreensaverProbe {
    async fn connect() -> Result<Self> {
        Ok(Self {
            connection: zbus::Connection::session()
                .await
                .context("connect to session bus")?,
        })
    }

    async fn inhibit_lock(&self) -> Result<u32> {
        self.proxy()
            .await?
            .call("Inhibit", &("sky-cua", "automation session active"))
            .await
            .context("acquire ScreenSaver inhibitor")
    }

    async fn uninhibit(&self, cookie: u32) -> Result<()> {
        let _: () = self
            .proxy()
            .await?
            .call("UnInhibit", &(cookie,))
            .await
            .with_context(|| format!("release ScreenSaver inhibitor cookie {cookie}"))?;
        Ok(())
    }

    async fn proxy(&self) -> Result<Proxy<'_>> {
        Proxy::new(
            &self.connection,
            SCREENSAVER_DEST,
            SCREENSAVER_PATH,
            SCREENSAVER_IFACE,
        )
        .await
        .context("create ScreenSaver proxy")
    }
}
