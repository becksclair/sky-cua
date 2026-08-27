//! Machine-level sky-cua configuration.
//!
//! Per-machine settings live in one TOML file instead of being baked into
//! every MCP host registration's environment:
//!
//! - Linux/macOS: `$XDG_CONFIG_HOME/sky-cua/sky-cua.toml`
//!   (default `~/.config/sky-cua/sky-cua.toml`)
//! - Windows: `%APPDATA%\sky-cua\sky-cua.toml`
//!
//! Environment variables remain per-process overrides on top of the file
//! (e.g. `SKY_CUA_BROWSER` beats the file's `browser`). Browser selection is
//! resolved on use. Longer-lived subsystem managers, such as phone-use, resolve
//! their selection when the daemon constructs them, so changed values apply after
//! rebuilding that manager/restarting the runtime. Tests can point
//! `SKY_CUA_CONFIG_PATH` at fixtures.

use std::{collections::BTreeMap, path::PathBuf};

use serde::Deserialize;

/// Test/operator override for the machine config file location.
pub const MACHINE_CONFIG_PATH_ENV: &str = "SKY_CUA_CONFIG_PATH";
pub const REPO_ROOT_ENV: &str = "SKY_CUA_REPO_ROOT";

pub const BROWSER_SELECTION_ENV: &str = "SKY_CUA_BROWSER";
pub const AGENT_SURFACES_ENV: &str = "SKY_CUA_SURFACES";
pub const BROWSER_CONTROL_MODE_ENV: &str = "SKY_CUA_BROWSER_CONTROL_MODE";
pub const CODEX_BROWSER_SOCKET_PATH_ENV: &str = "SKY_CUA_CODEX_BROWSER_SOCKET_PATH";

// Phone-use environment overrides. Each beats the matching `[phone]` config key
// for this process. Names are the public contract surface and are allowlisted in
// `.mcp.json`.
pub const PHONE_ENABLED_ENV: &str = "SKY_CUA_PHONE";
pub const PHONE_SERIAL_ENV: &str = "SKY_CUA_PHONE_SERIAL";
pub const PHONE_BACKEND_ENV: &str = "SKY_CUA_PHONE_BACKEND";
pub const PHONE_ADB_ENV: &str = "SKY_CUA_ADB";
pub const PHONE_SCRCPY_ENV: &str = "SKY_CUA_SCRCPY";
pub const PHONE_WIRELESS_AUTO_CONNECT_ENV: &str = "SKY_CUA_PHONE_WIRELESS_AUTO_CONNECT";
pub const PHONE_VISIBLE_OVERLAY_ENV: &str = "SKY_CUA_PHONE_VISIBLE_OVERLAY";
pub const PHONE_SCREENSHOT_CURSOR_ENV: &str = "SKY_CUA_PHONE_SCREENSHOT_CURSOR";
pub const PHONE_V4L2_SINK_ENV: &str = "SKY_CUA_PHONE_V4L2_SINK";
pub const PHONE_COMPANION_ENV: &str = "SKY_CUA_PHONE_COMPANION";
pub const PHONE_COMPANION_AUTO_INSTALL_ENV: &str = "SKY_CUA_PHONE_COMPANION_AUTO_INSTALL";
pub const PHONE_COMPANION_OPERATOR_MODE_ENV: &str = "SKY_CUA_PHONE_COMPANION_OPERATOR_MODE";
pub const PHONE_COMPANION_PACKAGE_ENV: &str = "SKY_CUA_PHONE_COMPANION_PACKAGE";
pub const PHONE_COMPANION_APK_ENV: &str = "SKY_CUA_PHONE_COMPANION_APK";
pub const PHONE_COMPANION_CERT_SHA256_ENV: &str = "SKY_CUA_PHONE_COMPANION_CERT_SHA256";
pub const PHONE_COMPANION_APK_SHA256_ENV: &str = "SKY_CUA_PHONE_COMPANION_APK_SHA256";
pub const PHONE_COMPANION_ALLOW_DOWNGRADE_ENV: &str = "SKY_CUA_PHONE_COMPANION_ALLOW_DOWNGRADE";
pub const PHONE_CAPABILITY_CACHE_TTL_MS_ENV: &str = "SKY_CUA_PHONE_CAPABILITY_CACHE_TTL_MS";
pub const PHONE_APPSHOT_TTL_MS_ENV: &str = "SKY_CUA_PHONE_APPSHOT_TTL_MS";
pub const PHONE_TARGET_MODELS_ENV: &str = "SKY_CUA_PHONE_TARGET_MODELS";
pub const PHONE_DIRECT_ENABLED_ENV: &str = "SKY_CUA_PHONE_DIRECT";
pub const PHONE_DIRECT_LISTEN_ADDR_ENV: &str = "SKY_CUA_PHONE_DIRECT_LISTEN_ADDR";
pub const PHONE_DIRECT_ADVERTISED_ENDPOINT_ENV: &str = "SKY_CUA_PHONE_DIRECT_ADVERTISED_ENDPOINT";
pub const PHONE_DIRECT_ENROLLMENT_TTL_MS_ENV: &str = "SKY_CUA_PHONE_DIRECT_ENROLLMENT_TTL_MS";
pub const PHONE_DIRECT_STATE_PATH_ENV: &str = "SKY_CUA_PHONE_DIRECT_STATE_PATH";

// Isolated-desktop environment overrides. Each beats the matching
// `[isolated_desktop]` config key for this process. Names are the public
// contract surface and are allowlisted in `.mcp.json`.
pub const ISOLATED_DESKTOP_ENABLED_ENV: &str = "SKY_CUA_ISOLATED_DESKTOP";
pub const ISOLATED_DESKTOP_DISPLAY_ENV: &str = "SKY_CUA_ISOLATED_DESKTOP_DISPLAY";
pub const ISOLATED_DESKTOP_RESOLUTION_ENV: &str = "SKY_CUA_ISOLATED_DESKTOP_RESOLUTION";
pub const ISOLATED_DESKTOP_VIEWER_ENV: &str = "SKY_CUA_ISOLATED_DESKTOP_VIEWER";
pub const ISOLATED_DESKTOP_LIFECYCLE_ENV: &str = "SKY_CUA_ISOLATED_DESKTOP_LIFECYCLE";
pub const ISOLATED_DESKTOP_WINDOW_MANAGER_ENV: &str = "SKY_CUA_ISOLATED_DESKTOP_WINDOW_MANAGER";

// Cross-cutting env keys declared once here and re-exported (via
// `pub(crate) use ... as ...;`) by every crate that previously declared its
// own independent copy of the same string. Keep additions here even when the
// only consumer today is a single crate outside `sky-cua-platform`, so a
// second consumer never has to re-declare the literal.
/// Overlay-host backend selection, read by both the service's overlay
/// connection manager and the overlay-host process itself.
pub const OVERLAY_BACKEND_ENV: &str = "SKY_CUA_OVERLAY_BACKEND";
/// Unix socket path for the privileged Linux input helper, read by the
/// virtual-input backend (client of the helper) and the overlay-host's
/// pointer-tracking module (which also talks to the helper for exact hover).
pub const INPUT_HELPER_SOCKET_ENV: &str = "SKY_CUA_INPUT_HELPER_SOCKET";
/// Model-facing screenshot re-encode container, shared by the desktop capture
/// pipeline and the browser viewport capture pipeline.
pub const MODEL_SCREENSHOT_FORMAT_ENV: &str = "SKY_CUA_MODEL_SCREENSHOT_FORMAT";
/// Model-facing JPEG encode quality (0-100), shared by the desktop capture
/// pipeline and the browser viewport capture pipeline.
pub const MODEL_SCREENSHOT_JPEG_QUALITY_ENV: &str = "SKY_CUA_MODEL_SCREENSHOT_JPEG_QUALITY";
/// Model-facing WebP encode quality (0-100), shared by the desktop capture
/// pipeline and the browser viewport capture pipeline.
pub const MODEL_SCREENSHOT_WEBP_QUALITY_ENV: &str = "SKY_CUA_MODEL_SCREENSHOT_WEBP_QUALITY";
/// Browser-use bridge socket directory override, shared by the service's
/// browser socket discovery and the standalone Chrome extension host process.
pub const BROWSER_USE_SOCKET_DIR_ENV: &str = "SKY_CUA_BROWSER_USE_SOCKET_DIR";

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct MachineConfig {
    /// Chrome-family browser selection: `brave`, `chrome`, `chromium`, or
    /// `all`. Unset means probe every Chrome-family browser.
    pub browser: Option<String>,
    /// Agent-facing MCP surfaces. Absent means all three are enabled.
    #[serde(default)]
    pub surfaces: AgentSurfaceConfig,
    /// Durable ownership for the service-level browser control ingress.
    #[serde(default)]
    pub browser_control: BrowserControlConfig,
    /// Phone-use `[phone]` table. Absent means defaults plus env overrides.
    #[serde(default)]
    pub phone: PhoneConfig,
    /// Isolated-desktop `[isolated_desktop]` table. Absent means defaults plus
    /// env overrides.
    #[serde(default)]
    pub isolated_desktop: IsolatedDesktopConfig,
}

/// Parsed `[surfaces]` table. Every field defaults to enabled.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentSurfaceConfig {
    pub desktop: Option<bool>,
    pub browser: Option<bool>,
    pub phone: Option<bool>,
}

/// Fully resolved agent-facing surface policy. The MCP process snapshots this
/// at initialize time; provisioning reads only the durable table and ignores
/// process-scoped overrides.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AgentSurfacePolicy {
    pub desktop: bool,
    pub browser: bool,
    pub phone: bool,
}

impl Default for AgentSurfacePolicy {
    fn default() -> Self {
        Self {
            desktop: true,
            browser: true,
            phone: true,
        }
    }
}

/// Parsed `[browser_control]` table. Environment values override these fields
/// independently, so one MCP client cannot erase a durable machine setting by
/// omitting it from that client's ambient environment.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct BrowserControlConfig {
    pub mode: Option<String>,
    pub codex_socket_path: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BrowserControlMode {
    Legacy,
    Hybrid,
    Strict,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResolvedBrowserControlConfig {
    pub mode: Option<BrowserControlMode>,
    pub codex_socket_path: Option<String>,
}

/// Parsed `[phone]` table. Every field is optional in the file; defaults and
/// per-process env overrides are layered on by [`PhoneConfig::resolved`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct PhoneConfig {
    pub enabled: Option<bool>,
    pub adb_path: Option<String>,
    pub scrcpy_path: Option<String>,
    pub default_serial: Option<String>,
    pub default_backend: Option<String>,
    pub wireless_auto_connect: Option<bool>,
    pub window_width: Option<u32>,
    pub window_height: Option<u32>,
    pub max_size: Option<u32>,
    pub max_fps: Option<u32>,
    pub video_codec: Option<String>,
    pub v4l2_sink: Option<String>,
    pub visible_overlay: Option<bool>,
    pub screenshot_cursor: Option<bool>,
    pub companion_enabled: Option<bool>,
    pub companion_auto_install: Option<bool>,
    pub companion_operator_mode: Option<bool>,
    pub companion_package: Option<String>,
    pub companion_apk_path: Option<String>,
    pub companion_expected_cert_sha256: Option<String>,
    pub companion_apk_sha256: Option<String>,
    pub companion_allow_downgrade: Option<bool>,
    pub capability_cache_ttl_ms: Option<u64>,
    pub appshot_ttl_ms: Option<u64>,
    pub direct_enabled: Option<bool>,
    pub direct_listen_addr: Option<String>,
    pub direct_advertised_endpoint: Option<String>,
    pub direct_enrollment_ttl_ms: Option<u64>,
    pub direct_state_path: Option<String>,
    /// Named, non-secret direct-device profiles used by observation-only
    /// phone data lanes such as SMS. Profiles are never inferred from the
    /// default serial or an active session.
    #[serde(default)]
    pub profiles: BTreeMap<String, PhoneProfileConfig>,
    #[serde(default)]
    pub primary_target_models: Vec<String>,
    /// Human-readable aliases for enrolled devices (`alias -> device_id`
    /// for CompanionDirect or `alias -> serial` for ADB). Agents can
    /// connect/select by alias instead of the raw identifier. Values are
    /// resolved at connect time; an unknown alias fails closed.
    #[serde(default)]
    pub aliases: BTreeMap<String, String>,
}

/// Non-secret operator selection for one named phone profile. Credentials stay
/// in the direct-link state store; this table only selects an enrolled device
/// and its explicitly allowed, read-only capabilities.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PhoneProfileConfig {
    pub device_id: String,
    pub transport: String,
    pub access: String,
    #[serde(default)]
    pub required_capabilities: Vec<String>,
}

/// Default companion package id when neither config nor env overrides it.
pub const PHONE_DEFAULT_COMPANION_PACKAGE: &str = "com.skycua.phonecompanion";
/// Default companion APK path relative to the repo/payload root.
pub const PHONE_DEFAULT_COMPANION_APK_PATH: &str = "resources/android/phone-companion.apk";
/// Default capability-cache soft refresh hint (30 seconds).
pub const PHONE_DEFAULT_CAPABILITY_CACHE_TTL_MS: u64 = 30_000;
/// Default AppShot freshness window (30 seconds). Tunable via
/// `SKY_CUA_PHONE_APPSHOT_TTL_MS` / `[phone].appshot_ttl_ms`.
pub const PHONE_DEFAULT_APPSHOT_TTL_MS: u64 = 30_000;
/// Direct enrollment codes are single-use and expire after five minutes.
pub const PHONE_DEFAULT_DIRECT_ENROLLMENT_TTL_MS: u64 = 300_000;

/// Parsed `[isolated_desktop]` table. Every field is optional in the file;
/// defaults and per-process env overrides are layered on by
/// [`resolve_isolated_desktop`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct IsolatedDesktopConfig {
    pub enabled: Option<bool>,
    /// `":100"` (default) or the literal `"auto"`, which a downstream module
    /// resolves to a free display number.
    pub display: Option<String>,
    /// Virtual display geometry, e.g. `"1920x1080"`, or the literal `"auto"` (the
    /// default) which the client sizes to 3/4 of the largest connected monitor.
    pub resolution: Option<String>,
    /// Window manager started inside the isolated desktop, e.g. `"openbox"`.
    pub window_manager: Option<String>,
    /// Read-only viewer mode: `"attach"`, `"html5"`, or `"none"`.
    pub viewer: Option<String>,
    /// Session lifecycle: `"persistent"` or `"ephemeral"`.
    pub lifecycle: Option<String>,
}

/// Default isolated-desktop X11 display when neither config nor env overrides it.
pub const ISOLATED_DESKTOP_DEFAULT_DISPLAY: &str = ":100";
/// Default isolated-desktop virtual display geometry: the literal `"auto"`, which
/// the client resolves to three-quarters of the largest connected monitor so the
/// read-only viewer is a comfortable window on the user's real screen. An explicit
/// `"<width>x<height>"` overrides it.
pub const ISOLATED_DESKTOP_DEFAULT_RESOLUTION: &str = "auto";
/// Default isolated-desktop window manager.
pub const ISOLATED_DESKTOP_DEFAULT_WINDOW_MANAGER: &str = "openbox";

/// Read-only viewer surfaced for the isolated desktop. Unrecognized strings fall
/// back to [`ViewerMode::Attach`] during resolution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewerMode {
    /// `xpra attach :N --readonly` window on the user's real screen.
    Attach,
    /// xpra HTML5 listener; the client logs the URL.
    Html5,
    /// No viewer is launched.
    None,
}

/// Isolated-desktop lifecycle. Unrecognized strings fall back to
/// [`Lifecycle::Persistent`] during resolution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lifecycle {
    /// The xpra session is reused idempotently across agent sessions and torn
    /// down only on explicit request.
    Persistent,
    /// The xpra session is torn down when the client exits.
    Ephemeral,
}

/// Fully resolved isolated-desktop selection: file values overlaid with
/// per-process environment overrides and concrete defaults. The literal display
/// string `"auto"` is preserved verbatim (a downstream module picks a free
/// display number), as is a `resolution` of `"auto"` (the client sizes it to 3/4
/// of the largest monitor). This is the shape the client consumes; it never
/// re-reads the file or env directly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedIsolatedDesktop {
    pub enabled: bool,
    pub display: String,
    pub resolution: String,
    pub window_manager: String,
    pub viewer: ViewerMode,
    pub lifecycle: Lifecycle,
}

/// Fully resolved phone-use selection: file values overlaid with per-process
/// environment overrides and concrete defaults. This is the shape backends and
/// the MCP client consume; they never re-read the file or env directly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedPhoneSelection {
    pub enabled: bool,
    pub adb_path: Option<String>,
    pub scrcpy_path: Option<String>,
    pub default_serial: Option<String>,
    pub default_backend: Option<String>,
    pub wireless_auto_connect: bool,
    pub visible_overlay: bool,
    pub screenshot_cursor: bool,
    pub v4l2_sink: Option<String>,
    /// scrcpy `--window-width`, from `[phone].window_width`. Config-only.
    pub window_width: Option<u32>,
    /// scrcpy `--window-height`, from `[phone].window_height`. Config-only.
    pub window_height: Option<u32>,
    /// scrcpy `--max-size`, from `[phone].max_size`. Config-only.
    pub max_size: Option<u32>,
    /// scrcpy `--max-fps`, from `[phone].max_fps`. Config-only.
    pub max_fps: Option<u32>,
    /// Preferred scrcpy video codec, from `[phone].video_codec`. Config-only.
    pub video_codec: Option<String>,
    pub companion_enabled: bool,
    pub companion_auto_install: bool,
    pub companion_operator_mode: bool,
    pub companion_package: String,
    pub companion_apk_path: String,
    pub companion_expected_cert_sha256: Option<String>,
    /// Expected packaged-APK SHA-256 from build metadata; compared against the
    /// installed package for defense-in-depth alongside the signing cert.
    pub companion_apk_sha256: Option<String>,
    pub companion_allow_downgrade: bool,
    pub capability_cache_ttl_ms: u64,
    pub appshot_ttl_ms: u64,
    pub primary_target_models: Vec<String>,
    /// Explicit opt-in for the phone-initiated `phone-control.v2` listener.
    pub direct_enabled: bool,
    /// Concrete tailnet address to bind. The listener rejects wildcard/public
    /// binds during startup; loopback is accepted for deterministic tests.
    pub direct_listen_addr: Option<String>,
    /// MagicDNS WebSocket endpoint encoded into enrollment QR payloads.
    pub direct_advertised_endpoint: Option<String>,
    pub direct_enrollment_ttl_ms: u64,
    pub direct_state_path: Option<String>,
    pub profiles: BTreeMap<String, PhoneProfileConfig>,
    pub aliases: BTreeMap<String, String>,
}

/// Every `SKY_CUA_*` environment key declared as a `pub const *_ENV` in this
/// crate (`config.rs`, `paths.rs`, and the crate root). This is the set of
/// keys `sky-cua-platform` owns as the canonical declaration site; it does
/// NOT include keys declared only in downstream crates (`sky-cua-service`,
/// `sky-cua-capture`, `sky-cua-overlay-host`, `sky-cua-chrome-host`,
/// `sky-cua-input-helper`), since this crate has no dependency on them.
/// Exposed via `sky-cua-client env-keys` for operator/CI inspection; a unit
/// test below asserts no duplicates and the `SKY_CUA_` prefix invariant.
pub fn all_env_keys() -> &'static [&'static str] {
    &[
        MACHINE_CONFIG_PATH_ENV,
        REPO_ROOT_ENV,
        BROWSER_SELECTION_ENV,
        AGENT_SURFACES_ENV,
        BROWSER_CONTROL_MODE_ENV,
        CODEX_BROWSER_SOCKET_PATH_ENV,
        PHONE_ENABLED_ENV,
        PHONE_SERIAL_ENV,
        PHONE_BACKEND_ENV,
        PHONE_ADB_ENV,
        PHONE_SCRCPY_ENV,
        PHONE_WIRELESS_AUTO_CONNECT_ENV,
        PHONE_VISIBLE_OVERLAY_ENV,
        PHONE_SCREENSHOT_CURSOR_ENV,
        PHONE_V4L2_SINK_ENV,
        PHONE_COMPANION_ENV,
        PHONE_COMPANION_AUTO_INSTALL_ENV,
        PHONE_COMPANION_OPERATOR_MODE_ENV,
        PHONE_COMPANION_PACKAGE_ENV,
        PHONE_COMPANION_APK_ENV,
        PHONE_COMPANION_CERT_SHA256_ENV,
        PHONE_COMPANION_APK_SHA256_ENV,
        PHONE_COMPANION_ALLOW_DOWNGRADE_ENV,
        PHONE_CAPABILITY_CACHE_TTL_MS_ENV,
        PHONE_APPSHOT_TTL_MS_ENV,
        PHONE_TARGET_MODELS_ENV,
        PHONE_DIRECT_ENABLED_ENV,
        PHONE_DIRECT_LISTEN_ADDR_ENV,
        PHONE_DIRECT_ADVERTISED_ENDPOINT_ENV,
        PHONE_DIRECT_ENROLLMENT_TTL_MS_ENV,
        PHONE_DIRECT_STATE_PATH_ENV,
        ISOLATED_DESKTOP_ENABLED_ENV,
        ISOLATED_DESKTOP_DISPLAY_ENV,
        ISOLATED_DESKTOP_RESOLUTION_ENV,
        ISOLATED_DESKTOP_VIEWER_ENV,
        ISOLATED_DESKTOP_LIFECYCLE_ENV,
        ISOLATED_DESKTOP_WINDOW_MANAGER_ENV,
        OVERLAY_BACKEND_ENV,
        INPUT_HELPER_SOCKET_ENV,
        MODEL_SCREENSHOT_FORMAT_ENV,
        MODEL_SCREENSHOT_JPEG_QUALITY_ENV,
        MODEL_SCREENSHOT_WEBP_QUALITY_ENV,
        BROWSER_USE_SOCKET_DIR_ENV,
        crate::paths::SERVICE_SOCKET_PATH_ENV,
        crate::paths::SERVICE_TCP_ADDR_ENV,
        crate::paths::OVERLAY_HOST_TCP_ADDR_ENV,
        crate::CLIENT_SESSION_ENV_REPAIRS_ENV,
        crate::CLIENT_CLEARED_SESSION_ENV_KEYS_ENV,
    ]
}

/// Resolve the machine config path for this platform, or None when no base
/// directory can be determined.
pub fn machine_config_path() -> Option<PathBuf> {
    if let Some(explicit) = std::env::var_os(MACHINE_CONFIG_PATH_ENV) {
        if explicit.is_empty() {
            return None;
        }
        return Some(PathBuf::from(explicit));
    }
    machine_config_base_dir().map(|base| base.join("sky-cua").join("sky-cua.toml"))
}

fn machine_config_base_dir() -> Option<PathBuf> {
    if cfg!(windows) {
        return std::env::var_os("APPDATA")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from);
    }
    if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME").filter(|value| !value.is_empty()) {
        return Some(PathBuf::from(xdg));
    }
    std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config"))
}

/// Load the machine config. A missing file is the empty config; an unreadable
/// or unparseable file is an error so callers can surface an honest
/// diagnostic instead of silently ignoring operator configuration.
pub fn load_machine_config() -> Result<MachineConfig, String> {
    let Some(path) = machine_config_path() else {
        return Ok(MachineConfig::default());
    };
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(MachineConfig::default());
        }
        Err(error) => {
            return Err(format!(
                "failed to read machine config {}: {error}",
                path.display()
            ));
        }
    };
    parse_machine_config(&text)
        .map_err(|error| format!("invalid machine config {}: {error}", path.display()))
}

fn parse_machine_config(text: &str) -> Result<MachineConfig, toml::de::Error> {
    toml::from_str(text)
}

/// Effective browser selection: the `SKY_CUA_BROWSER` environment override
/// beats the machine config's `browser`; both unset means no selection.
/// Config-file errors are returned so the caller can attach a diagnostic.
pub fn resolved_browser_selection() -> Result<Option<String>, String> {
    if let Some(env_value) = browser_selection_env_value() {
        return Ok(Some(env_value));
    }
    Ok(load_machine_config()?
        .browser
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .map(normalize_browser_selection_alias))
}

fn browser_selection_env_value() -> Option<String> {
    std::env::var(BROWSER_SELECTION_ENV)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .map(normalize_browser_selection_alias)
}

fn normalize_browser_selection_alias(value: String) -> String {
    match value.as_str() {
        "brave-origin" => "brave".to_owned(),
        "chrome-origin" => "chrome".to_owned(),
        "chromium-origin" => "chromium".to_owned(),
        _ => value,
    }
}

/// Resolve the effective agent-facing surface policy. `SKY_CUA_SURFACES`, when
/// present, is an exact comma-separated allowlist and overrides `[surfaces]`.
/// The existing phone master switch is then intersected with the phone surface,
/// so `[phone].enabled=false` / `SKY_CUA_PHONE=0` removes phone-use completely.
pub fn resolved_agent_surface_policy() -> Result<AgentSurfacePolicy, String> {
    let machine = load_machine_config()?;
    let mut policy = resolve_agent_surface_policy(&machine.surfaces)?;
    let phone_master = env_bool(PHONE_ENABLED_ENV)
        .or(machine.phone.enabled)
        .unwrap_or(true);
    policy.phone &= phone_master;
    Ok(policy)
}

/// Resolve an already-parsed `[surfaces]` table plus the process override.
pub fn resolve_agent_surface_policy(
    surfaces: &AgentSurfaceConfig,
) -> Result<AgentSurfacePolicy, String> {
    if let Some(raw) = std::env::var_os(AGENT_SURFACES_ENV) {
        let raw = raw
            .into_string()
            .map_err(|_| format!("invalid {AGENT_SURFACES_ENV}: value is not valid UTF-8"))?;
        let raw = raw.trim();
        if raw.is_empty() {
            return Err(format!(
                "invalid {AGENT_SURFACES_ENV}: value must be a comma-separated allowlist"
            ));
        }
        let mut policy = AgentSurfacePolicy {
            desktop: false,
            browser: false,
            phone: false,
        };
        for item in raw.split(',') {
            match item.trim().to_ascii_lowercase().as_str() {
                "desktop" => policy.desktop = true,
                "browser" => policy.browser = true,
                "phone" => policy.phone = true,
                "" => {
                    return Err(format!(
                        "invalid {AGENT_SURFACES_ENV}: empty surface name in {raw:?}"
                    ));
                }
                other => {
                    return Err(format!(
                        "invalid {AGENT_SURFACES_ENV}: unknown surface {other:?}; use desktop, browser, or phone"
                    ));
                }
            }
        }
        return Ok(policy);
    }

    Ok(AgentSurfacePolicy {
        desktop: surfaces.desktop.unwrap_or(true),
        browser: surfaces.browser.unwrap_or(true),
        phone: surfaces.phone.unwrap_or(true),
    })
}

/// Resolve service-level browser-control ownership per field. Explicit process
/// environment wins over the machine file; absent values remain unset so the
/// service preserves its legacy behavior.
pub fn resolved_browser_control_config() -> Result<ResolvedBrowserControlConfig, String> {
    let mode_env = std::env::var_os(BROWSER_CONTROL_MODE_ENV);
    let socket_env = std::env::var_os(CODEX_BROWSER_SOCKET_PATH_ENV);
    let socket_env_was_set = socket_env.is_some();
    let machine = if mode_env.is_none() || socket_env.is_none() {
        load_machine_config()?.browser_control
    } else {
        BrowserControlConfig::default()
    };

    let mode = match mode_env {
        Some(raw) => Some(parse_browser_control_mode_env(raw)?),
        None => machine
            .mode
            .map(|value| parse_browser_control_mode_value(value, "[browser_control].mode"))
            .transpose()?,
    };
    let machine_socket_was_set = machine.codex_socket_path.is_some();
    let codex_socket_path = match socket_env {
        Some(raw) => Some(parse_nonempty_utf8_env(CODEX_BROWSER_SOCKET_PATH_ENV, raw)?),
        None => machine
            .codex_socket_path
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
            .or_else(|| {
                Some(
                    crate::paths::codex_browser_socket_path()
                        .to_string_lossy()
                        .into_owned(),
                )
            }),
    };
    if machine_socket_was_set && codex_socket_path.is_none() && !socket_env_was_set {
        return Err("[browser_control].codex_socket_path must not be empty".to_owned());
    }
    Ok(ResolvedBrowserControlConfig {
        mode,
        codex_socket_path,
    })
}

fn parse_browser_control_mode_env(raw: std::ffi::OsString) -> Result<BrowserControlMode, String> {
    let value = parse_nonempty_utf8_env(BROWSER_CONTROL_MODE_ENV, raw)?;
    parse_browser_control_mode_value(value, BROWSER_CONTROL_MODE_ENV)
}

fn parse_browser_control_mode_value(
    value: String,
    source: &str,
) -> Result<BrowserControlMode, String> {
    let value = value.trim();
    match value {
        "legacy" => Ok(BrowserControlMode::Legacy),
        "hybrid" => Ok(BrowserControlMode::Hybrid),
        "strict" => Ok(BrowserControlMode::Strict),
        _ => Err(format!(
            "invalid {source}: unsupported value {value:?}; use legacy, hybrid, or strict"
        )),
    }
}

fn parse_nonempty_utf8_env(key: &str, raw: std::ffi::OsString) -> Result<String, String> {
    let value = raw
        .into_string()
        .map_err(|_| format!("invalid {key}: value is not valid UTF-8"))?;
    let value = value.trim().to_owned();
    if value.is_empty() {
        return Err(format!("invalid {key}: value must not be empty"));
    }
    Ok(value)
}

/// Non-empty trimmed value of an environment variable, or `None`.
fn env_string(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

/// Parse a boolean env override. Accepts `1/true/on/yes` and `0/false/off/no`
/// (case-insensitive); anything else (including unset) yields `None`.
fn env_bool(key: &str) -> Option<bool> {
    match env_string(key)?.to_ascii_lowercase().as_str() {
        "1" | "true" | "on" | "yes" => Some(true),
        "0" | "false" | "off" | "no" => Some(false),
        _ => None,
    }
}

#[allow(dead_code)]
fn env_u16(key: &str) -> Option<u16> {
    env_string(key).and_then(|value| value.parse().ok())
}

fn env_u64(key: &str) -> Option<u64> {
    env_string(key).and_then(|value| value.parse().ok())
}

/// Resolve the effective phone-use selection: per-process environment overrides
/// beat the machine config's `[phone]` table, and concrete defaults fill the
/// rest. Mirrors [`resolved_browser_selection`]; config-file errors are returned
/// so the caller can attach a diagnostic.
pub fn resolved_phone_selection() -> Result<ResolvedPhoneSelection, String> {
    let machine = load_machine_config()?;
    let surface_policy = resolve_agent_surface_policy(&machine.surfaces)?;
    let mut resolved = resolve_phone_selection(&machine.phone);
    resolved.enabled &= surface_policy.phone;
    Ok(resolved)
}

/// Pure resolver over an already-parsed `[phone]` table plus the current
/// environment. Split out so tests can exercise layering without file I/O.
pub fn resolve_phone_selection(phone: &PhoneConfig) -> ResolvedPhoneSelection {
    let target_models = match env_string(PHONE_TARGET_MODELS_ENV) {
        Some(raw) => raw
            .split(',')
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .collect(),
        None => phone.primary_target_models.clone(),
    };

    ResolvedPhoneSelection {
        enabled: env_bool(PHONE_ENABLED_ENV)
            .or(phone.enabled)
            .unwrap_or(true),
        adb_path: env_string(PHONE_ADB_ENV).or_else(|| normalize(phone.adb_path.clone())),
        scrcpy_path: env_string(PHONE_SCRCPY_ENV).or_else(|| normalize(phone.scrcpy_path.clone())),
        default_serial: env_string(PHONE_SERIAL_ENV)
            .or_else(|| normalize(phone.default_serial.clone())),
        default_backend: env_string(PHONE_BACKEND_ENV)
            .or_else(|| normalize(phone.default_backend.clone())),
        wireless_auto_connect: env_bool(PHONE_WIRELESS_AUTO_CONNECT_ENV)
            .or(phone.wireless_auto_connect)
            .unwrap_or(false),
        visible_overlay: env_bool(PHONE_VISIBLE_OVERLAY_ENV)
            .or(phone.visible_overlay)
            .unwrap_or(true),
        screenshot_cursor: env_bool(PHONE_SCREENSHOT_CURSOR_ENV)
            .or(phone.screenshot_cursor)
            .unwrap_or(true),
        v4l2_sink: env_string(PHONE_V4L2_SINK_ENV).or_else(|| normalize(phone.v4l2_sink.clone())),
        window_width: phone.window_width,
        window_height: phone.window_height,
        max_size: phone.max_size,
        max_fps: phone.max_fps,
        video_codec: normalize(phone.video_codec.clone()),
        companion_enabled: env_bool(PHONE_COMPANION_ENV)
            .or(phone.companion_enabled)
            .unwrap_or(true),
        companion_auto_install: env_bool(PHONE_COMPANION_AUTO_INSTALL_ENV)
            .or(phone.companion_auto_install)
            .unwrap_or(true),
        companion_operator_mode: env_bool(PHONE_COMPANION_OPERATOR_MODE_ENV)
            .or(phone.companion_operator_mode)
            .unwrap_or(true),
        companion_package: env_string(PHONE_COMPANION_PACKAGE_ENV)
            .or_else(|| normalize(phone.companion_package.clone()))
            .unwrap_or_else(|| PHONE_DEFAULT_COMPANION_PACKAGE.to_string()),
        companion_apk_path: env_string(PHONE_COMPANION_APK_ENV)
            .or_else(|| normalize(phone.companion_apk_path.clone()))
            .unwrap_or_else(default_companion_apk_path),
        companion_expected_cert_sha256: env_string(PHONE_COMPANION_CERT_SHA256_ENV)
            .or_else(|| normalize(phone.companion_expected_cert_sha256.clone())),
        companion_apk_sha256: env_string(PHONE_COMPANION_APK_SHA256_ENV)
            .or_else(|| normalize(phone.companion_apk_sha256.clone())),
        companion_allow_downgrade: env_bool(PHONE_COMPANION_ALLOW_DOWNGRADE_ENV)
            .or(phone.companion_allow_downgrade)
            .unwrap_or(false),
        capability_cache_ttl_ms: env_u64(PHONE_CAPABILITY_CACHE_TTL_MS_ENV)
            .or(phone.capability_cache_ttl_ms)
            .unwrap_or(PHONE_DEFAULT_CAPABILITY_CACHE_TTL_MS)
            .max(1_000),
        appshot_ttl_ms: env_u64(PHONE_APPSHOT_TTL_MS_ENV)
            .or(phone.appshot_ttl_ms)
            .unwrap_or(PHONE_DEFAULT_APPSHOT_TTL_MS)
            .max(1_000),
        primary_target_models: target_models,
        direct_enabled: env_bool(PHONE_DIRECT_ENABLED_ENV)
            .or(phone.direct_enabled)
            .unwrap_or(false),
        direct_listen_addr: env_string(PHONE_DIRECT_LISTEN_ADDR_ENV)
            .or_else(|| normalize(phone.direct_listen_addr.clone())),
        direct_advertised_endpoint: env_string(PHONE_DIRECT_ADVERTISED_ENDPOINT_ENV)
            .or_else(|| normalize(phone.direct_advertised_endpoint.clone())),
        direct_enrollment_ttl_ms: env_u64(PHONE_DIRECT_ENROLLMENT_TTL_MS_ENV)
            .or(phone.direct_enrollment_ttl_ms)
            .unwrap_or(PHONE_DEFAULT_DIRECT_ENROLLMENT_TTL_MS),
        direct_state_path: env_string(PHONE_DIRECT_STATE_PATH_ENV)
            .or_else(|| normalize(phone.direct_state_path.clone())),
        profiles: phone.profiles.clone(),
        aliases: normalize_aliases(&phone.aliases),
    }
}

fn normalize_aliases(raw: &BTreeMap<String, String>) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for (k, v) in raw {
        let key = k.trim().to_string();
        let val = v.trim().to_string();
        if key.is_empty() || val.is_empty() {
            continue;
        }
        out.insert(key, val);
    }
    out
}

fn normalize(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn default_companion_apk_path() -> String {
    let Some(root) = env_string(REPO_ROOT_ENV) else {
        return PHONE_DEFAULT_COMPANION_APK_PATH.to_string();
    };
    PathBuf::from(root)
        .join(PHONE_DEFAULT_COMPANION_APK_PATH)
        .to_string_lossy()
        .into_owned()
}

/// Resolve the effective isolated-desktop selection: per-process environment
/// overrides beat the machine config's `[isolated_desktop]` table, and concrete
/// defaults fill the rest. Mirrors [`resolved_phone_selection`]; config-file
/// errors are returned so the caller can attach a diagnostic.
pub fn resolve_isolated_desktop_selection() -> Result<ResolvedIsolatedDesktop, String> {
    let isolated_desktop = load_machine_config()?.isolated_desktop;
    Ok(resolve_isolated_desktop(&isolated_desktop))
}

/// Pure resolver over an already-parsed `[isolated_desktop]` table plus the
/// current environment. Split out so tests can exercise layering without file
/// I/O. Env beats file beats default; the literal display `"auto"` is preserved
/// verbatim, and unrecognized `viewer`/`lifecycle` strings fall back to their
/// defaults rather than erroring.
pub fn resolve_isolated_desktop(cfg: &IsolatedDesktopConfig) -> ResolvedIsolatedDesktop {
    let viewer = env_string(ISOLATED_DESKTOP_VIEWER_ENV)
        .or_else(|| normalize(cfg.viewer.clone()))
        .map(parse_viewer_mode)
        .unwrap_or(ViewerMode::Attach);
    let lifecycle = env_string(ISOLATED_DESKTOP_LIFECYCLE_ENV)
        .or_else(|| normalize(cfg.lifecycle.clone()))
        .map(parse_lifecycle)
        .unwrap_or(Lifecycle::Persistent);

    ResolvedIsolatedDesktop {
        enabled: env_bool(ISOLATED_DESKTOP_ENABLED_ENV)
            .or(cfg.enabled)
            .unwrap_or(false),
        display: env_string(ISOLATED_DESKTOP_DISPLAY_ENV)
            .or_else(|| normalize(cfg.display.clone()))
            .unwrap_or_else(|| ISOLATED_DESKTOP_DEFAULT_DISPLAY.to_string()),
        resolution: env_string(ISOLATED_DESKTOP_RESOLUTION_ENV)
            .or_else(|| normalize(cfg.resolution.clone()))
            .unwrap_or_else(|| ISOLATED_DESKTOP_DEFAULT_RESOLUTION.to_string()),
        window_manager: env_string(ISOLATED_DESKTOP_WINDOW_MANAGER_ENV)
            .or_else(|| normalize(cfg.window_manager.clone()))
            .unwrap_or_else(|| ISOLATED_DESKTOP_DEFAULT_WINDOW_MANAGER.to_string()),
        viewer,
        lifecycle,
    }
}

/// Map a `viewer` string to [`ViewerMode`], defaulting on unrecognized values.
fn parse_viewer_mode(value: String) -> ViewerMode {
    match value.to_ascii_lowercase().as_str() {
        "html5" => ViewerMode::Html5,
        "none" => ViewerMode::None,
        // "attach" and any unrecognized value fall back to the default.
        _ => ViewerMode::Attach,
    }
}

/// Map a `lifecycle` string to [`Lifecycle`], defaulting on unrecognized values.
fn parse_lifecycle(value: String) -> Lifecycle {
    match value.to_ascii_lowercase().as_str() {
        "ephemeral" => Lifecycle::Ephemeral,
        // "persistent" and any unrecognized value fall back to the default.
        _ => Lifecycle::Persistent,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    /// Serializes env-mutating config tests so they cannot race on shared keys.
    static ENV_TEST_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn all_env_keys_are_unique_and_prefixed() {
        let keys = all_env_keys();
        let mut seen = std::collections::HashSet::new();
        for key in keys {
            assert!(
                key.starts_with("SKY_CUA_"),
                "env key {key} must start with SKY_CUA_"
            );
            assert!(
                seen.insert(*key),
                "duplicate env key in all_env_keys: {key}"
            );
        }
    }

    #[test]
    fn missing_file_is_empty_config() {
        assert_eq!(parse_machine_config(""), Ok(MachineConfig::default()));
    }

    #[test]
    fn parses_browser_selection() {
        let config = parse_machine_config("browser = \"brave\"\n").expect("valid config");
        assert_eq!(config.browser.as_deref(), Some("brave"));
    }

    #[test]
    fn unknown_keys_are_tolerated_for_forward_compatibility() {
        let config =
            parse_machine_config("browser = \"chrome\"\nfuture_knob = 3\n").expect("valid config");
        assert_eq!(config.browser.as_deref(), Some("chrome"));
    }

    #[test]
    fn invalid_toml_is_an_error_not_a_silent_default() {
        assert!(parse_machine_config("browser = ").is_err());
    }

    #[test]
    fn parses_browser_control_table_and_rejects_unknown_mode() {
        let config = parse_machine_config(
            "[browser_control]\nmode = \"hybrid\"\ncodex_socket_path = \"/run/user/1000/codex.sock\"\n",
        )
        .expect("valid config");
        assert_eq!(config.browser_control.mode.as_deref(), Some("hybrid"));
        assert_eq!(
            config.browser_control.codex_socket_path.as_deref(),
            Some("/run/user/1000/codex.sock")
        );
        assert_eq!(
            parse_machine_config("[browser_control]\nmode = \"automatic\"\n")
                .expect("syntax is valid")
                .browser_control
                .mode
                .as_deref(),
            Some("automatic")
        );
    }

    #[test]
    fn browser_control_resolves_env_then_machine_then_unset() {
        let _serial = ENV_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let path = std::env::temp_dir().join(format!(
            "sky-cua-browser-control-config-{}.toml",
            std::process::id()
        ));
        std::fs::write(
            &path,
            "[browser_control]\nmode = \"hybrid\"\ncodex_socket_path = \"/machine/codex.sock\"\n",
        )
        .expect("write config");
        let path_text = path.to_string_lossy().into_owned();
        let _guard = EnvGuard::set(&[(MACHINE_CONFIG_PATH_ENV, &path_text)]);
        let _overrides =
            EnvGuard::clear(&[BROWSER_CONTROL_MODE_ENV, CODEX_BROWSER_SOCKET_PATH_ENV]);

        let resolved = resolved_browser_control_config().expect("machine config resolves");
        assert_eq!(resolved.mode, Some(BrowserControlMode::Hybrid));
        assert_eq!(
            resolved.codex_socket_path.as_deref(),
            Some("/machine/codex.sock")
        );

        let _mode_env = EnvGuard::set(&[(BROWSER_CONTROL_MODE_ENV, "strict")]);
        let resolved = resolved_browser_control_config().expect("env config resolves");
        assert_eq!(resolved.mode, Some(BrowserControlMode::Strict));
        assert_eq!(
            resolved.codex_socket_path.as_deref(),
            Some("/machine/codex.sock")
        );

        let _socket_env = EnvGuard::set(&[(CODEX_BROWSER_SOCKET_PATH_ENV, "/env/codex.sock")]);
        let resolved = resolved_browser_control_config().expect("both env fields resolve");
        assert_eq!(resolved.mode, Some(BrowserControlMode::Strict));
        assert_eq!(
            resolved.codex_socket_path.as_deref(),
            Some("/env/codex.sock")
        );
        drop(_socket_env);
        drop(_mode_env);

        std::fs::remove_file(&path).expect("remove config");
        let resolved = resolved_browser_control_config().expect("missing config is legacy/unset");
        let expected_socket = crate::paths::codex_browser_socket_path()
            .to_string_lossy()
            .into_owned();
        assert_eq!(
            resolved,
            ResolvedBrowserControlConfig {
                mode: None,
                codex_socket_path: Some(expected_socket),
            }
        );
    }

    #[test]
    fn browser_selection_normalizes_legacy_origin_aliases_from_runtime_env() {
        let _serial = ENV_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let _guard = EnvGuard::set(&[(BROWSER_SELECTION_ENV, "chrome-origin")]);
        assert_eq!(
            resolved_browser_selection()
                .expect("selection resolves")
                .as_deref(),
            Some("chrome")
        );
    }

    #[test]
    fn surface_table_rejects_unknown_names_without_rejecting_unrelated_top_level_keys() {
        assert!(
            parse_machine_config("[surfaces]\nbrowesr = false\n").is_err(),
            "surface typos must fail instead of silently widening capability"
        );
        assert!(
            parse_machine_config("future_knob = 3\n[surfaces]\nbrowser = false\n").is_ok(),
            "unrelated top-level keys remain forward-compatible"
        );
    }

    #[test]
    fn surface_policy_defaults_all_enabled_and_parses_table() {
        let _serial = ENV_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let _guard = EnvGuard::clear(&[AGENT_SURFACES_ENV, PHONE_ENABLED_ENV]);
        assert_eq!(
            resolve_agent_surface_policy(&AgentSurfaceConfig::default()).unwrap(),
            AgentSurfacePolicy::default()
        );
        let config =
            parse_machine_config("[surfaces]\ndesktop = true\nbrowser = false\nphone = true\n")
                .expect("valid surfaces config");
        assert_eq!(
            resolve_agent_surface_policy(&config.surfaces).unwrap(),
            AgentSurfacePolicy {
                desktop: true,
                browser: false,
                phone: true,
            }
        );
    }

    #[test]
    fn surface_env_is_exact_strict_allowlist() {
        let _serial = ENV_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let _guard = EnvGuard::set(&[(AGENT_SURFACES_ENV, "browser, phone")]);
        assert_eq!(
            resolve_agent_surface_policy(&AgentSurfaceConfig {
                desktop: Some(true),
                browser: Some(false),
                phone: Some(false),
            })
            .unwrap(),
            AgentSurfacePolicy {
                desktop: false,
                browser: true,
                phone: true,
            }
        );
        drop(_guard);
        let _invalid = EnvGuard::set(&[(AGENT_SURFACES_ENV, "browser,telepathy")]);
        assert!(
            resolve_agent_surface_policy(&AgentSurfaceConfig::default())
                .unwrap_err()
                .contains("telepathy")
        );
    }

    #[test]
    fn browser_selection_normalizes_legacy_origin_aliases_from_machine_config() {
        let _serial = ENV_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let path = std::env::temp_dir().join(format!(
            "sky-cua-browser-alias-config-{}.toml",
            std::process::id()
        ));
        std::fs::write(&path, "browser = \"brave-origin\"\n").expect("write config");
        let path_text = path.to_string_lossy().into_owned();
        let _path = EnvGuard::set(&[(MACHINE_CONFIG_PATH_ENV, &path_text)]);
        let _selection = EnvGuard::clear(&[BROWSER_SELECTION_ENV]);

        assert_eq!(
            resolved_browser_selection()
                .expect("selection resolves")
                .as_deref(),
            Some("brave")
        );

        std::fs::remove_file(path).expect("remove config");
    }

    const PHONE_TABLE: &str = r#"
browser = "chrome"

[phone]
enabled = true
adb_path = "/opt/adb"
scrcpy_path = "/opt/scrcpy"
default_serial = "ABC123"
default_backend = "companion"
wireless_auto_connect = true
window_width = 540
window_height = 1200
max_size = 1440
max_fps = 60
video_codec = "h265"
v4l2_sink = "/dev/video10"
visible_overlay = true
screenshot_cursor = true
companion_enabled = true
companion_auto_install = true
companion_operator_mode = true
companion_package = "com.skycua.phonecompanion"
companion_apk_path = "resources/android/phone-companion.apk"
companion_expected_cert_sha256 = "deadbeef"
companion_allow_downgrade = false
capability_cache_ttl_ms = 30000
direct_enabled = true
direct_listen_addr = "100.64.0.10:47684"
direct_advertised_endpoint = "wss://saga.example.ts.net/phone/control"
direct_enrollment_ttl_ms = 120000
direct_state_path = "/var/lib/sky-cua/phone-direct.json"
primary_target_models = ["Galaxy S26 Ultra", "Redmi Pad 15 Pro"]

[phone.profiles.primary]
device_id = "00000000-0000-4000-8000-000000000001"
transport = "companion_direct"
access = "observation_only"
required_capabilities = ["sms.read"]
"#;

    #[test]
    fn parses_phone_table() {
        let config = parse_machine_config(PHONE_TABLE).expect("valid config");
        let phone = config.phone;
        assert_eq!(phone.enabled, Some(true));
        assert_eq!(phone.adb_path.as_deref(), Some("/opt/adb"));
        assert_eq!(phone.default_backend.as_deref(), Some("companion"));
        assert_eq!(phone.capability_cache_ttl_ms, Some(30000));
        assert_eq!(phone.direct_enabled, Some(true));
        assert_eq!(
            phone.profiles["primary"].device_id,
            "00000000-0000-4000-8000-000000000001"
        );
        assert_eq!(
            phone.profiles["primary"].required_capabilities,
            vec!["sms.read"]
        );
        assert_eq!(
            phone.direct_listen_addr.as_deref(),
            Some("100.64.0.10:47684")
        );
        assert_eq!(phone.direct_enrollment_ttl_ms, Some(120000));
        assert_eq!(
            phone.primary_target_models,
            vec![
                "Galaxy S26 Ultra".to_string(),
                "Redmi Pad 15 Pro".to_string()
            ]
        );
    }

    #[test]
    fn parses_phone_aliases() {
        let config = parse_machine_config(
            r#"
[phone.aliases]
phone = "f6e5da20-d8df-4de0-ae4b-2fddf2791d17"
tablet = "0b44dcbb-a6a4-4836-8b17-617a786d3ffa"
"#,
        )
        .expect("valid aliases");
        assert_eq!(
            config.phone.aliases.get("phone").map(String::as_str),
            Some("f6e5da20-d8df-4de0-ae4b-2fddf2791d17")
        );
        assert_eq!(
            config.phone.aliases.get("tablet").map(String::as_str),
            Some("0b44dcbb-a6a4-4836-8b17-617a786d3ffa")
        );
        let resolved = resolve_phone_selection(&config.phone);
        assert_eq!(
            resolved.aliases.get("phone").map(String::as_str),
            Some("f6e5da20-d8df-4de0-ae4b-2fddf2791d17")
        );
    }

    #[test]
    fn phone_aliases_are_trimmed_and_empty_entries_ignored() {
        let config = parse_machine_config(
            r#"
[phone.aliases]
phone = "  f6e5da20-d8df-4de0-ae4b-2fddf2791d17  "
empty = "   "
"#,
        )
        .expect("valid aliases with whitespace");
        // Raw config keeps the original whitespace; resolved view is trimmed
        // and drops empty entries.
        assert_eq!(
            config.phone.aliases.get("phone").map(String::as_str),
            Some("  f6e5da20-d8df-4de0-ae4b-2fddf2791d17  ")
        );
        let resolved = resolve_phone_selection(&config.phone);
        assert_eq!(resolved.aliases.len(), 1);
        assert_eq!(
            resolved.aliases.get("phone").map(String::as_str),
            Some("f6e5da20-d8df-4de0-ae4b-2fddf2791d17")
        );
        assert!(!resolved.aliases.contains_key("empty"));
    }

    #[test]
    fn missing_phone_table_uses_defaults() {
        // `resolve_phone_selection` reads process-global env; take the shared
        // lock and clear the keys we assert on so a concurrent env-sensitive
        // test cannot leak overrides (e.g. `env_overrides_beat_phone_table`
        // setting `SKY_CUA_PHONE_COMPANION=false`) into these default checks.
        let _serial = ENV_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let _guard = EnvGuard::clear(&[
            PHONE_ENABLED_ENV,
            PHONE_COMPANION_ENV,
            PHONE_COMPANION_PACKAGE_ENV,
            PHONE_COMPANION_APK_ENV,
            PHONE_CAPABILITY_CACHE_TTL_MS_ENV,
            PHONE_ADB_ENV,
            PHONE_TARGET_MODELS_ENV,
            PHONE_DIRECT_ENABLED_ENV,
            PHONE_DIRECT_LISTEN_ADDR_ENV,
            PHONE_DIRECT_ADVERTISED_ENDPOINT_ENV,
            PHONE_DIRECT_ENROLLMENT_TTL_MS_ENV,
            PHONE_DIRECT_STATE_PATH_ENV,
            REPO_ROOT_ENV,
        ]);
        let config = parse_machine_config("browser = \"chrome\"\n").expect("valid config");
        assert_eq!(config.phone, PhoneConfig::default());
        let resolved = resolve_phone_selection(&config.phone);
        // Defaults: enabled on, companion on, canonical package/port/ttl.
        assert!(resolved.enabled);
        assert!(resolved.companion_enabled);
        assert_eq!(resolved.companion_package, PHONE_DEFAULT_COMPANION_PACKAGE);
        assert_eq!(
            resolved.companion_apk_path,
            PHONE_DEFAULT_COMPANION_APK_PATH
        );
        assert_eq!(
            resolved.capability_cache_ttl_ms,
            PHONE_DEFAULT_CAPABILITY_CACHE_TTL_MS
        );
        assert!(resolved.adb_path.is_none());
        assert!(resolved.primary_target_models.is_empty());
        assert!(!resolved.direct_enabled);
        assert!(resolved.direct_listen_addr.is_none());
        assert_eq!(
            resolved.direct_enrollment_ttl_ms,
            PHONE_DEFAULT_DIRECT_ENROLLMENT_TTL_MS
        );
    }

    #[test]
    fn resolved_phone_selection_intersects_surface_policy() {
        let _serial = ENV_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let path = std::env::temp_dir().join(format!(
            "sky-cua-phone-surface-config-{}.toml",
            std::process::id()
        ));
        std::fs::write(
            &path,
            "[surfaces]\nphone = false\n\n[phone]\nenabled = true\n",
        )
        .expect("write config");
        let path_text = path.to_string_lossy().into_owned();
        let _path = EnvGuard::set(&[(MACHINE_CONFIG_PATH_ENV, &path_text)]);
        let _guard = EnvGuard::clear(&[AGENT_SURFACES_ENV, PHONE_ENABLED_ENV]);
        assert!(!resolved_phone_selection().unwrap().enabled);

        let _surface_override = EnvGuard::set(&[(AGENT_SURFACES_ENV, "phone")]);
        assert!(resolved_phone_selection().unwrap().enabled);
        drop(_surface_override);

        let _phone_off = EnvGuard::set(&[(PHONE_ENABLED_ENV, "0")]);
        assert!(!resolved_agent_surface_policy().unwrap().phone);
        std::fs::remove_file(path).expect("remove config");
    }

    #[test]
    fn default_companion_apk_path_resolves_under_repo_root_env() {
        let _serial = ENV_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let _guard = EnvGuard::set(&[(REPO_ROOT_ENV, "/opt/sky-cua")]);
        let resolved = resolve_phone_selection(&PhoneConfig::default());
        assert_eq!(
            resolved.companion_apk_path,
            "/opt/sky-cua/resources/android/phone-companion.apk"
        );
    }

    #[test]
    fn resolves_phone_table_without_env_overrides() {
        // Env vars are process-global; the lock keeps the two env-sensitive
        // tests from racing on the same keys under the parallel test runner.
        let _serial = ENV_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        // Guard against ambient env from the test runner by clearing the keys we
        // assert on for the duration of this test.
        let _guard = EnvGuard::clear(&[
            PHONE_SERIAL_ENV,
            PHONE_BACKEND_ENV,
            PHONE_ADB_ENV,
            PHONE_COMPANION_ENV,
            PHONE_TARGET_MODELS_ENV,
            PHONE_DIRECT_ENABLED_ENV,
            PHONE_DIRECT_LISTEN_ADDR_ENV,
            PHONE_DIRECT_ADVERTISED_ENDPOINT_ENV,
            PHONE_DIRECT_ENROLLMENT_TTL_MS_ENV,
            PHONE_DIRECT_STATE_PATH_ENV,
        ]);
        let phone = parse_machine_config(PHONE_TABLE)
            .expect("valid config")
            .phone;
        let resolved = resolve_phone_selection(&phone);
        assert_eq!(resolved.adb_path.as_deref(), Some("/opt/adb"));
        assert_eq!(resolved.default_serial.as_deref(), Some("ABC123"));
        assert_eq!(resolved.default_backend.as_deref(), Some("companion"));
        assert!(resolved.wireless_auto_connect);
        assert_eq!(resolved.primary_target_models.len(), 2);
        assert!(resolved.direct_enabled);
        assert_eq!(
            resolved.direct_advertised_endpoint.as_deref(),
            Some("wss://saga.example.ts.net/phone/control")
        );
        assert_eq!(resolved.direct_enrollment_ttl_ms, 120000);
    }

    #[test]
    fn resolves_scrcpy_window_geometry_from_phone_table() {
        // The scrcpy window/codec geometry fields are config-only (no env layer)
        // and must surface verbatim from the `[phone]` table.
        let phone = parse_machine_config(PHONE_TABLE)
            .expect("valid config")
            .phone;
        let resolved = resolve_phone_selection(&phone);
        assert_eq!(resolved.window_width, Some(540));
        assert_eq!(resolved.window_height, Some(1200));
        assert_eq!(resolved.max_size, Some(1440));
        assert_eq!(resolved.max_fps, Some(60));
        assert_eq!(resolved.video_codec.as_deref(), Some("h265"));

        // Unset geometry stays unset rather than defaulting to a value.
        let defaults = resolve_phone_selection(&PhoneConfig::default());
        assert!(defaults.window_width.is_none());
        assert!(defaults.max_fps.is_none());
        assert!(defaults.video_codec.is_none());
    }

    #[test]
    fn env_overrides_beat_phone_table() {
        let _serial = ENV_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let _guard = EnvGuard::set(&[
            (PHONE_SERIAL_ENV, "10.0.0.5:5555"),
            (PHONE_BACKEND_ENV, "adb"),
            (PHONE_COMPANION_ENV, "false"),
            (PHONE_TARGET_MODELS_ENV, "Pixel 9, Galaxy S26 Ultra"),
        ]);
        let phone = parse_machine_config(PHONE_TABLE)
            .expect("valid config")
            .phone;
        let resolved = resolve_phone_selection(&phone);
        assert_eq!(resolved.default_serial.as_deref(), Some("10.0.0.5:5555"));
        assert_eq!(resolved.default_backend.as_deref(), Some("adb"));
        assert!(!resolved.companion_enabled);
        assert_eq!(
            resolved.primary_target_models,
            vec!["Pixel 9".to_string(), "Galaxy S26 Ultra".to_string()]
        );
    }

    const ISOLATED_DESKTOP_TABLE: &str = r#"
browser = "chrome"

[isolated_desktop]
enabled = true
display = ":150"
resolution = "2560x1440"
window_manager = "i3"
viewer = "html5"
lifecycle = "ephemeral"
"#;

    /// Keys touched by the isolated-desktop env-override tests; cleared together
    /// so an ambient override or a sibling test cannot leak across runs.
    const ISOLATED_DESKTOP_ENV_KEYS: &[&str] = &[
        ISOLATED_DESKTOP_ENABLED_ENV,
        ISOLATED_DESKTOP_DISPLAY_ENV,
        ISOLATED_DESKTOP_RESOLUTION_ENV,
        ISOLATED_DESKTOP_WINDOW_MANAGER_ENV,
        ISOLATED_DESKTOP_VIEWER_ENV,
        ISOLATED_DESKTOP_LIFECYCLE_ENV,
    ];

    #[test]
    fn parses_isolated_desktop_table() {
        let config = parse_machine_config(ISOLATED_DESKTOP_TABLE).expect("valid config");
        let isolated = config.isolated_desktop;
        assert_eq!(isolated.enabled, Some(true));
        assert_eq!(isolated.display.as_deref(), Some(":150"));
        assert_eq!(isolated.resolution.as_deref(), Some("2560x1440"));
        assert_eq!(isolated.window_manager.as_deref(), Some("i3"));
        assert_eq!(isolated.viewer.as_deref(), Some("html5"));
        assert_eq!(isolated.lifecycle.as_deref(), Some("ephemeral"));
    }

    #[test]
    fn missing_isolated_desktop_table_uses_defaults() {
        // The resolver reads process-global env; take the shared lock and clear
        // the isolated-desktop keys so a concurrent env-sensitive test cannot
        // leak overrides into these default checks.
        let _serial = ENV_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let _guard = EnvGuard::clear(ISOLATED_DESKTOP_ENV_KEYS);
        let config = parse_machine_config("browser = \"chrome\"\n").expect("valid config");
        assert_eq!(config.isolated_desktop, IsolatedDesktopConfig::default());
        let resolved = resolve_isolated_desktop(&config.isolated_desktop);
        assert!(!resolved.enabled);
        assert_eq!(resolved.display, ISOLATED_DESKTOP_DEFAULT_DISPLAY);
        assert_eq!(resolved.resolution, ISOLATED_DESKTOP_DEFAULT_RESOLUTION);
        assert_eq!(
            resolved.window_manager,
            ISOLATED_DESKTOP_DEFAULT_WINDOW_MANAGER
        );
        assert_eq!(resolved.viewer, ViewerMode::Attach);
        assert_eq!(resolved.lifecycle, Lifecycle::Persistent);
    }

    #[test]
    fn resolves_isolated_desktop_table_without_env_overrides() {
        // Env vars are process-global; the lock keeps the env-sensitive
        // isolated-desktop tests from racing on the same keys.
        let _serial = ENV_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let _guard = EnvGuard::clear(ISOLATED_DESKTOP_ENV_KEYS);
        let isolated = parse_machine_config(ISOLATED_DESKTOP_TABLE)
            .expect("valid config")
            .isolated_desktop;
        let resolved = resolve_isolated_desktop(&isolated);
        assert!(resolved.enabled);
        assert_eq!(resolved.display, ":150");
        assert_eq!(resolved.resolution, "2560x1440");
        assert_eq!(resolved.window_manager, "i3");
        assert_eq!(resolved.viewer, ViewerMode::Html5);
        assert_eq!(resolved.lifecycle, Lifecycle::Ephemeral);
    }

    #[test]
    fn env_overrides_beat_isolated_desktop_table() {
        let _serial = ENV_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let _guard = EnvGuard::set(&[
            (ISOLATED_DESKTOP_ENABLED_ENV, "1"),
            (ISOLATED_DESKTOP_DISPLAY_ENV, ":200"),
            (ISOLATED_DESKTOP_RESOLUTION_ENV, "3840x2160"),
            (ISOLATED_DESKTOP_WINDOW_MANAGER_ENV, "openbox"),
            (ISOLATED_DESKTOP_VIEWER_ENV, "none"),
            (ISOLATED_DESKTOP_LIFECYCLE_ENV, "persistent"),
        ]);
        let isolated = parse_machine_config(ISOLATED_DESKTOP_TABLE)
            .expect("valid config")
            .isolated_desktop;
        let resolved = resolve_isolated_desktop(&isolated);
        // File says disabled-defaults differ; env wins on every field.
        assert!(resolved.enabled);
        assert_eq!(resolved.display, ":200");
        assert_eq!(resolved.resolution, "3840x2160");
        assert_eq!(resolved.window_manager, "openbox");
        assert_eq!(resolved.viewer, ViewerMode::None);
        assert_eq!(resolved.lifecycle, Lifecycle::Persistent);
    }

    #[test]
    fn isolated_desktop_auto_display_passes_through_verbatim() {
        // The literal "auto" must survive resolution unchanged so a downstream
        // module can pick a free display number.
        let _serial = ENV_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let _guard = EnvGuard::clear(ISOLATED_DESKTOP_ENV_KEYS);
        let cfg = IsolatedDesktopConfig {
            display: Some("auto".to_string()),
            ..IsolatedDesktopConfig::default()
        };
        let resolved = resolve_isolated_desktop(&cfg);
        assert_eq!(resolved.display, "auto");
    }

    #[test]
    fn isolated_desktop_viewer_and_lifecycle_string_mapping() {
        // String-to-enum mapping is case-insensitive and falls back to the
        // default on an unrecognized value rather than erroring.
        assert_eq!(parse_viewer_mode("attach".to_string()), ViewerMode::Attach);
        assert_eq!(parse_viewer_mode("HTML5".to_string()), ViewerMode::Html5);
        assert_eq!(parse_viewer_mode("None".to_string()), ViewerMode::None);
        assert_eq!(
            parse_viewer_mode("nonsense".to_string()),
            ViewerMode::Attach
        );

        assert_eq!(
            parse_lifecycle("persistent".to_string()),
            Lifecycle::Persistent
        );
        assert_eq!(
            parse_lifecycle("Ephemeral".to_string()),
            Lifecycle::Ephemeral
        );
        assert_eq!(
            parse_lifecycle("nonsense".to_string()),
            Lifecycle::Persistent
        );
    }

    #[test]
    fn isolated_desktop_unrecognized_viewer_lifecycle_in_config_default() {
        // An unrecognized viewer/lifecycle in the config table resolves to the
        // documented default through the full resolver path.
        let _serial = ENV_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let _guard = EnvGuard::clear(ISOLATED_DESKTOP_ENV_KEYS);
        let cfg = IsolatedDesktopConfig {
            viewer: Some("bogus".to_string()),
            lifecycle: Some("bogus".to_string()),
            ..IsolatedDesktopConfig::default()
        };
        let resolved = resolve_isolated_desktop(&cfg);
        assert_eq!(resolved.viewer, ViewerMode::Attach);
        assert_eq!(resolved.lifecycle, Lifecycle::Persistent);
    }

    /// Scoped environment mutator that restores prior values on drop so env-based
    /// config tests do not leak into one another. These tests must not run
    /// concurrently against the same keys; they are scoped to disjoint keys
    /// except the shared guard discipline keeps each test self-contained.
    struct EnvGuard {
        saved: Vec<(String, Option<String>)>,
    }

    impl EnvGuard {
        fn set(pairs: &[(&str, &str)]) -> Self {
            let saved = pairs
                .iter()
                .map(|(key, value)| {
                    let prior = std::env::var(key).ok();
                    // SAFETY: tests are single-threaded per test function; the
                    // guard restores prior state on drop.
                    unsafe { std::env::set_var(key, value) };
                    ((*key).to_string(), prior)
                })
                .collect();
            Self { saved }
        }

        fn clear(keys: &[&str]) -> Self {
            let saved = keys
                .iter()
                .map(|key| {
                    let prior = std::env::var(key).ok();
                    unsafe { std::env::remove_var(key) };
                    ((*key).to_string(), prior)
                })
                .collect();
            Self { saved }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            for (key, prior) in &self.saved {
                match prior {
                    Some(value) => unsafe { std::env::set_var(key, value) },
                    None => unsafe { std::env::remove_var(key) },
                }
            }
        }
    }
}
