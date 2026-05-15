use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::sync::Arc;
use std::time::Duration;

use ashpd::desktop::PersistMode;
use ashpd::desktop::remote_desktop::{
    Axis, DeviceType, KeyState, NotifyKeyboardKeycodeOptions, NotifyKeyboardKeysymOptions,
    NotifyPointerAxisDiscreteOptions, NotifyPointerAxisOptions, NotifyPointerButtonOptions,
    NotifyPointerMotionAbsoluteOptions, RemoteDesktop, SelectDevicesOptions,
};
use ashpd::desktop::screencast::{CursorMode, Screencast, SelectSourcesOptions, SourceType};
use chrono::Utc;
use enumflags2::BitFlags;
use sky_cua_platform::diagnostics::{BackendError, BackendErrorCode};
use sky_cua_platform::model::{CoordinateSpace, PortalTokenResetOutcome, RectF};
use tokio::sync::Mutex;
use tracing::{debug, warn};

use crate::portal::pipewire::{self, PipeWireFrameCapture};
use crate::portal::preauthorize;
use crate::portal::session::portal_u32_property;
use crate::portal::token_store::{
    PersistedPortalToken, PortalTokenStore, current_compositor_hint,
    portal_token_compositor_mismatch,
};

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
    inner: Arc<Mutex<RemoteDesktopState>>,
    token_store: Option<PortalTokenStore>,
}

#[derive(Debug)]
struct ActiveRemoteDesktopSession {
    remote_desktop: RemoteDesktop,
    screencast: Screencast,
    session: ashpd::desktop::Session<RemoteDesktop>,
    primary_stream: Option<PortalStreamInfo>,
    pipewire_remote_fd: Option<OwnedFd>,
}

#[derive(Debug)]
struct StartedPortalSession {
    session: ActiveRemoteDesktopSession,
    lifecycle_events: Vec<PortalLifecycleEvent>,
}

#[derive(Debug, Default)]
struct RemoteDesktopState {
    session: Option<ActiveRemoteDesktopSession>,
    pending_events: Vec<PortalLifecycleEvent>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortalLifecycleEvent {
    pub code: &'static str,
    pub message: String,
    pub details: Option<String>,
}

const SESSION_START_TIMEOUT: Duration = Duration::from_secs(12);
const PORTAL_PREAUTHORIZATION_TIMEOUT: Duration = Duration::from_secs(5);
const PORTAL_SESSION_INPUT_SETTLE_DELAY: Duration = Duration::from_millis(120);
const PIPEWIRE_REMOTE_OPEN_TIMEOUT: Duration = Duration::from_secs(5);
const PORTAL_SESSION_STARTED: &str = "PortalSessionStarted";
const PORTAL_SESSION_RESTORED: &str = "PortalSessionRestored";
const PORTAL_SESSION_RESTORE_MISS: &str = "PortalSessionRestoreMiss";
const PORTAL_SESSION_REBUILT: &str = "PortalSessionRebuilt";
const PORTAL_SESSION_TOKEN_ROTATED: &str = "PortalSessionTokenRotated";

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
            inner: Arc::new(Mutex::new(RemoteDesktopState::default())),
            token_store,
        }
    }

    pub async fn ensure_started(&self) -> Result<Option<PortalStreamInfo>, BackendError> {
        let mut state = self.inner.lock().await;
        self.ensure_session_started_locked(&mut state).await?;
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
        let mut state = self.inner.lock().await;
        self.ensure_session_started_locked(&mut state).await?;
        match capture_frame_from_active_session(&mut state.session, snapshot_id).await {
            Ok(frame) => Ok(frame),
            Err(error)
                if error.code == BackendErrorCode::PipeWireUnavailable.as_str()
                    || error.code == BackendErrorCode::PipeWireStreamFailed.as_str() =>
            {
                warn!(
                    message = %error.message,
                    "PipeWire capture failed on the cached portal session; resetting and retrying once"
                );
                state.session = None;
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
                state.pending_events.extend(started.lifecycle_events);
                state.session = Some(started.session);
                capture_frame_from_active_session(&mut state.session, snapshot_id).await
            }
            Err(error) => Err(error),
        }
    }

    pub async fn take_lifecycle_events(&self) -> Vec<PortalLifecycleEvent> {
        let mut state = self.inner.lock().await;
        std::mem::take(&mut state.pending_events)
    }

    pub async fn reset_session(&self) {
        let mut state = self.inner.lock().await;
        state.session = None;
    }

    pub async fn preauthorize_permissions(&self) {
        preauthorize_with_timeout(self.token_store.as_ref()).await;
    }

    pub async fn pointer_move_absolute(&self, x: f64, y: f64) -> Result<(), BackendError> {
        let mut state = self.inner.lock().await;
        self.ensure_session_started_locked(&mut state).await?;
        let session = state.session.as_ref().expect("portal session should exist");
        let stream = session.primary_stream.as_ref().ok_or_else(|| {
            BackendError::new(
                BackendErrorCode::ActionUnsupportedForEnvironment,
                "RemoteDesktop session started without a screencast stream for absolute motion",
            )
        })?;
        session
            .remote_desktop
            .notify_pointer_motion_absolute(
                &session.session,
                stream.node_id,
                x,
                y,
                NotifyPointerMotionAbsoluteOptions::default(),
            )
            .await
            .map_err(|error| {
                BackendError::new(
                    BackendErrorCode::ActionUnsupportedForEnvironment,
                    format!("failed to inject absolute pointer motion through the portal: {error}"),
                )
            })
    }

    pub async fn pointer_button(
        &self,
        button: MouseButton,
        pressed: bool,
    ) -> Result<(), BackendError> {
        let mut state = self.inner.lock().await;
        self.ensure_session_started_locked(&mut state).await?;
        let session = state.session.as_ref().expect("portal session should exist");
        session
            .remote_desktop
            .notify_pointer_button(
                &session.session,
                evdev_button(button),
                if pressed {
                    KeyState::Pressed
                } else {
                    KeyState::Released
                },
                NotifyPointerButtonOptions::default(),
            )
            .await
            .map_err(|error| {
                BackendError::new(
                    BackendErrorCode::ActionUnsupportedForEnvironment,
                    format!("failed to inject pointer button through the portal: {error}"),
                )
            })
    }

    pub async fn click(&self, button: MouseButton) -> Result<(), BackendError> {
        self.pointer_button(button, true).await?;
        tokio::time::sleep(Duration::from_millis(30)).await;
        self.pointer_button(button, false).await
    }

    pub async fn scroll_vertical_discrete(&self, steps: i32) -> Result<(), BackendError> {
        let mut state = self.inner.lock().await;
        self.ensure_session_started_locked(&mut state).await?;
        let session = state.session.as_ref().expect("portal session should exist");
        session
            .remote_desktop
            .notify_pointer_axis_discrete(
                &session.session,
                Axis::Vertical,
                steps,
                NotifyPointerAxisDiscreteOptions::default(),
            )
            .await
            .map_err(|error| {
                BackendError::new(
                    BackendErrorCode::ActionUnsupportedForEnvironment,
                    format!("failed to inject vertical scroll through the portal: {error}"),
                )
            })
    }

    pub async fn scroll_vertical_smooth(&self, delta_y: f64) -> Result<(), BackendError> {
        let mut state = self.inner.lock().await;
        self.ensure_session_started_locked(&mut state).await?;
        let session = state.session.as_ref().expect("portal session should exist");
        session
            .remote_desktop
            .notify_pointer_axis(
                &session.session,
                0.0,
                delta_y,
                NotifyPointerAxisOptions::default().set_finish(true),
            )
            .await
            .map_err(|error| {
                BackendError::new(
                    BackendErrorCode::ActionUnsupportedForEnvironment,
                    format!("failed to inject smooth vertical scroll through the portal: {error}"),
                )
            })
    }

    pub async fn send_text(&self, text: &str) -> Result<(), BackendError> {
        for character in text.chars() {
            let keysym = keysym_for_char(character).ok_or_else(|| {
                BackendError::new(
                    BackendErrorCode::InvalidRequest,
                    format!(
                        "cannot type unsupported character {character:?} through keysym injection"
                    ),
                )
            })?;
            self.send_keysym_raw(keysym).await?;
        }
        Ok(())
    }

    pub async fn press_key_sequence(&self, keys: &[String]) -> Result<(), BackendError> {
        if keys.is_empty() {
            return Err(BackendError::new(
                BackendErrorCode::InvalidRequest,
                "press_key requires at least one key",
            ));
        }

        let mut resolved = Vec::new();
        for key in keys {
            let keysym = keysym_for_key_name(key).ok_or_else(|| {
                BackendError::new(
                    BackendErrorCode::InvalidRequest,
                    format!("unsupported key name {key:?}"),
                )
            })?;
            resolved.push(keysym);
        }

        if resolved.len() == 1 {
            return self.send_keysym_raw(resolved[0]).await;
        }

        for keysym in &resolved[..resolved.len() - 1] {
            self.send_keysym_state(*keysym, KeyState::Pressed).await?;
        }
        self.send_keysym_raw(*resolved.last().expect("chord has a last key"))
            .await?;
        for keysym in resolved[..resolved.len() - 1].iter().rev() {
            self.send_keysym_state(*keysym, KeyState::Released).await?;
        }
        Ok(())
    }

    pub async fn press_keycode_chord(
        &self,
        modifiers: &[i32],
        keycode: i32,
    ) -> Result<(), BackendError> {
        let mut pressed_modifiers = Vec::with_capacity(modifiers.len());
        for modifier in modifiers {
            self.send_keycode_state(*modifier, KeyState::Pressed)
                .await?;
            pressed_modifiers.push(*modifier);
        }

        let mut result = self.send_keycode_raw(keycode).await;
        for modifier in pressed_modifiers.iter().rev() {
            if let Err(error) = self.send_keycode_state(*modifier, KeyState::Released).await
                && result.is_ok()
            {
                result = Err(error);
            }
        }
        result
    }

    async fn send_keysym_raw(&self, keysym: i32) -> Result<(), BackendError> {
        self.send_keysym_state(keysym, KeyState::Pressed).await?;
        self.send_keysym_state(keysym, KeyState::Released).await
    }

    async fn send_keycode_raw(&self, keycode: i32) -> Result<(), BackendError> {
        self.send_keycode_state(keycode, KeyState::Pressed).await?;
        tokio::time::sleep(Duration::from_millis(35)).await;
        self.send_keycode_state(keycode, KeyState::Released).await
    }

    async fn send_keycode_state(&self, keycode: i32, state: KeyState) -> Result<(), BackendError> {
        let mut manager_state = self.inner.lock().await;
        self.ensure_session_started_locked(&mut manager_state)
            .await?;
        let session = manager_state
            .session
            .as_ref()
            .expect("portal session should exist");
        session
            .remote_desktop
            .notify_keyboard_keycode(
                &session.session,
                keycode,
                state,
                NotifyKeyboardKeycodeOptions::default(),
            )
            .await
            .map_err(|error| {
                BackendError::new(
                    BackendErrorCode::ActionUnsupportedForEnvironment,
                    format!("failed to inject keyboard keycode through the portal: {error}"),
                )
            })
    }

    async fn send_keysym_state(&self, keysym: i32, state: KeyState) -> Result<(), BackendError> {
        let mut manager_state = self.inner.lock().await;
        self.ensure_session_started_locked(&mut manager_state)
            .await?;
        let session = manager_state
            .session
            .as_ref()
            .expect("portal session should exist");
        session
            .remote_desktop
            .notify_keyboard_keysym(
                &session.session,
                keysym,
                state,
                NotifyKeyboardKeysymOptions::default(),
            )
            .await
            .map_err(|error| {
                BackendError::new(
                    BackendErrorCode::ActionUnsupportedForEnvironment,
                    format!("failed to inject keyboard keysym through the portal: {error}"),
                )
            })
    }

    async fn ensure_session_started_locked(
        &self,
        state: &mut RemoteDesktopState,
    ) -> Result<(), BackendError> {
        if state.session.is_some() {
            return Ok(());
        }
        let started = start_session_with_timeout(self.token_store.as_ref()).await?;
        state.pending_events.extend(started.lifecycle_events);
        state.session = Some(started.session);
        Ok(())
    }

    pub async fn reset_persisted_tokens(&self) -> Result<PortalTokenResetOutcome, BackendError> {
        let mut state = self.inner.lock().await;
        let dropped_cached_session = state.session.take().is_some();
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

async fn start_session(
    token_store: Option<&PortalTokenStore>,
) -> Result<StartedPortalSession, BackendError> {
    debug!("starting combined RemoteDesktop + ScreenCast portal session");
    let mut lifecycle_events = Vec::new();
    let remote_desktop = RemoteDesktop::new().await.map_err(|error| {
        BackendError::new(
            BackendErrorCode::PortalUnavailable,
            format!("failed to create RemoteDesktop portal proxy: {error}"),
        )
    })?;
    let screencast = Screencast::new().await.map_err(|error| {
        BackendError::new(
            BackendErrorCode::PortalUnavailable,
            format!("failed to create ScreenCast portal proxy: {error}"),
        )
    })?;

    let session = remote_desktop
        .create_session(Default::default())
        .await
        .map_err(|error| {
            BackendError::new(
                BackendErrorCode::PortalRequestDenied,
                format!("failed to create a combined RemoteDesktop session: {error}"),
            )
        })?;

    let stored_token = load_stored_token(token_store, &mut lifecycle_events);

    remote_desktop
        .select_devices(
            &session,
            SelectDevicesOptions::default()
                .set_devices(DeviceType::Keyboard | DeviceType::Pointer)
                .set_restore_token(
                    stored_token
                        .as_ref()
                        .map(|record| record.restore_token.as_str()),
                )
                .set_persist_mode(PersistMode::ExplicitlyRevoked),
        )
        .await
        .map_err(|error| {
            BackendError::new(
                BackendErrorCode::PortalRequestDenied,
                format!("failed to request remote-control devices from the portal: {error}"),
            )
        })?
        .response()
        .map_err(|error| {
            BackendError::new(
                BackendErrorCode::PortalRequestDenied,
                format!("portal device selection did not complete successfully: {error}"),
            )
        })?;

    screencast
        .select_sources(
            &session,
            SelectSourcesOptions::default()
                .set_cursor_mode(CursorMode::Metadata)
                .set_sources(BitFlags::from_flag(SourceType::Monitor))
                .set_multiple(false)
                .set_persist_mode(PersistMode::DoNot),
        )
        .await
        .map_err(|error| {
            BackendError::new(
                BackendErrorCode::PortalRequestDenied,
                format!("failed to request screencast sources from the portal: {error}"),
            )
        })?
        .response()
        .map_err(|error| {
            BackendError::new(
                BackendErrorCode::PortalRequestDenied,
                format!("portal source selection did not complete successfully: {error}"),
            )
        })?;

    let selected = remote_desktop
        .start(&session, None, Default::default())
        .await
        .map_err(|error| {
            BackendError::new(
                BackendErrorCode::PortalRequestDenied,
                format!("failed to start the RemoteDesktop session: {error}"),
            )
        })?
        .response()
        .map_err(|error| {
            BackendError::new(
                BackendErrorCode::PortalRequestDenied,
                format!("the RemoteDesktop session did not start successfully: {error}"),
            )
        })?;

    let primary_stream = selected.streams().first().map(stream_to_info);

    if stored_token.is_some() {
        lifecycle_events.push(PortalLifecycleEvent {
            code: PORTAL_SESSION_RESTORED,
            message:
                "Reused a persisted RemoteDesktop approval token for the combined portal session."
                    .to_string(),
            details: None,
        });
    } else {
        lifecycle_events.push(PortalLifecycleEvent {
            code: PORTAL_SESSION_STARTED,
            message: "Started a new combined RemoteDesktop and ScreenCast portal session."
                .to_string(),
            details: None,
        });
    }

    persist_selected_restore_token(
        token_store,
        stored_token.as_ref(),
        selected.restore_token(),
        &mut lifecycle_events,
    )
    .await;

    tokio::time::sleep(PORTAL_SESSION_INPUT_SETTLE_DELAY).await;

    Ok(StartedPortalSession {
        session: ActiveRemoteDesktopSession {
            remote_desktop,
            screencast,
            session,
            primary_stream,
            pipewire_remote_fd: None,
        },
        lifecycle_events,
    })
}

async fn start_session_with_timeout(
    token_store: Option<&PortalTokenStore>,
) -> Result<StartedPortalSession, BackendError> {
    preauthorize_with_timeout(token_store).await;
    tokio::time::timeout(SESSION_START_TIMEOUT, start_session(token_store))
        .await
        .map_err(|_| {
            BackendError::new(
                BackendErrorCode::PortalApprovalPending,
                "timed out waiting for the RemoteDesktop portal session to start; the approval prompt may still be waiting for user input",
            )
        })?
}

fn load_stored_token(
    token_store: Option<&PortalTokenStore>,
    lifecycle_events: &mut Vec<PortalLifecycleEvent>,
) -> Option<PersistedPortalToken> {
    let token_store = token_store?;
    match token_store.load() {
        Ok(Some(record)) => {
            if let Some(details) = portal_token_compositor_mismatch(&record) {
                lifecycle_events.push(PortalLifecycleEvent {
                    code: PORTAL_SESSION_RESTORE_MISS,
                    message: "Persisted portal restore token belongs to another compositor; falling back to a fresh portal session."
                        .to_string(),
                    details: Some(details),
                });
                None
            } else {
                Some(record)
            }
        }
        Ok(None) => None,
        Err(error) => {
            lifecycle_events.push(PortalLifecycleEvent {
                code: PORTAL_SESSION_RESTORE_MISS,
                message: "Could not load the persisted portal restore token; falling back to a fresh portal session."
                    .to_string(),
                details: Some(error.to_string()),
            });
            None
        }
    }
}

async fn persist_selected_restore_token(
    token_store: Option<&PortalTokenStore>,
    previous_record: Option<&PersistedPortalToken>,
    restore_token: Option<&str>,
    lifecycle_events: &mut Vec<PortalLifecycleEvent>,
) {
    let Some(token_store) = token_store else {
        return;
    };
    let Some(restore_token) = restore_token else {
        if previous_record.is_some() {
            lifecycle_events.push(PortalLifecycleEvent {
                code: PORTAL_SESSION_RESTORE_MISS,
                message: "Portal startup succeeded but did not return a replacement restore token."
                    .to_string(),
                details: None,
            });
        }
        return;
    };

    let record = PersistedPortalToken {
        restore_token: restore_token.to_string(),
        updated_at: Utc::now(),
        xdg_session_type: std::env::var("XDG_SESSION_TYPE").ok(),
        compositor: current_compositor_hint(),
        remote_desktop_version: version().await.ok(),
        screencast_version: crate::portal::screencast::version().await.ok(),
    };

    match token_store.save(&record) {
        Ok(()) => lifecycle_events.push(PortalLifecycleEvent {
            code: PORTAL_SESSION_TOKEN_ROTATED,
            message: match previous_record {
                Some(previous) if previous.restore_token != record.restore_token => {
                    "Rotated the persisted RemoteDesktop restore token for future sessions."
                        .to_string()
                }
                Some(_) => {
                    "Refreshed the persisted RemoteDesktop restore token for future sessions."
                        .to_string()
                }
                None => "Stored a persisted RemoteDesktop restore token for future sessions."
                    .to_string(),
            },
            details: Some(format!("token_path={}", token_store.path().display())),
        }),
        Err(error) => lifecycle_events.push(PortalLifecycleEvent {
            code: PORTAL_SESSION_RESTORE_MISS,
            message:
                "Portal startup succeeded but the replacement restore token could not be persisted."
                    .to_string(),
            details: Some(error.to_string()),
        }),
    }
}

async fn preauthorize_with_timeout(token_store: Option<&PortalTokenStore>) {
    match tokio::time::timeout(
        PORTAL_PREAUTHORIZATION_TIMEOUT,
        preauthorize::preauthorize_remote_desktop(token_store),
    )
    .await
    {
        Ok(()) => {}
        Err(_) => warn!(
            "timed out preauthorizing RemoteDesktop portal state; falling back to the normal portal startup path"
        ),
    }
}

async fn capture_frame_from_active_session(
    guard: &mut Option<ActiveRemoteDesktopSession>,
    snapshot_id: &str,
) -> Result<PipeWireFrameCapture, BackendError> {
    let session = guard.as_mut().expect("portal session should exist");
    let stream = session.primary_stream.as_ref().ok_or_else(|| {
        BackendError::new(
            BackendErrorCode::PipeWireStreamFailed,
            "RemoteDesktop session started without a screencast stream, so no PipeWire node is available",
        )
    })?;
    let node_id = stream.node_id;

    if session.pipewire_remote_fd.is_none() {
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
                format!("failed to open the PipeWire remote for the screencast session: {error}"),
            )
        })?;
        session.pipewire_remote_fd = Some(remote_fd);
    }

    let remote_fd = duplicate_remote_fd(
        session
            .pipewire_remote_fd
            .as_ref()
            .expect("PipeWire remote should be cached once opened"),
    )?;

    pipewire::capture_png_frame(snapshot_id, node_id, remote_fd).await
}

fn duplicate_remote_fd(remote_fd: &OwnedFd) -> Result<OwnedFd, BackendError> {
    let duplicated = unsafe { libc::dup(remote_fd.as_raw_fd()) };
    if duplicated < 0 {
        return Err(BackendError::new(
            BackendErrorCode::PipeWireUnavailable,
            format!(
                "failed to duplicate the cached PipeWire remote fd: {}",
                std::io::Error::last_os_error()
            ),
        ));
    }

    let duplicated = unsafe { OwnedFd::from_raw_fd(duplicated) };
    Ok(duplicated)
}

fn stream_to_info(stream: &ashpd::desktop::screencast::Stream) -> PortalStreamInfo {
    let logical_rect = match (stream.position(), stream.size()) {
        (Some((x, y)), Some((width, height))) => Some(RectF {
            x: f64::from(x),
            y: f64::from(y),
            width: f64::from(width),
            height: f64::from(height),
            space: CoordinateSpace::DesktopLogical,
        }),
        _ => None,
    };

    PortalStreamInfo {
        node_id: stream.pipe_wire_node_id(),
        stream_id: stream.id().map(ToOwned::to_owned),
        mapping_id: stream.mapping_id().map(ToOwned::to_owned),
        source_type: stream.source_type().map(source_type_code),
        logical_rect,
    }
}

fn source_type_code(source_type: SourceType) -> u32 {
    match source_type {
        SourceType::Monitor => 1,
        SourceType::Window => 2,
        SourceType::Virtual => 4,
    }
}

fn evdev_button(button: MouseButton) -> i32 {
    match button {
        MouseButton::Left => 0x110,
        MouseButton::Right => 0x111,
        MouseButton::Middle => 0x112,
    }
}

fn keysym_for_char(character: char) -> Option<i32> {
    match character {
        '\n' | '\r' => Some(0xff0d),
        '\t' => Some(0xff09),
        _ if character.is_ascii() => Some(i32::from(character as u8)),
        _ if u32::from(character) <= 0x10ffff => Some((0x01000000 | u32::from(character)) as i32),
        _ => None,
    }
}

pub fn keysym_for_key_name(key: &str) -> Option<i32> {
    let normalized = key.trim().to_ascii_lowercase().replace('_', "");
    match normalized.as_str() {
        "enter" | "return" => Some(0xff0d),
        "tab" => Some(0xff09),
        "backspace" => Some(0xff08),
        "escape" | "esc" => Some(0xff1b),
        "space" => Some(0x20),
        "delete" | "del" => Some(0xffff),
        "left" => Some(0xff51),
        "up" => Some(0xff52),
        "right" => Some(0xff53),
        "down" => Some(0xff54),
        "home" => Some(0xff50),
        "end" => Some(0xff57),
        "pageup" => Some(0xff55),
        "pagedown" => Some(0xff56),
        "shift" | "shiftl" => Some(0xffe1),
        "control" | "ctrl" | "ctrll" => Some(0xffe3),
        "alt" | "altl" => Some(0xffe9),
        "meta" | "super" | "superl" | "metal" => Some(0xffeb),
        "capslock" => Some(0xffe5),
        "f1" => Some(0xffbe),
        "f2" => Some(0xffbf),
        "f3" => Some(0xffc0),
        "f4" => Some(0xffc1),
        "f5" => Some(0xffc2),
        "f6" => Some(0xffc3),
        "f7" => Some(0xffc4),
        "f8" => Some(0xffc5),
        "f9" => Some(0xffc6),
        "f10" => Some(0xffc7),
        "f11" => Some(0xffc8),
        "f12" => Some(0xffc9),
        _ => {
            let mut chars = key.chars();
            match (chars.next(), chars.next()) {
                (Some(character), None) => keysym_for_char(character),
                _ => None,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{keysym_for_char, keysym_for_key_name};

    #[test]
    fn resolves_ascii_character_keysyms() {
        assert_eq!(keysym_for_char('a'), Some(i32::from(b'a')));
        assert_eq!(keysym_for_char('\n'), Some(0xff0d));
    }

    #[test]
    fn resolves_named_keysyms() {
        assert_eq!(keysym_for_key_name("Enter"), Some(0xff0d));
        assert_eq!(keysym_for_key_name("Ctrl"), Some(0xffe3));
        assert_eq!(keysym_for_key_name("f5"), Some(0xffc2));
    }
}
