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
    PROTOCOL_VERSION, parse_request_line, response_line,
};
use crate::uinput::UinputKeyboardDevice;

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
    let state = Arc::new(Mutex::new(HelperState { keyboard }));
    let listener = UnixListener::bind(&options.socket_path)
        .with_context(|| format!("failed to bind {}", options.socket_path.display()))?;
    configure_socket_file(&options)?;
    listener
        .set_nonblocking(true)
        .context("failed to configure helper socket as nonblocking")?;

    loop {
        match listener.accept() {
            Ok((stream, _addr)) => {
                let state = Arc::clone(&state);
                thread::spawn(move || {
                    let _ = handle_stream(stream, state);
                });
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(25));
            }
            Err(error) => return Err(error).context("failed to accept helper connection"),
        }
    }
}

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
    };

    match result {
        Ok(()) => HelperResponse::ok(),
        Err(error) => HelperResponse::error("uinput_error", error.to_string()),
    }
}

fn emit_key_events(
    keyboard: &mut UinputKeyboardDevice,
    events: &[KeyEventCommand],
) -> crate::uinput::Result<()> {
    for event in events {
        keyboard.key_event(event.code, event.pressed)?;
    }
    Ok(())
}
