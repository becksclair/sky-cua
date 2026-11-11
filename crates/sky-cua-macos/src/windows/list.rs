//! Reads the CoreGraphics window list and filters it to application windows.

use objc2_core_foundation::{
    CFBoolean, CFDictionary, CFNumber, CFRetained, CFString, CFType, CGPoint, CGRect, CGSize,
};
use objc2_core_graphics::{
    CGRectMakeWithDictionaryRepresentation, CGWindowListCopyWindowInfo, CGWindowListOption,
    kCGNullWindowID, kCGWindowAlpha, kCGWindowBounds, kCGWindowIsOnscreen, kCGWindowLayer,
    kCGWindowName, kCGWindowNumber, kCGWindowOwnerName, kCGWindowOwnerPID,
};

/// The normal window level (`kCGNormalWindowLevel`). Menus, the menu bar, the
/// Dock, overlays, and status items all live on other layers.
const NORMAL_WINDOW_LAYER: i32 = 0;

/// Windows narrower or shorter than this, in points, are helper surfaces
/// (offscreen key-event sinks, 1x1 tracking windows), not user windows.
const MIN_WINDOW_EDGE: f64 = 2.0;

/// Untitled normal-layer windows below this size, in points, are transient
/// popups an application owns (autocorrect and text-suggestion bubbles,
/// tooltips), not windows a user or agent can target. Listing them would let
/// a suggestion bubble take the focused flag from the document under it and
/// make an application selector ambiguous.
const MIN_UNTITLED_WINDOW_WIDTH: f64 = 100.0;
const MIN_UNTITLED_WINDOW_HEIGHT: f64 = 60.0;

/// One entry of the CoreGraphics window list, before filtering.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct RawWindow {
    pub window_id: u32,
    pub pid: i32,
    pub owner_name: String,
    pub title: Option<String>,
    /// Global points, top-left origin: `[x, y, width, height]`.
    pub bounds: [f64; 4],
    pub layer: i32,
    pub alpha: f64,
    pub on_screen: bool,
}

/// Reads on-screen windows, excluding desktop elements, in the window
/// server's front-to-back order.
pub(super) fn read_window_list() -> Vec<RawWindow> {
    let options =
        CGWindowListOption::OptionOnScreenOnly | CGWindowListOption::ExcludeDesktopElements;
    let Some(list) = CGWindowListCopyWindowInfo(options, kCGNullWindowID) else {
        return Vec::new();
    };
    // SAFETY: CGWindowListCopyWindowInfo returns an array of dictionaries
    // keyed by CFString window-description keys.
    let list: CFRetained<objc2_core_foundation::CFArray<CFDictionary<CFString, CFType>>> =
        unsafe { CFRetained::cast_unchecked(list) };
    let mut windows = Vec::with_capacity(list.len());
    for index in 0..list.len() {
        // SAFETY: `index` is in bounds and the array is immutable.
        let entry = unsafe { list.get_unchecked(index as isize) };
        if let Some(window) = raw_window(entry) {
            windows.push(window);
        }
    }
    windows
}

fn raw_window(entry: &CFDictionary<CFString, CFType>) -> Option<RawWindow> {
    // SAFETY: the `kCGWindow*` keys are immutable framework constants, and the
    // window description dictionary is not mutated while borrowed.
    unsafe {
        let window_id = number(entry, kCGWindowNumber)?.as_i64()?;
        let pid = number(entry, kCGWindowOwnerPID)?.as_i32()?;
        let layer = number(entry, kCGWindowLayer)?.as_i32()?;
        let alpha = number(entry, kCGWindowAlpha)
            .and_then(CFNumber::as_f64)
            .unwrap_or(1.0);
        let on_screen = entry
            .get_unchecked(kCGWindowIsOnscreen)
            .and_then(|value| value.downcast_ref::<CFBoolean>())
            .is_some_and(CFBoolean::as_bool);
        let owner_name = string(entry, kCGWindowOwnerName).unwrap_or_default();
        let title = string(entry, kCGWindowName).filter(|title| !title.is_empty());
        let bounds = entry
            .get_unchecked(kCGWindowBounds)
            .and_then(|value| value.downcast_ref::<CFDictionary>())
            .and_then(|dict| {
                let mut rect = CGRect::new(CGPoint::new(0.0, 0.0), CGSize::new(0.0, 0.0));
                CGRectMakeWithDictionaryRepresentation(Some(dict), &mut rect).then_some(rect)
            })?;
        Some(RawWindow {
            window_id: u32::try_from(window_id).ok()?,
            pid,
            owner_name,
            title,
            bounds: [
                bounds.origin.x,
                bounds.origin.y,
                bounds.size.width,
                bounds.size.height,
            ],
            layer,
            alpha,
            on_screen,
        })
    }
}

/// # Safety
///
/// `entry` must not be mutated while the returned reference is live.
unsafe fn number<'a>(
    entry: &'a CFDictionary<CFString, CFType>,
    key: &CFString,
) -> Option<&'a CFNumber> {
    unsafe { entry.get_unchecked(key) }.and_then(|value| value.downcast_ref::<CFNumber>())
}

/// # Safety
///
/// `entry` must not be mutated while the returned reference is live.
unsafe fn string(entry: &CFDictionary<CFString, CFType>, key: &CFString) -> Option<String> {
    unsafe { entry.get_unchecked(key) }
        .and_then(|value| value.downcast_ref::<CFString>())
        .map(ToString::to_string)
}

/// True for a visible, normal-layer window of user-meaningful size.
pub(super) fn is_application_window(window: &RawWindow) -> bool {
    window.layer == NORMAL_WINDOW_LAYER
        && window.alpha > 0.0
        && window.bounds[2] >= MIN_WINDOW_EDGE
        && window.bounds[3] >= MIN_WINDOW_EDGE
        && !is_untitled_popup(window)
}

fn is_untitled_popup(window: &RawWindow) -> bool {
    window.title.as_deref().is_none_or(str::is_empty)
        && (window.bounds[2] < MIN_UNTITLED_WINDOW_WIDTH
            || window.bounds[3] < MIN_UNTITLED_WINDOW_HEIGHT)
}
