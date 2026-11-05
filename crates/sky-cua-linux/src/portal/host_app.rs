//! Stable portal identity for the daemon.
//!
//! xdg-desktop-portal identifies an unsandboxed caller by the systemd scope it
//! runs in (`app-com.anthropic.Claude-1234.scope` -> `com.anthropic.Claude`,
//! a plain shell -> `""`), so the daemon's portal app id used to change with
//! whichever host spawned it. Restore tokens are owned per app id: a daemon
//! started from another scope could not restore the stored token, received a
//! fresh grant, and overwrote the file with a token the next daemon could not
//! use either, which on KDE meant an interactive approval dialog.
//!
//! Registering through `org.freedesktop.host.portal.Registry` pins the id for
//! the daemon's portal connection. The portal requires a resolvable
//! `<id>.desktop` entry, which the installer projects; without it (checkout
//! runs) registration fails and the scope-derived id is used as before.

use std::time::Duration;

use tokio::sync::OnceCell;
use tracing::{debug, warn};

/// The app id sky-cua registers with the portal. The installer writes the
/// matching hidden `com.heliasar.SkyCua.desktop` entry.
pub(crate) const PORTAL_APP_ID: &str = "com.heliasar.SkyCua";

const REGISTER_TIMEOUT: Duration = Duration::from_secs(2);

static REGISTERED: OnceCell<()> = OnceCell::const_new();

/// Register the daemon's shared portal connection as [`PORTAL_APP_ID`] once.
///
/// Must run before the first ashpd portal request on that connection: the
/// portal rejects registration for a connection it has already identified.
/// Failure is logged and tolerated; the portal then falls back to the
/// scope-derived id.
pub(crate) async fn ensure_registered() {
    REGISTERED
        .get_or_init(|| async {
            let app_id = match ashpd::AppID::try_from(PORTAL_APP_ID) {
                Ok(app_id) => app_id,
                Err(error) => {
                    warn!(%error, app_id = PORTAL_APP_ID, "invalid portal app id");
                    return;
                }
            };
            match tokio::time::timeout(REGISTER_TIMEOUT, ashpd::register_host_app(app_id)).await {
                Ok(Ok(())) => debug!(app_id = PORTAL_APP_ID, "registered portal host app id"),
                Ok(Err(error)) => warn!(
                    %error,
                    app_id = PORTAL_APP_ID,
                    "portal host app registration failed; restore tokens stay tied to the spawning scope"
                ),
                Err(_) => warn!(
                    app_id = PORTAL_APP_ID,
                    "portal host app registration timed out; restore tokens stay tied to the spawning scope"
                ),
            }
        })
        .await;
}
