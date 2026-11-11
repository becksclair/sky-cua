//! Brief reuse of ScreenCaptureKit shareable content.
//!
//! Fetching `SCShareableContent` costs roughly 12-17 ms per capture. A recent
//! result is reused only as a handle lookup: before a cached window or display
//! is captured, its geometry is checked against a fresh CoreGraphics read
//! (well under a millisecond), and any mismatch, disappearance, or owner
//! change falls back to a fresh fetch. The reported rectangle therefore always
//! matches the live window-server state.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use objc2::rc::Retained;
use objc2_core_foundation::{
    CFArray, CFBoolean, CFDictionary, CFNumber, CFRetained, CFString, CFType, CGPoint, CGRect,
    CGSize,
};
use objc2_core_graphics::{
    CGDisplayBounds, CGRectMakeWithDictionaryRepresentation, CGWindowListCopyWindowInfo,
    CGWindowListOption, kCGWindowBounds, kCGWindowIsOnscreen, kCGWindowOwnerPID,
};
use objc2_screen_capture_kit::SCShareableContent;

use super::sck::Sendable;

/// How long a fetched shareable-content result may serve as a handle lookup.
const REUSE_FOR: Duration = Duration::from_secs(5);

struct Entry {
    fetched: Instant,
    content: Sendable<Retained<SCShareableContent>>,
}

static CACHE: Mutex<Option<Entry>> = Mutex::new(None);

/// The most recent shareable content, when fetched within `REUSE_FOR`.
pub(super) fn recent() -> Option<Sendable<Retained<SCShareableContent>>> {
    let cache = CACHE.lock().ok()?;
    let entry = cache.as_ref()?;
    (entry.fetched.elapsed() < REUSE_FOR).then(|| Sendable(entry.content.0.clone()))
}

/// Records a freshly fetched shareable-content result.
pub(super) fn store(content: &Sendable<Retained<SCShareableContent>>) {
    if let Ok(mut cache) = CACHE.lock() {
        *cache = Some(Entry {
            fetched: Instant::now(),
            content: Sendable(content.0.clone()),
        });
    }
}

/// True when the window server currently shows `window_id`, owned by `pid`,
/// on screen at exactly `frame`.
pub(super) fn window_matches(window_id: u32, pid: i32, frame: CGRect) -> bool {
    let Some(list) =
        CGWindowListCopyWindowInfo(CGWindowListOption::OptionIncludingWindow, window_id)
    else {
        return false;
    };
    // SAFETY: CGWindowListCopyWindowInfo returns an array of dictionaries
    // keyed by CFString window-description keys.
    let list: CFRetained<CFArray<CFDictionary<CFString, CFType>>> =
        unsafe { CFRetained::cast_unchecked(list) };
    if list.len() != 1 {
        return false;
    }
    // SAFETY: index 0 is in bounds, the array and its dictionary are
    // immutable, and the `kCGWindow*` keys are framework constants.
    unsafe {
        let entry = list.get_unchecked(0);
        let owner = entry
            .get_unchecked(kCGWindowOwnerPID)
            .and_then(|value| value.downcast_ref::<CFNumber>())
            .and_then(CFNumber::as_i32);
        let on_screen = entry
            .get_unchecked(kCGWindowIsOnscreen)
            .and_then(|value| value.downcast_ref::<CFBoolean>())
            .is_some_and(CFBoolean::as_bool);
        let bounds = entry
            .get_unchecked(kCGWindowBounds)
            .and_then(|value| value.downcast_ref::<CFDictionary>())
            .and_then(|dict| {
                let mut rect = CGRect::new(CGPoint::new(0.0, 0.0), CGSize::new(0.0, 0.0));
                CGRectMakeWithDictionaryRepresentation(Some(dict), &mut rect).then_some(rect)
            });
        owner == Some(pid) && on_screen && bounds.is_some_and(|bounds| same_rect(bounds, frame))
    }
}

/// True when display `display_id` currently occupies exactly `frame`.
pub(super) fn display_matches(display_id: u32, frame: CGRect) -> bool {
    same_rect(CGDisplayBounds(display_id), frame)
}

fn same_rect(a: CGRect, b: CGRect) -> bool {
    a.origin.x == b.origin.x
        && a.origin.y == b.origin.y
        && a.size.width == b.size.width
        && a.size.height == b.size.height
}
