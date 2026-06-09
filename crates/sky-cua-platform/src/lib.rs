pub mod app_instructions;
pub mod backend;
pub mod diagnostics;
pub mod model;
pub mod paths;
pub mod snapshot;

pub const DESKTOP_ENV_KEYS: &[&str] = &[
    "DBUS_SESSION_BUS_ADDRESS",
    "DESKTOP_SESSION",
    "DISPLAY",
    "PATH",
    "WAYLAND_DISPLAY",
    "XDG_CURRENT_DESKTOP",
    "XDG_RUNTIME_DIR",
    "XDG_SESSION_TYPE",
];
pub const CURRENT_ENV_HEALTH_KEYS: &[&str] = &[
    "DBUS_SESSION_BUS_ADDRESS",
    "DESKTOP_SESSION",
    "DISPLAY",
    "WAYLAND_DISPLAY",
    "XDG_CURRENT_DESKTOP",
    "XDG_RUNTIME_DIR",
    "XDG_SESSION_TYPE",
];
pub const BROWSER_ENV_HEALTH_KEYS: &[&str] = &[
    "SKY_CUA_BROWSER_USE_SOCKET_DIR",
    "CODEX_BROWSER_USE_SOCKET_DIR",
    "SKY_CUA_BROWSER",
];

pub use app_instructions::{
    AppInstructionEntry, AppInstructionIndex, SetValueFallbackMode, SetValueRouting,
    app_instruction_entry_matches, app_instructions_index_path, app_instructions_root,
    focused_app_instruction_keys, normalize_app_instruction_key,
};
pub use backend::{
    AppDiscoveryBackend, CaptureBackend, DesktopBackend, FocusTracker, HeuristicsResolver,
    InputBackend, SemanticBackend,
};
pub use diagnostics::{BackendError, BackendErrorCode, DiagnosticBuilder};
pub use model::*;
pub use paths::{
    OVERLAY_HOST_TCP_ADDR_ENV, SERVICE_SOCKET_PATH_ENV, SERVICE_TCP_ADDR_ENV, approvals_path,
    overlay_host_tcp_addr, portal_tokens_path, service_socket_path, service_tcp_addr,
    sky_cua_state_dir,
};
pub use snapshot::new_snapshot_id;
