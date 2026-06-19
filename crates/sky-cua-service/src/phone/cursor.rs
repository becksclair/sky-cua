//! Agent cursor planes for phone sessions.
//!
//! Three planes, matching the desktop overlay design: a host-visible overlay
//! (drawn on the scrcpy/host window), a screenshot-synthetic marker (composited
//! into the returned image), and a phone-native overlay (drawn on the device by
//! the companion accessibility service). [`PhoneCursorCapabilities`] reports
//! which planes are live; [`PhoneCursorState`] carries the last position.
//!
//! ## Per-session isolation
//!
//! There is no global phone cursor. The manager keeps one [`PhoneCursorTracker`]
//! per session (keyed by serial), and a tracker only ever updates from actions
//! that name its own `session_id`/`serial`. An action against another session is
//! rejected, so a cursor can never leak across serials. This is the invariant
//! the research doc calls out: "never one global phone cursor for every device."
//!
//! ## Update / idle-hide semantics
//!
//! A successful coordinate action updates the cursor to the action point and
//! marks it visible. After [`IDLE_HIDE_MS`] with no further action the cursor is
//! considered hidden for composition purposes (mirroring the desktop overlay's
//! 1.5s idle hide). A *failed* action must not update the cursor — callers only
//! call [`PhoneCursorTracker::record_action`] after dispatch succeeds.
//!
//! ## Screenshot-synthetic composition
//!
//! [`compose_synthetic_cursor`] paints the bundled agent cursor asset
//! (`sky_cua_overlay_host::cursor_asset`) into an RGBA screenshot at a
//! screenshot-pixel point, reusing the same hotspot/alpha-blend approach as
//! `crate::overlay::synthetic_cursor` but operating on an in-memory image so the
//! phone lane stays independent of the desktop capture-file pipeline.

use std::sync::LazyLock;

use image::imageops::FilterType;
use image::{Rgba, RgbaImage};
use sky_cua_overlay_host::cursor_asset;
use sky_cua_platform::model::{
    DiagnosticEntry, PhoneCursorCapabilities, PhoneCursorState, PhonePoint,
};

/// Idle window after which a phone cursor is treated as hidden, matching the
/// desktop overlay's 1.5s idle hide.
pub(super) const IDLE_HIDE_MS: u64 = 1_500;

/// Cursor capabilities for a session with no live overlay planes (no host
/// overlay, no native overlay, no synthetic marker). Retained for the cursor
/// lane's own tests; the live routing path derives capabilities from the profile
/// via `cursor_capabilities`/`adb_only_capabilities` instead.
#[cfg_attr(not(test), expect(dead_code))]
pub(super) fn no_cursor_capabilities() -> PhoneCursorCapabilities {
    PhoneCursorCapabilities {
        host_visible_overlay: false,
        screenshot_synthetic_cursor: false,
        phone_native_overlay: false,
        visible_overlay_reason: Some(
            "phone cursor overlay planes are not available for this session".to_string(),
        ),
    }
}

/// Capabilities for an ADB-only session: no host or native overlay, but the
/// screenshot-synthetic marker is available when screenshot synthesis is enabled
/// in config. This is the exact contract Phase 3 requires for ADB-only mode.
pub(super) fn adb_only_capabilities(screenshot_cursor_enabled: bool) -> PhoneCursorCapabilities {
    PhoneCursorCapabilities {
        host_visible_overlay: false,
        screenshot_synthetic_cursor: screenshot_cursor_enabled,
        phone_native_overlay: false,
        visible_overlay_reason: Some(
            "ADB-only session has no host window or companion overlay; cursor is composited into screenshots only"
                .to_string(),
        ),
    }
}

/// Per-session cursor state owner. One per `(session_id, serial)`; the manager
/// holds a map of these and never shares a tracker across serials.
#[derive(Debug, Clone)]
pub(super) struct PhoneCursorTracker {
    session_id: String,
    serial: String,
    sequence: u64,
    state: Option<PhoneCursorState>,
}

impl PhoneCursorTracker {
    /// A tracker for a session with no cursor yet.
    pub(super) fn new(session_id: impl Into<String>, serial: impl Into<String>) -> Self {
        Self {
            session_id: session_id.into(),
            serial: serial.into(),
            sequence: 0,
            state: None,
        }
    }

    /// Update the cursor after a *successful* action against this session. The
    /// `session_id`/`serial` are verified to belong to this tracker so a caller
    /// cannot leak another device's cursor in here. Returns the new state.
    pub(super) fn record_action(
        &mut self,
        session_id: &str,
        serial: &str,
        action: &str,
        snapshot_id: Option<&str>,
        device_point: Option<PhonePoint>,
        screenshot_point: Option<PhonePoint>,
        now_ms: u64,
    ) -> Result<PhoneCursorState, CursorError> {
        if session_id != self.session_id {
            return Err(CursorError::SessionMismatch {
                expected: self.session_id.clone(),
                found: session_id.to_string(),
            });
        }
        if serial != self.serial {
            return Err(CursorError::SerialMismatch {
                expected: self.serial.clone(),
                found: serial.to_string(),
            });
        }
        self.sequence = self.sequence.saturating_add(1);
        let state = PhoneCursorState {
            visible: true,
            sequence: self.sequence,
            device_point,
            screenshot_point,
            snapshot_id: snapshot_id.map(str::to_string),
            source_action: Some(action.to_string()),
            updated_at_ms: now_ms,
        };
        self.state = Some(state.clone());
        Ok(state)
    }

    /// The current cursor state, with `visible` recomputed against the idle
    /// window: a cursor untouched for longer than [`IDLE_HIDE_MS`] reports
    /// `visible=false`. Returns `None` if no action has ever updated it.
    pub(super) fn current(&self, now_ms: u64) -> Option<PhoneCursorState> {
        let mut state = self.state.clone()?;
        if now_ms.saturating_sub(state.updated_at_ms) > IDLE_HIDE_MS {
            state.visible = false;
        }
        Some(state)
    }

    /// Whether the cursor should be drawn into a screenshot right now: it exists,
    /// is within the idle window, and has a screenshot-plane point.
    pub(super) fn screenshot_point(&self, now_ms: u64) -> Option<PhonePoint> {
        let state = self.current(now_ms)?;
        if !state.visible {
            return None;
        }
        state.screenshot_point
    }
}

/// Why a cursor update was rejected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum CursorError {
    SessionMismatch { expected: String, found: String },
    SerialMismatch { expected: String, found: String },
}

impl CursorError {
    #[cfg_attr(not(test), expect(dead_code))]
    pub(super) fn code(&self) -> &'static str {
        match self {
            CursorError::SessionMismatch { .. } => "PhoneCursorSessionMismatch",
            CursorError::SerialMismatch { .. } => "PhoneCursorSerialMismatch",
        }
    }
}

impl std::fmt::Display for CursorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CursorError::SessionMismatch { expected, found } => {
                write!(
                    f,
                    "cursor update for session {found}, tracker owns {expected}"
                )
            }
            CursorError::SerialMismatch { expected, found } => {
                write!(
                    f,
                    "cursor update for serial {found}, tracker owns {expected}"
                )
            }
        }
    }
}

/// Bundled agent cursor, decoded once and resized to the standard display size,
/// matching `crate::overlay::synthetic_cursor`.
static AGENT_CURSOR_IMAGE: LazyLock<Result<RgbaImage, String>> = LazyLock::new(|| {
    let image = image::load_from_memory(cursor_asset::AGENT_CURSOR_PNG)
        .map_err(|error| error.to_string())?
        .to_rgba8();
    if image.width() != cursor_asset::AGENT_CURSOR_SOURCE_WIDTH
        || image.height() != cursor_asset::AGENT_CURSOR_SOURCE_HEIGHT
    {
        return Err(format!(
            "expected {}x{} cursor asset, got {}x{}",
            cursor_asset::AGENT_CURSOR_SOURCE_WIDTH,
            cursor_asset::AGENT_CURSOR_SOURCE_HEIGHT,
            image.width(),
            image.height()
        ));
    }
    Ok(image::imageops::resize(
        &image,
        cursor_asset::AGENT_CURSOR_WIDTH,
        cursor_asset::AGENT_CURSOR_HEIGHT,
        FilterType::Lanczos3,
    ))
});

/// Composite the agent cursor into an RGBA screenshot at a screenshot-pixel
/// point. Returns the modified image. A point whose cursor footprint does not
/// overlap the image at all is a structured out-of-bounds error, matching the
/// desktop synthetic-cursor contract.
pub(super) fn compose_synthetic_cursor(
    screenshot: &mut RgbaImage,
    point: PhonePoint,
) -> Result<(), DiagnosticEntry> {
    let cursor = agent_cursor_image().map_err(|error| {
        diagnostic(
            "PhoneCursorSyntheticFailed",
            "Failed to decode bundled agent cursor image.",
            Some(error),
        )
    })?;
    if !draw_cursor_image(screenshot, cursor, point.x, point.y) {
        return Err(diagnostic(
            "PhoneCursorSyntheticOutOfBounds",
            "Agent cursor point did not overlap the phone screenshot.",
            Some(format!(
                "point=({}, {}) image={}x{}",
                point.x,
                point.y,
                screenshot.width(),
                screenshot.height()
            )),
        ));
    }
    Ok(())
}

fn agent_cursor_image() -> Result<&'static RgbaImage, String> {
    match AGENT_CURSOR_IMAGE.as_ref() {
        Ok(image) => Ok(image),
        Err(error) => Err(error.clone()),
    }
}

/// Alpha-blend the cursor sprite onto `destination`, anchored at the hotspot.
/// Returns whether any pixel was drawn. Mirrors
/// `crate::overlay::synthetic_cursor::draw_cursor_image`.
fn draw_cursor_image(destination: &mut RgbaImage, cursor: &RgbaImage, x: f64, y: f64) -> bool {
    if !x.is_finite() || !y.is_finite() {
        return false;
    }

    let left = x.round() as i32 - cursor_asset::AGENT_CURSOR_HOTSPOT_X;
    let top = y.round() as i32 - cursor_asset::AGENT_CURSOR_HOTSPOT_Y;
    let width = i32::try_from(destination.width()).unwrap_or(i32::MAX);
    let height = i32::try_from(destination.height()).unwrap_or(i32::MAX);
    let mut changed = false;

    for source_y in 0..cursor.height() {
        for source_x in 0..cursor.width() {
            let source = *cursor.get_pixel(source_x, source_y);
            if source[3] == 0 {
                continue;
            }
            let px = left + source_x as i32;
            let py = top + source_y as i32;
            if px < 0 || py < 0 || px >= width || py >= height {
                continue;
            }
            blend_pixel(destination.get_pixel_mut(px as u32, py as u32), source);
            changed = true;
        }
    }

    changed
}

fn blend_pixel(destination: &mut Rgba<u8>, source: Rgba<u8>) {
    let alpha = f32::from(source[3]) / 255.0;
    for channel in 0..3 {
        destination[channel] = ((f32::from(source[channel]) * alpha)
            + (f32::from(destination[channel]) * (1.0 - alpha)))
            .round()
            .clamp(0.0, 255.0) as u8;
    }
    destination[3] = 255;
}

fn diagnostic(code: &str, message: &str, details: Option<String>) -> DiagnosticEntry {
    DiagnosticEntry {
        code: code.to_string(),
        message: message.to_string(),
        details,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::ImageBuffer;

    fn pt(x: f64, y: f64) -> PhonePoint {
        PhonePoint { x, y }
    }

    #[test]
    fn no_cursor_capabilities_reports_no_planes() {
        let caps = no_cursor_capabilities();
        assert!(!caps.host_visible_overlay);
        assert!(!caps.screenshot_synthetic_cursor);
        assert!(!caps.phone_native_overlay);
    }

    #[test]
    fn adb_only_capabilities_match_phase3_contract() {
        let caps = adb_only_capabilities(true);
        assert!(!caps.host_visible_overlay);
        assert!(caps.screenshot_synthetic_cursor);
        assert!(!caps.phone_native_overlay);

        let disabled = adb_only_capabilities(false);
        assert!(!disabled.screenshot_synthetic_cursor);
    }

    #[test]
    fn successful_action_updates_cursor_and_increments_sequence() {
        let mut tracker = PhoneCursorTracker::new("sess-1", "serial-1");
        assert!(tracker.current(0).is_none());

        let state = tracker
            .record_action(
                "sess-1",
                "serial-1",
                "phone_tap",
                Some("snap-1"),
                Some(pt(100.0, 200.0)),
                Some(pt(50.0, 100.0)),
                1_000,
            )
            .expect("own session");
        assert!(state.visible);
        assert_eq!(state.sequence, 1);
        assert_eq!(state.source_action.as_deref(), Some("phone_tap"));
        assert_eq!(state.snapshot_id.as_deref(), Some("snap-1"));

        let next = tracker
            .record_action(
                "sess-1",
                "serial-1",
                "phone_swipe",
                None,
                Some(pt(10.0, 20.0)),
                Some(pt(5.0, 10.0)),
                1_100,
            )
            .expect("own session");
        assert_eq!(next.sequence, 2);
    }

    #[test]
    fn cursor_never_leaks_across_session_or_serial() {
        let mut tracker = PhoneCursorTracker::new("sess-1", "serial-1");

        let wrong_session = tracker
            .record_action(
                "sess-2",
                "serial-1",
                "phone_tap",
                None,
                Some(pt(1.0, 1.0)),
                Some(pt(1.0, 1.0)),
                1,
            )
            .expect_err("foreign session rejected");
        assert!(matches!(wrong_session, CursorError::SessionMismatch { .. }));
        assert_eq!(wrong_session.code(), "PhoneCursorSessionMismatch");

        let wrong_serial = tracker
            .record_action(
                "sess-1",
                "serial-OTHER",
                "phone_tap",
                None,
                Some(pt(1.0, 1.0)),
                Some(pt(1.0, 1.0)),
                1,
            )
            .expect_err("foreign serial rejected");
        assert!(matches!(wrong_serial, CursorError::SerialMismatch { .. }));

        // No state leaked from the rejected updates.
        assert!(tracker.current(1).is_none());
    }

    #[test]
    fn idle_cursor_reports_hidden_after_window() {
        let mut tracker = PhoneCursorTracker::new("sess-1", "serial-1");
        tracker
            .record_action(
                "sess-1",
                "serial-1",
                "phone_tap",
                None,
                Some(pt(1.0, 1.0)),
                Some(pt(1.0, 1.0)),
                1_000,
            )
            .expect("ok");

        // Within window: visible.
        assert!(
            tracker
                .current(1_000 + IDLE_HIDE_MS)
                .expect("state")
                .visible
        );
        // Past window: hidden.
        assert!(
            !tracker
                .current(1_000 + IDLE_HIDE_MS + 1)
                .expect("state")
                .visible
        );
        // And no screenshot point is offered once hidden.
        assert!(tracker.screenshot_point(1_000 + IDLE_HIDE_MS + 1).is_none());
        // But it is offered while fresh.
        assert_eq!(
            tracker.screenshot_point(1_001).expect("point"),
            pt(1.0, 1.0)
        );
    }

    #[test]
    fn synthetic_marker_appears_near_action_point() {
        // Solid light-grey screenshot; composite at (60,60).
        let mut image: RgbaImage = ImageBuffer::from_pixel(128, 128, Rgba([240u8, 240, 240, 255]));
        compose_synthetic_cursor(&mut image, pt(60.0, 60.0)).expect("composite");

        // The cursor sprite has an opaque black outline near its hotspot region.
        // Verify at least one pixel inside the cursor footprint changed away from
        // the background, and that a far corner is untouched.
        let left = 60_i32 - cursor_asset::AGENT_CURSOR_HOTSPOT_X;
        let top = 60_i32 - cursor_asset::AGENT_CURSOR_HOTSPOT_Y;
        let mut changed_in_footprint = false;
        for dy in 0..cursor_asset::AGENT_CURSOR_HEIGHT as i32 {
            for dx in 0..cursor_asset::AGENT_CURSOR_WIDTH as i32 {
                let px = left + dx;
                let py = top + dy;
                if px < 0 || py < 0 || px >= 128 || py >= 128 {
                    continue;
                }
                if image.get_pixel(px as u32, py as u32) != &Rgba([240u8, 240, 240, 255]) {
                    changed_in_footprint = true;
                }
            }
        }
        assert!(
            changed_in_footprint,
            "cursor must paint near the action point"
        );
        assert_eq!(image.get_pixel(127, 127), &Rgba([240u8, 240, 240, 255]));
    }

    #[test]
    fn synthetic_marker_absent_for_stale_or_unrelated_session() {
        // An unrelated session never produced a cursor, so there is no
        // screenshot point and therefore no marker is composited.
        let unrelated = PhoneCursorTracker::new("sess-unrelated", "serial-X");
        assert!(unrelated.screenshot_point(10_000).is_none());

        // A stale cursor (past idle window) also yields no point, so the
        // composition step is skipped and the screenshot stays clean.
        let mut tracker = PhoneCursorTracker::new("sess-1", "serial-1");
        tracker
            .record_action(
                "sess-1",
                "serial-1",
                "phone_tap",
                None,
                Some(pt(1.0, 1.0)),
                Some(pt(40.0, 40.0)),
                0,
            )
            .expect("ok");
        assert!(tracker.screenshot_point(IDLE_HIDE_MS + 1).is_none());

        // Prove the clean image really is clean: skipping composition leaves it
        // identical to the background.
        let mut image: RgbaImage = ImageBuffer::from_pixel(64, 64, Rgba([10u8, 20, 30, 255]));
        if let Some(point) = tracker.screenshot_point(IDLE_HIDE_MS + 1) {
            compose_synthetic_cursor(&mut image, point).expect("composite");
        }
        assert!(
            image
                .pixels()
                .all(|pixel| pixel == &Rgba([10u8, 20, 30, 255])),
            "no marker should be drawn for a stale cursor"
        );
    }

    #[test]
    fn out_of_bounds_point_returns_diagnostic() {
        let mut image: RgbaImage = ImageBuffer::from_pixel(8, 8, Rgba([0u8, 0, 0, 255]));
        let err = compose_synthetic_cursor(&mut image, pt(500.0, 500.0)).expect_err("oob");
        assert_eq!(err.code, "PhoneCursorSyntheticOutOfBounds");
    }

    #[test]
    fn non_finite_point_returns_diagnostic() {
        let mut image: RgbaImage = ImageBuffer::from_pixel(8, 8, Rgba([0u8, 0, 0, 255]));
        let err = compose_synthetic_cursor(&mut image, pt(f64::NAN, 4.0)).expect_err("nan");
        assert_eq!(err.code, "PhoneCursorSyntheticOutOfBounds");
    }
}
