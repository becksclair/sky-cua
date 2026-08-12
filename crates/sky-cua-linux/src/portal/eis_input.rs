use std::collections::{HashMap, HashSet};
use std::fmt;
use std::fmt::Write as _;
use std::os::fd::{AsFd, OwnedFd};
use std::os::unix::net::UnixStream;
use std::thread;
use std::time::{Duration, Instant};

use enumflags2::BitFlags;
use reis::ei;
use reis::event::{DeviceCapability, EiEvent, EiEventConverter};
use sky_cua_platform::diagnostics::{BackendError, BackendErrorCode};
use sky_cua_platform::model::CuaCancellation;
use tracing::warn;
use xkbcommon::xkb;

use crate::portal::eis_keymap::{
    EisKeyStroke, build_keysym_cache, clear_modifiers_already_present_in_chord,
    find_keycodes_from_cache, keysym_for_char, keysym_for_key_name, required_modifier_keycodes,
    resolve_eis_keystroke,
};
use crate::portal::remote_desktop::{EIS_POINT_OUTSIDE_REGION_PREFIX, MouseButton, evdev_button};

pub(crate) struct EisInput {
    context: ei::Context,
    connection: reis::event::Connection,
    converter: EiEventConverter,
    sequence: u32,
}

#[derive(Clone)]
pub(crate) struct EisPointerDevice {
    pub device: reis::event::Device,
    pub pointer_absolute: ei::PointerAbsolute,
    pub button: ei::Button,
    pub scroll: Option<ei::Scroll>,
    pub description: String,
}

#[derive(Clone)]
pub(crate) struct EisKeyboardDevice {
    pub device: reis::event::Device,
    pub keyboard: ei::Keyboard,
    pub shift_keycodes: Vec<u32>,
    pub level3_keycodes: Vec<u32>,
    pub keysym_cache: HashMap<u32, EisKeyStroke>,
    pub description: String,
}

#[derive(Clone)]
pub(crate) enum EisReadyDevice {
    Pointer(EisPointerDevice),
    Keyboard(EisKeyboardDevice),
    DevicePaused(reis::event::Device),
    DeviceStopEmulating(reis::event::Device),
    DeviceRemoved(reis::event::Device),
}

#[derive(Clone)]
pub(crate) struct EisWorkerHandle {
    sender: tokio::sync::mpsc::Sender<EisCommand>,
}

impl fmt::Debug for EisWorkerHandle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EisWorkerHandle")
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone)]
pub(crate) enum EisAction {
    Move {
        x: f64,
        y: f64,
    },
    Click {
        x: f64,
        y: f64,
        button: MouseButton,
    },
    Drag {
        waypoints: std::sync::Arc<[(f64, f64)]>,
        step_delay: Duration,
        cancellation: Option<CuaCancellation>,
    },
    Scroll {
        target: Option<(f64, f64)>,
        axis: EisScrollAxis,
        delta: Option<f64>,
        steps: i32,
    },
    SendText {
        text: std::sync::Arc<str>,
    },
    PressKeySequence {
        keys: std::sync::Arc<[String]>,
    },
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum EisScrollAxis {
    Horizontal,
    Vertical,
}

pub(crate) enum EisCommand {
    Execute {
        action: EisAction,
        reply: tokio::sync::oneshot::Sender<Result<String, BackendError>>,
    },
}

#[derive(Debug)]
pub(crate) struct EisOperationError {
    pub error: BackendError,
    pub established: bool,
}

impl EisInput {
    pub(crate) fn new(fd: OwnedFd) -> Result<Self, BackendError> {
        let stream = UnixStream::from(fd);
        stream
            .set_read_timeout(Some(Duration::from_millis(250)))
            .map_err(|error| {
                BackendError::new(
                    BackendErrorCode::ActionUnsupportedForEnvironment,
                    format!("failed to configure EIS fd read timeout: {error}"),
                )
            })?;
        stream
            .set_write_timeout(Some(Duration::from_secs(3)))
            .map_err(|error| {
                BackendError::new(
                    BackendErrorCode::ActionUnsupportedForEnvironment,
                    format!("failed to configure EIS fd write timeout: {error}"),
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

    pub(crate) fn last_serial(&self) -> u32 {
        self.connection.serial()
    }

    pub(crate) fn flush(&self) -> Result<(), BackendError> {
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
                            let description = describe_eis_device(&event.device);
                            return Ok(Some(EisReadyDevice::Pointer(EisPointerDevice {
                                scroll: event.device.interface::<ei::Scroll>(),
                                device: event.device,
                                pointer_absolute,
                                button,
                                description,
                            })));
                        }
                        if let Some(keyboard) = event.device.interface::<ei::Keyboard>() {
                            return Ok(Some(EisReadyDevice::Keyboard(keyboard_device_from_event(
                                event.device,
                                keyboard,
                            )?)));
                        }
                    }
                    EiEvent::DevicePaused(event) => {
                        return Ok(Some(EisReadyDevice::DevicePaused(event.device)));
                    }
                    EiEvent::DeviceStopEmulating(event) => {
                        return Ok(Some(EisReadyDevice::DeviceStopEmulating(event.device)));
                    }
                    EiEvent::DeviceRemoved(event) => {
                        return Ok(Some(EisReadyDevice::DeviceRemoved(event.device)));
                    }
                    _ => {}
                }
            }
        }
        Ok(None)
    }
}

impl EisWorkerHandle {
    pub(crate) async fn execute(&self, action: EisAction) -> Result<String, EisOperationError> {
        let (reply, receiver) = tokio::sync::oneshot::channel();
        self.sender
            .send(EisCommand::Execute { action, reply })
            .await
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

pub(crate) struct EisWorker {
    input: EisInput,
    pointer: Option<EisPointerDevice>,
    keyboard: Option<EisKeyboardDevice>,
    emulating_devices: HashSet<ei::Device>,
}

impl EisWorker {
    pub(crate) fn new(fd: OwnedFd) -> Result<Self, BackendError> {
        Ok(Self {
            input: EisInput::new(fd)?,
            pointer: None,
            keyboard: None,
            emulating_devices: HashSet::new(),
        })
    }

    pub(crate) fn run(mut self, mut receiver: tokio::sync::mpsc::Receiver<EisCommand>) {
        while let Some(command) = receiver.blocking_recv() {
            match command {
                EisCommand::Execute { action, reply } => {
                    let _ = reply.send(self.execute(action));
                }
            }
        }
    }

    fn execute(&mut self, action: EisAction) -> Result<String, BackendError> {
        match action {
            EisAction::Move { x, y } => self.move_pointer_absolute(x, y),
            EisAction::Click { x, y, button } => self.click_at(x, y, button),
            EisAction::Drag {
                waypoints,
                step_delay,
                cancellation,
            } => self.drag(waypoints.as_ref(), step_delay, cancellation.as_ref()),
            EisAction::Scroll {
                target,
                axis,
                delta,
                steps,
            } => self.scroll(target, axis, delta, steps),
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
                EisReadyDevice::DevicePaused(device)
                | EisReadyDevice::DeviceStopEmulating(device)
                | EisReadyDevice::DeviceRemoved(device) => {
                    self.clear_cached_device(&device);
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
                EisReadyDevice::DevicePaused(device)
                | EisReadyDevice::DeviceStopEmulating(device)
                | EisReadyDevice::DeviceRemoved(device) => {
                    self.clear_cached_device(&device);
                }
            }
        }
    }

    fn clear_cached_device(&mut self, device: &reis::event::Device) {
        let protocol_device = device.device();
        self.emulating_devices.remove(protocol_device);
        if self
            .pointer
            .as_ref()
            .map(|d| d.device.device().eq(protocol_device))
            .unwrap_or(false)
        {
            self.pointer = None;
        }
        if self
            .keyboard
            .as_ref()
            .map(|d| d.device.device().eq(protocol_device))
            .unwrap_or(false)
        {
            self.keyboard = None;
        }
    }

    fn ensure_emulating(
        &mut self,
        device: &reis::event::Device,
    ) -> Result<EmulationResult, BackendError> {
        let protocol_device = device.device().clone();
        if !self.emulating_devices.insert(protocol_device.clone()) {
            return Ok(EmulationResult::already_active());
        }
        let serial = self.input.last_serial();
        let sequence = self.input.next_sequence();
        protocol_device.start_emulating(serial, sequence);
        self.input.flush()?;
        thread::sleep(EIS_FRAME_GAP_DELAY);
        Ok(EmulationResult::newly_started(sequence))
    }

    /// Like [`ensure_emulating`], but sleeps after the first start so the
    /// virtual keyboard has time to settle. Use this for keyboard actions only.
    fn ensure_emulating_with_settle(
        &mut self,
        device: &reis::event::Device,
    ) -> Result<String, BackendError> {
        let emulation = self.ensure_emulating(device)?;
        if emulation.just_started {
            thread::sleep(EIS_KEYBOARD_EMULATION_SETTLE_DELAY);
        }
        Ok(emulation.detail)
    }

    fn click_at(&mut self, x: f64, y: f64, button: MouseButton) -> Result<String, BackendError> {
        let device = self.pointer_device()?;
        validate_eis_absolute_point(&device.device, x, y)?;
        let details = device.description.clone();
        let emulation = self.ensure_emulating(&device.device)?;
        let serial = self.input.last_serial();
        let evdev = evdev_button(button) as u32;
        device.pointer_absolute.motion_absolute(x as f32, y as f32);
        device.device.device().frame(serial, monotonic_micros());
        device.button.button(evdev, ei::button::ButtonState::Press);
        device.device.device().frame(serial, monotonic_micros());
        self.input.flush()?;
        thread::sleep(EIS_POINTER_BUTTON_HOLD_DELAY);
        device
            .button
            .button(evdev, ei::button::ButtonState::Released);
        device.device.device().frame(serial, monotonic_micros());
        self.input.flush()?;
        thread::sleep(EIS_FINAL_FLUSH_DELAY);
        Ok(format_action_result(&details, &emulation.detail, None))
    }

    fn move_pointer_absolute(&mut self, x: f64, y: f64) -> Result<String, BackendError> {
        let device = self.pointer_device()?;
        validate_eis_absolute_point(&device.device, x, y)?;
        let details = device.description.clone();
        let emulation = self.ensure_emulating(&device.device)?;
        let serial = self.input.last_serial();
        device.pointer_absolute.motion_absolute(x as f32, y as f32);
        device.device.device().frame(serial, monotonic_micros());
        self.input.flush()?;
        thread::sleep(EIS_FINAL_FLUSH_DELAY);
        Ok(format_action_result(&details, &emulation.detail, None))
    }

    fn drag(
        &mut self,
        waypoints: &[(f64, f64)],
        step_delay: Duration,
        cancellation: Option<&CuaCancellation>,
    ) -> Result<String, BackendError> {
        let device = self.pointer_device()?;
        let Some((first, rest)) = waypoints.split_first() else {
            return Err(BackendError::new(
                BackendErrorCode::ActionUnsupportedForEnvironment,
                "drag requires at least one waypoint",
            ));
        };
        // The press and release must land on valid points; require those. Interior
        // points are validated per-frame below so a non-convex region (e.g. the
        // gap between two monitors) cannot make us emit an off-region motion.
        let last = rest.last().copied().unwrap_or(*first);
        validate_eis_absolute_point(&device.device, first.0, first.1)?;
        validate_eis_absolute_point(&device.device, last.0, last.1)?;
        let details = device.description.clone();
        let emulation = self.ensure_emulating(&device.device)?;
        // One serial for the whole action, matching click/scroll: `last_serial()`
        // only advances when we process inbound EIS events, and the drag loop
        // never pumps them, so the value is stable for the action's duration.
        let serial = self.input.last_serial();
        let evdev = evdev_button(MouseButton::Left) as u32;
        // Press, then emit every waypoint as its own frame+flush inside one grab
        // so the compositor sees continuous motion rather than a teleport.
        device
            .pointer_absolute
            .motion_absolute(first.0 as f32, first.1 as f32);
        device.device.device().frame(serial, monotonic_micros());
        device.button.button(evdev, ei::button::ButtonState::Press);
        device.device.device().frame(serial, monotonic_micros());
        if let Err(error) = self.input.flush() {
            device
                .button
                .button(evdev, ei::button::ButtonState::Released);
            device.device.device().frame(serial, monotonic_micros());
            let _ = self.input.flush();
            return Err(error);
        }
        thread::sleep(EIS_FRAME_GAP_DELAY);
        let mut cancelled = cancellation.is_some_and(CuaCancellation::is_cancelled);
        let mut motion_error = None;
        for &(x, y) in rest {
            if cancellation.is_some_and(CuaCancellation::is_cancelled) {
                cancelled = true;
                break;
            }
            // Skip any interpolated point outside the advertised regions rather
            // than emit a motion the compositor would drop; the destination
            // (`last`) is validated above so it is always emitted.
            if validate_eis_absolute_point(&device.device, x, y).is_err() {
                continue;
            }
            device.pointer_absolute.motion_absolute(x as f32, y as f32);
            device.device.device().frame(serial, monotonic_micros());
            if let Err(error) = self.input.flush() {
                motion_error = Some(error);
                break;
            }
            thread::sleep(step_delay);
        }
        device
            .button
            .button(evdev, ei::button::ButtonState::Released);
        device.device.device().frame(serial, monotonic_micros());
        let release = self.input.flush();
        thread::sleep(EIS_FINAL_FLUSH_DELAY);
        if let Some(error) = motion_error {
            return Err(error);
        }
        release?;
        if cancelled {
            return Err(BackendError::new(
                BackendErrorCode::CuaActionOutcomeUnknown,
                "the CUA drag was cancelled after pointer-down; EIS pointer release completed",
            ));
        }
        Ok(format_action_result(&details, &emulation.detail, None))
    }

    fn scroll(
        &mut self,
        target: Option<(f64, f64)>,
        axis: EisScrollAxis,
        delta: Option<f64>,
        steps: i32,
    ) -> Result<String, BackendError> {
        let device = self.pointer_device()?;
        let details = device.description.clone();
        let scroll = device.scroll.as_ref().ok_or_else(|| {
            BackendError::new(
                BackendErrorCode::ActionUnsupportedForEnvironment,
                "EIS pointer device did not advertise scroll support",
            )
        })?;
        let emulation = self.ensure_emulating(&device.device)?;
        let serial = self.input.last_serial();
        if let Some((x, y)) = target {
            validate_eis_absolute_point(&device.device, x, y)?;
            device.pointer_absolute.motion_absolute(x as f32, y as f32);
            device.device.device().frame(serial, monotonic_micros());
        }
        let mut scroll_details = String::with_capacity(32);
        let axis_name = match axis {
            EisScrollAxis::Horizontal => "x",
            EisScrollAxis::Vertical => "y",
        };
        if let Some(delta) = delta {
            let eis_delta = eis_scroll_delta_from_action(axis, delta);
            match axis {
                EisScrollAxis::Horizontal => scroll.scroll(eis_delta, 0.0),
                EisScrollAxis::Vertical => scroll.scroll(0.0, eis_delta),
            }
            let _ = write!(&mut scroll_details, "scroll_delta_{axis_name}={eis_delta}");
        } else {
            let eis_steps = eis_scroll_steps_from_action(axis, steps);
            match axis {
                EisScrollAxis::Horizontal => scroll.scroll_discrete(eis_steps, 0),
                EisScrollAxis::Vertical => scroll.scroll_discrete(0, eis_steps),
            }
            let _ = write!(
                &mut scroll_details,
                "scroll_discrete_{axis_name}={eis_steps}"
            );
        }
        device.device.device().frame(serial, monotonic_micros());
        match axis {
            EisScrollAxis::Horizontal => scroll.scroll_stop(1, 0, 0),
            EisScrollAxis::Vertical => scroll.scroll_stop(0, 1, 0),
        }
        device.device.device().frame(serial, monotonic_micros());
        self.input.flush()?;
        thread::sleep(EIS_FINAL_FLUSH_DELAY);
        Ok(format_action_result(
            &details,
            &emulation.detail,
            Some(scroll_details),
        ))
    }

    fn send_text(&mut self, text: &str) -> Result<String, BackendError> {
        let device = self.keyboard_device()?;
        let details = device.description.clone();
        let emulation_detail = self.ensure_emulating_with_settle(&device.device)?;
        // `text.len()` is a safe O(1) upper bound for the stroke Vec capacity
        // (byte length >= char count in UTF-8). The Vec may be slightly
        // over-allocated for multi-byte characters, which is harmless.
        let mut strokes = Vec::with_capacity(text.len());
        for character in text.chars() {
            let keysym = keysym_for_char(character).ok_or_else(|| {
                BackendError::new(
                    BackendErrorCode::InvalidRequest,
                    format!(
                        "cannot type unsupported character {character:?} through EIS keymap injection"
                    ),
                )
            })?;
            strokes.push(resolve_eis_keystroke(&device.keysym_cache, keysym)?);
        }
        for stroke in &strokes {
            self.send_eis_key_stroke(&device, *stroke)?;
            thread::sleep(EIS_TEXT_INTER_CHAR_DELAY);
        }
        thread::sleep(EIS_FINAL_FLUSH_DELAY);
        Ok(format_action_result(
            &details,
            &emulation_detail,
            Some(format!("typed_chars={}", strokes.len())),
        ))
    }

    fn press_key_sequence(&mut self, keys: &[String]) -> Result<String, BackendError> {
        if keys.is_empty() {
            return Err(BackendError::new(
                BackendErrorCode::InvalidRequest,
                "press_key requires at least one key",
            ));
        }

        let device = self.keyboard_device()?;
        let details = device.description.clone();
        let emulation_detail = self.ensure_emulating_with_settle(&device.device)?;
        let mut resolved = Vec::with_capacity(keys.len());
        for key in keys {
            let keysym = keysym_for_key_name(key).ok_or_else(|| {
                BackendError::new(
                    BackendErrorCode::InvalidRequest,
                    format!("unsupported key name {key:?}"),
                )
            })?;
            resolved.push(resolve_eis_keystroke(&device.keysym_cache, keysym)?);
        }

        if resolved.len() == 1 {
            self.send_eis_key_stroke(&device, resolved[0])?;
            // Inter-keystroke pacing before the final flush, matching the gap
            // used between individual characters in `send_text`.
            thread::sleep(EIS_FRAME_GAP_DELAY);
        } else {
            clear_modifiers_already_present_in_chord(
                &mut resolved,
                &device.shift_keycodes,
                &device.level3_keycodes,
            );
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
        Ok(format_action_result(
            &details,
            &emulation_detail,
            Some(format!("pressed_keys={}", keys.len())),
        ))
    }

    fn send_eis_key_stroke(
        &self,
        device: &EisKeyboardDevice,
        stroke: EisKeyStroke,
    ) -> Result<(), BackendError> {
        // Flushing after both Press and Release ensures each keystroke is
        // fully transmitted before the next event, which GNOME/mutter needs
        // for reliable virtual keyboard delivery.
        self.send_eis_key_state(device, stroke, ei::keyboard::KeyState::Press)?;
        self.input.flush()?;
        thread::sleep(EIS_KEY_HOLD_DELAY);
        self.send_eis_key_state(device, stroke, ei::keyboard::KeyState::Released)?;
        self.input.flush()
    }

    fn send_eis_key_state(
        &self,
        device: &EisKeyboardDevice,
        stroke: EisKeyStroke,
        state: ei::keyboard::KeyState,
    ) -> Result<(), BackendError> {
        let serial = self.input.last_serial();
        let modifier_keycodes =
            required_modifier_keycodes(stroke, &device.shift_keycodes, &device.level3_keycodes)?;
        match state {
            ei::keyboard::KeyState::Press => {
                for keycode in &modifier_keycodes {
                    device.keyboard.key(*keycode, ei::keyboard::KeyState::Press);
                }
                device.keyboard.key(stroke.keycode, state);
            }
            ei::keyboard::KeyState::Released => {
                device.keyboard.key(stroke.keycode, state);
                for keycode in modifier_keycodes.iter().rev() {
                    device
                        .keyboard
                        .key(*keycode, ei::keyboard::KeyState::Released);
                }
            }
        }
        device.device.device().frame(serial, monotonic_micros());
        Ok(())
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
pub(crate) async fn spawn_eis_worker(fd: OwnedFd) -> Result<EisWorkerHandle, BackendError> {
    let (sender, receiver) = tokio::sync::mpsc::channel(8);
    let (ready_sender, ready_receiver) = tokio::sync::oneshot::channel();
    thread::Builder::new()
        .name("sky-cua-eis-input".to_string())
        .spawn(move || {
            match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| EisWorker::new(fd))) {
                Ok(Ok(worker)) => {
                    let _ = ready_sender.send(Ok(()));
                    let run_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        worker.run(receiver)
                    }));
                    if let Err(payload) = run_result {
                        let message = panic_payload_message(payload.as_ref());
                        warn!(
                            panic = %message,
                            "RemoteDesktop EIS input worker panicked during run"
                        );
                    }
                }
                Ok(Err(error)) => {
                    let _ = ready_sender.send(Err(error));
                }
                Err(payload) => {
                    let message = panic_payload_message(payload.as_ref());
                    warn!(
                        panic = %message,
                        "RemoteDesktop EIS input worker panicked during startup"
                    );
                    let _ = ready_sender.send(Err(BackendError::new(
                        BackendErrorCode::ActionUnsupportedForEnvironment,
                        format!(
                            "RemoteDesktop EIS input worker panicked during startup: {message}"
                        ),
                    )));
                }
            }
        })
        .map_err(|error| {
            BackendError::new(
                BackendErrorCode::ActionUnsupportedForEnvironment,
                format!("failed to spawn RemoteDesktop EIS input worker: {error}"),
            )
        })?;

    match tokio::time::timeout(EIS_WORKER_START_TIMEOUT, ready_receiver).await {
        Ok(Ok(Ok(()))) => Ok(EisWorkerHandle { sender }),
        Ok(Ok(Err(error))) => Err(error),
        Ok(Err(_)) => Err(BackendError::new(
            BackendErrorCode::ActionUnsupportedForEnvironment,
            "RemoteDesktop EIS input worker exited before startup completed",
        )),
        Err(_) => Err(BackendError::new(
            BackendErrorCode::ActionUnsupportedForEnvironment,
            "timed out starting the RemoteDesktop EIS input worker",
        )),
    }
}

fn panic_payload_message(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(message) = payload.downcast_ref::<&str>() {
        (*message).to_string()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        "non-string panic payload".to_string()
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
    let keysym_cache = build_keysym_cache(&xkb_keymap);
    let shift_keycodes = find_keycodes_from_cache(
        &keysym_cache,
        &[xkb::keysyms::KEY_Shift_L, xkb::keysyms::KEY_Shift_R],
    );
    let level3_keycodes = find_keycodes_from_cache(
        &keysym_cache,
        &[
            xkb::keysyms::KEY_ISO_Level3_Shift,
            xkb::keysyms::KEY_Mode_switch,
            xkb::keysyms::KEY_Alt_R,
        ],
    );
    let description = describe_eis_device(&device);
    Ok(EisKeyboardDevice {
        device,
        keyboard,
        shift_keycodes,
        level3_keycodes,
        keysym_cache,
        description,
    })
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

pub(crate) fn eis_scroll_delta_from_action(axis: EisScrollAxis, delta: f64) -> f32 {
    match axis {
        EisScrollAxis::Horizontal => delta as f32,
        EisScrollAxis::Vertical => (-delta) as f32,
    }
}

pub(crate) fn eis_scroll_steps_from_action(axis: EisScrollAxis, steps: i32) -> i32 {
    match axis {
        EisScrollAxis::Horizontal => steps,
        EisScrollAxis::Vertical => -steps,
    }
}

const EIS_DEVICE_READY_TIMEOUT: Duration = Duration::from_secs(3);
const EIS_WORKER_START_TIMEOUT: Duration = Duration::from_secs(3);
const EIS_POINTER_BUTTON_HOLD_DELAY: Duration = Duration::from_millis(35);
const EIS_KEY_HOLD_DELAY: Duration = Duration::from_millis(5);
const EIS_FRAME_GAP_DELAY: Duration = Duration::from_millis(20);
const EIS_TEXT_INTER_CHAR_DELAY: Duration = Duration::from_millis(10);
const EIS_FINAL_FLUSH_DELAY: Duration = Duration::from_millis(80);
/// Settle delay after the first EIS keyboard emulation start.
/// GNOME/mutter needs ~120 ms before the virtual keyboard is ready to
/// consume keystrokes; 140 ms gives a small headroom without adding
/// latency to every subsequent action on an already-started device.
const EIS_KEYBOARD_EMULATION_SETTLE_DELAY: Duration = Duration::from_millis(140);

/// Build an action result string from device details, emulation details,
/// and an optional extra clause. Avoids the repeated copy-paste pattern
/// across every EIS action method.
fn format_action_result(details: &str, emulation: &str, extra: Option<String>) -> String {
    let extra_len = extra.as_ref().map_or(0, |s| s.len());
    let mut result = String::with_capacity(details.len() + emulation.len() + extra_len + 6);
    result.push_str(details);
    result.push_str("; ");
    result.push_str(emulation);
    if let Some(extra) = extra {
        result.push_str("; ");
        result.push_str(&extra);
    }
    result
}

/// Whether a device was already emulating or just started, with a
/// pre-formatted detail string so callers do not double-allocate.
struct EmulationResult {
    just_started: bool,
    detail: String,
}

impl EmulationResult {
    fn already_active() -> Self {
        Self {
            just_started: false,
            detail: "emulation_started=false".to_string(),
        }
    }

    fn newly_started(sequence: u32) -> Self {
        Self {
            just_started: true,
            detail: format!("emulation_started=true,emulation_sequence={sequence}"),
        }
    }
}

pub(crate) fn should_fallback_to_legacy(_failure: &EisOperationError) -> bool {
    // All EIS errors warrant trying legacy/native fallbacks, including
    // InvalidRequest. Region mismatches are a known pointer-specific case,
    // but unsupported characters or keysyms for keyboard are also recoverable
    // through xdotool/ydotool.
    true
}

pub(crate) fn should_reset_session_before_legacy_fallback(failure: &EisOperationError) -> bool {
    failure.established && failure.error.code != BackendErrorCode::InvalidRequest.as_str()
}

pub(crate) fn eis_fallback_details(failure: &EisOperationError) -> String {
    format!(
        "{}; eis_established={}; reset_cached_session={}",
        failure.error.message,
        failure.established,
        should_reset_session_before_legacy_fallback(failure)
    )
}

#[cfg(test)]
mod tests {
    use sky_cua_platform::diagnostics::{BackendError, BackendErrorCode};

    use super::{
        EIS_POINT_OUTSIDE_REGION_PREFIX, EisOperationError, EisScrollAxis, eis_fallback_details,
        eis_scroll_delta_from_action, eis_scroll_steps_from_action, should_fallback_to_legacy,
        should_reset_session_before_legacy_fallback,
    };

    #[test]
    fn maps_action_scroll_direction_to_eis_direction() {
        assert_eq!(
            eis_scroll_delta_from_action(EisScrollAxis::Vertical, -180.0),
            180.0
        );
        assert_eq!(
            eis_scroll_delta_from_action(EisScrollAxis::Vertical, 120.0),
            -120.0
        );
        assert_eq!(eis_scroll_steps_from_action(EisScrollAxis::Vertical, -2), 2);
        assert_eq!(eis_scroll_steps_from_action(EisScrollAxis::Vertical, 3), -3);
        assert_eq!(
            eis_scroll_delta_from_action(EisScrollAxis::Horizontal, -180.0),
            -180.0
        );
        assert_eq!(
            eis_scroll_delta_from_action(EisScrollAxis::Horizontal, 120.0),
            120.0
        );
        assert_eq!(
            eis_scroll_steps_from_action(EisScrollAxis::Horizontal, -2),
            -2
        );
        assert_eq!(
            eis_scroll_steps_from_action(EisScrollAxis::Horizontal, 3),
            3
        );
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
    fn arbitrary_pointer_invalid_request_fallbacks_to_legacy() {
        let failure = EisOperationError {
            error: BackendError::new(BackendErrorCode::InvalidRequest, "bad pointer request"),
            established: true,
        };

        assert!(should_fallback_to_legacy(&failure));
    }
}
