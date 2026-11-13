//! Crate-internal value types shared by the macOS backend modules.

use sky_cua_platform::model::RectF;

/// Backend label reported in `WindowInfo.backend` and `DisplayInfo.backend`.
pub const WINDOW_BACKEND: &str = "macos-coregraphics";

/// One on-screen, normal-layer window from the CoreGraphics window list.
///
/// `bounds` is in global points with a top-left origin (`DesktopLogical`),
/// the same space AX and CGEvent use.
#[derive(Debug, Clone, PartialEq)]
pub struct MacWindow {
    /// `CGWindowID`.
    pub window_id: u32,
    pub pid: i32,
    /// Owning application name as CoreGraphics reports it.
    pub owner_name: String,
    pub bundle_id: Option<String>,
    pub executable: Option<String>,
    /// Window title. `None` when the window has no title or Screen Recording
    /// permission is missing (CoreGraphics withholds titles without it).
    pub title: Option<String>,
    pub bounds: RectF,
    pub layer: i32,
    pub on_screen: bool,
    /// True for the front-most window of the front-most application.
    pub focused: bool,
    /// Front-to-back position in the window list; 0 is front-most.
    pub z_index: usize,
}

/// Pointer button for physical input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Right,
}
