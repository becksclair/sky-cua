use std::collections::HashMap;
use std::env;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::FileTypeExt;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use sky_cua_input_helper::protocol::{
    HelperCommand, KeyEventCommand, parse_response_line, request_line,
};
use sky_cua_platform::diagnostics::{BackendError, BackendErrorCode};
use xkbcommon::xkb;

use crate::portal::eis_keymap::{
    EisKeyStroke, build_keysym_cache, clear_modifiers_already_present_in_chord,
    find_keycodes_from_cache, keysym_for_char, keysym_for_key_name, required_modifier_keycodes,
    resolve_eis_keystroke,
};
use crate::portal::remote_desktop::MouseButton;

const COMMAND_TIMEOUT: Duration = Duration::from_secs(3);
const HELPER_COMMAND_TIMEOUT: Duration = Duration::from_secs(3);
const DEFAULT_HELPER_SOCKET_PATH: &str = "/run/sky-cua/input-helper.sock";
const SKY_CUA_INPUT_HELPER_SOCKET: &str = "SKY_CUA_INPUT_HELPER_SOCKET";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VirtualInputAdapterKind {
    PrivilegedHelper,
    Ydotool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VirtualInputCoordinatePlane {
    DesktopLogical,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VirtualInputProbe {
    pub adapter: VirtualInputAdapterKind,
    pub coordinate_plane: VirtualInputCoordinatePlane,
    pub ydotool_path: Option<PathBuf>,
    pub socket_path: Option<PathBuf>,
    pub helper_socket_path: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VirtualInputUnavailable {
    pub reason: String,
    pub ydotool_path: Option<PathBuf>,
    pub socket_path: Option<PathBuf>,
    pub helper_socket_path: Option<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct LinuxVirtualInput {
    probe: VirtualInputProbe,
}

impl LinuxVirtualInput {
    pub fn new() -> Result<Self, BackendError> {
        let probe = probe_virtual_input().map_err(|unavailable| {
            BackendError::new(
                BackendErrorCode::ActionUnsupportedForEnvironment,
                format!("Linux virtual input is unavailable: {}", unavailable.reason),
            )
        })?;
        Ok(Self { probe })
    }

    pub fn move_absolute(&self, x: f64, y: f64) -> Result<(), BackendError> {
        match self.probe.adapter {
            VirtualInputAdapterKind::PrivilegedHelper => Err(pointer_requires_ydotool_error()),
            VirtualInputAdapterKind::Ydotool => self.run_ydotool(move_absolute_args(x, y)),
        }
    }

    pub fn click(&self, button: MouseButton) -> Result<(), BackendError> {
        match self.probe.adapter {
            VirtualInputAdapterKind::PrivilegedHelper => Err(pointer_requires_ydotool_error()),
            VirtualInputAdapterKind::Ydotool => {
                self.run_ydotool(["click".to_string(), click_code(button, ClickAction::Click)])
            }
        }
    }

    pub fn pointer_button(&self, button: MouseButton, pressed: bool) -> Result<(), BackendError> {
        match self.probe.adapter {
            VirtualInputAdapterKind::PrivilegedHelper => Err(pointer_requires_ydotool_error()),
            VirtualInputAdapterKind::Ydotool => self.run_ydotool([
                "click".to_string(),
                click_code(
                    button,
                    if pressed {
                        ClickAction::Down
                    } else {
                        ClickAction::Up
                    },
                ),
            ]),
        }
    }

    pub fn click_at(&self, x: f64, y: f64, button: MouseButton) -> Result<(), BackendError> {
        match self.probe.adapter {
            VirtualInputAdapterKind::PrivilegedHelper => Err(pointer_requires_ydotool_error()),
            VirtualInputAdapterKind::Ydotool => {
                self.move_absolute(x, y)?;
                self.click(button)
            }
        }
    }

    pub fn pointer_mapping_details(&self, x: f64, y: f64) -> String {
        match self.probe.adapter {
            VirtualInputAdapterKind::PrivilegedHelper => {
                let socket = self
                    .probe
                    .helper_socket_path
                    .as_ref()
                    .map(|path| path.display().to_string())
                    .unwrap_or_else(|| "unknown".to_string());
                format!(
                    "adapter=privileged_helper socket={socket} coordinate_plane=desktop_logical requested=({x:.1},{y:.1}) pointer=unsupported"
                )
            }
            VirtualInputAdapterKind::Ydotool => format!(
                "adapter=ydotool coordinate_plane=desktop_logical requested=({x:.1},{y:.1}) emitted_absolute=({},{})",
                round_coordinate(x),
                round_coordinate(y)
            ),
        }
    }

    pub fn drag(&self, from: (f64, f64), to: (f64, f64)) -> Result<(), BackendError> {
        match self.probe.adapter {
            VirtualInputAdapterKind::PrivilegedHelper => Err(pointer_requires_ydotool_error()),
            VirtualInputAdapterKind::Ydotool => {
                self.move_absolute(from.0, from.1)?;
                self.pointer_button(MouseButton::Left, true)?;
                thread::sleep(Duration::from_millis(40));
                let result = self.move_absolute(to.0, to.1);
                if result.is_err() {
                    let _ = self.pointer_button(MouseButton::Left, false);
                }
                result?;
                thread::sleep(Duration::from_millis(40));
                self.pointer_button(MouseButton::Left, false)
            }
        }
    }

    pub fn scroll_vertical(&self, steps: i32) -> Result<(), BackendError> {
        if steps == 0 {
            return Ok(());
        }
        match self.probe.adapter {
            VirtualInputAdapterKind::PrivilegedHelper => Err(pointer_requires_ydotool_error()),
            VirtualInputAdapterKind::Ydotool => self.run_ydotool(scroll_vertical_args(steps)),
        }
    }

    pub fn scroll_vertical_at(&self, x: f64, y: f64, steps: i32) -> Result<(), BackendError> {
        match self.probe.adapter {
            VirtualInputAdapterKind::PrivilegedHelper => Err(pointer_requires_ydotool_error()),
            VirtualInputAdapterKind::Ydotool => {
                self.move_absolute(x, y)?;
                self.scroll_vertical(steps)
            }
        }
    }

    pub fn type_text(&self, text: &str) -> Result<(), BackendError> {
        if let Some(socket_path) = self.probe.helper_socket_path.as_deref()
            && socket_is_connectable(socket_path)
        {
            match LinuxKeyResolver::from_environment()
                .and_then(|resolver| resolver.text_events(text))
                .and_then(|events| {
                    run_helper_command(socket_path, HelperCommand::KeyEvents { events })
                }) {
                Ok(()) => return Ok(()),
                Err(error)
                    if self.probe.adapter != VirtualInputAdapterKind::Ydotool
                        || !self.probe.supports_keyboard() =>
                {
                    return Err(error);
                }
                Err(_) => {}
            }
        }
        match self.probe.adapter {
            VirtualInputAdapterKind::PrivilegedHelper => Err(self.missing_helper_error()),
            VirtualInputAdapterKind::Ydotool => {
                self.require_keyboard_adapter()?;
                self.run_ydotool(type_text_args(text))
            }
        }
    }

    pub fn press_key_sequence(&self, keys: &[String]) -> Result<(), BackendError> {
        if let Some(socket_path) = self.probe.helper_socket_path.as_deref()
            && socket_is_connectable(socket_path)
        {
            match LinuxKeyResolver::from_environment()
                .and_then(|resolver| resolver.key_sequence_events(keys))
                .and_then(|events| {
                    run_helper_command(socket_path, HelperCommand::KeyEvents { events })
                }) {
                Ok(()) => return Ok(()),
                Err(error)
                    if self.probe.adapter != VirtualInputAdapterKind::Ydotool
                        || !self.probe.supports_keyboard() =>
                {
                    return Err(error);
                }
                Err(_) => {}
            }
        }
        match self.probe.adapter {
            VirtualInputAdapterKind::PrivilegedHelper => Err(self.missing_helper_error()),
            VirtualInputAdapterKind::Ydotool => {
                self.require_keyboard_adapter()?;
                let events = key_sequence_events(keys)?;
                let mut args = vec!["key".to_string()];
                args.extend(events);
                self.run_ydotool(args)
            }
        }
    }

    fn run_ydotool<I>(&self, args: I) -> Result<(), BackendError>
    where
        I: IntoIterator<Item = String>,
    {
        run_ydotool_command(&self.probe, args)
    }

    fn helper_socket_path(&self) -> Result<&Path, BackendError> {
        self.probe.helper_socket_path.as_deref().ok_or_else(|| {
            BackendError::new(
                BackendErrorCode::ActionUnsupportedForEnvironment,
                "Linux privileged input helper socket path is missing",
            )
        })
    }

    fn missing_helper_error(&self) -> BackendError {
        self.helper_socket_path().err().unwrap_or_else(|| {
            BackendError::new(
                BackendErrorCode::ActionUnsupportedForEnvironment,
                "Linux privileged input helper socket is not connectable",
            )
        })
    }

    fn require_keyboard_adapter(&self) -> Result<(), BackendError> {
        if self.probe.supports_keyboard() {
            return Ok(());
        }
        let socket_detail = self
            .probe
            .socket_path
            .as_ref()
            .map(|path| format!(" socket={}", path.display()))
            .unwrap_or_default();
        Err(BackendError::new(
            BackendErrorCode::ActionUnsupportedForEnvironment,
            format!(
                "Linux virtual input keyboard actions require the privileged input helper or a usable ydotool daemon.{socket_detail}"
            ),
        ))
    }
}

pub fn probe_virtual_input() -> Result<VirtualInputProbe, VirtualInputUnavailable> {
    let helper_socket_path = configured_helper_socket_path();
    let helper_available = helper_is_available(&helper_socket_path);
    let ydotool_path = find_executable("ydotool");
    let socket_path = configured_socket_path();

    let Some(ydotool_path) = ydotool_path else {
        if helper_available {
            return Ok(helper_keyboard_only_probe(
                helper_socket_path,
                None,
                socket_path,
            ));
        }
        return Err(VirtualInputUnavailable {
            reason: "ydotool executable was not found on PATH and the privileged input helper is unavailable".to_string(),
            ydotool_path: None,
            socket_path,
            helper_socket_path: Some(helper_socket_path),
        });
    };

    if let Some(path) = socket_path.as_ref() {
        if !path.exists() {
            if helper_available {
                return Ok(helper_keyboard_only_probe(
                    helper_socket_path,
                    Some(ydotool_path),
                    socket_path,
                ));
            }
            return Err(VirtualInputUnavailable {
                reason: format!("ydotool socket does not exist: {}", path.display()),
                ydotool_path: Some(ydotool_path),
                socket_path,
                helper_socket_path: Some(helper_socket_path),
            });
        }
        if !socket_is_connectable(path) {
            if helper_available {
                return Ok(helper_keyboard_only_probe(
                    helper_socket_path,
                    Some(ydotool_path),
                    socket_path,
                ));
            }
            return Err(VirtualInputUnavailable {
                reason: format!(
                    "ydotool socket path is not a Unix socket: {}",
                    path.display()
                ),
                ydotool_path: Some(ydotool_path),
                socket_path,
                helper_socket_path: Some(helper_socket_path),
            });
        }
    }

    Ok(VirtualInputProbe {
        adapter: VirtualInputAdapterKind::Ydotool,
        coordinate_plane: VirtualInputCoordinatePlane::DesktopLogical,
        ydotool_path: Some(ydotool_path),
        socket_path,
        helper_socket_path: Some(helper_socket_path),
    })
}

fn helper_keyboard_only_probe(
    helper_socket_path: PathBuf,
    ydotool_path: Option<PathBuf>,
    socket_path: Option<PathBuf>,
) -> VirtualInputProbe {
    VirtualInputProbe {
        adapter: VirtualInputAdapterKind::PrivilegedHelper,
        coordinate_plane: VirtualInputCoordinatePlane::DesktopLogical,
        ydotool_path,
        socket_path,
        helper_socket_path: Some(helper_socket_path),
    }
}

pub fn virtual_input_keyboard_available() -> bool {
    probe_virtual_input().is_ok_and(|probe| probe.supports_keyboard())
}

impl VirtualInputProbe {
    pub fn supports_keyboard(&self) -> bool {
        match self.adapter {
            VirtualInputAdapterKind::PrivilegedHelper => true,
            VirtualInputAdapterKind::Ydotool => {
                self.ydotool_path.is_some()
                    && self
                        .socket_path
                        .as_ref()
                        .is_none_or(|path| socket_is_connectable(path))
            }
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum ClickAction {
    Click,
    Down,
    Up,
}

fn click_code(button: MouseButton, action: ClickAction) -> String {
    let base = match button {
        MouseButton::Left => 0x00,
        MouseButton::Right => 0x01,
        MouseButton::Middle => 0x02,
    };
    let mask = match action {
        ClickAction::Click => 0xC0,
        ClickAction::Down => 0x40,
        ClickAction::Up => 0x80,
    };
    format!("0x{:02X}", base | mask)
}

fn pointer_requires_ydotool_error() -> BackendError {
    BackendError::new(
        BackendErrorCode::ActionUnsupportedForEnvironment,
        "Linux virtual input pointer actions require ydotool; uinput pointer injection has been removed",
    )
}

fn run_helper_command(path: &Path, command: HelperCommand) -> Result<(), BackendError> {
    let mut stream = UnixStream::connect(path).map_err(|error| {
        BackendError::new(
            BackendErrorCode::ActionUnsupportedForEnvironment,
            format!(
                "failed to connect to privileged input helper {}: {error}",
                path.display()
            ),
        )
    })?;
    stream
        .set_read_timeout(Some(HELPER_COMMAND_TIMEOUT))
        .map_err(|error| helper_io_error("configure helper read timeout", error))?;
    stream
        .set_write_timeout(Some(HELPER_COMMAND_TIMEOUT))
        .map_err(|error| helper_io_error("configure helper write timeout", error))?;
    let line = request_line(command).map_err(|error| {
        BackendError::new(
            BackendErrorCode::Internal,
            format!("failed to encode privileged input helper request: {error}"),
        )
    })?;
    stream
        .write_all(line.as_bytes())
        .map_err(|error| helper_io_error("write helper request", error))?;
    let mut reader = BufReader::new(stream);
    let mut response = String::new();
    reader
        .read_line(&mut response)
        .map_err(|error| helper_io_error("read helper response", error))?;
    let response = parse_response_line(response.trim_end()).map_err(|error| {
        BackendError::new(
            BackendErrorCode::ActionUnsupportedForEnvironment,
            format!("failed to parse privileged input helper response: {error}"),
        )
    })?;
    if response.ok {
        Ok(())
    } else {
        let message = response
            .error
            .map(|error| format!("{}: {}", error.code, error.message))
            .unwrap_or_else(|| "helper returned an unknown error".to_string());
        Err(BackendError::new(
            BackendErrorCode::ActionUnsupportedForEnvironment,
            format!("privileged input helper failed: {message}"),
        ))
    }
}

fn helper_io_error(context: &str, error: std::io::Error) -> BackendError {
    BackendError::new(
        BackendErrorCode::ActionUnsupportedForEnvironment,
        format!("failed to {context}: {error}"),
    )
}

fn helper_is_available(path: &Path) -> bool {
    if !socket_is_connectable(path) {
        return false;
    }
    run_helper_command(path, HelperCommand::Hello).is_ok()
}

#[derive(Debug, Clone)]
struct LinuxKeyResolver {
    keysym_cache: HashMap<u32, EisKeyStroke>,
    shift_keycodes: Vec<u32>,
    level3_keycodes: Vec<u32>,
}

impl LinuxKeyResolver {
    fn from_environment() -> Result<Self, BackendError> {
        let context = xkb::Context::new(0);
        let names = XkbNames::from_environment();
        let keymap = xkb::Keymap::new_from_names(
            &context,
            &names.rules,
            &names.model,
            &names.layout,
            &names.variant,
            Some(names.options),
            xkb::KEYMAP_COMPILE_NO_FLAGS,
        )
        .ok_or_else(|| {
            BackendError::new(
                BackendErrorCode::ActionUnsupportedForEnvironment,
                format!(
                    "failed to compile XKB keymap rules={:?} model={:?} layout={:?} variant={:?}",
                    names.rules, names.model, names.layout, names.variant
                ),
            )
        })?;
        let keysym_cache = build_keysym_cache(&keymap);
        let shift_keycodes = find_keycodes_from_cache(
            &keysym_cache,
            &[xkb::keysyms::KEY_Shift_L, xkb::keysyms::KEY_Shift_R],
        );
        let level3_keycodes = find_keycodes_from_cache(
            &keysym_cache,
            &[
                xkb::keysyms::KEY_ISO_Level3_Shift,
                xkb::keysyms::KEY_Mode_switch,
            ],
        );
        Ok(Self {
            keysym_cache,
            shift_keycodes,
            level3_keycodes,
        })
    }

    fn text_events(&self, text: &str) -> Result<Vec<KeyEventCommand>, BackendError> {
        let mut events = Vec::with_capacity(text.len().saturating_mul(4));
        for character in text.chars() {
            let keysym = keysym_for_char(character).ok_or_else(|| {
                BackendError::new(
                    BackendErrorCode::InvalidRequest,
                    format!("cannot type unsupported character {character:?} through uinput"),
                )
            })?;
            let stroke = resolve_eis_keystroke(&self.keysym_cache, keysym)?;
            self.push_key_stroke(&mut events, stroke)?;
        }
        Ok(events)
    }

    fn key_sequence_events(&self, keys: &[String]) -> Result<Vec<KeyEventCommand>, BackendError> {
        if keys.is_empty() {
            return Err(BackendError::new(
                BackendErrorCode::InvalidRequest,
                "press_key requires at least one key",
            ));
        }
        let mut resolved = Vec::with_capacity(keys.len());
        for key in keys {
            let keysym = keysym_for_key_name(key).ok_or_else(|| {
                BackendError::new(
                    BackendErrorCode::InvalidRequest,
                    format!("unsupported key name {key:?}"),
                )
            })?;
            resolved.push(resolve_eis_keystroke(&self.keysym_cache, keysym)?);
        }

        let mut events = Vec::with_capacity(resolved.len() * 4);
        if resolved.len() == 1 {
            self.push_key_stroke(&mut events, resolved[0])?;
            return Ok(events);
        }

        clear_modifiers_already_present_in_chord(
            &mut resolved,
            &self.shift_keycodes,
            &self.level3_keycodes,
        );
        for stroke in &resolved[..resolved.len() - 1] {
            self.push_key_state(&mut events, *stroke, true)?;
        }
        self.push_key_stroke(&mut events, *resolved.last().expect("chord has a last key"))?;
        for stroke in resolved[..resolved.len() - 1].iter().rev() {
            self.push_key_state(&mut events, *stroke, false)?;
        }
        Ok(events)
    }

    fn push_key_stroke(
        &self,
        events: &mut Vec<KeyEventCommand>,
        stroke: EisKeyStroke,
    ) -> Result<(), BackendError> {
        self.push_key_state(events, stroke, true)?;
        self.push_key_state(events, stroke, false)
    }

    fn push_key_state(
        &self,
        events: &mut Vec<KeyEventCommand>,
        stroke: EisKeyStroke,
        pressed: bool,
    ) -> Result<(), BackendError> {
        let modifier_keycodes =
            required_modifier_keycodes(stroke, &self.shift_keycodes, &self.level3_keycodes)?;
        if pressed {
            for keycode in &modifier_keycodes {
                events.push(key_event(*keycode, true)?);
            }
            events.push(key_event(stroke.keycode, true)?);
        } else {
            events.push(key_event(stroke.keycode, false)?);
            for keycode in modifier_keycodes.iter().rev() {
                events.push(key_event(*keycode, false)?);
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct XkbNames {
    rules: String,
    model: String,
    layout: String,
    variant: String,
    options: String,
}

impl XkbNames {
    fn from_environment() -> Self {
        let probed = probe_xkb_names();
        Self {
            rules: xkb_env("RULES")
                .or_else(|| {
                    probed
                        .as_ref()
                        .and_then(|names| non_empty_string(&names.rules))
                })
                .unwrap_or_else(|| "evdev".to_string()),
            model: xkb_env("MODEL")
                .or_else(|| {
                    probed
                        .as_ref()
                        .and_then(|names| non_empty_string(&names.model))
                })
                .unwrap_or_else(|| "pc105".to_string()),
            layout: xkb_env("LAYOUT")
                .or_else(|| {
                    probed
                        .as_ref()
                        .and_then(|names| non_empty_string(&names.layout))
                })
                .unwrap_or_else(|| "us".to_string()),
            variant: xkb_env("VARIANT")
                .or_else(|| {
                    probed
                        .as_ref()
                        .and_then(|names| non_empty_string(&names.variant))
                })
                .unwrap_or_default(),
            options: xkb_env("OPTIONS")
                .or_else(|| {
                    probed
                        .as_ref()
                        .and_then(|names| non_empty_string(&names.options))
                })
                .unwrap_or_default(),
        }
    }
}

fn xkb_env(suffix: &str) -> Option<String> {
    env::var(format!("SKY_CUA_XKB_{suffix}"))
        .ok()
        .and_then(|value| non_empty_string(&value))
        .or_else(|| {
            env::var(format!("XKB_DEFAULT_{suffix}"))
                .ok()
                .and_then(|value| non_empty_string(&value))
        })
}

fn probe_xkb_names() -> Option<XkbNames> {
    probe_setxkbmap_names().or_else(probe_localectl_xkb_names)
}

fn probe_setxkbmap_names() -> Option<XkbNames> {
    let output = command_output_with_timeout(
        Command::new("setxkbmap")
            .arg("-query")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null()),
        COMMAND_TIMEOUT,
    )?;
    output
        .status
        .success()
        .then(|| parse_setxkbmap_query(&String::from_utf8_lossy(&output.stdout)))
        .flatten()
}

fn probe_localectl_xkb_names() -> Option<XkbNames> {
    let output = command_output_with_timeout(
        Command::new("localectl")
            .arg("status")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null()),
        COMMAND_TIMEOUT,
    )?;
    output
        .status
        .success()
        .then(|| parse_localectl_status(&String::from_utf8_lossy(&output.stdout)))
        .flatten()
}

fn parse_setxkbmap_query(output: &str) -> Option<XkbNames> {
    let mut names = XkbNames::empty();
    for line in output.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim().to_string();
        match key.trim() {
            "rules" => names.rules = value,
            "model" => names.model = value,
            "layout" => names.layout = value,
            "variant" => names.variant = value,
            "options" => names.options = value,
            _ => {}
        }
    }
    names.has_any_value().then_some(names)
}

fn parse_localectl_status(output: &str) -> Option<XkbNames> {
    let mut names = XkbNames::empty();
    for line in output.lines() {
        let Some((key, value)) = line.trim().split_once(':') else {
            continue;
        };
        let value = value.trim().to_string();
        match key.trim() {
            "X11 Model" => names.model = value,
            "X11 Layout" => names.layout = value,
            "X11 Variant" => names.variant = value,
            "X11 Options" => names.options = value,
            _ => {}
        }
    }
    names.has_any_value().then_some(names)
}

impl XkbNames {
    fn empty() -> Self {
        Self {
            rules: String::new(),
            model: String::new(),
            layout: String::new(),
            variant: String::new(),
            options: String::new(),
        }
    }

    fn has_any_value(&self) -> bool {
        [
            &self.rules,
            &self.model,
            &self.layout,
            &self.variant,
            &self.options,
        ]
        .iter()
        .any(|value| !value.trim().is_empty())
    }
}

fn non_empty_string(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

fn key_event(keycode: u32, pressed: bool) -> Result<KeyEventCommand, BackendError> {
    let code = u16::try_from(keycode).map_err(|_| {
        BackendError::new(
            BackendErrorCode::ActionUnsupportedForEnvironment,
            format!("uinput keycode {keycode} is outside the supported range"),
        )
    })?;
    Ok(KeyEventCommand { code, pressed })
}

fn key_sequence_events(keys: &[String]) -> Result<Vec<String>, BackendError> {
    let codes: Vec<u16> = keys
        .iter()
        .map(|key| key_code(key).ok_or_else(|| unsupported_key_error(key)))
        .collect::<Result<_, _>>()?;
    let mut events = Vec::with_capacity(codes.len() * 2);
    for code in &codes {
        events.push(format!("{code}:1"));
    }
    for code in codes.iter().rev() {
        events.push(format!("{code}:0"));
    }
    Ok(events)
}

fn move_absolute_args(x: f64, y: f64) -> Vec<String> {
    vec![
        "mousemove".to_string(),
        "--absolute".to_string(),
        "-x".to_string(),
        round_coordinate(x),
        "-y".to_string(),
        round_coordinate(y),
    ]
}

fn scroll_vertical_args(steps: i32) -> Vec<String> {
    vec![
        "mousemove".to_string(),
        "--wheel".to_string(),
        "--".to_string(),
        "0".to_string(),
        steps.to_string(),
    ]
}

fn type_text_args(text: &str) -> Vec<String> {
    vec![
        "type".to_string(),
        "--key-delay=20".to_string(),
        "--key-hold=20".to_string(),
        "--".to_string(),
        text.to_string(),
    ]
}

fn unsupported_key_error(key: &str) -> BackendError {
    BackendError::new(
        BackendErrorCode::ActionUnsupportedForEnvironment,
        format!("Linux virtual input does not know how to press key {key:?}"),
    )
}

fn key_code(key: &str) -> Option<u16> {
    let normalized = key.trim().to_ascii_lowercase();
    Some(match normalized.as_str() {
        "ctrl" | "control" => 29,
        "alt" => 56,
        "shift" => 42,
        "super" | "meta" | "win" | "windows" => 125,
        "enter" | "return" => 28,
        "escape" | "esc" => 1,
        "tab" => 15,
        "backspace" => 14,
        "delete" | "del" => 111,
        "space" => 57,
        "left" | "arrowleft" => 105,
        "right" | "arrowright" => 106,
        "up" | "arrowup" => 103,
        "down" | "arrowdown" => 108,
        "home" => 102,
        "end" => 107,
        "pageup" | "page_up" => 104,
        "pagedown" | "page_down" => 109,
        "a" => 30,
        "b" => 48,
        "c" => 46,
        "d" => 32,
        "e" => 18,
        "f" => 33,
        "g" => 34,
        "h" => 35,
        "i" => 23,
        "j" => 36,
        "k" => 37,
        "l" => 38,
        "m" => 50,
        "n" => 49,
        "o" => 24,
        "p" => 25,
        "q" => 16,
        "r" => 19,
        "s" => 31,
        "t" => 20,
        "u" => 22,
        "v" => 47,
        "w" => 17,
        "x" => 45,
        "y" => 21,
        "z" => 44,
        "0" => 11,
        "1" => 2,
        "2" => 3,
        "3" => 4,
        "4" => 5,
        "5" => 6,
        "6" => 7,
        "7" => 8,
        "8" => 9,
        "9" => 10,
        _ => return None,
    })
}

fn run_ydotool_command<I>(probe: &VirtualInputProbe, args: I) -> Result<(), BackendError>
where
    I: IntoIterator<Item = String>,
{
    let Some(ydotool_path) = probe.ydotool_path.as_ref() else {
        return Err(BackendError::new(
            BackendErrorCode::ActionUnsupportedForEnvironment,
            "ydotool adapter is unavailable for this Linux virtual input action",
        ));
    };
    let args: Vec<String> = args.into_iter().collect();
    let mut command = Command::new(ydotool_path);
    command
        .args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(socket_path) = probe.socket_path.as_ref() {
        command.env("YDOTOOL_SOCKET", socket_path);
    }
    let mut child = command.spawn().map_err(|error| {
        BackendError::new(
            BackendErrorCode::ActionUnsupportedForEnvironment,
            format!("failed to start ydotool: {error}"),
        )
    })?;

    let deadline = Instant::now() + COMMAND_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(_status)) => break,
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(10)),
            Ok(None) => {
                let _ = child.kill();
                return Err(BackendError::new(
                    BackendErrorCode::ActionUnsupportedForEnvironment,
                    format!("ydotool command timed out: ydotool {}", args.join(" ")),
                ));
            }
            Err(error) => {
                let _ = child.kill();
                return Err(BackendError::new(
                    BackendErrorCode::ActionUnsupportedForEnvironment,
                    format!("failed to wait for ydotool: {error}"),
                ));
            }
        }
    }

    let output = child.wait_with_output().map_err(|error| {
        BackendError::new(
            BackendErrorCode::ActionUnsupportedForEnvironment,
            format!("failed to collect ydotool output: {error}"),
        )
    })?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    Err(BackendError::new(
        BackendErrorCode::ActionUnsupportedForEnvironment,
        format!(
            "ydotool command failed with status {}: ydotool {}{}{}",
            output.status,
            args.join(" "),
            if stderr.is_empty() { "" } else { ": " },
            stderr
        ),
    ))
}

fn configured_socket_path() -> Option<PathBuf> {
    preferred_socket_path(
        env::var_os("YDOTOOL_SOCKET").map(PathBuf::from),
        socket_path_candidates(),
    )
}

fn configured_helper_socket_path() -> PathBuf {
    env::var_os(SKY_CUA_INPUT_HELPER_SOCKET)
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| PathBuf::from(DEFAULT_HELPER_SOCKET_PATH))
}

fn preferred_socket_path<I>(explicit: Option<PathBuf>, candidates: I) -> Option<PathBuf>
where
    I: IntoIterator<Item = PathBuf>,
{
    if let Some(path) = explicit.filter(|path| !path.as_os_str().is_empty()) {
        return Some(path);
    }
    let mut fallback = None;
    for path in candidates {
        if fallback.is_none() {
            fallback = Some(path.clone());
        }
        if socket_is_connectable(&path) {
            return Some(path);
        }
    }
    fallback
}

fn socket_path_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(runtime_socket) = env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .map(|runtime_dir| runtime_dir.join(".ydotool_socket"))
    {
        candidates.push(runtime_socket);
    }
    let uid = unsafe { libc::geteuid() };
    candidates.push(PathBuf::from(format!("/run/user/{uid}/.ydotool_socket")));
    candidates.push(PathBuf::from("/tmp/.ydotool_socket"));
    candidates
}

fn socket_is_connectable(path: &Path) -> bool {
    path.metadata()
        .is_ok_and(|metadata| metadata.file_type().is_socket())
}

fn command_output_with_timeout(command: &mut Command, timeout: Duration) -> Option<Output> {
    let mut child = command.spawn().ok()?;
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(_status)) => return child.wait_with_output().ok(),
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(10)),
            Ok(None) | Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
}

fn find_executable(name: &str) -> Option<PathBuf> {
    let path = env::var_os("PATH")?;
    env::split_paths(&path)
        .map(|dir| dir.join(name))
        .find(|candidate| candidate.is_file())
}

fn round_coordinate(value: f64) -> String {
    format!("{:.0}", value.round())
}

#[cfg(test)]
mod tests {
    use std::os::unix::net::UnixListener;
    use std::path::PathBuf;

    use super::{
        ClickAction, LinuxVirtualInput, VirtualInputAdapterKind, VirtualInputCoordinatePlane,
        VirtualInputProbe, click_code, helper_keyboard_only_probe, key_sequence_events,
        move_absolute_args, parse_localectl_status, parse_setxkbmap_query, preferred_socket_path,
        scroll_vertical_args, type_text_args,
    };
    use crate::portal::remote_desktop::MouseButton;

    #[test]
    fn builds_ydotool_click_codes() {
        assert_eq!(click_code(MouseButton::Left, ClickAction::Click), "0xC0");
        assert_eq!(click_code(MouseButton::Right, ClickAction::Click), "0xC1");
        assert_eq!(click_code(MouseButton::Left, ClickAction::Down), "0x40");
        assert_eq!(click_code(MouseButton::Left, ClickAction::Up), "0x80");
    }

    #[test]
    fn builds_key_sequence_events_with_reversed_release_order() {
        let events = key_sequence_events(&["Ctrl".to_string(), "L".to_string()]).unwrap();
        assert_eq!(events, vec!["29:1", "38:1", "38:0", "29:0"]);
    }

    #[test]
    fn builds_ydotool_pointer_and_text_argv_without_shell_escaping() {
        assert_eq!(
            move_absolute_args(10.4, 20.6),
            vec!["mousemove", "--absolute", "-x", "10", "-y", "21"]
        );
        assert_eq!(
            scroll_vertical_args(-2),
            vec!["mousemove", "--wheel", "--", "0", "-2"]
        );
        assert_eq!(
            type_text_args("--not-a-flag hello"),
            vec![
                "type",
                "--key-delay=20",
                "--key-hold=20",
                "--",
                "--not-a-flag hello"
            ]
        );
    }

    #[test]
    fn ydotool_socket_selection_uses_connectable_later_candidate() {
        let socket_dir = std::env::temp_dir().join(format!(
            "sky-cua-ydotool-socket-test-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&socket_dir).unwrap();
        let missing_runtime = socket_dir.join("missing-runtime.sock");
        let connectable_tmp = socket_dir.join("tmp.sock");
        let _listener = UnixListener::bind(&connectable_tmp).unwrap();

        assert_eq!(
            preferred_socket_path(None, [missing_runtime.clone(), connectable_tmp.clone()]),
            Some(connectable_tmp)
        );
        assert_eq!(
            preferred_socket_path(Some(PathBuf::from("/custom.sock")), [missing_runtime]),
            Some(PathBuf::from("/custom.sock"))
        );
        std::fs::remove_dir_all(socket_dir).unwrap();
    }

    #[test]
    fn helper_without_ydotool_is_keyboard_only() {
        let input = LinuxVirtualInput {
            probe: VirtualInputProbe {
                adapter: VirtualInputAdapterKind::PrivilegedHelper,
                coordinate_plane: VirtualInputCoordinatePlane::DesktopLogical,
                ydotool_path: None,
                socket_path: None,
                helper_socket_path: None,
            },
        };

        assert!(input.type_text("hello").is_err());
        assert!(input.probe.supports_keyboard());
    }

    #[test]
    fn helper_pointer_action_requires_ydotool() {
        let probe =
            helper_keyboard_only_probe(PathBuf::from("/run/sky-cua/input-helper.sock"), None, None);
        assert_eq!(probe.adapter, VirtualInputAdapterKind::PrivilegedHelper);
        assert!(probe.supports_keyboard());

        let input = LinuxVirtualInput { probe };
        let error = input
            .move_absolute(10.0, 20.0)
            .expect_err("helper should no longer inject pointer events");

        assert!(error.message.contains("ydotool"));
    }

    #[test]
    fn parses_setxkbmap_query_for_layout_probe() {
        let names = parse_setxkbmap_query(
            "rules:      evdev\nmodel:      pc105\nlayout:     es\nvariant:    nodeadkeys\noptions:    compose:ralt\n",
        )
        .unwrap();

        assert_eq!(names.rules, "evdev");
        assert_eq!(names.model, "pc105");
        assert_eq!(names.layout, "es");
        assert_eq!(names.variant, "nodeadkeys");
        assert_eq!(names.options, "compose:ralt");
    }

    #[test]
    fn parses_localectl_status_for_layout_probe() {
        let names = parse_localectl_status(
            "System Locale: LANG=en_US.UTF-8\n    X11 Layout: de\n     X11 Model: pc105\n   X11 Variant: nodeadkeys\n   X11 Options: terminate:ctrl_alt_bksp\n",
        )
        .unwrap();

        assert_eq!(names.rules, "");
        assert_eq!(names.model, "pc105");
        assert_eq!(names.layout, "de");
        assert_eq!(names.variant, "nodeadkeys");
        assert_eq!(names.options, "terminate:ctrl_alt_bksp");
    }
}
