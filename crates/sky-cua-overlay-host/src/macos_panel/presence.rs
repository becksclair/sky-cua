//! Where the overlay panels are relative to the screen, and what each frame
//! does about it.
//!
//! A Metal layer may only be presented to while the window server is
//! compositing its window: a drawable presented to an off-screen window is
//! held until the window is shown, and once the layer's few drawables are all
//! held the next frame blocks the main thread. AppKit commits ordering to the
//! window server on a later run loop turn and reports visibility back later
//! still, so the panels are drawn only after the window server itself lists
//! them on screen.
//!
//! Panels that are on screen can still become occluded: an opaque window
//! above them, as the lock screen puts up, leaves them ordered in but not
//! composited, and the renderer skips an occluded window. A frame that wants
//! the overlay visible is therefore skipped whenever no panel is unoccluded,
//! before the host advances its motion driver for it. A sleeping display does
//! not occlude them; frames keep landing on it.
//!
//! Hidden panels are ordered out, so a hidden overlay costs neither GPU work
//! nor window-server compositing. The frame that hides is still drawn,
//! transparent, before the panels are ordered out, so they hold no stale
//! image when they come back.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PanelPresence {
    OrderedOut,
    /// Ordered in, not yet able to take a frame: the window server has not
    /// listed the panels on screen, or AppKit still reports them occluded.
    OrderingIn,
    OnScreen,
}

/// What the start of a frame does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum FrameStart {
    /// The panels are on screen: render into them.
    Draw,
    /// Nothing can be presented this frame.
    Skip,
    /// Order the panels in, and render from a later frame on.
    OrderIn,
}

impl PanelPresence {
    /// Decides the start of a frame that wants the overlay `visible`.
    ///
    /// `unoccluded` asks whether AppKit reports at least one panel as not
    /// occluded; it is cheap and consulted on every visible frame once the
    /// panels are ordered in. `listed_on_screen` asks whether the window
    /// server lists the panels on screen; it allocates and is consulted only
    /// while the panels are being ordered in, after `unoccluded` passed.
    pub(super) fn begin_frame(
        self,
        visible: bool,
        unoccluded: impl Fn() -> bool,
        listed_on_screen: impl FnOnce() -> bool,
    ) -> (Self, FrameStart) {
        match self {
            // Every panel got covered under a visible overlay: none can take
            // the frame, and they stay ordered in for when one is uncovered.
            Self::OnScreen if visible && !unoccluded() => (Self::OnScreen, FrameStart::Skip),
            // A hiding frame is drawn too: it is the transparent one.
            Self::OnScreen => (Self::OnScreen, FrameStart::Draw),
            Self::OrderedOut if visible => (Self::OrderingIn, FrameStart::OrderIn),
            Self::OrderedOut => (Self::OrderedOut, FrameStart::Skip),
            Self::OrderingIn if visible && unoccluded() && listed_on_screen() => {
                (Self::OnScreen, FrameStart::Draw)
            }
            Self::OrderingIn => (Self::OrderingIn, FrameStart::Skip),
        }
    }

    /// Whether the end of a frame that wanted the overlay `visible` orders the
    /// panels out.
    pub(super) fn orders_out_after_frame(self, visible: bool) -> bool {
        !visible && self != Self::OrderedOut
    }
}

#[cfg(test)]
mod tests {
    use super::{FrameStart, PanelPresence};

    fn never_asked() -> bool {
        panic!("this frame must not consult the windowing system")
    }

    #[test]
    fn showing_orders_in_then_waits_for_the_window_server_before_drawing() {
        let (presence, start) =
            PanelPresence::OrderedOut.begin_frame(true, never_asked, never_asked);
        assert_eq!(
            (presence, start),
            (PanelPresence::OrderingIn, FrameStart::OrderIn)
        );
        assert!(!presence.orders_out_after_frame(true));

        // Still occluded: no present, the window list is not queried, and
        // the panels stay ordered in.
        let (presence, start) = presence.begin_frame(true, || false, never_asked);
        assert_eq!(
            (presence, start),
            (PanelPresence::OrderingIn, FrameStart::Skip)
        );
        assert!(!presence.orders_out_after_frame(true));

        // Unoccluded but not listed yet: still no present.
        let (presence, start) = presence.begin_frame(true, || true, || false);
        assert_eq!(
            (presence, start),
            (PanelPresence::OrderingIn, FrameStart::Skip)
        );

        let (presence, start) = presence.begin_frame(true, || true, || true);
        assert_eq!(
            (presence, start),
            (PanelPresence::OnScreen, FrameStart::Draw)
        );

        // Once on screen the window list is not consulted again.
        let (presence, start) = presence.begin_frame(true, || true, never_asked);
        assert_eq!(
            (presence, start),
            (PanelPresence::OnScreen, FrameStart::Draw)
        );
    }

    #[test]
    fn covered_panels_skip_visible_frames_and_stay_ordered_in() {
        let (presence, start) = PanelPresence::OnScreen.begin_frame(true, || false, never_asked);
        assert_eq!(
            (presence, start),
            (PanelPresence::OnScreen, FrameStart::Skip)
        );
        assert!(!presence.orders_out_after_frame(true));

        // A panel is uncovered: frames resume without ordering in again.
        let (presence, start) = presence.begin_frame(true, || true, never_asked);
        assert_eq!(
            (presence, start),
            (PanelPresence::OnScreen, FrameStart::Draw)
        );
    }

    #[test]
    fn hiding_draws_the_transparent_frame_then_orders_out() {
        // Occlusion is not asked about: a hiding frame is attempted either
        // way, and the panels are ordered out after it.
        let (presence, start) =
            PanelPresence::OnScreen.begin_frame(false, never_asked, never_asked);
        assert_eq!(
            (presence, start),
            (PanelPresence::OnScreen, FrameStart::Draw)
        );
        assert!(presence.orders_out_after_frame(false));
    }

    #[test]
    fn hiding_before_the_panels_reach_the_screen_presents_nothing() {
        // Nothing may promote a panel the frame wants hidden.
        let (presence, start) =
            PanelPresence::OrderingIn.begin_frame(false, never_asked, never_asked);
        assert_eq!(
            (presence, start),
            (PanelPresence::OrderingIn, FrameStart::Skip)
        );
        assert!(presence.orders_out_after_frame(false));
    }

    #[test]
    fn a_hidden_overlay_that_is_already_off_screen_does_nothing() {
        let (presence, start) =
            PanelPresence::OrderedOut.begin_frame(false, never_asked, never_asked);
        assert_eq!(
            (presence, start),
            (PanelPresence::OrderedOut, FrameStart::Skip)
        );
        assert!(!presence.orders_out_after_frame(false));
    }
}
