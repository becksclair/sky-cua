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
    /// `listed_on_screen` asks whether the panels can take a frame now (the
    /// window server lists them on screen and they are not occluded); it is
    /// consulted only while the panels are being ordered in.
    pub(super) fn begin_frame(
        self,
        visible: bool,
        listed_on_screen: impl FnOnce() -> bool,
    ) -> (Self, FrameStart) {
        match self {
            // A hiding frame is drawn too: it is the transparent one.
            Self::OnScreen => (Self::OnScreen, FrameStart::Draw),
            Self::OrderedOut if visible => (Self::OrderingIn, FrameStart::OrderIn),
            Self::OrderedOut => (Self::OrderedOut, FrameStart::Skip),
            Self::OrderingIn if visible && listed_on_screen() => (Self::OnScreen, FrameStart::Draw),
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
        panic!("the window list is consulted only while ordering in")
    }

    #[test]
    fn showing_orders_in_then_waits_for_the_window_server_before_drawing() {
        let (presence, start) = PanelPresence::OrderedOut.begin_frame(true, never_asked);
        assert_eq!(
            (presence, start),
            (PanelPresence::OrderingIn, FrameStart::OrderIn)
        );
        assert!(!presence.orders_out_after_frame(true));

        // Not listed yet: no present, and the panels stay ordered in.
        let (presence, start) = presence.begin_frame(true, || false);
        assert_eq!(
            (presence, start),
            (PanelPresence::OrderingIn, FrameStart::Skip)
        );
        assert!(!presence.orders_out_after_frame(true));

        let (presence, start) = presence.begin_frame(true, || true);
        assert_eq!(
            (presence, start),
            (PanelPresence::OnScreen, FrameStart::Draw)
        );

        // Once on screen the window list is not consulted again.
        let (presence, start) = presence.begin_frame(true, never_asked);
        assert_eq!(
            (presence, start),
            (PanelPresence::OnScreen, FrameStart::Draw)
        );
    }

    #[test]
    fn hiding_draws_the_transparent_frame_then_orders_out() {
        let (presence, start) = PanelPresence::OnScreen.begin_frame(false, never_asked);
        assert_eq!(
            (presence, start),
            (PanelPresence::OnScreen, FrameStart::Draw)
        );
        assert!(presence.orders_out_after_frame(false));
    }

    #[test]
    fn hiding_before_the_panels_reach_the_screen_presents_nothing() {
        // The window list must not promote a panel the frame wants hidden.
        let (presence, start) = PanelPresence::OrderingIn.begin_frame(false, || true);
        assert_eq!(
            (presence, start),
            (PanelPresence::OrderingIn, FrameStart::Skip)
        );
        assert!(presence.orders_out_after_frame(false));
    }

    #[test]
    fn a_hidden_overlay_that_is_already_off_screen_does_nothing() {
        let (presence, start) = PanelPresence::OrderedOut.begin_frame(false, never_asked);
        assert_eq!(
            (presence, start),
            (PanelPresence::OrderedOut, FrameStart::Skip)
        );
        assert!(!presence.orders_out_after_frame(false));
    }
}
