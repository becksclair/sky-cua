use std::collections::{HashMap, HashSet};
use std::fmt;
use std::fmt::Write as _;
use std::os::fd::{AsFd, AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::net::UnixStream;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::thread;
use std::time::{Duration, Instant};

use ashpd::desktop::PersistMode;
use ashpd::desktop::remote_desktop::{
    Axis, ConnectToEISOptions, DeviceType, KeyState, NotifyKeyboardKeycodeOptions,
    NotifyKeyboardKeysymOptions, NotifyPointerAxisDiscreteOptions, NotifyPointerAxisOptions,
    NotifyPointerButtonOptions, NotifyPointerMotionAbsoluteOptions, RemoteDesktop,
    SelectDevicesOptions,
};
use ashpd::desktop::screencast::{CursorMode, Screencast, SelectSourcesOptions, SourceType};
use chrono::Utc;
use enumflags2::BitFlags;
use reis::ei;
use reis::event::{DeviceCapability, EiEvent, EiEventConverter};
use sky_cua_platform::diagnostics::{BackendError, BackendErrorCode};
use sky_cua_platform::model::{CoordinateSpace, PortalTokenResetOutcome, RectF};
use tokio::sync::RwLock;
use tracing::{debug, warn};
use xkbcommon::xkb;

use crate::portal::pipewire::{self, PipeWireFrameCapture};
use crate::portal::preauthorize;
use crate::portal::session::portal_u32_property;
use crate::portal::token_store::{
    PersistedPortalToken, PortalTokenStore, current_compositor_hint,
    portal_token_compositor_mismatch,
};
use crate::virtual_input::{LinuxVirtualInput, virtual_input_keyboard_available};
use crate::x11::input_xtest;

const CURSOR_MODE_HIDDEN: u32 = 1;
const CURSOR_MODE_EMBEDDED: u32 = 2;
const CURSOR_MODE_METADATA: u32 = 4;

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
    inner: Arc<RwLock<RemoteDesktopState>>,
    token_store: Option<PortalTokenStore>,
}

#[derive(Debug)]
struct ActiveRemoteDesktopSession {
    remote_desktop: RemoteDesktop,
    screencast: Screencast,
    session: ashpd::desktop::Session<RemoteDesktop>,
    primary_stream: Option<PortalStreamInfo>,
    pipewire_remote_fd: Option<OwnedFd>,
    eis_worker: Option<EisWorkerHandle>,
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

struct EisInput {
    context: ei::Context,
    connection: reis::event::Connection,
    converter: EiEventConverter,
    sequence: u32,
}

#[derive(Clone)]
struct EisPointerDevice {
    device: reis::event::Device,
    pointer_absolute: ei::PointerAbsolute,
    button: ei::Button,
    scroll: Option<ei::Scroll>,
}

#[derive(Clone)]
struct EisKeyboardDevice {
    device: reis::event::Device,
    keyboard: ei::Keyboard,
    shift_keycode: Option<u32>,
    keysym_cache: HashMap<u32, EisKeyStroke>,
}

#[derive(Debug, Clone, Copy)]
struct EisKeyStroke {
    keycode: u32,
    needs_shift: bool,
}

#[derive(Clone)]
enum EisReadyDevice {
    Pointer(EisPointerDevice),
    Keyboard(EisKeyboardDevice),
}

#[derive(Clone)]
struct EisWorkerHandle {
    sender: SyncSender<EisCommand>,
}

impl fmt::Debug for EisWorkerHandle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EisWorkerHandle")
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone)]
enum EisAction {
    Click {
        x: f64,
        y: f64,
        button: MouseButton,
    },
    Drag {
        from: (f64, f64),
        to: (f64, f64),
    },
    ScrollVertical {
        x: f64,
        y: f64,
        delta_y: Option<f64>,
        steps: i32,
    },
    SendText {
        text: Arc<str>,
    },
    PressKeySequence {
        keys: Arc<[String]>,
    },
}

enum EisCommand {
    Execute {
        action: EisAction,
        reply: tokio::sync::oneshot::Sender<Result<String, BackendError>>,
    },
}

struct EisWorker {
    input: EisInput,
    pointer: Option<EisPointerDevice>,
    keyboard: Option<EisKeyboardDevice>,
    emulating_devices: HashSet<ei::Device>,
}

#[derive(Debug)]
struct EisOperationError {
    error: BackendError,
    established: bool,
}

impl EisInput {
    fn new(fd: OwnedFd) -> Result<Self, BackendError> {
        let stream = UnixStream::from(fd);
        stream
            .set_read_timeout(Some(Duration::from_millis(250)))
            .map_err(|error| {
                BackendError::new(
                    BackendErrorCode::ActionUnsupportedForEnvironment,
                    format!("failed to configure EIS fd read timeout: {error}"),
                )
            })?;
        let context = ei::Context::new(stream).map_err(|error| {
            BackendError::new(
                BackendErrorCode::ActionUnsupportedForEnvironment,
                format!("failed to create EIS context: {error}"),
            )
        })?;
        let handshake = reis::handshake::ei_handshake_blocking(
            &context,
            "sky-cua",
            ei::handshake::ContextType::Sender,
        )
        .map_err(|error| {
            BackendError::new(
                BackendErrorCode::ActionUnsupportedForEnvironment,
                format!("failed to handshake with EIS: {error}"),
            )
        })?;
        let converter = EiEventConverter::new(&context, handshake);
        let connection = converter.connection().clone();
        Ok(Self {
            context,
            connection,
            converter,
            sequence: 1,
        })
    }

    fn next_sequence(&mut self) -> u32 {
        let sequence = self.sequence;
        self.sequence = self.sequence.saturating_add(1);
        sequence
    }

    fn last_serial(&self) -> u32 {
        self.connection.serial()
    }

    fn flush(&self) -> Result<(), BackendError> {
        self.connection.flush().map_err(|error| {
            BackendError::new(
                BackendErrorCode::ActionUnsupportedForEnvironment,
                format!("failed to flush EIS input events: {error}"),
            )
        })
    }

    fn next_ready_device(
        &mut self,
        capabilities: BitFlags<DeviceCapability>,
    ) -> Result<EisReadyDevice, BackendError> {
        let deadline = Instant::now() + EIS_DEVICE_READY_TIMEOUT;
        loop {
            if Instant::now() >= deadline {
                return Err(BackendError::new(
                    BackendErrorCode::ActionUnsupportedForEnvironment,
                    "timed out waiting for an EIS input device to become ready",
                ));
            }
            if let Some(ready) = self.drain_eis_events(capabilities)? {
                return Ok(ready);
            }
            match self.context.read() {
                Ok(_) => {}
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) => {}
                Err(error) => {
                    return Err(BackendError::new(
                        BackendErrorCode::ActionUnsupportedForEnvironment,
                        format!("failed to read EIS input event: {error}"),
                    ));
                }
            }
        }
    }

    fn drain_eis_events(
        &mut self,
        capabilities: BitFlags<DeviceCapability>,
    ) -> Result<Option<EisReadyDevice>, BackendError> {
        while let Some(result) = self.context.pending_event() {
            let event = match result {
                reis::PendingRequestResult::Request(event) => event,
                reis::PendingRequestResult::ParseError(error) => {
                    return Err(BackendError::new(
                        BackendErrorCode::ActionUnsupportedForEnvironment,
                        format!("failed to parse EIS input event: {error}"),
                    ));
                }
                reis::PendingRequestResult::InvalidObject(_) => continue,
            };
            self.converter.handle_event(event).map_err(|error| {
                BackendError::new(
                    BackendErrorCode::ActionUnsupportedForEnvironment,
                    format!("failed to convert EIS input event: {error}"),
                )
            })?;
            while let Some(event) = self.converter.next_event() {
                match event {
                    EiEvent::SeatAdded(event) => {
                        event.seat.bind_capabilities(capabilities);
                        self.flush()?;
                    }
                    EiEvent::DeviceResumed(event) => {
                        if let (Some(pointer_absolute), Some(button)) = (
                            event.device.interface::<ei::PointerAbsolute>(),
                            event.device.interface::<ei::Button>(),
                        ) {
                            return Ok(Some(EisReadyDevice::Pointer(EisPointerDevice {
                                scroll: event.device.interface::<ei::Scroll>(),
                                device: event.device,
                                pointer_absolute,
                                button,
                            })));
                        }
                        if let Some(keyboard) = event.device.interface::<ei::Keyboard>() {
                            return Ok(Some(EisReadyDevice::Keyboard(keyboard_device_from_event(
                                event.device,
                                keyboard,
                            )?)));
                        }
                    }
                    _ => {}
                }
            }
        }
        Ok(None)
    }
}

impl EisWorkerHandle {
    async fn execute(&self, action: EisAction) -> Result<String, EisOperationError> {
        let (reply, receiver) = tokio::sync::oneshot::channel();
        self.sender
            .send(EisCommand::Execute { action, reply })
            .map_err(|_| EisOperationError {
                error: BackendError::new(
                    BackendErrorCode::ActionUnsupportedForEnvironment,
                    "RemoteDesktop EIS input worker is no longer running",
                ),
                established: false,
            })?;
        receiver
            .await
            .map_err(|_| EisOperationError {
                error: BackendError::new(
                    BackendErrorCode::ActionUnsupportedForEnvironment,
                    "RemoteDesktop EIS input worker stopped before returning a result",
                ),
                established: false,
            })?
            .map_err(|error| EisOperationError {
                error,
                established: true,
            })
    }
}

impl EisWorker {
    fn new(fd: OwnedFd) -> Result<Self, BackendError> {
        Ok(Self {
            input: EisInput::new(fd)?,
            pointer: None,
            keyboard: None,
            emulating_devices: HashSet::new(),
        })
    }

    fn run(mut self, receiver: Receiver<EisCommand>) {
        while let Ok(command) = receiver.recv() {
            match command {
                EisCommand::Execute { action, reply } => {
                    let _ = reply.send(self.execute(action));
                }
            }
        }
    }

    fn execute(&mut self, action: EisAction) -> Result<String, BackendError> {
        match action {
            EisAction::Click { x, y, button } => self.click_at(x, y, button),
            EisAction::Drag { from, to } => self.drag(from, to),
            EisAction::ScrollVertical {
                x,
                y,
                delta_y,
                steps,
            } => self.scroll_vertical_at(x, y, delta_y, steps),
            EisAction::SendText { text } => self.send_text(&text),
            EisAction::PressKeySequence { keys } => self.press_key_sequence(&keys),
        }
    }

    fn pointer_device(&mut self) -> Result<EisPointerDevice, BackendError> {
        if let Some(device) = self.pointer.as_ref() {
            return Ok(device.clone());
        }
        loop {
            match self.input.next_ready_device(eis_device_capabilities())? {
                EisReadyDevice::Pointer(device) => {
                    self.pointer = Some(device.clone());
                    return Ok(device);
                }
                EisReadyDevice::Keyboard(device) => {
                    self.keyboard = Some(device);
                }
            }
        }
    }

    fn keyboard_device(&mut self) -> Result<EisKeyboardDevice, BackendError> {
        if let Some(device) = self.keyboard.as_ref() {
            return Ok(device.clone());
        }
        loop {
            match self.input.next_ready_device(eis_device_capabilities())? {
                EisReadyDevice::Pointer(device) => {
                    self.pointer = Some(device);
                }
                EisReadyDevice::Keyboard(device) => {
                    self.keyboard = Some(device.clone());
                    return Ok(device);
                }
            }
        }
    }

    fn ensure_emulating(&mut self, device: &reis::event::Device) -> Result<String, BackendError> {
        let protocol_device = device.device().clone();
        if !self.emulating_devices.insert(protocol_device.clone()) {
            return Ok("emulation_started=false".to_string());
        }
        let serial = self.input.last_serial();
        let sequence = self.input.next_sequence();
        protocol_device.start_emulating(serial, sequence);
        self.input.flush()?;
        thread::sleep(EIS_FRAME_GAP_DELAY);
        let mut result = String::with_capacity(48);
        let _ = write!(
            &mut result,
            "emulation_started=true,emulation_sequence={sequence}"
        );
        Ok(result)
    }

    fn click_at(&mut self, x: f64, y: f64, button: MouseButton) -> Result<String, BackendError> {
        let device = self.pointer_device()?;
        validate_eis_absolute_point(&device.device, x, y)?;
        let details = describe_eis_device(&device.device);
        let emulation_details = self.ensure_emulating(&device.device)?;
        let serial = self.input.last_serial();
        let evdev = evdev_button(button) as u32;
        device.pointer_absolute.motion_absolute(x as f32, y as f32);
        device.device.device().frame(serial, monotonic_micros());
        device.button.button(evdev, ei::button::ButtonState::Press);
        device.device.device().frame(serial, monotonic_micros());
        self.input.flush()?;
        thread::sleep(EIS_BUTTON_HOLD_DELAY);
        device
            .button
            .button(evdev, ei::button::ButtonState::Released);
        device.device.device().frame(serial, monotonic_micros());
        self.input.flush()?;
        thread::sleep(EIS_FINAL_FLUSH_DELAY);
        let mut result = String::with_capacity(details.len() + emulation_details.len() + 2);
        result.push_str(&details);
        result.push_str("; ");
        result.push_str(&emulation_details);
        Ok(result)
    }

    fn drag(&mut self, from: (f64, f64), to: (f64, f64)) -> Result<String, BackendError> {
        let device = self.pointer_device()?;
        validate_eis_absolute_point(&device.device, from.0, from.1)?;
        validate_eis_absolute_point(&device.device, to.0, to.1)?;
        let details = describe_eis_device(&device.device);
        let emulation_details = self.ensure_emulating(&device.device)?;
        let serial = self.input.last_serial();
        let evdev = evdev_button(MouseButton::Left) as u32;
        device
            .pointer_absolute
            .motion_absolute(from.0 as f32, from.1 as f32);
        device.device.device().frame(serial, monotonic_micros());
        device.button.button(evdev, ei::button::ButtonState::Press);
        device.device.device().frame(serial, monotonic_micros());
        self.input.flush()?;
        thread::sleep(EIS_FRAME_GAP_DELAY);
        device
            .pointer_absolute
            .motion_absolute(to.0 as f32, to.1 as f32);
        device.device.device().frame(serial, monotonic_micros());
        self.input.flush()?;
        thread::sleep(EIS_FRAME_GAP_DELAY);
        device
            .button
            .button(evdev, ei::button::ButtonState::Released);
        device.device.device().frame(serial, monotonic_micros());
        self.input.flush()?;
        thread::sleep(EIS_FINAL_FLUSH_DELAY);
        let mut result = String::with_capacity(details.len() + emulation_details.len() + 2);
        result.push_str(&details);
        result.push_str("; ");
        result.push_str(&emulation_details);
        Ok(result)
    }

    fn scroll_vertical_at(
        &mut self,
        x: f64,
        y: f64,
        delta_y: Option<f64>,
        steps: i32,
    ) -> Result<String, BackendError> {
        let device = self.pointer_device()?;
        validate_eis_absolute_point(&device.device, x, y)?;
        let details = describe_eis_device(&device.device);
        let scroll = device.scroll.as_ref().ok_or_else(|| {
            BackendError::new(
                BackendErrorCode::ActionUnsupportedForEnvironment,
                "EIS pointer device did not advertise scroll support",
            )
        })?;
        let emulation_details = self.ensure_emulating(&device.device)?;
        let serial = self.input.last_serial();
        device.pointer_absolute.motion_absolute(x as f32, y as f32);
        device.device.device().frame(serial, monotonic_micros());
        let mut scroll_details = String::with_capacity(32);
        if let Some(delta_y) = delta_y {
            let eis_delta_y = eis_scroll_delta_from_action(delta_y);
            scroll.scroll(0.0, eis_delta_y);
            let _ = write!(&mut scroll_details, "scroll_delta_y={eis_delta_y}");
        } else {
            let eis_steps = eis_scroll_steps_from_action(steps);
            scroll.scroll_discrete(0, eis_steps);
            let _ = write!(&mut scroll_details, "scroll_discrete_y={eis_steps}");
        }
        device.device.device().frame(serial, monotonic_micros());
        scroll.scroll_stop(0, 1, 0);
        device.device.device().frame(serial, monotonic_micros());
        self.input.flush()?;
        thread::sleep(EIS_FINAL_FLUSH_DELAY);
        let mut result = String::with_capacity(
            details.len() + emulation_details.len() + scroll_details.len() + 4,
        );
        result.push_str(&details);
        result.push_str("; ");
        result.push_str(&emulation_details);
        result.push_str("; ");
        result.push_str(&scroll_details);
        Ok(result)
    }

    fn send_text(&mut self, text: &str) -> Result<String, BackendError> {
        let device = self.keyboard_device()?;
        let details = describe_eis_device(&device.device);
        let emulation_details = self.ensure_emulating(&device.device)?;
        let mut strokes = Vec::with_capacity(text.chars().count());
        for character in text.chars() {
            let keysym = keysym_for_char(character).ok_or_else(|| {
                BackendError::new(
                    BackendErrorCode::InvalidRequest,
                    format!(
                        "cannot type unsupported character {character:?} through EIS keymap injection"
                    ),
                )
            })?;
            strokes.push(resolve_eis_keystroke(&device, keysym)?);
        }
        for stroke in &strokes {
            self.send_eis_key_state(&device, *stroke, ei::keyboard::KeyState::Press)?;
        }
        self.input.flush()?;
        thread::sleep(EIS_BUTTON_HOLD_DELAY);
        for stroke in &strokes {
            self.send_eis_key_state(&device, *stroke, ei::keyboard::KeyState::Released)?;
        }
        self.input.flush()?;
        thread::sleep(EIS_FINAL_FLUSH_DELAY);
        let mut result = String::with_capacity(details.len() + emulation_details.len() + 32);
        result.push_str(&details);
        result.push_str("; ");
        result.push_str(&emulation_details);
        let _ = write!(&mut result, "; typed_chars={}", strokes.len());
        Ok(result)
    }

    fn press_key_sequence(&mut self, keys: &[String]) -> Result<String, BackendError> {
        if keys.is_empty() {
            return Err(BackendError::new(
                BackendErrorCode::InvalidRequest,
                "press_key requires at least one key",
            ));
        }

        let device = self.keyboard_device()?;
        let details = describe_eis_device(&device.device);
        let emulation_details = self.ensure_emulating(&device.device)?;
        let mut resolved = Vec::with_capacity(keys.len());
        for key in keys {
            let keysym = keysym_for_key_name(key).ok_or_else(|| {
                BackendError::new(
                    BackendErrorCode::InvalidRequest,
                    format!("unsupported key name {key:?}"),
                )
            })?;
            resolved.push(resolve_eis_keystroke(&device, keysym)?);
        }

        if resolved.len() == 1 {
            self.send_eis_key_stroke(&device, resolved[0])?;
        } else {
            if let Some(shift_keycode) = device.shift_keycode
                && let Some((last_stroke, modifiers)) = resolved.split_last_mut()
                && last_stroke.needs_shift
                && modifiers
                    .iter()
                    .any(|stroke| stroke.keycode == shift_keycode)
            {
                last_stroke.needs_shift = false;
            }
            for stroke in &resolved[..resolved.len() - 1] {
                self.send_eis_key_state(&device, *stroke, ei::keyboard::KeyState::Press)?;
            }
            self.send_eis_key_stroke(&device, *resolved.last().expect("chord has a last key"))?;
            for stroke in resolved[..resolved.len() - 1].iter().rev() {
                self.send_eis_key_state(&device, *stroke, ei::keyboard::KeyState::Released)?;
            }
        }
        self.input.flush()?;
        thread::sleep(EIS_FINAL_FLUSH_DELAY);
        let mut result = String::with_capacity(details.len() + emulation_details.len() + 32);
        result.push_str(&details);
        result.push_str("; ");
        result.push_str(&emulation_details);
        let _ = write!(&mut result, "; pressed_keys={}", keys.len());
        Ok(result)
    }

    fn send_eis_key_stroke(
        &self,
        device: &EisKeyboardDevice,
        stroke: EisKeyStroke,
    ) -> Result<(), BackendError> {
        self.send_eis_key_state(device, stroke, ei::keyboard::KeyState::Press)?;
        thread::sleep(EIS_BUTTON_HOLD_DELAY);
        self.send_eis_key_state(device, stroke, ei::keyboard::KeyState::Released)
    }

    fn send_eis_key_state(
        &self,
        device: &EisKeyboardDevice,
        stroke: EisKeyStroke,
        state: ei::keyboard::KeyState,
    ) -> Result<(), BackendError> {
        let serial = self.input.last_serial();
        if stroke.needs_shift {
            let shift_keycode = device.shift_keycode.ok_or_else(|| {
                BackendError::new(
                    BackendErrorCode::ActionUnsupportedForEnvironment,
                    "EIS keymap needs Shift for this key but did not expose a Shift keycode",
                )
            })?;
            match state {
                ei::keyboard::KeyState::Press => {
                    device
                        .keyboard
                        .key(shift_keycode, ei::keyboard::KeyState::Press);
                    device.keyboard.key(stroke.keycode, state);
                }
                ei::keyboard::KeyState::Released => {
                    device.keyboard.key(stroke.keycode, state);
                    device
                        .keyboard
                        .key(shift_keycode, ei::keyboard::KeyState::Released);
                }
            }
        } else {
            device.keyboard.key(stroke.keycode, state);
        }
        device.device.device().frame(serial, monotonic_micros());
        Ok(())
    }
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
const EIS_DEVICE_READY_TIMEOUT: Duration = Duration::from_secs(3);
const EIS_WORKER_START_TIMEOUT: Duration = Duration::from_secs(3);
const EIS_BUTTON_HOLD_DELAY: Duration = Duration::from_millis(35);
const EIS_FRAME_GAP_DELAY: Duration = Duration::from_millis(20);
const EIS_FINAL_FLUSH_DELAY: Duration = Duration::from_millis(80);
const PORTAL_SESSION_STARTED: &str = "PortalSessionStarted";
const PORTAL_SESSION_RESTORED: &str = "PortalSessionRestored";
const PORTAL_SESSION_RESTORE_MISS: &str = "PortalSessionRestoreMiss";
const PORTAL_SESSION_REBUILT: &str = "PortalSessionRebuilt";
const PORTAL_SESSION_TOKEN_ROTATED: &str = "PortalSessionTokenRotated";
const PORTAL_EIS_INPUT_USED: &str = "PortalEisInputUsed";
const PORTAL_EIS_INPUT_FALLBACK: &str = "PortalEisInputFallback";
const EIS_POINT_OUTSIDE_REGION_PREFIX: &str = "EIS absolute point";

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
        let mut state = self.inner.write().await;
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
        let mut state = self.inner.write().await;
        std::mem::take(&mut state.pending_events)
    }

    async fn push_lifecycle_event(&self, event: PortalLifecycleEvent) {
        let mut state = self.inner.write().await;
        state.pending_events.push(event);
    }

    pub async fn reset_session(&self) {
        let mut state = self.inner.write().await;
        state.session = None;
    }

    pub async fn preauthorize_permissions(&self) {
        preauthorize_with_timeout(self.token_store.as_ref()).await;
    }

    pub async fn pointer_move_absolute(&self, x: f64, y: f64) -> Result<(), BackendError> {
        self.ensure_session_started().await?;
        let state = self.inner.read().await;
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
        self.ensure_session_started().await?;
        let state = self.inner.read().await;
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

    pub async fn click_at(&self, x: f64, y: f64, button: MouseButton) -> Result<(), BackendError> {
        let action = EisAction::Click { x, y, button };
        match self.run_eis_action_with_retry(action).await {
            Ok(details) => {
                self.push_lifecycle_event(PortalLifecycleEvent {
                    code: PORTAL_EIS_INPUT_USED,
                    message: "Injected the pointer click through RemoteDesktop EIS.".to_string(),
                    details: Some(details),
                })
                .await;
                Ok(())
            }
            Err(failure) => {
                if !should_fallback_to_legacy(&failure) {
                    return Err(failure.error);
                }
                debug!(
                    message = %failure.error.message,
                    "EIS pointer click failed; falling back to legacy RemoteDesktop NotifyPointer calls"
                );
                if should_reset_session_before_legacy_fallback(&failure) {
                    self.reset_session().await;
                }
                self.push_lifecycle_event(PortalLifecycleEvent {
                    code: PORTAL_EIS_INPUT_FALLBACK,
                    message:
                        "RemoteDesktop EIS pointer click failed; fell back to legacy portal input."
                            .to_string(),
                    details: Some(eis_fallback_details(&failure)),
                })
                .await;
                self.pointer_move_absolute(x, y).await?;
                self.click(button).await
            }
        }
    }

    pub async fn drag(&self, from: (f64, f64), to: (f64, f64)) -> Result<(), BackendError> {
        let action = EisAction::Drag { from, to };
        match self.run_eis_action_with_retry(action).await {
            Ok(details) => {
                self.push_lifecycle_event(PortalLifecycleEvent {
                    code: PORTAL_EIS_INPUT_USED,
                    message: "Injected the pointer drag through RemoteDesktop EIS.".to_string(),
                    details: Some(details),
                })
                .await;
                Ok(())
            }
            Err(failure) => {
                if !should_fallback_to_legacy(&failure) {
                    return Err(failure.error);
                }
                debug!(
                    message = %failure.error.message,
                    "EIS pointer drag failed; falling back to legacy RemoteDesktop NotifyPointer calls"
                );
                if should_reset_session_before_legacy_fallback(&failure) {
                    self.reset_session().await;
                }
                self.push_lifecycle_event(PortalLifecycleEvent {
                    code: PORTAL_EIS_INPUT_FALLBACK,
                    message:
                        "RemoteDesktop EIS pointer drag failed; fell back to legacy portal input."
                            .to_string(),
                    details: Some(eis_fallback_details(&failure)),
                })
                .await;
                self.pointer_move_absolute(from.0, from.1).await?;
                self.pointer_button(MouseButton::Left, true).await?;
                tokio::time::sleep(Duration::from_millis(40)).await;
                let result = async {
                    self.pointer_move_absolute(to.0, to.1).await?;
                    tokio::time::sleep(Duration::from_millis(40)).await;
                    self.pointer_button(MouseButton::Left, false).await
                }
                .await;
                if result.is_err() {
                    let _ = self.pointer_button(MouseButton::Left, false).await;
                }
                result
            }
        }
    }

    pub async fn scroll_vertical_at(
        &self,
        x: f64,
        y: f64,
        delta_y: Option<f64>,
        steps: i32,
    ) -> Result<(), BackendError> {
        let action = EisAction::ScrollVertical {
            x,
            y,
            delta_y,
            steps,
        };
        match self.run_eis_action_with_retry(action).await {
            Ok(details) => {
                self.push_lifecycle_event(PortalLifecycleEvent {
                    code: PORTAL_EIS_INPUT_USED,
                    message: "Injected the pointer scroll through RemoteDesktop EIS.".to_string(),
                    details: Some(details),
                })
                .await;
                Ok(())
            }
            Err(failure) => {
                if !should_fallback_to_legacy(&failure) {
                    return Err(failure.error);
                }
                debug!(
                    message = %failure.error.message,
                    "EIS pointer scroll failed; falling back to legacy RemoteDesktop NotifyPointer calls"
                );
                if should_reset_session_before_legacy_fallback(&failure) {
                    self.reset_session().await;
                }
                self.push_lifecycle_event(PortalLifecycleEvent {
                    code: PORTAL_EIS_INPUT_FALLBACK,
                    message:
                        "RemoteDesktop EIS pointer scroll failed; fell back to legacy portal input."
                            .to_string(),
                    details: Some(eis_fallback_details(&failure)),
                })
                .await;
                self.pointer_move_absolute(x, y).await?;
                if let Some(delta_y) = delta_y {
                    self.scroll_vertical_smooth(delta_y).await
                } else {
                    self.scroll_vertical_discrete(steps).await
                }
            }
        }
    }

    pub async fn scroll_vertical_discrete(&self, steps: i32) -> Result<(), BackendError> {
        self.ensure_session_started().await?;
        let state = self.inner.read().await;
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
        self.ensure_session_started().await?;
        let state = self.inner.read().await;
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
        let action = EisAction::SendText {
            text: Arc::from(text),
        };
        match self.run_eis_action_with_retry(action).await {
            Ok(details) => {
                self.push_lifecycle_event(PortalLifecycleEvent {
                    code: PORTAL_EIS_INPUT_USED,
                    message: "Injected keyboard text through RemoteDesktop EIS.".to_string(),
                    details: Some(details),
                })
                .await;
                return Ok(());
            }
            Err(failure) if failure.error.code != BackendErrorCode::InvalidRequest.as_str() => {
                warn!(
                    message = %failure.error.message,
                    "EIS keyboard text failed; falling back to legacy RemoteDesktop NotifyKeyboard calls"
                );
                if should_reset_session_before_legacy_fallback(&failure) {
                    self.reset_session().await;
                }
                self.push_lifecycle_event(PortalLifecycleEvent {
                    code: PORTAL_EIS_INPUT_FALLBACK,
                    message:
                        "RemoteDesktop EIS keyboard text failed; fell back to legacy portal input."
                            .to_string(),
                    details: Some(eis_fallback_details(&failure)),
                })
                .await;

                // Prefer X11/XTest or LinuxVirtualInput over per-character D-Bus round-trips.
                if input_xtest::xtest_is_available() {
                    match input_xtest::send_text(text) {
                        Ok(()) => {
                            self.push_lifecycle_event(PortalLifecycleEvent {
                                code: PORTAL_EIS_INPUT_FALLBACK,
                                message:
                                    "RemoteDesktop EIS text failed; fell back to X11/XTest input."
                                        .to_string(),
                                details: None,
                            })
                            .await;
                            return Ok(());
                        }
                        Err(error) => {
                            warn!(message = %error.message, "XTest text fallback failed");
                        }
                    }
                }
                if virtual_input_keyboard_available() {
                    match LinuxVirtualInput::new() {
                        Ok(vi) => match vi.type_text(text) {
                            Ok(()) => {
                                self.push_lifecycle_event(PortalLifecycleEvent {
                                    code: PORTAL_EIS_INPUT_FALLBACK,
                                    message: "RemoteDesktop EIS text failed; fell back to Linux virtual input.".to_string(),
                                    details: None,
                                })
                                .await;
                                return Ok(());
                            }
                            Err(error) => {
                                warn!(message = %error.message, "LinuxVirtualInput text fallback failed");
                            }
                        },
                        Err(error) => {
                            warn!(message = %error.message, "LinuxVirtualInput initialization failed");
                        }
                    }
                }
            }
            Err(failure) => return Err(failure.error),
        }

        // Final resort: legacy per-character keysym injection via D-Bus.
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

        let action = EisAction::PressKeySequence {
            keys: Arc::from(keys.to_vec().into_boxed_slice()),
        };
        match self.run_eis_action_with_retry(action).await {
            Ok(details) => {
                self.push_lifecycle_event(PortalLifecycleEvent {
                    code: PORTAL_EIS_INPUT_USED,
                    message: "Injected the key sequence through RemoteDesktop EIS.".to_string(),
                    details: Some(details),
                })
                .await;
                return Ok(());
            }
            Err(failure) if failure.error.code != BackendErrorCode::InvalidRequest.as_str() => {
                warn!(
                    message = %failure.error.message,
                    "EIS key sequence failed; falling back to legacy RemoteDesktop NotifyKeyboard calls"
                );
                if should_reset_session_before_legacy_fallback(&failure) {
                    self.reset_session().await;
                }
                self.push_lifecycle_event(PortalLifecycleEvent {
                    code: PORTAL_EIS_INPUT_FALLBACK,
                    message:
                        "RemoteDesktop EIS key sequence failed; fell back to legacy portal input."
                            .to_string(),
                    details: Some(eis_fallback_details(&failure)),
                })
                .await;

                // Prefer X11/XTest or LinuxVirtualInput over per-keysym D-Bus round-trips.
                if input_xtest::xtest_is_available() {
                    match input_xtest::press_key_sequence(keys) {
                        Ok(()) => {
                            self.push_lifecycle_event(PortalLifecycleEvent {
                                code: PORTAL_EIS_INPUT_FALLBACK,
                                message: "RemoteDesktop EIS key sequence failed; fell back to X11/XTest input.".to_string(),
                                details: None,
                            })
                            .await;
                            return Ok(());
                        }
                        Err(error) => {
                            warn!(message = %error.message, "XTest key sequence fallback failed");
                        }
                    }
                }
                if virtual_input_keyboard_available() {
                    match LinuxVirtualInput::new() {
                        Ok(vi) => match vi.press_key_sequence(keys) {
                            Ok(()) => {
                                self.push_lifecycle_event(PortalLifecycleEvent {
                                    code: PORTAL_EIS_INPUT_FALLBACK,
                                    message: "RemoteDesktop EIS key sequence failed; fell back to Linux virtual input.".to_string(),
                                    details: None,
                                })
                                .await;
                                return Ok(());
                            }
                            Err(error) => {
                                warn!(message = %error.message, "LinuxVirtualInput key sequence fallback failed");
                            }
                        },
                        Err(error) => {
                            warn!(message = %error.message, "LinuxVirtualInput initialization failed");
                        }
                    }
                }
            }
            Err(failure) => return Err(failure.error),
        }

        // Final resort: legacy per-keysym injection via D-Bus.
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

    async fn run_eis_action_with_retry(
        &self,
        action: EisAction,
    ) -> Result<String, EisOperationError> {
        match self.run_eis_action_once(action.clone()).await {
            Ok(details) => Ok(details),
            Err(failure)
                if failure.established
                    && failure.error.code != BackendErrorCode::InvalidRequest.as_str() =>
            {
                self.push_lifecycle_event(PortalLifecycleEvent {
                    code: PORTAL_SESSION_REBUILT,
                    message: "Rebuilding the cached RemoteDesktop session after EIS input failed."
                        .to_string(),
                    details: Some(failure.error.message.clone()),
                })
                .await;
                self.reset_session().await;
                self.run_eis_action_once(action).await
            }
            Err(failure) => Err(failure),
        }
    }

    async fn run_eis_action_once(&self, action: EisAction) -> Result<String, EisOperationError> {
        let worker = self.eis_worker().await?;
        worker.execute(action).await
    }

    async fn eis_worker(&self) -> Result<EisWorkerHandle, EisOperationError> {
        let mut state = self.inner.write().await;
        self.ensure_session_started_locked(&mut state)
            .await
            .map_err(|error| EisOperationError {
                error,
                established: false,
            })?;
        let session = state.session.as_mut().expect("portal session should exist");
        if let Some(worker) = session.eis_worker.as_ref() {
            return Ok(worker.clone());
        }

        let fd = session
            .remote_desktop
            .connect_to_eis(&session.session, ConnectToEISOptions::default())
            .await
            .map_err(|error| EisOperationError {
                error: BackendError::new(
                    BackendErrorCode::ActionUnsupportedForEnvironment,
                    format!("failed to connect RemoteDesktop session to EIS: {error}"),
                ),
                established: false,
            })?;
        let worker = tokio::task::spawn_blocking(move || spawn_eis_worker(fd))
            .await
            .map_err(|error| EisOperationError {
                error: BackendError::new(
                    BackendErrorCode::ActionUnsupportedForEnvironment,
                    format!("EIS worker spawn task panicked: {error}"),
                ),
                established: false,
            })?
            .map_err(|error| EisOperationError {
                error,
                established: false,
            })?;
        session.eis_worker = Some(worker.clone());
        Ok(worker)
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
        self.ensure_session_started().await?;
        let manager_state = self.inner.read().await;
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
        self.ensure_session_started().await?;
        let manager_state = self.inner.read().await;
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

    /// Fast-path session check: tries a read lock first; only upgrades to a
    /// write lock if the session is missing. Once a session is established
    /// (which happens early), almost all portal calls become cheap read-only
    /// operations and can run concurrently.
    async fn ensure_session_started(&self) -> Result<(), BackendError> {
        {
            let state = self.inner.read().await;
            if state.session.is_some() {
                return Ok(());
            }
        }
        let mut state = self.inner.write().await;
        if state.session.is_some() {
            return Ok(());
        }
        let started = start_session_with_timeout(self.token_store.as_ref()).await?;
        state.pending_events.extend(started.lifecycle_events);
        state.session = Some(started.session);
        Ok(())
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
        let mut state = self.inner.write().await;
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

/// Spawn the EIS input worker thread.
///
/// NOTE: This is a single dedicated thread that processes one command at a time,
/// with 20–80 ms sleeps baked into every action. There is no parallelism for
/// independent input streams (pointer vs. keyboard). This is an accepted
/// architectural throughput ceiling for a desktop automation tool; splitting
/// into separate pointer and keyboard workers would only be needed if much
/// higher input rates become a requirement.
fn spawn_eis_worker(fd: OwnedFd) -> Result<EisWorkerHandle, BackendError> {
    let (sender, receiver) = mpsc::sync_channel(8);
    let (ready_sender, ready_receiver) = mpsc::channel();
    thread::Builder::new()
        .name("sky-cua-eis-input".to_string())
        .spawn(move || match EisWorker::new(fd) {
            Ok(worker) => {
                let _ = ready_sender.send(Ok(()));
                worker.run(receiver);
            }
            Err(error) => {
                let _ = ready_sender.send(Err(error));
            }
        })
        .map_err(|error| {
            BackendError::new(
                BackendErrorCode::ActionUnsupportedForEnvironment,
                format!("failed to spawn RemoteDesktop EIS input worker: {error}"),
            )
        })?;

    match ready_receiver.recv_timeout(EIS_WORKER_START_TIMEOUT) {
        Ok(Ok(())) => Ok(EisWorkerHandle { sender }),
        Ok(Err(error)) => Err(error),
        Err(mpsc::RecvTimeoutError::Timeout) => Err(BackendError::new(
            BackendErrorCode::ActionUnsupportedForEnvironment,
            "timed out starting the RemoteDesktop EIS input worker",
        )),
        Err(mpsc::RecvTimeoutError::Disconnected) => Err(BackendError::new(
            BackendErrorCode::ActionUnsupportedForEnvironment,
            "RemoteDesktop EIS input worker exited before startup completed",
        )),
    }
}

fn keyboard_device_from_event(
    device: reis::event::Device,
    keyboard: ei::Keyboard,
) -> Result<EisKeyboardDevice, BackendError> {
    let keymap = device.keymap().ok_or_else(|| {
        BackendError::new(
            BackendErrorCode::ActionUnsupportedForEnvironment,
            "EIS keyboard device did not advertise an XKB keymap",
        )
    })?;
    if keymap.type_ != ei::keyboard::KeymapType::Xkb {
        return Err(BackendError::new(
            BackendErrorCode::ActionUnsupportedForEnvironment,
            format!(
                "EIS keyboard advertised an unsupported keymap type: {:?}",
                keymap.type_
            ),
        ));
    }
    let keymap_fd = keymap.fd.as_fd().try_clone_to_owned().map_err(|error| {
        BackendError::new(
            BackendErrorCode::ActionUnsupportedForEnvironment,
            format!("failed to duplicate EIS keyboard keymap fd: {error}"),
        )
    })?;
    let context = xkb::Context::new(0);
    let xkb_keymap = unsafe {
        xkb::Keymap::new_from_fd(
            &context,
            keymap_fd,
            keymap.size as usize,
            xkb::KEYMAP_FORMAT_TEXT_V1,
            0,
        )
    }
    .map_err(|error| {
        BackendError::new(
            BackendErrorCode::ActionUnsupportedForEnvironment,
            format!("failed to read EIS keyboard keymap: {error}"),
        )
    })?
    .ok_or_else(|| {
        BackendError::new(
            BackendErrorCode::ActionUnsupportedForEnvironment,
            "failed to parse EIS keyboard keymap",
        )
    })?;
    let shift_keycode = find_eis_keycode_for_keysym(&xkb_keymap, xkb::keysyms::KEY_Shift_L as i32)
        .or_else(|| find_eis_keycode_for_keysym(&xkb_keymap, xkb::keysyms::KEY_Shift_R as i32))
        .map(|stroke| stroke.keycode);
    let keysym_cache = build_keysym_cache(&xkb_keymap);
    Ok(EisKeyboardDevice {
        device,
        keyboard,
        shift_keycode,
        keysym_cache,
    })
}

fn resolve_eis_keystroke(
    device: &EisKeyboardDevice,
    keysym: i32,
) -> Result<EisKeyStroke, BackendError> {
    device
        .keysym_cache
        .get(&(keysym as u32))
        .copied()
        .ok_or_else(|| {
            BackendError::new(
                BackendErrorCode::ActionUnsupportedForEnvironment,
                format!("EIS keyboard keymap cannot produce keysym 0x{keysym:x}"),
            )
        })
}

fn build_keysym_cache(keymap: &xkb::Keymap) -> HashMap<u32, EisKeyStroke> {
    let mut cache = HashMap::new();
    let min_keycode = keymap.min_keycode().raw();
    let max_keycode = keymap.max_keycode().raw();
    for raw_keycode in min_keycode..=max_keycode {
        let keycode = xkb::Keycode::new(raw_keycode);
        let layout_count = keymap.num_layouts_for_key(keycode).max(1);
        for layout in 0..layout_count {
            let level_count = keymap.num_levels_for_key(keycode, layout).max(1);
            for level in 0..level_count {
                for keysym in keymap.key_get_syms_by_level(keycode, layout, level) {
                    let evdev_keycode = raw_keycode.saturating_sub(8);
                    let stroke = EisKeyStroke {
                        keycode: evdev_keycode,
                        needs_shift: level == 1,
                    };
                    cache.insert(keysym.raw(), stroke);
                }
            }
        }
    }
    cache
}

fn find_eis_keycode_for_keysym(keymap: &xkb::Keymap, keysym: i32) -> Option<EisKeyStroke> {
    let keysym = xkb::Keysym::new(u32::try_from(keysym).ok()?);
    let min_keycode = keymap.min_keycode().raw();
    let max_keycode = keymap.max_keycode().raw();
    for raw_keycode in min_keycode..=max_keycode {
        let keycode = xkb::Keycode::new(raw_keycode);
        let layout_count = keymap.num_layouts_for_key(keycode).max(1);
        for layout in 0..layout_count {
            let level_count = keymap.num_levels_for_key(keycode, layout).max(1);
            for level in 0..level_count {
                if keymap
                    .key_get_syms_by_level(keycode, layout, level)
                    .contains(&keysym)
                {
                    return raw_keycode.checked_sub(8).map(|keycode| EisKeyStroke {
                        keycode,
                        needs_shift: level == 1,
                    });
                }
            }
        }
    }
    None
}

fn validate_eis_absolute_point(
    device: &reis::event::Device,
    x: f64,
    y: f64,
) -> Result<(), BackendError> {
    let regions = device.regions();
    if regions.is_empty() {
        return Ok(());
    }
    let in_region = regions.iter().any(|region| {
        let min_x = f64::from(region.x);
        let min_y = f64::from(region.y);
        let max_x = min_x + f64::from(region.width);
        let max_y = min_y + f64::from(region.height);
        x >= min_x && x < max_x && y >= min_y && y < max_y
    });
    if in_region {
        return Ok(());
    }
    Err(BackendError::new(
        BackendErrorCode::InvalidRequest,
        format!(
            "{EIS_POINT_OUTSIDE_REGION_PREFIX} ({x:.1}, {y:.1}) is outside the advertised input regions: {}",
            describe_eis_device(device)
        ),
    ))
}

fn should_fallback_to_legacy(failure: &EisOperationError) -> bool {
    if failure.error.code != BackendErrorCode::InvalidRequest.as_str() {
        return true;
    }
    is_eis_region_mismatch(&failure.error)
}

fn should_reset_session_before_legacy_fallback(failure: &EisOperationError) -> bool {
    failure.established && failure.error.code != BackendErrorCode::InvalidRequest.as_str()
}

fn is_eis_region_mismatch(error: &BackendError) -> bool {
    error.code == BackendErrorCode::InvalidRequest.as_str()
        && error.message.starts_with(EIS_POINT_OUTSIDE_REGION_PREFIX)
}

fn eis_fallback_details(failure: &EisOperationError) -> String {
    format!(
        "{}; eis_established={}; reset_cached_session={}",
        failure.error.message,
        failure.established,
        should_reset_session_before_legacy_fallback(failure)
    )
}

fn describe_eis_device(device: &reis::event::Device) -> String {
    let mut description = String::with_capacity(128);
    let _ = write!(
        &mut description,
        "device_type={:?}; regions=[",
        device.device_type()
    );
    for (i, region) in device.regions().iter().enumerate() {
        if i > 0 {
            description.push_str("; ");
        }
        let _ = write!(
            &mut description,
            "x={},y={},width={},height={},scale={},mapping_id={}",
            region.x,
            region.y,
            region.width,
            region.height,
            region.scale,
            region.mapping_id.as_deref().unwrap_or("")
        );
    }
    description.push(']');
    description
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

    let mut source_options = SelectSourcesOptions::default()
        .set_sources(BitFlags::from_flag(SourceType::Monitor))
        .set_multiple(false)
        .set_persist_mode(PersistMode::DoNot);
    if let Some(cursor_mode) = supported_cursor_mode().await {
        source_options = source_options.set_cursor_mode(cursor_mode);
    }

    screencast
        .select_sources(&session, source_options)
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
            eis_worker: None,
        },
        lifecycle_events,
    })
}

async fn supported_cursor_mode() -> Option<CursorMode> {
    match portal_u32_property("org.freedesktop.portal.ScreenCast", "AvailableCursorModes").await {
        Ok(mask) => cursor_mode_for_available_modes(mask).or_else(|| {
            warn!(
                available_cursor_modes = mask,
                "portal reported no supported cursor modes; omitting cursor mode from ScreenCast SelectSources"
            );
            None
        }),
        Err(error) => {
            warn!(
                error = %error,
                "could not read portal cursor modes; falling back to metadata cursor mode"
            );
            Some(CursorMode::Metadata)
        }
    }
}

fn cursor_mode_for_available_modes(mask: u32) -> Option<CursorMode> {
    if mask & CURSOR_MODE_METADATA != 0 {
        Some(CursorMode::Metadata)
    } else if mask & CURSOR_MODE_HIDDEN != 0 {
        Some(CursorMode::Hidden)
    } else if mask & CURSOR_MODE_EMBEDDED != 0 {
        Some(CursorMode::Embedded)
    } else {
        None
    }
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

fn monotonic_micros() -> u64 {
    let mut ts = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    let result = unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts) };
    if result != 0 {
        return 0;
    }
    u64::try_from(ts.tv_sec)
        .unwrap_or(0)
        .saturating_mul(1_000_000)
        .saturating_add(u64::try_from(ts.tv_nsec).unwrap_or(0) / 1_000)
}

fn eis_device_capabilities() -> BitFlags<DeviceCapability> {
    DeviceCapability::Pointer
        | DeviceCapability::PointerAbsolute
        | DeviceCapability::Keyboard
        | DeviceCapability::Button
        | DeviceCapability::Scroll
}

fn eis_scroll_delta_from_action(delta_y: f64) -> f32 {
    (-delta_y) as f32
}

fn eis_scroll_steps_from_action(steps: i32) -> i32 {
    -steps
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
    let key = key.trim();
    // Single-character shortcut (after trimming).
    let mut chars = key.chars();
    if let (Some(c), None) = (chars.next(), chars.next()) {
        return keysym_for_char(c);
    }

    // Helper: compare `key` against `name` case-insensitively, ignoring underscores in `key`.
    let eq = |name: &str| {
        let mut key_chars = key.chars().filter(|&c| c != '_');
        let mut name_chars = name.chars();
        loop {
            match (key_chars.next(), name_chars.next()) {
                (None, None) => return true,
                (Some(k), Some(n)) if k.eq_ignore_ascii_case(&n) => continue,
                _ => return false,
            }
        }
    };

    if eq("enter") || eq("return") {
        return Some(0xff0d);
    }
    if eq("tab") {
        return Some(0xff09);
    }
    if eq("backspace") {
        return Some(0xff08);
    }
    if eq("escape") || eq("esc") {
        return Some(0xff1b);
    }
    if eq("space") {
        return Some(0x20);
    }
    if eq("delete") || eq("del") {
        return Some(0xffff);
    }
    if eq("left") {
        return Some(0xff51);
    }
    if eq("up") {
        return Some(0xff52);
    }
    if eq("right") {
        return Some(0xff53);
    }
    if eq("down") {
        return Some(0xff54);
    }
    if eq("home") {
        return Some(0xff50);
    }
    if eq("end") {
        return Some(0xff57);
    }
    if eq("pageup") {
        return Some(0xff55);
    }
    if eq("pagedown") {
        return Some(0xff56);
    }
    if eq("shift") || eq("shiftl") {
        return Some(0xffe1);
    }
    if eq("control") || eq("ctrl") || eq("ctrll") {
        return Some(0xffe3);
    }
    if eq("alt") || eq("altl") {
        return Some(0xffe9);
    }
    if eq("meta") || eq("super") || eq("superl") || eq("metal") {
        return Some(0xffeb);
    }
    if eq("capslock") {
        return Some(0xffe5);
    }
    if eq("f1") {
        return Some(0xffbe);
    }
    if eq("f2") {
        return Some(0xffbf);
    }
    if eq("f3") {
        return Some(0xffc0);
    }
    if eq("f4") {
        return Some(0xffc1);
    }
    if eq("f5") {
        return Some(0xffc2);
    }
    if eq("f6") {
        return Some(0xffc3);
    }
    if eq("f7") {
        return Some(0xffc4);
    }
    if eq("f8") {
        return Some(0xffc5);
    }
    if eq("f9") {
        return Some(0xffc6);
    }
    if eq("f10") {
        return Some(0xffc7);
    }
    if eq("f11") {
        return Some(0xffc8);
    }
    if eq("f12") {
        return Some(0xffc9);
    }
    None
}

#[cfg(test)]
mod tests {
    use ashpd::desktop::screencast::CursorMode;
    use sky_cua_platform::diagnostics::{BackendError, BackendErrorCode};

    use super::{
        EIS_POINT_OUTSIDE_REGION_PREFIX, EisOperationError, cursor_mode_for_available_modes,
        eis_fallback_details, eis_scroll_delta_from_action, eis_scroll_steps_from_action,
        keysym_for_char, keysym_for_key_name, should_fallback_to_legacy,
        should_reset_session_before_legacy_fallback,
    };

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

    #[test]
    fn chooses_supported_portal_cursor_mode() {
        assert!(matches!(
            cursor_mode_for_available_modes(4),
            Some(CursorMode::Metadata)
        ));
        assert!(matches!(
            cursor_mode_for_available_modes(1),
            Some(CursorMode::Hidden)
        ));
        assert!(matches!(
            cursor_mode_for_available_modes(2),
            Some(CursorMode::Embedded)
        ));
        assert!(cursor_mode_for_available_modes(0).is_none());
    }

    #[test]
    fn maps_action_scroll_direction_to_eis_direction() {
        assert_eq!(eis_scroll_delta_from_action(-180.0), 180.0);
        assert_eq!(eis_scroll_delta_from_action(120.0), -120.0);
        assert_eq!(eis_scroll_steps_from_action(-2), 2);
        assert_eq!(eis_scroll_steps_from_action(3), -3);
    }

    #[test]
    fn eis_pointer_region_mismatch_falls_back_without_resetting_session() {
        let failure = EisOperationError {
            error: BackendError::new(
                BackendErrorCode::InvalidRequest,
                format!(
                    "{EIS_POINT_OUTSIDE_REGION_PREFIX} (2000.0, 1200.0) is outside the advertised input regions"
                ),
            ),
            established: true,
        };

        assert!(should_fallback_to_legacy(&failure));
        assert!(!should_reset_session_before_legacy_fallback(&failure));
        assert!(eis_fallback_details(&failure).contains("reset_cached_session=false"));
    }

    #[test]
    fn arbitrary_pointer_invalid_request_stays_a_hard_eis_failure() {
        let failure = EisOperationError {
            error: BackendError::new(BackendErrorCode::InvalidRequest, "bad pointer request"),
            established: true,
        };

        assert!(!should_fallback_to_legacy(&failure));
    }
}
