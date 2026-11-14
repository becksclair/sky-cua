//! Whether the frames the overlay wants shown are reaching a surface.
//!
//! A frame that should be visible can find no surface to take it: a platform
//! is still bringing its surfaces on screen, or every one of them is covered.
//! The host must then neither report what the motion driver did (nobody saw
//! it) nor hold a request for long, and once the surfaces have been away for
//! a full wait it stops treating the overlay as something a person is
//! watching: gesture feedback is abandoned and the frame timer idles.

use std::time::{Duration, Instant};

/// How long surfaces may fail to take a visible frame before the host stops
/// waiting for them. A freshly started macOS host needs about 250 ms before
/// its first panel can take a frame, and the service's request timeout is
/// 2 s.
pub(super) const PRESENT_WAIT: Duration = Duration::from_secs(1);

/// What a frame did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum FrameOutcome {
    /// At least one surface took the frame.
    Presented,
    /// The overlay is hidden and its surfaces are off screen: there was
    /// nothing to present.
    NothingToShow,
    /// The overlay should be visible, but no surface could take a frame (a
    /// platform still bringing its surfaces on screen, or surfaces the
    /// windowing system reports as occluded).
    AwaitingSurfaces,
}

#[derive(Debug, Default)]
pub(super) struct Presentation {
    /// When the current run of visible frames that no surface took began.
    awaiting_since: Option<Instant>,
    /// The current run has lasted `PRESENT_WAIT`.
    surfaces_gone: bool,
    /// A run lasted `PRESENT_WAIT` and no frame has landed since. Outlives
    /// the run: a hidden frame that presented nothing proves nothing, and
    /// without this every hide and show pair on covered surfaces would hold
    /// its request for a full wait again.
    wait_exhausted: bool,
}

impl Presentation {
    pub(super) fn record(&mut self, outcome: FrameOutcome, now: Instant) {
        match outcome {
            FrameOutcome::Presented => *self = Self::default(),
            FrameOutcome::NothingToShow => {
                self.awaiting_since = None;
                self.surfaces_gone = false;
            }
            FrameOutcome::AwaitingSurfaces => {
                let since = *self.awaiting_since.get_or_insert(now);
                if now.saturating_duration_since(since) >= PRESENT_WAIT {
                    self.surfaces_gone = true;
                    self.wait_exhausted = true;
                }
            }
        }
    }

    /// The latest frame should have been visible but no surface took it. The
    /// motion echo and the arrival wait never report an arrival meanwhile.
    pub(super) fn awaiting(&self) -> bool {
        self.awaiting_since.is_some()
    }

    /// A request should not wait for a frame to land: the wait already ran
    /// out, in this run or in an earlier one that no frame has ended. Costs
    /// surfaces that show nothing one wait, not one per request.
    pub(super) fn wait_exhausted(&self) -> bool {
        self.awaiting() && self.wait_exhausted
    }

    /// The surfaces have themselves been away for a full wait, measured in
    /// this run. Stricter than [`Self::wait_exhausted`], which a show after
    /// an earlier stall reports at once although its frame may be about to
    /// land.
    pub(super) fn surfaces_gone(&self) -> bool {
        self.surfaces_gone
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::{FrameOutcome, PRESENT_WAIT, Presentation};

    const FRAME: Duration = Duration::from_millis(8);

    #[test]
    fn a_frame_that_lands_is_neither_awaited_nor_stalled() {
        let mut presentation = Presentation::default();
        presentation.record(FrameOutcome::Presented, Instant::now());
        assert!(!presentation.awaiting());
        assert!(!presentation.wait_exhausted());
        assert!(!presentation.surfaces_gone());
    }

    #[test]
    fn surfaces_count_as_gone_only_after_a_full_wait_of_unshown_frames() {
        let start = Instant::now();
        let mut presentation = Presentation::default();
        presentation.record(FrameOutcome::AwaitingSurfaces, start);
        presentation.record(FrameOutcome::AwaitingSurfaces, start + PRESENT_WAIT - FRAME);
        assert!(presentation.awaiting());
        assert!(!presentation.wait_exhausted());
        assert!(!presentation.surfaces_gone());

        presentation.record(FrameOutcome::AwaitingSurfaces, start + PRESENT_WAIT);
        assert!(presentation.wait_exhausted());
        assert!(presentation.surfaces_gone());
    }

    #[test]
    fn a_frame_that_lands_ends_the_run_and_the_next_one_is_timed_afresh() {
        let start = Instant::now();
        let mut presentation = Presentation::default();
        presentation.record(FrameOutcome::AwaitingSurfaces, start);
        presentation.record(FrameOutcome::AwaitingSurfaces, start + PRESENT_WAIT);
        presentation.record(FrameOutcome::Presented, start + PRESENT_WAIT + FRAME);
        assert!(!presentation.awaiting());
        assert!(!presentation.surfaces_gone());

        let again = start + PRESENT_WAIT * 3;
        presentation.record(FrameOutcome::AwaitingSurfaces, again);
        assert!(presentation.awaiting());
        assert!(!presentation.wait_exhausted());
        assert!(!presentation.surfaces_gone());
    }

    #[test]
    fn a_show_after_a_stall_and_a_hide_does_not_wait_again_but_is_timed_afresh() {
        let start = Instant::now();
        let mut presentation = Presentation::default();
        presentation.record(FrameOutcome::AwaitingSurfaces, start);
        presentation.record(FrameOutcome::AwaitingSurfaces, start + PRESENT_WAIT);

        // The hide presents nothing, so it does not prove a frame can land.
        presentation.record(FrameOutcome::NothingToShow, start + PRESENT_WAIT + FRAME);
        assert!(!presentation.awaiting());
        assert!(!presentation.wait_exhausted());
        assert!(!presentation.surfaces_gone());

        // The next show must not hold its request, but its surfaces may be
        // about to take a frame: they are not yet known to be gone.
        let shown = start + PRESENT_WAIT * 2;
        presentation.record(FrameOutcome::AwaitingSurfaces, shown);
        assert!(presentation.wait_exhausted());
        assert!(!presentation.surfaces_gone());

        presentation.record(FrameOutcome::AwaitingSurfaces, shown + PRESENT_WAIT);
        assert!(presentation.surfaces_gone());
    }
}
