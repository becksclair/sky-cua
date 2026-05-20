pub mod common;
pub mod cosmic;
pub mod gnome_extension;
pub mod gnome_introspect;
pub mod hyprland;
pub mod i3;
pub mod probe;
pub mod registry;
pub mod target;
pub mod terminal;
pub mod types;

pub use registry::{
    activate_window, discover_activation_windows, discover_app_windows, discover_windows,
    focus_verification_available, focused_window_override, probe_backends, verify_window_focused,
};
pub use target::resolve_window_target;
pub use types::LinuxWindowInfo;
