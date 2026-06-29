use std::ffi::CString;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};

use crate::protocol::{
    HelperCapabilities, HelperCommand, HelperRequest, HelperResponse, KeyEventCommand,
    PROTOCOL_VERSION, PointerAction, parse_request_line, response_line,
};
use crate::uinput::{DesktopBounds, UinputKeyboardDevice, UinputPointerDevice};

#[derive(Debug, Clone)]
pub struct ServerOptions {
    pub socket_path: PathBuf,
    pub socket_mode: u32,
    pub socket_group: Option<String>,
}

pub fn run_server(options: ServerOptions) -> Result<()> {
    if let Some(parent) = options.socket_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create socket directory {}", parent.display()))?;
    }
    if options.socket_path.exists() {
        fs::remove_file(&options.socket_path).with_context(|| {
            format!(
                "failed to remove stale helper socket {}",
                options.socket_path.display()
            )
        })?;
    }

    let keyboard = UinputKeyboardDevice::create().context("failed to create uinput keyboard")?;
    let state = Arc::new(Mutex::new(HelperState {
        keyboard,
        pointer: None,
    }));
    let listener = UnixListener::bind(&options.socket_path)
        .with_context(|| format!("failed to bind {}", options.socket_path.display()))?;
    configure_socket_file(&options)?;
    listener
        .set_nonblocking(true)
        .context("failed to configure helper socket as nonblocking")?;

    let mut accept_backoff = ACCEPT_ERROR_BACKOFF_MIN;
    loop {
        match listener.accept() {
            Ok((stream, _addr)) => {
                accept_backoff = ACCEPT_ERROR_BACKOFF_MIN;
                let state = Arc::clone(&state);
                thread::spawn(move || {
                    let _ = handle_stream(stream, state);
                });
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(25));
            }
            Err(error) => {
                // The helper owns the only uinput keyboard/pointer devices, so a
                // transient accept error (EINTR, ECONNABORTED, EMFILE, ...) must
                // not take it down. Log and keep serving. A per-connection error
                // clears on the next Ok; a sustained one (e.g. fd exhaustion)
                // returns immediately every iteration, so back off exponentially
                // (reset only on a successful accept) to cap the journal noise
                // without abandoning the devices.
                eprintln!("sky-cua-input-helper: accept error (continuing): {error}");
                thread::sleep(accept_backoff);
                accept_backoff = (accept_backoff * 2).min(ACCEPT_ERROR_BACKOFF_MAX);
            }
        }
    }
}

/// Initial / reset backoff after an `accept()` error, doubled on each
/// consecutive failure up to [`ACCEPT_ERROR_BACKOFF_MAX`].
const ACCEPT_ERROR_BACKOFF_MIN: Duration = Duration::from_millis(100);
const ACCEPT_ERROR_BACKOFF_MAX: Duration = Duration::from_secs(5);

fn configure_socket_file(options: &ServerOptions) -> Result<()> {
    if let Some(group) = options.socket_group.as_deref() {
        let gid = group_id(group)?;
        let path = CString::new(options.socket_path.as_os_str().as_encoded_bytes())
            .map_err(|_| anyhow!("socket path contains an interior NUL"))?;
        let rc = unsafe { libc::chown(path.as_ptr(), u32::MAX, gid) };
        if rc != 0 {
            return Err(std::io::Error::last_os_error())
                .with_context(|| format!("failed to chown socket to group {group}"));
        }
    }
    fs::set_permissions(
        &options.socket_path,
        fs::Permissions::from_mode(options.socket_mode),
    )
    .with_context(|| {
        format!(
            "failed to chmod helper socket {}",
            options.socket_path.display()
        )
    })
}

fn group_id(group: &str) -> Result<u32> {
    let group = CString::new(group).map_err(|_| anyhow!("group contains an interior NUL"))?;
    let entry = unsafe { libc::getgrnam(group.as_ptr()) };
    if entry.is_null() {
        return Err(anyhow!("group not found"));
    }
    Ok(unsafe { (*entry).gr_gid })
}

fn handle_stream(stream: UnixStream, state: Arc<Mutex<HelperState>>) -> Result<()> {
    let mut writer = stream
        .try_clone()
        .context("failed to clone helper stream")?;
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    loop {
        line.clear();
        if reader.read_line(&mut line)? == 0 {
            return Ok(());
        }
        let response = match parse_request_line(line.trim_end()) {
            Ok(request) if matches!(request.command, HelperCommand::ObservePointer { .. }) => {
                return handle_observe_pointer_request(request, writer);
            }
            Ok(request) => handle_request(request, &state),
            Err(error) => HelperResponse::error("invalid_json", error.to_string()),
        };
        writer.write_all(response_line(&response)?.as_bytes())?;
        writer.flush()?;
    }
}

fn handle_observe_pointer_request(request: HelperRequest, mut writer: UnixStream) -> Result<()> {
    if request.version != PROTOCOL_VERSION {
        writer.write_all(
            response_line(&HelperResponse::error(
                "unsupported_version",
                format!(
                    "unsupported protocol version {}; expected {PROTOCOL_VERSION}",
                    request.version
                ),
            ))?
            .as_bytes(),
        )?;
        writer.flush()?;
        return Ok(());
    }
    let HelperCommand::ObservePointer { bounds } = request.command else {
        return Ok(());
    };
    writer.write_all(response_line(&HelperResponse::ok())?.as_bytes())?;
    writer.flush()?;
    crate::observe::observe_pointer(writer, bounds)
}

struct HelperState {
    keyboard: UinputKeyboardDevice,
    pointer: Option<UinputPointerDevice>,
}

fn handle_request(request: HelperRequest, state: &Arc<Mutex<HelperState>>) -> HelperResponse {
    if request.version != PROTOCOL_VERSION {
        return HelperResponse::error(
            "unsupported_version",
            format!(
                "unsupported protocol version {}; expected {PROTOCOL_VERSION}",
                request.version
            ),
        );
    }

    // The `KeyEvents` / `PointerActions` arms below hold the state lock for the
    // whole batch, including the `Settle` / key-pacing sleeps, on purpose: an
    // input batch (e.g. a drag's press → interpolated moves → release) must be
    // atomic so a concurrent request cannot interleave events into the middle of
    // it. Releasing the lock around the sleeps would reintroduce that race.
    // Concurrent input requests therefore serialize here, which is the intended
    // behavior; the lock-free `observe_pointer` stream path is never blocked.
    let result = match request.command {
        HelperCommand::Hello => return HelperResponse::capabilities(HelperCapabilities::default()),
        HelperCommand::ObservePointer { .. } => {
            return HelperResponse::error(
                "stream_required",
                "observe_pointer must be handled by the streaming request path",
            );
        }
        HelperCommand::KeyEvents { events } => {
            let mut state = state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            emit_key_events(&mut state.keyboard, &events)
        }
        HelperCommand::PointerActions { bounds, actions } => {
            let mut state = state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            apply_pointer_actions(&mut state, bounds, &actions)
        }
    };

    match result {
        Ok(()) => HelperResponse::ok(),
        Err(error) => HelperResponse::error("uinput_error", error.to_string()),
    }
}

fn apply_pointer_actions(
    state: &mut HelperState,
    bounds: DesktopBounds,
    actions: &[PointerAction],
) -> crate::uinput::Result<()> {
    if state
        .pointer
        .as_ref()
        .is_none_or(|device| device.bounds() != bounds)
    {
        state.pointer = Some(UinputPointerDevice::create(bounds)?);
    }
    let pointer = state
        .pointer
        .as_mut()
        .expect("pointer device was just created");
    for action in actions {
        match *action {
            PointerAction::MoveAbsolute { x, y } => pointer.move_absolute(x, y)?,
            PointerAction::Button { button, pressed } => pointer.button(button, pressed)?,
            PointerAction::ScrollVertical { steps } => pointer.scroll_vertical(steps)?,
            PointerAction::Settle { millis } => {
                thread::sleep(Duration::from_millis(u64::from(millis)));
            }
        }
    }
    Ok(())
}

/// Pause inserted after every emitted key event so a multi-character batch does
/// not flood the device faster than evdev consumers drain it.
///
/// uinput key events written back-to-back land in the same instant. A long
/// `type_text` is dozens of input events once each press/release carries its
/// own `SYN_REPORT`, which can overrun the kernel's per-client evdev ring
/// buffer (~64 packets) and make a consumer drop part of the burst on
/// `SYN_DROPPED`. Pacing each event lets libinput drain between writes and gives
/// every keystroke a real hold duration, matching the portal EIS keyboard path
/// (which paces per-key hold plus an inter-character gap for reliable virtual
/// keyboard delivery) and ydotool's default key delay.
const KEY_EVENT_PACING: Duration = Duration::from_millis(6);

fn emit_key_events(
    keyboard: &mut UinputKeyboardDevice,
    events: &[KeyEventCommand],
) -> crate::uinput::Result<()> {
    for (index, event) in events.iter().enumerate() {
        keyboard.key_event(event.code, event.pressed)?;
        if index + 1 < events.len() {
            thread::sleep(KEY_EVENT_PACING);
        }
    }
    Ok(())
}
