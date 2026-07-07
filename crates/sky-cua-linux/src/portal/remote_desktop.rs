use std::os::fd::OwnedFd;
use std::sync::Arc;
use std::time::Duration;

use ashpd::desktop::remote_desktop::RemoteDesktop;
use ashpd::desktop::screencast::Screencast;
use sky_cua_platform::diagnostics::{BackendError, BackendErrorCode};
use sky_cua_platform::model::{PortalTokenResetOutcome, RectF};
use tokio::sync::RwLock;
use tracing::{debug, warn};

use crate::portal::eis_input::EisWorkerHandle;
use crate::portal::legacy_input;
use crate::portal::pipewire::{self, PipeWireFrameCapture};
use crate::portal::portal_session::{preauthorize_with_timeout, start_session_with_timeout};
use crate::portal::session::portal_u32_property;
use crate::portal::token_store::PortalTokenStore;

// Backward-compatible re-exports of items that were previously declared in this module.
pub use crate::portal::eis_keymap::{keysym_for_char, keysym_for_key_name};

#[derive(Debug, Clone, PartialEq)]
pub struct PortalStreamInfo {
    pub node_id: u32,
    pub stream_id: Option<String>,
    pub mapping_id: Option<String>,
    pub source_type: Option<u32>,
    pub logical_rect: Option<RectF>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Middle,
    Right,
}

#[derive(Debug, Clone, Default)]
pub struct RemoteDesktopSessionManager {
    pub(crate) inner: Arc<RwLock<RemoteDesktopState>>,
    pub(crate) token_store: Option<PortalTokenStore>,
}

#[derive(Debug)]
pub(crate) struct ActiveRemoteDesktopSession {
    pub remote_desktop: RemoteDesktop,
    pub screencast: Screencast,
    pub session: ashpd::desktop::Session<RemoteDesktop>,
    pub primary_stream: Option<PortalStreamInfo>,
    pub pipewire_remote_fd: Option<OwnedFd>,
    pub eis_worker: Option<EisWorkerHandle>,
}

#[derive(Debug, Default)]
pub(crate) struct RemoteDesktopState {
    pub session: Option<ActiveRemoteDesktopSession>,
    pub session_generation: u64,
    pub pending_events: Vec<PortalLifecycleEvent>,
}

impl RemoteDesktopState {
    pub(crate) fn set_session(&mut self, session: ActiveRemoteDesktopSession) {
        self.session = Some(session);
        self.session_generation = self.session_generation.wrapping_add(1);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortalLifecycleEvent {
    pub code: &'static str,
    pub message: String,
    pub details: Option<String>,
}

const PIPEWIRE_REMOTE_OPEN_TIMEOUT: Duration = Duration::from_secs(5);
pub(crate) const PORTAL_SESSION_STARTED: &str = "PortalSessionStarted";
pub(crate) const PORTAL_SESSION_RESTORED: &str = "PortalSessionRestored";
pub(crate) const PORTAL_SESSION_RESTORE_MISS: &str = "PortalSessionRestoreMiss";
pub(crate) const PORTAL_SESSION_REBUILT: &str = "PortalSessionRebuilt";
pub(crate) const PORTAL_SESSION_TOKEN_ROTATED: &str = "PortalSessionTokenRotated";
pub(crate) const PORTAL_EIS_INPUT_USED: &str = "PortalEisInputUsed";
pub(crate) const PORTAL_EIS_INPUT_FALLBACK: &str = "PortalEisInputFallback";
pub(crate) const PORTAL_EIS_POINTER_DISABLED: &str = "PortalEisPointerDisabled";
pub(crate) const EIS_POINT_OUTSIDE_REGION_PREFIX: &str = "EIS absolute point";

pub async fn version() -> Result<u32, BackendError> {
    portal_u32_property("org.freedesktop.portal.RemoteDesktop", "version").await
}

pub async fn available_device_types() -> Result<u32, BackendError> {
    portal_u32_property(
        "org.freedesktop.portal.RemoteDesktop",
        "AvailableDeviceTypes",
    )
    .await
}

impl RemoteDesktopSessionManager {
    #[must_use]
    pub fn new() -> Self {
        let token_store = match PortalTokenStore::new() {
            Ok(store) => Some(store),
            Err(error) => {
                warn!(
                    message = %error,
                    "failed to initialize the persisted portal token store; portal approvals will not survive process restarts"
                );
                None
            }
        };
        Self {
            inner: Arc::new(RwLock::new(RemoteDesktopState::default())),
            token_store,
        }
    }

    pub async fn ensure_started(&self) -> Result<Option<PortalStreamInfo>, BackendError> {
        self.ensure_session_started().await?;
        let state = self.inner.read().await;
        Ok(state
            .session
            .as_ref()
            .and_then(|session| session.primary_stream.clone()))
    }

    pub async fn primary_stream(&self) -> Result<Option<PortalStreamInfo>, BackendError> {
        self.ensure_started().await
    }

    pub async fn capture_frame(
        &self,
        snapshot_id: &str,
    ) -> Result<PipeWireFrameCapture, BackendError> {
        match self.prepare_capture().await {
            Ok((node_id, remote_fd)) => {
                pipewire::capture_png_frame(snapshot_id, node_id, remote_fd).await
            }
            Err(error)
                if error.code == BackendErrorCode::PipeWireUnavailable.as_str()
                    || error.code == BackendErrorCode::PipeWireStreamFailed.as_str() =>
            {
                warn!(
                    message = %error.message,
                    "PipeWire capture failed on the cached portal session; resetting and retrying once"
                );
                let old_session = {
                    let mut state = self.inner.write().await;
                    state.session.take()
                };
                if let Some(session) = old_session {
                    close_session(&session).await;
                }
                let mut started = start_session_with_timeout(self.token_store.as_ref()).await?;
                started.lifecycle_events.insert(
                    0,
                    PortalLifecycleEvent {
                        code: PORTAL_SESSION_REBUILT,
                        message: "Rebuilt the cached portal session after PipeWire capture failed."
                            .to_string(),
                        details: Some(error.message),
                    },
                );
                let (unused_session, extra_events) = {
                    let mut state = self.inner.write().await;
                    if state.session.is_none() {
                        state.pending_events.extend(started.lifecycle_events);
                        state.set_session(started.session);
                        (None, Vec::new())
                    } else {
                        (Some(started.session), started.lifecycle_events)
                    }
                };
                if let Some(session) = unused_session {
                    self.push_lifecycle_events(extra_events).await;
                    close_session(&session).await;
                }
                let (node_id, remote_fd) = self.prepare_capture().await?;
                pipewire::capture_png_frame(snapshot_id, node_id, remote_fd).await
            }
            Err(error) => Err(error),
        }
    }

    /// Ensures the portal session is started and the PipeWire remote fd is cached,
    /// then returns the capture parameters (node_id, duplicated fd) so the caller
    /// can perform the blocking GStreamer capture without holding the RwLock.
    ///
    /// This method uses a read lock for the async D-Bus call and only briefly
    /// acquires a write lock to store the cached fd, so concurrent input actions
    /// are not blocked for the full portal startup or PipeWire open duration.
    async fn prepare_capture(&self) -> Result<(u32, OwnedFd), BackendError> {
        // Step 1: Ensure the session exists. This manages its own lock and may
        // perform the full portal handshake, so it must not run while we hold
        // any lock on `inner`.
        self.ensure_session_started().await?;

        // Step 2: Fast path – the PipeWire fd is already cached.
        {
            let state = self.inner.read().await;
            if let Some(session) = state.session.as_ref()
                && let Some(stream) = session.primary_stream.as_ref()
            {
                let node_id = stream.node_id;
                if let Some(ref fd) = session.pipewire_remote_fd {
                    let dup = pipewire::duplicate_remote_fd(fd)?;
                    return Ok((node_id, dup));
                }
            }
        }

        // Step 3: Open the PipeWire remote while holding only a *read* lock.
        // This lets concurrent input actions proceed in parallel.
        let (node_id, generation_when_opened, remote_fd) = {
            let state = self.inner.read().await;
            let session = state.session.as_ref().ok_or_else(|| {
                BackendError::new(
                    BackendErrorCode::Internal,
                    "portal session was reset concurrently",
                )
            })?;
            let stream = session.primary_stream.as_ref().ok_or_else(|| {
                BackendError::new(
                    BackendErrorCode::PipeWireStreamFailed,
                    "RemoteDesktop session started without a screencast stream, so no PipeWire node is available",
                )
            })?;
            let node_id = stream.node_id;

            // Another task may have opened the fd while we were acquiring the read lock.
            if let Some(ref fd) = session.pipewire_remote_fd {
                let dup = pipewire::duplicate_remote_fd(fd)?;
                return Ok((node_id, dup));
            }

            let generation = state.session_generation;
            debug!(
                node_id,
                "opening PipeWire remote for the active portal session"
            );
            let remote_fd = tokio::time::timeout(
                PIPEWIRE_REMOTE_OPEN_TIMEOUT,
                session
                    .screencast
                    .open_pipe_wire_remote(&session.session, Default::default()),
            )
            .await
            .map_err(|_| {
                BackendError::new(
                    BackendErrorCode::PipeWireUnavailable,
                    "timed out opening the PipeWire remote for the screencast session",
                )
            })?
            .map_err(|error| {
                BackendError::new(
                    BackendErrorCode::PipeWireUnavailable,
                    format!(
                        "failed to open the PipeWire remote for the screencast session: {error}"
                    ),
                )
            })?;

            (node_id, generation, remote_fd)
        };

        // Step 4: Store the fd under a brief write lock. Re-check the generation
        // so we do not cache the fd for a session that was replaced while we were
        // waiting on the D-Bus call.
        let cached_fd = {
            let mut state = self.inner.write().await;
            if state.session_generation == generation_when_opened
                && let Some(session) = state.session.as_mut()
            {
                session.pipewire_remote_fd = Some(remote_fd);
            }
            state
                .session
                .as_ref()
                .and_then(|s| s.pipewire_remote_fd.as_ref().map(|fd| fd.try_clone()))
                .ok_or_else(|| {
                    BackendError::new(
                        BackendErrorCode::Internal,
                        "portal session was replaced while opening the PipeWire remote",
                    )
                })?
                .map_err(|error| {
                    BackendError::new(
                        BackendErrorCode::Internal,
                        format!("failed to duplicate PipeWire remote fd: {error}"),
                    )
                })?
        };
        pipewire::duplicate_remote_fd(&cached_fd).map(|dup| (node_id, dup))
    }

    pub async fn take_lifecycle_events(&self) -> Vec<PortalLifecycleEvent> {
        let mut state = self.inner.write().await;
        std::mem::take(&mut state.pending_events)
    }

    pub(crate) async fn push_lifecycle_event(&self, event: PortalLifecycleEvent) {
        let mut state = self.inner.write().await;
        state.pending_events.push(event);
    }

    async fn push_lifecycle_events(&self, events: Vec<PortalLifecycleEvent>) {
        if events.is_empty() {
            return;
        }
        let mut state = self.inner.write().await;
        state.pending_events.extend(events);
    }

    pub async fn reset_session(&self) {
        let old_session = {
            let mut state = self.inner.write().await;
            state.session.take()
        };
        if let Some(session) = old_session {
            close_session(&session).await;
        }
    }

    pub async fn preauthorize_permissions(&self) {
        preauthorize_with_timeout(self.token_store.as_ref()).await;
    }

    pub async fn pointer_move_absolute(&self, x: f64, y: f64) -> Result<(), BackendError> {
        self.ensure_session_started().await?;
        let state = self.inner.read().await;
        let session = state.session.as_ref().ok_or_else(|| {
            BackendError::new(
                BackendErrorCode::Internal,
                "portal session was reset concurrently",
            )
        })?;
        let stream = session.primary_stream.as_ref().ok_or_else(|| {
            BackendError::new(
                BackendErrorCode::ActionUnsupportedForEnvironment,
                "RemoteDesktop session started without a screencast stream for absolute motion",
            )
        })?;
        legacy_input::pointer_move_absolute(&session.remote_desktop, &session.session, stream, x, y)
            .await
    }

    pub async fn pointer_button(
        &self,
        button: MouseButton,
        pressed: bool,
    ) -> Result<(), BackendError> {
        self.ensure_session_started().await?;
        let state = self.inner.read().await;
        let session = state.session.as_ref().ok_or_else(|| {
            BackendError::new(
                BackendErrorCode::Internal,
                "portal session was reset concurrently",
            )
        })?;
        legacy_input::pointer_button(&session.remote_desktop, &session.session, button, pressed)
            .await
    }

    pub async fn click(&self, button: MouseButton) -> Result<(), BackendError> {
        self.pointer_button(button, true).await?;
        tokio::time::sleep(Duration::from_millis(15)).await;
        self.pointer_button(button, false).await
    }

    pub async fn scroll_vertical_discrete(&self, steps: i32) -> Result<(), BackendError> {
        self.ensure_session_started().await?;
        let state = self.inner.read().await;
        let session = state.session.as_ref().ok_or_else(|| {
            BackendError::new(
                BackendErrorCode::Internal,
                "portal session was reset concurrently",
            )
        })?;
        legacy_input::scroll_vertical_discrete(&session.remote_desktop, &session.session, steps)
            .await
    }

    pub async fn scroll_vertical_smooth(&self, delta_y: f64) -> Result<(), BackendError> {
        self.ensure_session_started().await?;
        let state = self.inner.read().await;
        let session = state.session.as_ref().ok_or_else(|| {
            BackendError::new(
                BackendErrorCode::Internal,
                "portal session was reset concurrently",
            )
        })?;
        legacy_input::scroll_vertical_smooth(&session.remote_desktop, &session.session, delta_y)
            .await
    }

    pub(crate) async fn send_keysym_raw(&self, keysym: i32) -> Result<(), BackendError> {
        self.ensure_session_started().await?;
        let state = self.inner.read().await;
        let session = state.session.as_ref().ok_or_else(|| {
            BackendError::new(
                BackendErrorCode::Internal,
                "portal session was reset concurrently",
            )
        })?;
        legacy_input::send_keysym_raw(&session.remote_desktop, &session.session, keysym).await
    }

    pub(crate) async fn send_keysym_state(
        &self,
        keysym: i32,
        state: ashpd::desktop::remote_desktop::KeyState,
    ) -> Result<(), BackendError> {
        self.ensure_session_started().await?;
        let manager_state = self.inner.read().await;
        let session = manager_state.session.as_ref().ok_or_else(|| {
            BackendError::new(
                BackendErrorCode::Internal,
                "portal session was reset concurrently",
            )
        })?;
        legacy_input::send_keysym_state(&session.remote_desktop, &session.session, keysym, state)
            .await
    }

    /// Fast-path session check: tries a read lock first; only upgrades to a
    /// write lock if the session is missing. The full portal startup handshake
    /// runs without holding any lock so concurrent input and capture are not
    /// blocked for the approval timeout duration.
    pub(crate) async fn ensure_session_started(&self) -> Result<(), BackendError> {
        {
            let state = self.inner.read().await;
            if state.session.is_some() {
                return Ok(());
            }
        }
        let started = start_session_with_timeout(self.token_store.as_ref()).await?;
        let mut state = self.inner.write().await;
        if state.session.is_none() {
            state.pending_events.extend(started.lifecycle_events);
            state.set_session(started.session);
            return Ok(());
        }
        drop(state);
        // Another task already established a session while we were waiting
        // on the portal handshake; close the losing session instead of
        // dropping it, or the compositor-side session (and its PipeWire fd)
        // leaks.
        close_session(&started.session).await;
        Ok(())
    }

    pub async fn reset_persisted_tokens(&self) -> Result<PortalTokenResetOutcome, BackendError> {
        let old_session = {
            let mut state = self.inner.write().await;
            state.session.take()
        };
        let dropped_cached_session = old_session.is_some();
        if let Some(session) = old_session {
            close_session(&session).await;
        }
        let token_store = self.token_store.as_ref().ok_or_else(|| {
            BackendError::new(
                BackendErrorCode::Internal,
                "portal token storage is unavailable in this environment",
            )
        })?;
        let cleared = token_store.clear().map_err(|error| {
            BackendError::new(
                BackendErrorCode::Internal,
                format!("failed to clear persisted portal tokens: {error}"),
            )
        })?;
        Ok(PortalTokenResetOutcome {
            token_path: token_store.path().display().to_string(),
            cleared,
            dropped_cached_session,
        })
    }
}

pub(crate) fn evdev_button(button: MouseButton) -> i32 {
    match button {
        MouseButton::Left => 0x110,
        MouseButton::Right => 0x111,
        MouseButton::Middle => 0x112,
    }
}

async fn close_session(session: &ActiveRemoteDesktopSession) {
    if let Err(error) = session.session.close().await {
        warn!(
            error = %error,
            "failed to close RemoteDesktop portal session"
        );
    }
}

#[cfg(test)]
mod tests {
    use sky_cua_platform::diagnostics::{BackendError, BackendErrorCode};

    use crate::portal::eis_input::EisOperationError;

    #[test]
    fn eis_operation_error_is_debuggable() {
        let failure = EisOperationError {
            error: BackendError::new(BackendErrorCode::InvalidRequest, "test"),
            established: false,
        };
        let _ = format!("{failure:?}");
    }
}
