pub mod app_instructions;
pub mod backend;
pub mod diagnostics;
pub mod model;
pub mod paths;
pub mod snapshot;

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
    SERVICE_SOCKET_PATH_ENV, SERVICE_TCP_ADDR_ENV, approvals_path, portal_tokens_path,
    service_socket_path, service_tcp_addr, sky_cua_state_dir,
};
pub use snapshot::new_snapshot_id;
