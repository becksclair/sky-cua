use std::collections::HashMap;

use anyhow::{Context, Result, anyhow};
use chrono::Utc;
use sky_cua_platform::new_snapshot_id;
use zbus::{
    Proxy,
    zvariant::{OwnedValue, Value},
};

use crate::portal::session::{portal_u32_property, session_bus};
use crate::portal::token_store::{
    PersistedPortalToken, PortalCompositorFamily, PortalTokenStore, current_compositor_hint,
    portal_compositor_family,
};

const PERMISSION_STORE_BUS_NAME: &str = "org.freedesktop.impl.portal.PermissionStore";
const PERMISSION_STORE_OBJECT_PATH: &str = "/org/freedesktop/impl/portal/PermissionStore";
const PERMISSION_STORE_INTERFACE: &str = "org.freedesktop.impl.portal.PermissionStore";
const GNOME_DISPLAY_CONFIG_BUS_NAME: &str = "org.gnome.Mutter.DisplayConfig";
const GNOME_DISPLAY_CONFIG_OBJECT_PATH: &str = "/org/gnome/Mutter/DisplayConfig";
const GNOME_DISPLAY_CONFIG_INTERFACE: &str = "org.gnome.Mutter.DisplayConfig";
const KDE_AUTHORIZED_TABLE: &str = "kde-authorized";
const REMOTE_DESKTOP_TABLE: &str = "remote-desktop";
const REMOTE_DESKTOP_ID: &str = "remote-desktop";
const ALLOW_PERMISSION: &str = "yes";
const KDE_REMOTE_DESKTOP_APP_IDS: &[&str] = &["", "desktop"];
const GNOME_RESTORE_BACKEND_NAME: &str = "GNOME";
const GNOME_RESTORE_VERSION: u32 = 1;
const GNOME_DEFAULT_DEVICE_TYPES: u32 = 3;
const GNOME_DEFAULT_STREAM_ID: u32 = 0;
const GNOME_DEFAULT_MAPPING_ID: u32 = 1;

type PortalPermissions = HashMap<String, Vec<String>>;
type Properties = HashMap<String, OwnedValue>;
type MonitorSpec = (String, String, String, String);
type MonitorMode = (String, i32, i32, f64, f64, Vec<f64>, Properties);
type Monitor = (MonitorSpec, Vec<MonitorMode>, Properties);
type LogicalMonitor = (i32, i32, f64, u32, bool, Vec<MonitorSpec>, Properties);
type DisplayConfigState = (u32, Vec<Monitor>, Vec<LogicalMonitor>, Properties);

#[derive(Debug, Clone, PartialEq, Eq)]
struct MonitorIdentity {
    connector: String,
    vendor: String,
    product: String,
    serial: String,
}

impl MonitorIdentity {
    fn match_string(&self) -> String {
        if self.vendor == "unknown" && self.product == "unknown" && self.serial == "unknown" {
            self.connector.clone()
        } else {
            format!("{}:{}:{}", self.vendor, self.product, self.serial)
        }
    }
}

pub(crate) async fn preauthorize_remote_desktop(token_store: Option<&PortalTokenStore>) {
    let connection = match session_bus().await {
        Ok(connection) => connection,
        Err(error) => {
            tracing::debug!(
                message = %error.message,
                "skipping RemoteDesktop portal preauthorization; session bus is unavailable"
            );
            return;
        }
    };

    match preauthorize_kde_remote_desktop(&connection).await {
        Ok(()) => tracing::info!("preauthorized KDE RemoteDesktop portal permission"),
        Err(error) => tracing::debug!(
            message = %error,
            "skipping KDE RemoteDesktop portal preauthorization"
        ),
    }

    if should_attempt_gnome_preauthorization() {
        match preauthorize_gnome_remote_desktop(&connection, token_store).await {
            Ok(details) => tracing::info!(
                token_path = details.token_path.as_deref().unwrap_or("<unavailable>"),
                monitor_match = details.monitor_match,
                "preauthorized GNOME RemoteDesktop portal restore token"
            ),
            Err(error) => tracing::debug!(
                message = %error,
                "skipping GNOME RemoteDesktop portal preauthorization"
            ),
        }
    } else {
        tracing::debug!(
            compositor = current_compositor_hint().as_deref().unwrap_or("<unknown>"),
            "skipping GNOME RemoteDesktop portal preauthorization on a non-GNOME compositor"
        );
    }
}

async fn preauthorize_kde_remote_desktop(connection: &zbus::Connection) -> Result<()> {
    let proxy = permission_store_proxy(connection).await?;
    let permissions = vec![ALLOW_PERMISSION.to_string()];
    for app_id in KDE_REMOTE_DESKTOP_APP_IDS {
        let _: () = proxy
            .call(
                "SetPermission",
                &(
                    KDE_AUTHORIZED_TABLE,
                    true,
                    REMOTE_DESKTOP_ID,
                    app_id,
                    permissions.clone(),
                ),
            )
            .await
            .with_context(|| {
                format!("failed to set KDE RemoteDesktop authorization for app_id={app_id:?}")
            })?;
    }

    let (roundtrip_permissions, _data): (PortalPermissions, OwnedValue) = proxy
        .call("Lookup", &(KDE_AUTHORIZED_TABLE, REMOTE_DESKTOP_ID))
        .await
        .context("failed to verify KDE RemoteDesktop authorization")?;
    let missing_app_ids = KDE_REMOTE_DESKTOP_APP_IDS
        .iter()
        .copied()
        .filter(|app_id| {
            let granted_permissions = roundtrip_permissions
                .get(*app_id)
                .cloned()
                .unwrap_or_default();
            !granted_permissions
                .iter()
                .any(|permission| permission == ALLOW_PERMISSION)
        })
        .collect::<Vec<_>>();
    if !missing_app_ids.is_empty() {
        return Err(anyhow!(
            "KDE RemoteDesktop authorization did not round-trip for app ids {missing_app_ids:?}: {roundtrip_permissions:?}"
        ));
    }

    Ok(())
}

#[derive(Debug)]
struct GnomePreauthorizationDetails {
    monitor_match: String,
    token_path: Option<String>,
}

async fn preauthorize_gnome_remote_desktop(
    connection: &zbus::Connection,
    token_store: Option<&PortalTokenStore>,
) -> Result<GnomePreauthorizationDetails> {
    let token_store = token_store.context("persisted portal token storage is unavailable")?;
    let token = reusable_gnome_token(token_store).unwrap_or_else(new_snapshot_id);
    let monitor_match = primary_gnome_monitor_identity(connection)
        .await?
        .match_string();
    let restore_data = gnome_restore_data(
        &monitor_match,
        GNOME_DEFAULT_DEVICE_TYPES,
        false,
        Utc::now().timestamp_micros(),
    );
    let proxy = permission_store_proxy(connection).await?;
    let permissions = HashMap::from([(String::new(), vec![ALLOW_PERMISSION.to_string()])]);
    let _: () = proxy
        .call(
            "Set",
            &(
                REMOTE_DESKTOP_TABLE,
                true,
                token.as_str(),
                permissions,
                restore_data,
            ),
        )
        .await
        .context("failed to seed GNOME RemoteDesktop restore data")?;
    let _: (PortalPermissions, OwnedValue) = proxy
        .call("Lookup", &(REMOTE_DESKTOP_TABLE, token.as_str()))
        .await
        .context("failed to verify GNOME RemoteDesktop restore data")?;

    let record = PersistedPortalToken {
        restore_token: token,
        updated_at: Utc::now(),
        xdg_session_type: std::env::var("XDG_SESSION_TYPE").ok(),
        compositor: gnome_compositor_hint(),
        remote_desktop_version: portal_u32_property(
            "org.freedesktop.portal.RemoteDesktop",
            "version",
        )
        .await
        .ok(),
        screencast_version: portal_u32_property("org.freedesktop.portal.ScreenCast", "version")
            .await
            .ok(),
    };
    token_store.save(&record).with_context(|| {
        format!(
            "failed to persist GNOME RemoteDesktop restore token to {}",
            token_store.path().display()
        )
    })?;

    Ok(GnomePreauthorizationDetails {
        monitor_match,
        token_path: Some(token_store.path().display().to_string()),
    })
}

async fn permission_store_proxy(connection: &zbus::Connection) -> Result<Proxy<'_>> {
    Proxy::new(
        connection,
        PERMISSION_STORE_BUS_NAME,
        PERMISSION_STORE_OBJECT_PATH,
        PERMISSION_STORE_INTERFACE,
    )
    .await
    .context("failed to create PermissionStore proxy")
}

async fn primary_gnome_monitor_identity(connection: &zbus::Connection) -> Result<MonitorIdentity> {
    let proxy = Proxy::new(
        connection,
        GNOME_DISPLAY_CONFIG_BUS_NAME,
        GNOME_DISPLAY_CONFIG_OBJECT_PATH,
        GNOME_DISPLAY_CONFIG_INTERFACE,
    )
    .await
    .context("failed to create GNOME Mutter DisplayConfig proxy")?;
    let (_serial, monitors, logical_monitors, _properties): DisplayConfigState = proxy
        .call("GetCurrentState", &())
        .await
        .context("GNOME Mutter DisplayConfig GetCurrentState call failed")?;
    primary_monitor_identity_from_state(&monitors, &logical_monitors)
}

fn primary_monitor_identity_from_state(
    monitors: &[Monitor],
    logical_monitors: &[LogicalMonitor],
) -> Result<MonitorIdentity> {
    let monitor_by_connector = monitors
        .iter()
        .map(|monitor| {
            let identity = monitor_identity(monitor);
            (identity.connector.clone(), identity)
        })
        .collect::<HashMap<_, _>>();

    for logical_monitor in logical_monitors {
        let primary = logical_monitor.4;
        let logical_monitor_specs = &logical_monitor.5;
        if let Some((connector, _vendor, _product, _serial)) = primary
            .then_some(logical_monitor_specs)
            .and_then(|specs| specs.first())
            && let Some(identity) = monitor_by_connector.get(connector)
        {
            return Ok(identity.clone());
        }
    }

    monitors
        .first()
        .map(monitor_identity)
        .ok_or_else(|| anyhow!("GNOME Mutter DisplayConfig did not report any monitors"))
}

fn monitor_identity(monitor: &Monitor) -> MonitorIdentity {
    let (connector, vendor, product, serial) = &monitor.0;
    MonitorIdentity {
        connector: connector.clone(),
        vendor: vendor.clone(),
        product: product.clone(),
        serial: serial.clone(),
    }
}

fn reusable_gnome_token(token_store: &PortalTokenStore) -> Option<String> {
    let record = token_store.load().ok().flatten()?;
    if record.restore_token.trim().is_empty() {
        return None;
    }
    if record
        .compositor
        .as_deref()
        .and_then(portal_compositor_family)
        == Some(PortalCompositorFamily::Kde)
    {
        return None;
    }
    Some(record.restore_token)
}

fn gnome_compositor_hint() -> Option<String> {
    match current_compositor_hint() {
        Some(compositor)
            if portal_compositor_family(&compositor) == Some(PortalCompositorFamily::Gnome) =>
        {
            Some(compositor)
        }
        _ => Some("gnome-mutter".to_string()),
    }
}

fn should_attempt_gnome_preauthorization() -> bool {
    let Some(compositor) = current_compositor_hint() else {
        return true;
    };
    portal_compositor_family(&compositor) == Some(PortalCompositorFamily::Gnome)
}

fn gnome_restore_data(
    monitor_match: &str,
    device_types: u32,
    clipboard_enabled: bool,
    now_micros: i64,
) -> Value<'static> {
    let streams = vec![(
        GNOME_DEFAULT_STREAM_ID,
        GNOME_DEFAULT_MAPPING_ID,
        Value::new(monitor_match.to_string()),
    )];
    let impl_data = (
        now_micros,
        now_micros,
        device_types,
        clipboard_enabled,
        streams,
    );
    Value::new((
        GNOME_RESTORE_BACKEND_NAME,
        GNOME_RESTORE_VERSION,
        Value::new(impl_data),
    ))
}

#[cfg(test)]
mod tests {
    use super::{
        DisplayConfigState, GNOME_RESTORE_BACKEND_NAME, GNOME_RESTORE_VERSION, LogicalMonitor,
        Monitor, MonitorIdentity, gnome_restore_data, primary_monitor_identity_from_state,
    };
    use std::collections::HashMap;
    use zbus::zvariant::{Structure, Value};

    fn monitor(connector: &str, vendor: &str, product: &str, serial: &str) -> Monitor {
        (
            (
                connector.to_string(),
                vendor.to_string(),
                product.to_string(),
                serial.to_string(),
            ),
            Vec::new(),
            HashMap::new(),
        )
    }

    fn logical_monitor(connector: &str, primary: bool) -> LogicalMonitor {
        (
            0,
            0,
            1.0,
            0,
            primary,
            vec![(
                connector.to_string(),
                "vendor".to_string(),
                "product".to_string(),
                "serial".to_string(),
            )],
            HashMap::new(),
        )
    }

    #[test]
    fn primary_monitor_identity_prefers_primary_logical_monitor() {
        let monitors = vec![
            monitor("Virtual-1", "unknown", "unknown", "unknown"),
            monitor("Virtual-2", "Acme", "Panel", "42"),
        ];
        let logical_monitors = vec![
            logical_monitor("Virtual-1", false),
            logical_monitor("Virtual-2", true),
        ];

        assert_eq!(
            primary_monitor_identity_from_state(&monitors, &logical_monitors)
                .expect("primary monitor should resolve"),
            MonitorIdentity {
                connector: "Virtual-2".to_string(),
                vendor: "Acme".to_string(),
                product: "Panel".to_string(),
                serial: "42".to_string(),
            }
        );
    }

    #[test]
    fn monitor_identity_match_string_uses_connector_for_unknown_edid() {
        let identity = MonitorIdentity {
            connector: "Virtual-1".to_string(),
            vendor: "unknown".to_string(),
            product: "unknown".to_string(),
            serial: "unknown".to_string(),
        };

        assert_eq!(identity.match_string(), "Virtual-1");
    }

    #[test]
    fn gnome_restore_data_matches_expected_shape() {
        let restore_data = gnome_restore_data("Virtual-1", 3, false, 123);
        assert_eq!(restore_data.value_signature().to_string(), "(suv)");
        let mut fields = Structure::try_from(restore_data)
            .expect("restore data should be a structure")
            .into_fields();
        let backend = String::try_from(fields.remove(0)).expect("backend should unpack");
        let version = u32::try_from(fields.remove(0)).expect("version should unpack");
        let Value::Value(impl_data) = fields.remove(0) else {
            panic!("implementation restore data should be wrapped in a variant");
        };
        assert_eq!(backend, GNOME_RESTORE_BACKEND_NAME);
        assert_eq!(version, GNOME_RESTORE_VERSION);
        assert_eq!(impl_data.value_signature().to_string(), "(xxuba(uuv))");
    }

    #[test]
    fn display_config_state_type_compiles_for_mutter_signature() {
        let state: DisplayConfigState = (
            1,
            vec![monitor("Virtual-1", "unknown", "unknown", "unknown")],
            vec![logical_monitor("Virtual-1", true)],
            HashMap::new(),
        );

        assert_eq!(state.0, 1);
    }
}
