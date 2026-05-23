use std::env;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::os::fd::AsRawFd;
use std::os::unix::fs::FileTypeExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use sky_cua_platform::diagnostics::{BackendError, BackendErrorCode};

use crate::portal::remote_desktop::MouseButton;

const COMMAND_TIMEOUT: Duration = Duration::from_secs(3);
const UINPUT_PATH: &str = "/dev/uinput";
const UINPUT_SETTLE_DELAY: Duration = Duration::from_millis(650);
const POINTER_ACTION_SETTLE_DELAY: Duration = Duration::from_millis(180);
const BUTTON_HOLD_DELAY: Duration = Duration::from_millis(120);

const EV_SYN: i32 = 0x00;
const EV_KEY: i32 = 0x01;
const EV_REL: i32 = 0x02;
const EV_ABS: i32 = 0x03;
const SYN_REPORT: i32 = 0;
const REL_WHEEL: i32 = 0x08;
const REL_WHEEL_HI_RES: i32 = 0x0b;
const ABS_X: i32 = 0x00;
const ABS_Y: i32 = 0x01;
const BTN_LEFT: i32 = 0x110;
const BTN_RIGHT: i32 = 0x111;
const BTN_MIDDLE: i32 = 0x112;
const BUS_USB: u16 = 0x03;
const UI_SET_EVBIT: libc::c_ulong = 0x40045564;
const UI_SET_KEYBIT: libc::c_ulong = 0x40045565;
const UI_SET_RELBIT: libc::c_ulong = 0x40045566;
const UI_SET_ABSBIT: libc::c_ulong = 0x40045567;
const UI_DEV_CREATE: libc::c_ulong = 0x5501;
const UI_DEV_DESTROY: libc::c_ulong = 0x5502;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VirtualInputAdapterKind {
    DirectUinput,
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
    pub desktop_bounds: Option<DesktopBounds>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VirtualInputUnavailable {
    pub reason: String,
    pub ydotool_path: Option<PathBuf>,
    pub socket_path: Option<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct LinuxVirtualInput {
    probe: VirtualInputProbe,
    uinput_device: Arc<std::sync::Mutex<Option<UinputPointerDevice>>>,
}

impl LinuxVirtualInput {
    pub fn new() -> Result<Self, BackendError> {
        let probe = probe_virtual_input().map_err(|unavailable| {
            BackendError::new(
                BackendErrorCode::ActionUnsupportedForEnvironment,
                format!("Linux virtual input is unavailable: {}", unavailable.reason),
            )
        })?;
        Ok(Self {
            probe,
            uinput_device: Arc::new(std::sync::Mutex::new(None)),
        })
    }

    fn with_uinput_device<F, R>(&self, f: F) -> Result<R, BackendError>
    where
        F: FnOnce(&mut UinputPointerDevice) -> Result<R, BackendError>,
    {
        let bounds = self.desktop_bounds()?;
        let mut guard = self
            .uinput_device
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if guard.as_ref().is_none_or(|d| d.bounds != bounds) {
            *guard = Some(UinputPointerDevice::create(bounds)?);
        }
        f(guard.as_mut().expect("uinput device was just created"))
    }

    pub fn move_absolute(&self, x: f64, y: f64) -> Result<(), BackendError> {
        match self.probe.adapter {
            VirtualInputAdapterKind::DirectUinput => {
                self.with_uinput_device(|device| device.move_absolute(x, y))
            }
            VirtualInputAdapterKind::Ydotool => self.run_ydotool(move_absolute_args(x, y)),
        }
    }

    pub fn click(&self, button: MouseButton) -> Result<(), BackendError> {
        match self.probe.adapter {
            VirtualInputAdapterKind::DirectUinput => {
                self.with_uinput_device(|device| device.click(button))
            }
            VirtualInputAdapterKind::Ydotool => {
                self.run_ydotool(["click".to_string(), click_code(button, ClickAction::Click)])
            }
        }
    }

    pub fn pointer_button(&self, button: MouseButton, pressed: bool) -> Result<(), BackendError> {
        match self.probe.adapter {
            VirtualInputAdapterKind::DirectUinput => {
                self.with_uinput_device(|device| device.pointer_button(button, pressed))
            }
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
            VirtualInputAdapterKind::DirectUinput => self.with_uinput_device(|device| {
                device.move_absolute(x, y)?;
                thread::sleep(POINTER_ACTION_SETTLE_DELAY);
                device.click(button)
            }),
            VirtualInputAdapterKind::Ydotool => {
                self.move_absolute(x, y)?;
                self.click(button)
            }
        }
    }

    pub fn pointer_mapping_details(&self, x: f64, y: f64) -> String {
        match self.probe.adapter {
            VirtualInputAdapterKind::DirectUinput => {
                if let Some(bounds) = self.probe.desktop_bounds {
                    let (absolute_x, absolute_y) = bounds.logical_to_absolute(x, y);
                    format!(
                        "adapter=direct_uinput coordinate_plane=desktop_logical requested=({x:.1},{y:.1}) emitted_absolute=({absolute_x},{absolute_y}) bounds=x:{} y:{} width:{} height:{} scale_milli:{}",
                        bounds.x, bounds.y, bounds.width, bounds.height, bounds.scale_milli
                    )
                } else {
                    format!(
                        "adapter=direct_uinput coordinate_plane=desktop_logical requested=({x:.1},{y:.1}) bounds=missing"
                    )
                }
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
            VirtualInputAdapterKind::DirectUinput => self.with_uinput_device(|device| {
                device.move_absolute(from.0, from.1)?;
                thread::sleep(POINTER_ACTION_SETTLE_DELAY);
                device.pointer_button(MouseButton::Left, true)?;
                thread::sleep(Duration::from_millis(40));
                device.move_absolute(to.0, to.1)?;
                thread::sleep(Duration::from_millis(40));
                device.pointer_button(MouseButton::Left, false)
            }),
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
            VirtualInputAdapterKind::DirectUinput => {
                self.with_uinput_device(|device| device.scroll_vertical(steps))
            }
            VirtualInputAdapterKind::Ydotool => self.run_ydotool(scroll_vertical_args(steps)),
        }
    }

    pub fn scroll_vertical_at(&self, x: f64, y: f64, steps: i32) -> Result<(), BackendError> {
        match self.probe.adapter {
            VirtualInputAdapterKind::DirectUinput => self.with_uinput_device(|device| {
                device.move_absolute(x, y)?;
                thread::sleep(POINTER_ACTION_SETTLE_DELAY);
                device.scroll_vertical(steps)
            }),
            VirtualInputAdapterKind::Ydotool => {
                self.move_absolute(x, y)?;
                self.scroll_vertical(steps)
            }
        }
    }

    pub fn type_text(&self, text: &str) -> Result<(), BackendError> {
        self.require_keyboard_adapter()?;
        self.run_ydotool(type_text_args(text))
    }

    pub fn press_key_sequence(&self, keys: &[String]) -> Result<(), BackendError> {
        self.require_keyboard_adapter()?;
        let events = key_sequence_events(keys)?;
        let mut args = vec!["key".to_string()];
        args.extend(events);
        self.run_ydotool(args)
    }

    fn run_ydotool<I>(&self, args: I) -> Result<(), BackendError>
    where
        I: IntoIterator<Item = String>,
    {
        run_ydotool_command(&self.probe, args)
    }

    fn desktop_bounds(&self) -> Result<DesktopBounds, BackendError> {
        self.probe.desktop_bounds.ok_or_else(|| {
            BackendError::new(
                BackendErrorCode::ActionUnsupportedForEnvironment,
                "Linux direct uinput requires detected desktop bounds",
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
                "Linux virtual input keyboard actions require a usable ydotool daemon; direct uinput only supports pointer actions.{socket_detail}"
            ),
        ))
    }
}

pub fn probe_virtual_input() -> Result<VirtualInputProbe, VirtualInputUnavailable> {
    if uinput_is_writable()
        && let Some(bounds) = detect_desktop_bounds()
    {
        return Ok(VirtualInputProbe {
            adapter: VirtualInputAdapterKind::DirectUinput,
            coordinate_plane: VirtualInputCoordinatePlane::DesktopLogical,
            ydotool_path: find_executable("ydotool"),
            socket_path: configured_socket_path(),
            desktop_bounds: Some(bounds),
        });
    }

    let Some(ydotool_path) = find_executable("ydotool") else {
        return Err(VirtualInputUnavailable {
            reason: if uinput_is_writable() {
                "direct uinput is writable but no desktop bounds could be detected, and ydotool executable was not found on PATH".to_string()
            } else {
                "direct uinput is unavailable and ydotool executable was not found on PATH"
                    .to_string()
            },
            ydotool_path: None,
            socket_path: preferred_socket_path(None, socket_path_candidates()),
        });
    };

    let socket_path = configured_socket_path();
    if let Some(path) = socket_path.as_ref() {
        if !path.exists() {
            return Err(VirtualInputUnavailable {
                reason: format!("ydotool socket does not exist: {}", path.display()),
                ydotool_path: Some(ydotool_path),
                socket_path,
            });
        }
        if !socket_is_connectable(path) {
            return Err(VirtualInputUnavailable {
                reason: format!(
                    "ydotool socket path is not a Unix socket: {}",
                    path.display()
                ),
                ydotool_path: Some(ydotool_path),
                socket_path,
            });
        }
    }

    Ok(VirtualInputProbe {
        adapter: VirtualInputAdapterKind::Ydotool,
        coordinate_plane: VirtualInputCoordinatePlane::DesktopLogical,
        ydotool_path: Some(ydotool_path),
        socket_path,
        desktop_bounds: None,
    })
}

pub fn virtual_input_keyboard_available() -> bool {
    probe_virtual_input().is_ok_and(|probe| probe.supports_keyboard())
}

impl VirtualInputProbe {
    pub fn supports_keyboard(&self) -> bool {
        self.ydotool_path.is_some()
            && self
                .socket_path
                .as_ref()
                .is_none_or(|path| socket_is_connectable(path))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DesktopBounds {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub scale_milli: u32,
}

impl DesktopBounds {
    pub fn logical_to_absolute(&self, x: f64, y: f64) -> (i32, i32) {
        let scale = f64::from(self.scale_milli) / 1000.0;
        let max_x = i32::try_from(self.width.saturating_sub(1)).unwrap_or(i32::MAX);
        let max_y = i32::try_from(self.height.saturating_sub(1)).unwrap_or(i32::MAX);
        (
            clamp_round_to_i32((x - f64::from(self.x)) * scale, 0, max_x),
            clamp_round_to_i32((y - f64::from(self.y)) * scale, 0, max_y),
        )
    }
}

#[derive(Debug)]
struct UinputPointerDevice {
    file: File,
    bounds: DesktopBounds,
}

impl UinputPointerDevice {
    fn create(bounds: DesktopBounds) -> Result<Self, BackendError> {
        if bounds.width == 0 || bounds.height == 0 {
            return Err(BackendError::new(
                BackendErrorCode::ActionUnsupportedForEnvironment,
                "direct uinput pointer requires nonzero desktop bounds",
            ));
        }
        let file = OpenOptions::new()
            .write(true)
            .open(UINPUT_PATH)
            .map_err(|error| {
                BackendError::new(
                    BackendErrorCode::ActionUnsupportedForEnvironment,
                    format!("failed to open {UINPUT_PATH} for direct uinput pointer: {error}"),
                )
            })?;
        let fd = file.as_raw_fd();
        ioctl_set(fd, UI_SET_EVBIT, EV_KEY, "UI_SET_EVBIT EV_KEY")?;
        ioctl_set(fd, UI_SET_EVBIT, EV_REL, "UI_SET_EVBIT EV_REL")?;
        ioctl_set(fd, UI_SET_EVBIT, EV_ABS, "UI_SET_EVBIT EV_ABS")?;
        ioctl_set(fd, UI_SET_KEYBIT, BTN_LEFT, "UI_SET_KEYBIT BTN_LEFT")?;
        ioctl_set(fd, UI_SET_KEYBIT, BTN_RIGHT, "UI_SET_KEYBIT BTN_RIGHT")?;
        ioctl_set(fd, UI_SET_KEYBIT, BTN_MIDDLE, "UI_SET_KEYBIT BTN_MIDDLE")?;
        ioctl_set(fd, UI_SET_RELBIT, REL_WHEEL, "UI_SET_RELBIT REL_WHEEL")?;
        ioctl_set(
            fd,
            UI_SET_RELBIT,
            REL_WHEEL_HI_RES,
            "UI_SET_RELBIT REL_WHEEL_HI_RES",
        )?;
        ioctl_set(fd, UI_SET_ABSBIT, ABS_X, "UI_SET_ABSBIT ABS_X")?;
        ioctl_set(fd, UI_SET_ABSBIT, ABS_Y, "UI_SET_ABSBIT ABS_Y")?;

        let mut user_dev = UinputUserDev::named("sky-cua absolute pointer");
        user_dev.id.bustype = BUS_USB;
        user_dev.id.vendor = 0x5c1a;
        user_dev.id.product = 0x0002;
        user_dev.id.version = 1;
        user_dev.absmin[ABS_X as usize] = 0;
        user_dev.absmin[ABS_Y as usize] = 0;
        user_dev.absmax[ABS_X as usize] =
            i32::try_from(bounds.width.saturating_sub(1)).unwrap_or(i32::MAX);
        user_dev.absmax[ABS_Y as usize] =
            i32::try_from(bounds.height.saturating_sub(1)).unwrap_or(i32::MAX);
        let bytes = unsafe {
            std::slice::from_raw_parts(
                (&user_dev as *const UinputUserDev).cast::<u8>(),
                std::mem::size_of::<UinputUserDev>(),
            )
        };
        (&file).write_all(bytes).map_err(|error| {
            BackendError::new(
                BackendErrorCode::ActionUnsupportedForEnvironment,
                format!("failed to write uinput pointer device definition: {error}"),
            )
        })?;
        ioctl_no_arg(fd, UI_DEV_CREATE, "UI_DEV_CREATE")?;
        thread::sleep(UINPUT_SETTLE_DELAY);
        Ok(Self { file, bounds })
    }

    fn move_absolute(&mut self, x: f64, y: f64) -> Result<(), BackendError> {
        let (x, y) = self.bounds.logical_to_absolute(x, y);
        self.emit(EV_ABS, ABS_X, x)?;
        self.emit(EV_ABS, ABS_Y, y)?;
        self.syn()
    }

    fn click(&mut self, button: MouseButton) -> Result<(), BackendError> {
        self.pointer_button(button, true)?;
        thread::sleep(BUTTON_HOLD_DELAY);
        self.pointer_button(button, false)
    }

    fn pointer_button(&mut self, button: MouseButton, pressed: bool) -> Result<(), BackendError> {
        self.emit(
            EV_KEY,
            linux_button_code(button),
            if pressed { 1 } else { 0 },
        )?;
        self.syn()
    }

    fn scroll_vertical(&mut self, steps: i32) -> Result<(), BackendError> {
        let uinput_steps = -steps;
        self.emit(EV_REL, REL_WHEEL_HI_RES, uinput_steps.saturating_mul(120))?;
        self.emit(EV_REL, REL_WHEEL, uinput_steps)?;
        self.syn()
    }

    fn emit(&mut self, event_type: i32, code: i32, value: i32) -> Result<(), BackendError> {
        let event = InputEvent {
            time: libc::timeval {
                tv_sec: 0,
                tv_usec: 0,
            },
            type_: u16::try_from(event_type).unwrap_or(0),
            code: u16::try_from(code).unwrap_or(0),
            value,
        };
        let bytes = unsafe {
            std::slice::from_raw_parts(
                (&event as *const InputEvent).cast::<u8>(),
                std::mem::size_of::<InputEvent>(),
            )
        };
        self.file.write_all(bytes).map_err(|error| {
            BackendError::new(
                BackendErrorCode::ActionUnsupportedForEnvironment,
                format!("failed to write direct uinput event: {error}"),
            )
        })
    }

    fn syn(&mut self) -> Result<(), BackendError> {
        self.emit(EV_SYN, SYN_REPORT, 0)
    }
}

impl Drop for UinputPointerDevice {
    fn drop(&mut self) {
        let _ = unsafe { libc::ioctl(self.file.as_raw_fd(), UI_DEV_DESTROY) };
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
struct InputId {
    bustype: u16,
    vendor: u16,
    product: u16,
    version: u16,
}

#[repr(C)]
struct UinputUserDev {
    name: [u8; 80],
    id: InputId,
    ff_effects_max: u32,
    absmax: [i32; 64],
    absmin: [i32; 64],
    absfuzz: [i32; 64],
    absflat: [i32; 64],
}

impl UinputUserDev {
    fn named(name: &str) -> Self {
        let mut device = Self {
            name: [0; 80],
            id: InputId {
                bustype: 0,
                vendor: 0,
                product: 0,
                version: 0,
            },
            ff_effects_max: 0,
            absmax: [0; 64],
            absmin: [0; 64],
            absfuzz: [0; 64],
            absflat: [0; 64],
        };
        let name_bytes = name.as_bytes();
        let len = name_bytes.len().min(device.name.len().saturating_sub(1));
        device.name[..len].copy_from_slice(&name_bytes[..len]);
        device
    }
}

#[repr(C)]
struct InputEvent {
    time: libc::timeval,
    type_: u16,
    code: u16,
    value: i32,
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

fn linux_button_code(button: MouseButton) -> i32 {
    match button {
        MouseButton::Left => BTN_LEFT,
        MouseButton::Right => BTN_RIGHT,
        MouseButton::Middle => BTN_MIDDLE,
    }
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
        "--".to_string(),
        round_coordinate(x),
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

fn uinput_is_writable() -> bool {
    OpenOptions::new().write(true).open(UINPUT_PATH).is_ok()
}

fn detect_desktop_bounds() -> Option<DesktopBounds> {
    desktop_bounds_from_env()
        .or_else(desktop_bounds_from_cosmic_randr)
        .or_else(desktop_bounds_from_xrandr)
}

fn desktop_bounds_from_env() -> Option<DesktopBounds> {
    let width = env::var("SKY_CUA_VIRTUAL_INPUT_WIDTH")
        .ok()?
        .trim()
        .parse::<u32>()
        .ok()?;
    let height = env::var("SKY_CUA_VIRTUAL_INPUT_HEIGHT")
        .ok()?
        .trim()
        .parse::<u32>()
        .ok()?;
    if width == 0 || height == 0 {
        return None;
    }
    let x = env::var("SKY_CUA_VIRTUAL_INPUT_X")
        .ok()
        .and_then(|value| value.trim().parse::<i32>().ok())
        .unwrap_or(0);
    let y = env::var("SKY_CUA_VIRTUAL_INPUT_Y")
        .ok()
        .and_then(|value| value.trim().parse::<i32>().ok())
        .unwrap_or(0);
    Some(DesktopBounds {
        x,
        y,
        width,
        height,
        scale_milli: env::var("SKY_CUA_VIRTUAL_INPUT_SCALE")
            .ok()
            .and_then(|value| parse_scale_milli(&value))
            .unwrap_or(1000),
    })
}

fn desktop_bounds_from_cosmic_randr() -> Option<DesktopBounds> {
    if !should_probe_cosmic_randr() {
        return None;
    }
    let output = command_output_with_timeout(
        Command::new("cosmic-randr")
            .arg("list")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null()),
        COMMAND_TIMEOUT,
    )?;
    if !output.status.success() {
        return None;
    }
    parse_cosmic_randr_bounds(&String::from_utf8_lossy(&output.stdout))
}

fn desktop_bounds_from_xrandr() -> Option<DesktopBounds> {
    let output = command_output_with_timeout(
        Command::new("xrandr")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null()),
        COMMAND_TIMEOUT,
    )?;
    if !output.status.success() {
        return None;
    }
    parse_xrandr_bounds(&String::from_utf8_lossy(&output.stdout))
}

fn should_probe_cosmic_randr() -> bool {
    if truthy_env("SKY_CUA_VIRTUAL_INPUT_PROBE_COSMIC_RANDR") {
        return true;
    }
    desktop_name_allows_cosmic_randr(env::var("XDG_CURRENT_DESKTOP").ok().as_deref())
        || desktop_name_allows_cosmic_randr(env::var("DESKTOP_SESSION").ok().as_deref())
}

fn desktop_name_allows_cosmic_randr(value: Option<&str>) -> bool {
    value
        .unwrap_or_default()
        .split([':', ';', ','])
        .any(|part| part.trim().eq_ignore_ascii_case("cosmic"))
}

fn truthy_env(name: &str) -> bool {
    env::var(name)
        .ok()
        .map(|value| {
            let normalized = value.trim().to_ascii_lowercase();
            matches!(normalized.as_str(), "1" | "true" | "yes" | "on")
        })
        .unwrap_or(false)
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

fn parse_cosmic_randr_bounds(output: &str) -> Option<DesktopBounds> {
    let mut current_position: Option<(i32, i32)> = None;
    let mut current_scale_milli = 1000;
    for line in output.lines() {
        let stripped = strip_ansi(line).trim().to_string();
        if let Some(value) = stripped.strip_prefix("Position:") {
            let (x, y) = parse_position(value.trim())?;
            current_position = Some((x, y));
            continue;
        }
        if let Some(value) = stripped.strip_prefix("Scale:") {
            current_scale_milli = parse_scale_milli(value.trim()).unwrap_or(1000);
            continue;
        }
        if stripped.contains("(current)")
            && let Some((width, height)) = parse_first_mode_size(&stripped)
        {
            let (x, y) = current_position.unwrap_or((0, 0));
            return Some(DesktopBounds {
                x,
                y,
                width,
                height,
                scale_milli: current_scale_milli,
            });
        }
    }
    None
}

fn parse_xrandr_bounds(output: &str) -> Option<DesktopBounds> {
    for line in output.lines() {
        if !line.contains(" connected") {
            continue;
        }
        for part in line.split_whitespace() {
            if let Some(bounds) = parse_xrandr_geometry(part) {
                return Some(bounds);
            }
        }
    }
    None
}

fn parse_xrandr_geometry(value: &str) -> Option<DesktopBounds> {
    let (size, rest) = value.split_once('+')?;
    let (width, height) = parse_size(size)?;
    let (x, y) = rest.split_once('+')?;
    Some(DesktopBounds {
        x: x.parse().ok()?,
        y: y.parse().ok()?,
        width,
        height,
        scale_milli: 1000,
    })
}

fn parse_first_mode_size(line: &str) -> Option<(u32, u32)> {
    line.split_whitespace().find_map(parse_size)
}

fn parse_size(value: &str) -> Option<(u32, u32)> {
    let clean = value.trim();
    let (width, height) = clean.split_once('x')?;
    let width = width.trim().parse::<u32>().ok()?;
    let height = height.trim().parse::<u32>().ok()?;
    (width > 0 && height > 0).then_some((width, height))
}

fn parse_position(value: &str) -> Option<(i32, i32)> {
    let (x, y) = value.split_once(',')?;
    Some((x.trim().parse().ok()?, y.trim().parse().ok()?))
}

fn parse_scale_milli(value: &str) -> Option<u32> {
    let value = value.trim();
    let scale = if let Some(percent) = value.strip_suffix('%') {
        percent.trim().parse::<f64>().ok()? / 100.0
    } else {
        value.parse::<f64>().ok()?
    };
    if !scale.is_finite() || scale <= 0.0 {
        return None;
    }
    Some((scale * 1000.0).round().clamp(1.0, f64::from(u32::MAX)) as u32)
}

fn strip_ansi(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    let mut chars = value.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' && chars.peek() == Some(&'[') {
            let _ = chars.next();
            for code in chars.by_ref() {
                if code.is_ascii_alphabetic() {
                    break;
                }
            }
            continue;
        }
        output.push(ch);
    }
    output
}

fn ioctl_set(fd: i32, request: libc::c_ulong, value: i32, label: &str) -> Result<(), BackendError> {
    let rc = unsafe { libc::ioctl(fd, request, value) };
    if rc == 0 {
        Ok(())
    } else {
        Err(BackendError::new(
            BackendErrorCode::ActionUnsupportedForEnvironment,
            format!("{label} failed: {}", std::io::Error::last_os_error()),
        ))
    }
}

fn ioctl_no_arg(fd: i32, request: libc::c_ulong, label: &str) -> Result<(), BackendError> {
    let rc = unsafe { libc::ioctl(fd, request) };
    if rc == 0 {
        Ok(())
    } else {
        Err(BackendError::new(
            BackendErrorCode::ActionUnsupportedForEnvironment,
            format!("{label} failed: {}", std::io::Error::last_os_error()),
        ))
    }
}

fn clamp_round_to_i32(value: f64, min: i32, max: i32) -> i32 {
    if !value.is_finite() {
        return min;
    }
    let rounded = value.round();
    if rounded < f64::from(min) {
        min
    } else if rounded > f64::from(max) {
        max
    } else {
        rounded as i32
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
    use std::sync::Arc;

    use super::{
        ClickAction, DesktopBounds, LinuxVirtualInput, VirtualInputAdapterKind,
        VirtualInputCoordinatePlane, VirtualInputProbe, click_code,
        desktop_name_allows_cosmic_randr, key_sequence_events, move_absolute_args,
        parse_cosmic_randr_bounds, parse_scale_milli, parse_xrandr_bounds, preferred_socket_path,
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
            vec!["mousemove", "--absolute", "--", "10", "21"]
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
    fn direct_uinput_without_ydotool_is_pointer_only() {
        let input = LinuxVirtualInput {
            probe: VirtualInputProbe {
                adapter: VirtualInputAdapterKind::DirectUinput,
                coordinate_plane: VirtualInputCoordinatePlane::DesktopLogical,
                ydotool_path: None,
                socket_path: None,
                desktop_bounds: Some(DesktopBounds {
                    x: 0,
                    y: 0,
                    width: 1280,
                    height: 720,
                    scale_milli: 1000,
                }),
            },
            uinput_device: Arc::new(std::sync::Mutex::new(None)),
        };

        let error = input
            .type_text("hello")
            .expect_err("keyboard action should fail");

        assert!(error.message.contains("ydotool daemon"));
    }

    #[test]
    fn parses_cosmic_randr_current_output_bounds() {
        let output = "\u{1b}[1mVirtual-1\u{1b}[0m \u{1b}[1;32m(enabled)\u{1b}[0m\n  Position: 0,0\n  Scale: 100%\n  Modes:\n    1920x1080 @ 60.000 Hz\n    1280x800 @ 74.994 Hz (current) (preferred)\n";

        assert_eq!(
            parse_cosmic_randr_bounds(output),
            Some(DesktopBounds {
                x: 0,
                y: 0,
                width: 1280,
                height: 800,
                scale_milli: 1000,
            })
        );
    }

    #[test]
    fn parses_cosmic_randr_scale_for_absolute_uinput_bounds() {
        let output = "\u{1b}[1mVirtual-1\u{1b}[0m \u{1b}[1;32m(enabled)\u{1b}[0m\n  Position: 0,0\n  Scale: 125%\n  Modes:\n    1600x1200 @ 60.000 Hz (current)\n";

        let bounds = parse_cosmic_randr_bounds(output).unwrap();
        assert_eq!(
            bounds,
            DesktopBounds {
                x: 0,
                y: 0,
                width: 1600,
                height: 1200,
                scale_milli: 1250,
            }
        );
        assert_eq!(bounds.logical_to_absolute(194.0, 314.0), (243, 393));
    }

    #[test]
    fn parses_scale_values_as_milli_factors() {
        assert_eq!(parse_scale_milli("100%"), Some(1000));
        assert_eq!(parse_scale_milli("125%"), Some(1250));
        assert_eq!(parse_scale_milli("1.5"), Some(1500));
        assert_eq!(parse_scale_milli("0"), None);
    }

    #[test]
    fn cosmic_randr_probe_is_scoped_to_cosmic_desktops() {
        assert!(desktop_name_allows_cosmic_randr(Some("COSMIC")));
        assert!(desktop_name_allows_cosmic_randr(Some("pop:COSMIC")));
        assert!(desktop_name_allows_cosmic_randr(Some("gnome;cosmic")));
        assert!(!desktop_name_allows_cosmic_randr(Some("KDE")));
        assert!(!desktop_name_allows_cosmic_randr(Some("GNOME")));
        assert!(!desktop_name_allows_cosmic_randr(None));
    }

    #[test]
    fn parses_xrandr_connected_geometry() {
        let output = "Virtual-1 connected primary 1280x800+10+20 normal left inverted right x axis y axis 320mm x 200mm\n";

        assert_eq!(
            parse_xrandr_bounds(output),
            Some(DesktopBounds {
                x: 10,
                y: 20,
                width: 1280,
                height: 800,
                scale_milli: 1000,
            })
        );
    }
}
