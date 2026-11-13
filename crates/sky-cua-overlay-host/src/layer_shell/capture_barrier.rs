//! The Wayland hide-for-capture barrier: a sequence-bearing `Hide` presents
//! transparent frames on every active surface and the reply is held until the
//! compositor has acknowledged `CAPTURE_BARRIER_FRAMES` frame callbacks per
//! surface, so capture never reads a half-hidden overlay.

use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};

use super::LayerShellState;

const BARRIER_TIMEOUT: Duration = Duration::from_millis(1500);

pub(super) fn wait(
    event_queue: &mut wayland_client::EventQueue<LayerShellState>,
    state: &mut LayerShellState,
) -> Result<()> {
    let deadline = Instant::now() + BARRIER_TIMEOUT;
    while Instant::now() < deadline {
        if !state.capture_barrier_pending {
            return Ok(());
        }
        event_queue
            .roundtrip(state)
            .context("Wayland roundtrip failed while waiting for capture barrier")?;
        if !state.capture_barrier_pending {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    state.clear_capture_barrier();
    bail!("capture barrier timed out waiting for compositor frame acknowledgement")
}

impl LayerShellState {
    pub(super) fn start_capture_barrier(&mut self) {
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
        self.capture_barrier_pending = active_surfaces > 0;
    }

    /// Clears the pending flag once every active surface has counted its
    /// frame callbacks down. Called from the handlers that change the counts.
    pub(super) fn settle_capture_barrier(&mut self) {
        if self.capture_barrier_pending
            && self
                .layers
                .iter()
                .filter(|entry| !entry.closed && entry.configured)
                .all(|entry| entry.capture_barrier_frames_remaining == 0)
        {
            self.capture_barrier_pending = false;
        }
    }

    pub(super) fn clear_capture_barrier(&mut self) {
        self.capture_barrier_pending = false;
        for entry in &mut self.layers {
            entry.capture_barrier_frames_remaining = 0;
        }
    }
}
