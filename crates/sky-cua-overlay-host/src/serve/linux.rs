//! The Linux socket loop: a calloop event loop with the listener as an
//! event source and a re-armable timer for frame pacing.

use std::io;
use std::os::unix::net::UnixListener;
use std::time::Duration;

use anyhow::{Context, Result};
use calloop::{
    EventLoop, Interest, LoopHandle, LoopSignal, Mode, PostAction, RegistrationToken,
    generic::Generic,
    timer::{TimeoutAction, Timer},
};
use sky_cua_overlay_host::OverlayHostBackend;

use super::{ParentWatch, serve_client, shut_down_backend};

// Initial `armed_interval` for the socket loop before its timer is first
// armed; every armed period comes from OverlayHostBackend::pointer_tick_interval().
const SOCKET_LOOP_TICK_INTERVAL: Duration = Duration::from_millis(16);

pub(super) fn run_unix_socket_event_loop(listener: UnixListener) -> Result<()> {
    let mut event_loop: EventLoop<UnixSocketLoopState> =
        EventLoop::try_new().context("failed to create overlay host socket event loop")?;
    let signal = event_loop.get_signal();
    let handle = event_loop.handle();
    let listener_source = Generic::new(
        listener
            .try_clone()
            .context("failed to clone overlay host socket listener for event loop")?,
        Interest::READ,
        Mode::Level,
    );
    handle
        .insert_source(listener_source, |_readiness, _listener, state| {
            state.accept_ready();
            if let Err(error) = state.rearm_tick_timer_if_sooner() {
                eprintln!("{error:#}");
            }
            Ok(PostAction::Continue)
        })
        .context("failed to register overlay host socket listener")?;

    let mut state = UnixSocketLoopState {
        listener,
        backend: OverlayHostBackend::from_env(),
        signal,
        label: "socket",
        handle: handle.clone(),
        tick_timer: None,
        armed_interval: SOCKET_LOOP_TICK_INTERVAL,
        parent: ParentWatch::new(),
    };
    state.rearm_tick_timer_if_sooner()?;
    event_loop
        .run(None::<Duration>, &mut state, |_state| {})
        .context("overlay host socket event loop failed")
}

struct UnixSocketLoopState {
    listener: UnixListener,
    backend: OverlayHostBackend,
    signal: LoopSignal,
    label: &'static str,
    handle: LoopHandle<'static, UnixSocketLoopState>,
    tick_timer: Option<RegistrationToken>,
    armed_interval: Duration,
    parent: ParentWatch,
}

impl UnixSocketLoopState {
    /// (Re)arms the frame-pacing timer when the backend now wants a faster
    /// cadence than the armed one, so a request that makes the overlay visible
    /// does not wait out an idle period armed while it was hidden. The new
    /// timer is registered before the old one is removed, so a failed insert
    /// keeps the existing timer ticking.
    fn rearm_tick_timer_if_sooner(&mut self) -> Result<()> {
        let interval = self.backend.pointer_tick_interval();
        if self.tick_timer.is_some() && interval >= self.armed_interval {
            return Ok(());
        }
        let token = self
            .handle
            .insert_source(
                Timer::from_duration(interval),
                |_deadline, _timer, state| {
                    if state.parent.orphaned() {
                        shut_down_backend(&mut state.backend);
                        state.signal.stop();
                        return TimeoutAction::Drop;
                    }
                    state.backend.tick();
                    state.armed_interval = state.backend.pointer_tick_interval();
                    TimeoutAction::ToDuration(state.armed_interval)
                },
            )
            .map_err(|error| {
                anyhow::anyhow!("failed to register overlay host socket timer: {error}")
            })?;
        if let Some(previous) = self.tick_timer.replace(token) {
            self.handle.remove(previous);
        }
        self.armed_interval = interval;
        Ok(())
    }

    fn accept_ready(&mut self) {
        loop {
            let mut stream = match self.listener.accept() {
                Ok((stream, _addr)) => stream,
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => return,
                Err(error) => {
                    eprintln!(
                        "failed to accept overlay host {} connection: {error}",
                        self.label
                    );
                    return;
                }
            };
            if serve_client(&mut self.backend, &mut stream, self.label) {
                self.signal.stop();
                return;
            }
        }
    }
}
