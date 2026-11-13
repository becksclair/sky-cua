//! The macOS socket loop, on the main run loop.

use std::io;
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixListener;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use sky_cua_overlay_host::{OverlayHostBackend, ReadableWake};

use super::{ParentWatch, run_accept_loop, serve_client, shut_down_backend};

/// Shortest run loop turn between frames, so AppKit is serviced even when
/// frames run late. It matches the turn the capture barrier is proven with.
const MIN_RUN_LOOP_TURN: Duration = Duration::from_millis(1);

/// On macOS the main thread owns AppKit, so the socket endpoint is served from
/// the main run loop: a file-descriptor source wakes it when a client
/// connects, and its timeout is the frame-pacing timer. Off the main thread
/// (the transport tests) there is no run loop to integrate with and the
/// backend is the no-op overlay, so the shared polling accept loop serves.
pub(super) fn run_unix_socket_event_loop(listener: UnixListener) -> Result<()> {
    let Some(wake) = ReadableWake::attach(listener.as_raw_fd()) else {
        return run_accept_loop(|| listener.accept().map(|(stream, _)| stream), "socket");
    };
    let mut backend = OverlayHostBackend::from_env();
    let parent = ParentWatch::new();
    let mut last_tick = Instant::now();
    loop {
        if parent.orphaned() {
            shut_down_backend(&mut backend);
            return Ok(());
        }
        loop {
            let mut stream = match listener.accept() {
                Ok((stream, _addr)) => stream,
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                // A client that gave up while queued, or a signal, costs
                // nothing; the next accept proceeds.
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::ConnectionAborted | io::ErrorKind::Interrupted
                    ) =>
                {
                    continue;
                }
                // Anything else is not transient. Exiting lets the service
                // start a fresh host instead of this one spinning on a
                // listener that stays readable.
                Err(error) => {
                    return Err(error).context("failed to accept overlay host socket connection");
                }
            };
            if serve_client(&mut backend, &mut stream, "socket") {
                return Ok(());
            }
        }
        // Frames are paced by deadline, not by a fixed sleep after each one,
        // so render time does not stretch the period. The interval is re-read
        // every turn: a request that made the overlay visible shortens it at
        // once.
        let interval = backend.pointer_tick_interval();
        let mut next_tick = last_tick + interval;
        let now = Instant::now();
        if now >= next_tick {
            // A late wake-up does not push the following frames back; after a
            // stall longer than a frame the cadence restarts from now instead
            // of bursting to catch up.
            last_tick = if now.duration_since(next_tick) > interval {
                now
            } else {
                next_tick
            };
            backend.tick();
            next_tick = last_tick + interval;
        }
        // The run loop is turned on every pass, even when frames are running
        // late, so AppKit is never starved by rendering.
        let pause = next_tick
            .saturating_duration_since(Instant::now())
            .max(MIN_RUN_LOOP_TURN);
        wake.wait(pause);
        backend.dispatch_native_events();
    }
}
