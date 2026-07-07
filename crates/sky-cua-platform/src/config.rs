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

use std::path::PathBuf;

use serde::Deserialize;

/// Test/operator override for the machine config file location.
pub const MACHINE_CONFIG_PATH_ENV: &str = "SKY_CUA_CONFIG_PATH";
pub const REPO_ROOT_ENV: &str = "SKY_CUA_REPO_ROOT";

pub const BROWSER_SELECTION_ENV: &str = "SKY_CUA_BROWSER";

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
pub const PHONE_COMPANION_RPC_PORT_ENV: &str = "SKY_CUA_PHONE_COMPANION_RPC_PORT";
pub const PHONE_COMPANION_RPC_TOKEN_TTL_MS_ENV: &str = "SKY_CUA_PHONE_COMPANION_RPC_TOKEN_TTL_MS";
pub const PHONE_CAPABILITY_CACHE_TTL_MS_ENV: &str = "SKY_CUA_PHONE_CAPABILITY_CACHE_TTL_MS";
pub const PHONE_TARGET_MODELS_ENV: &str = "SKY_CUA_PHONE_TARGET_MODELS";

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
    /// Phone-use `[phone]` table. Absent means defaults plus env overrides.
    #[serde(default)]
    pub phone: PhoneConfig,
    /// Isolated-desktop `[isolated_desktop]` table. Absent means defaults plus
    /// env overrides.
    #[serde(default)]
    pub isolated_desktop: IsolatedDesktopConfig,
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
    pub companion_rpc_port: Option<u16>,
    pub companion_rpc_token_ttl_ms: Option<u64>,
    pub capability_cache_ttl_ms: Option<u64>,
    #[serde(default)]
    pub primary_target_models: Vec<String>,
}

/// Default companion package id when neither config nor env overrides it.
pub const PHONE_DEFAULT_COMPANION_PACKAGE: &str = "com.skycua.phonecompanion";
/// Default companion APK path relative to the repo/payload root.
pub const PHONE_DEFAULT_COMPANION_APK_PATH: &str = "resources/android/phone-companion.apk";
/// Default companion RPC port reached through host-managed `adb forward`.
pub const PHONE_DEFAULT_COMPANION_RPC_PORT: u16 = 47683;
/// Default ephemeral RPC token lifetime (15 minutes).
pub const PHONE_DEFAULT_COMPANION_RPC_TOKEN_TTL_MS: u64 = 900_000;
/// Default capability-cache soft refresh hint (30 seconds).
pub const PHONE_DEFAULT_CAPABILITY_CACHE_TTL_MS: u64 = 30_000;

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
    pub companion_rpc_port: u16,
    pub companion_rpc_token_ttl_ms: u64,
    pub capability_cache_ttl_ms: u64,
    pub primary_target_models: Vec<String>,
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
        PHONE_COMPANION_RPC_PORT_ENV,
        PHONE_COMPANION_RPC_TOKEN_TTL_MS_ENV,
        PHONE_CAPABILITY_CACHE_TTL_MS_ENV,
        PHONE_TARGET_MODELS_ENV,
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
        .filter(|value| !value.is_empty()))
}

fn browser_selection_env_value() -> Option<String> {
    std::env::var(BROWSER_SELECTION_ENV)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
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
    let phone = load_machine_config()?.phone;
    Ok(resolve_phone_selection(&phone))
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
        companion_rpc_port: env_u16(PHONE_COMPANION_RPC_PORT_ENV)
            .or(phone.companion_rpc_port)
            .unwrap_or(PHONE_DEFAULT_COMPANION_RPC_PORT),
        companion_rpc_token_ttl_ms: env_u64(PHONE_COMPANION_RPC_TOKEN_TTL_MS_ENV)
            .or(phone.companion_rpc_token_ttl_ms)
            .unwrap_or(PHONE_DEFAULT_COMPANION_RPC_TOKEN_TTL_MS),
        capability_cache_ttl_ms: env_u64(PHONE_CAPABILITY_CACHE_TTL_MS_ENV)
            .or(phone.capability_cache_ttl_ms)
            .unwrap_or(PHONE_DEFAULT_CAPABILITY_CACHE_TTL_MS),
        primary_target_models: target_models,
    }
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
companion_rpc_port = 47683
companion_rpc_token_ttl_ms = 900000
capability_cache_ttl_ms = 30000
primary_target_models = ["Galaxy S26 Ultra", "Redmi Pad 15 Pro"]
"#;

    #[test]
    fn parses_phone_table() {
        let config = parse_machine_config(PHONE_TABLE).expect("valid config");
        let phone = config.phone;
        assert_eq!(phone.enabled, Some(true));
        assert_eq!(phone.adb_path.as_deref(), Some("/opt/adb"));
        assert_eq!(phone.default_backend.as_deref(), Some("companion"));
        assert_eq!(phone.companion_rpc_port, Some(47683));
        assert_eq!(phone.capability_cache_ttl_ms, Some(30000));
        assert_eq!(
            phone.primary_target_models,
            vec![
                "Galaxy S26 Ultra".to_string(),
                "Redmi Pad 15 Pro".to_string()
            ]
        );
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
            PHONE_COMPANION_RPC_PORT_ENV,
            PHONE_CAPABILITY_CACHE_TTL_MS_ENV,
            PHONE_ADB_ENV,
            PHONE_TARGET_MODELS_ENV,
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
            resolved.companion_rpc_port,
            PHONE_DEFAULT_COMPANION_RPC_PORT
        );
        assert_eq!(
            resolved.capability_cache_ttl_ms,
            PHONE_DEFAULT_CAPABILITY_CACHE_TTL_MS
        );
        assert!(resolved.adb_path.is_none());
        assert!(resolved.primary_target_models.is_empty());
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
            PHONE_COMPANION_RPC_PORT_ENV,
            PHONE_TARGET_MODELS_ENV,
        ]);
        let phone = parse_machine_config(PHONE_TABLE)
            .expect("valid config")
            .phone;
        let resolved = resolve_phone_selection(&phone);
        assert_eq!(resolved.adb_path.as_deref(), Some("/opt/adb"));
        assert_eq!(resolved.default_serial.as_deref(), Some("ABC123"));
        assert_eq!(resolved.default_backend.as_deref(), Some("companion"));
        assert!(resolved.wireless_auto_connect);
        assert_eq!(resolved.companion_rpc_port, 47683);
        assert_eq!(resolved.primary_target_models.len(), 2);
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
            (PHONE_COMPANION_RPC_PORT_ENV, "50000"),
            (PHONE_COMPANION_ENV, "false"),
            (PHONE_TARGET_MODELS_ENV, "Pixel 9, Galaxy S26 Ultra"),
        ]);
        let phone = parse_machine_config(PHONE_TABLE)
            .expect("valid config")
            .phone;
        let resolved = resolve_phone_selection(&phone);
        assert_eq!(resolved.default_serial.as_deref(), Some("10.0.0.5:5555"));
        assert_eq!(resolved.default_backend.as_deref(), Some("adb"));
        assert_eq!(resolved.companion_rpc_port, 50000);
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
