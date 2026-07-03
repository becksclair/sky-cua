//! The hide-for-capture barrier: a sequence-bearing `Hide` presents
//! transparent frames on every active surface and the reply is held until the
//! compositor has acknowledged `CAPTURE_BARRIER_FRAMES` frame callbacks per
//! surface, so capture never reads a half-hidden overlay.

use super::*;

#[derive(Debug, Clone, Copy)]
pub(super) struct CaptureBarrierState {
    pub(super) sequence: u64,
}

impl LayerShellOverlayBackend {
    pub(super) fn wait_for_capture_barrier(&mut self) -> Result<()> {
        use std::time::{Duration, Instant};
        const BARRIER_TIMEOUT: Duration = Duration::from_millis(1500);
        let deadline = Instant::now() + BARRIER_TIMEOUT;
        while Instant::now() < deadline {
            if self.app.capture_barrier.is_none() {
                return Ok(());
            }
            self.event_queue
                .roundtrip(&mut self.app)
                .context("Wayland roundtrip failed while waiting for capture barrier")?;
            if self.app.capture_barrier.is_none() {
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        self.app.clear_capture_barrier();
        bail!("capture barrier timed out waiting for compositor frame acknowledgement")
    }
}

impl LayerShellApp {
    pub(super) fn applied_sequence(&self) -> Option<u64> {
        self.capture_barrier.map(|barrier| barrier.sequence)
    }
    pub(super) fn start_capture_barrier(&mut self, sequence: u64) {
        let frames = sky_cua_platform::overlay_spec::shared::effects::CAPTURE_BARRIER_FRAMES;
        let mut active_surfaces = 0;
        for entry in &mut self.layers {
            if !entry.closed && entry.configured {
                entry.capture_barrier_frames_remaining = frames;
                active_surfaces += 1;
            } else {
                entry.capture_barrier_frames_remaining = 0;
            }
        }
        self.capture_barrier = (active_surfaces > 0).then_some(CaptureBarrierState { sequence });
    }
    pub(super) fn capture_barrier_complete(&self) -> bool {
        self.layers
            .iter()
            .filter(|entry| !entry.closed && entry.configured)
            .all(|entry| entry.capture_barrier_frames_remaining == 0)
    }
    pub(super) fn clear_capture_barrier(&mut self) {
        self.capture_barrier = None;
        for entry in &mut self.layers {
            entry.capture_barrier_frames_remaining = 0;
        }
    }
}
