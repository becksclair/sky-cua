use std::fs::{File, OpenOptions};
use std::io::Write;
use std::os::fd::AsRawFd;
use std::thread;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const UINPUT_PATH: &str = "/dev/uinput";
const UINPUT_SETTLE_DELAY: Duration = Duration::from_millis(650);

const EV_SYN: i32 = 0x00;
const EV_KEY: i32 = 0x01;
const SYN_REPORT: i32 = 0;
const BUS_USB: u16 = 0x03;
const UI_SET_EVBIT: libc::c_ulong = 0x40045564;
const UI_SET_KEYBIT: libc::c_ulong = 0x40045565;
const UI_DEV_CREATE: libc::c_ulong = 0x5501;
const UI_DEV_DESTROY: libc::c_ulong = 0x5502;

#[derive(Debug, Error)]
#[error("{message}")]
pub struct UinputError {
    pub message: String,
}

impl UinputError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    fn io(label: &str, error: std::io::Error) -> Self {
        Self::new(format!("{label}: {error}"))
    }
}

pub type Result<T> = std::result::Result<T, UinputError>;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
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
pub struct UinputKeyboardDevice {
    file: File,
}

impl UinputKeyboardDevice {
    pub fn create() -> Result<Self> {
        let file = OpenOptions::new()
            .write(true)
            .open(UINPUT_PATH)
            .map_err(|error| UinputError::io(&format!("failed to open {UINPUT_PATH}"), error))?;
        let fd = file.as_raw_fd();
        ioctl_set(fd, UI_SET_EVBIT, EV_KEY, "UI_SET_EVBIT EV_KEY")?;
        for keycode in 1..=255 {
            ioctl_set(fd, UI_SET_KEYBIT, keycode, "UI_SET_KEYBIT keyboard key")?;
        }

        let mut user_dev = UinputUserDev::named("sky-cua keyboard");
        user_dev.id.bustype = BUS_USB;
        user_dev.id.vendor = 0x5c1a;
        user_dev.id.product = 0x0003;
        user_dev.id.version = 1;
        write_user_dev(
            &file,
            &user_dev,
            "failed to write uinput keyboard device definition",
        )?;
        ioctl_no_arg(fd, UI_DEV_CREATE, "UI_DEV_CREATE")?;
        thread::sleep(UINPUT_SETTLE_DELAY);
        Ok(Self { file })
    }

    pub fn key_event(&mut self, code: u16, pressed: bool) -> Result<()> {
        self.emit(EV_KEY, i32::from(code), if pressed { 1 } else { 0 })?;
        self.syn()
    }

    fn emit(&mut self, event_type: i32, code: i32, value: i32) -> Result<()> {
        emit_event(&mut self.file, event_type, code, value)
    }

    fn syn(&mut self) -> Result<()> {
        self.emit(EV_SYN, SYN_REPORT, 0)
    }
}

impl Drop for UinputKeyboardDevice {
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
pub struct InputEvent {
    time: libc::timeval,
    type_: u16,
    code: u16,
    value: i32,
}

fn write_user_dev(mut file: &File, user_dev: &UinputUserDev, label: &str) -> Result<()> {
    let bytes = unsafe {
        std::slice::from_raw_parts(
            (user_dev as *const UinputUserDev).cast::<u8>(),
            std::mem::size_of::<UinputUserDev>(),
        )
    };
    file.write_all(bytes)
        .map_err(|error| UinputError::io(label, error))
}

fn emit_event(file: &mut File, event_type: i32, code: i32, value: i32) -> Result<()> {
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
    file.write_all(bytes)
        .map_err(|error| UinputError::io("failed to write uinput event", error))
}

fn ioctl_set(fd: i32, request: libc::c_ulong, value: i32, label: &str) -> Result<()> {
    let rc = unsafe { libc::ioctl(fd, request, value) };
    if rc == 0 {
        Ok(())
    } else {
        Err(UinputError::new(format!(
            "{label} failed: {}",
            std::io::Error::last_os_error()
        )))
    }
}

fn ioctl_no_arg(fd: i32, request: libc::c_ulong, label: &str) -> Result<()> {
    let rc = unsafe { libc::ioctl(fd, request) };
    if rc == 0 {
        Ok(())
    } else {
        Err(UinputError::new(format!(
            "{label} failed: {}",
            std::io::Error::last_os_error()
        )))
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

#[cfg(test)]
mod tests {
    use super::DesktopBounds;

    #[test]
    fn maps_logical_coordinates_to_absolute_bounds() {
        let bounds = DesktopBounds {
            x: 10,
            y: 20,
            width: 100,
            height: 50,
            scale_milli: 2000,
        };

        assert_eq!(bounds.logical_to_absolute(20.0, 30.0), (20, 20));
        assert_eq!(bounds.logical_to_absolute(-100.0, 500.0), (0, 49));
    }
}
