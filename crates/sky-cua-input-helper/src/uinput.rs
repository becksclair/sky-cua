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

/// Absolute `EV_ABS` pointer device that maps desktop-logical coordinates
/// linearly onto the output, sidestepping libinput pointer acceleration. Created
/// inside the privileged helper so the unprivileged service can drive precise
/// pointer actions on compositors without a RemoteDesktop portal (COSMIC).
#[derive(Debug)]
pub struct UinputPointerDevice {
    file: File,
    bounds: DesktopBounds,
    /// Buttons currently pressed, as a bitmask keyed by the `button()` index
    /// (bit 0 = left, 1 = right, 2 = middle). Tracked so `Drop` can release a
    /// held button before destroying the device.
    held_buttons: u8,
}

impl UinputPointerDevice {
    pub fn create(bounds: DesktopBounds) -> Result<Self> {
        if bounds.width == 0 || bounds.height == 0 {
            return Err(UinputError::new(
                "absolute uinput pointer requires nonzero desktop bounds",
            ));
        }
        let file = OpenOptions::new()
            .write(true)
            .open(UINPUT_PATH)
            .map_err(|error| UinputError::io(&format!("failed to open {UINPUT_PATH}"), error))?;
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

        // NOTE: deliberately no INPUT_PROP_DIRECT and no BTN_TOUCH/BTN_TOOL_FINGER.
        // Declaring either makes udev tag the device ID_INPUT_TOUCHSCREEN, and the
        // compositor would deliver touch events instead of moving the cursor.
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
        write_user_dev(
            &file,
            &user_dev,
            "failed to write uinput pointer device definition",
        )?;
        ioctl_no_arg(fd, UI_DEV_CREATE, "UI_DEV_CREATE")?;
        thread::sleep(UINPUT_SETTLE_DELAY);
        Ok(Self {
            file,
            bounds,
            held_buttons: 0,
        })
    }

    pub fn bounds(&self) -> DesktopBounds {
        self.bounds
    }

    pub fn move_absolute(&mut self, x: f64, y: f64) -> Result<()> {
        let (ax, ay) = self.bounds.logical_to_absolute(x, y);
        self.emit(EV_ABS, ABS_X, ax)?;
        self.emit(EV_ABS, ABS_Y, ay)?;
        self.syn()
    }

    pub fn button(&mut self, button: u8, pressed: bool) -> Result<()> {
        let code = match button {
            0 => BTN_LEFT,
            1 => BTN_RIGHT,
            2 => BTN_MIDDLE,
            other => {
                return Err(UinputError::new(format!(
                    "unsupported pointer button index {other}"
                )));
            }
        };
        self.emit(EV_KEY, code, if pressed { 1 } else { 0 })?;
        let bit = 1u8 << button;
        if pressed {
            self.held_buttons |= bit;
        } else {
            self.held_buttons &= !bit;
        }
        self.syn()
    }

    pub fn scroll_vertical(&mut self, steps: i32) -> Result<()> {
        let s = -steps;
        self.emit(EV_REL, REL_WHEEL_HI_RES, s.saturating_mul(120))?;
        self.emit(EV_REL, REL_WHEEL, s)?;
        self.syn()
    }

    fn emit(&mut self, event_type: i32, code: i32, value: i32) -> Result<()> {
        emit_event(&mut self.file, event_type, code, value)
    }

    fn syn(&mut self) -> Result<()> {
        self.emit(EV_SYN, SYN_REPORT, 0)
    }
}

impl Drop for UinputPointerDevice {
    fn drop(&mut self) {
        // Release any button still held before tearing the device down so a
        // rebuild (or shutdown) cannot strand a press on the compositor. The
        // client brackets press+release in one batch today, so this is normally
        // a no-op, but it keeps the device self-cleaning regardless of caller.
        for (index, code) in [(0u8, BTN_LEFT), (1, BTN_RIGHT), (2, BTN_MIDDLE)] {
            if self.held_buttons & (1 << index) != 0 {
                let _ = self.emit(EV_KEY, code, 0);
                let _ = self.syn();
            }
        }
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

    #[test]
    fn identity_scale_maps_logical_one_to_one() {
        let bounds = DesktopBounds {
            x: 0,
            y: 0,
            width: 1920,
            height: 1080,
            scale_milli: 1000,
        };

        assert_eq!(bounds.logical_to_absolute(0.0, 0.0), (0, 0));
        assert_eq!(bounds.logical_to_absolute(640.0, 480.0), (640, 480));
    }

    #[test]
    fn fractional_scale_maps_logical_to_device_units() {
        let bounds = DesktopBounds {
            x: 0,
            y: 0,
            width: 1600,
            height: 1200,
            scale_milli: 1250,
        };

        assert_eq!(bounds.logical_to_absolute(194.0, 314.0), (243, 393));
    }

    #[test]
    fn nonzero_origin_is_subtracted_before_scaling() {
        let bounds = DesktopBounds {
            x: 100,
            y: 50,
            width: 1920,
            height: 1080,
            scale_milli: 1000,
        };

        assert_eq!(bounds.logical_to_absolute(100.0, 50.0), (0, 0));
        assert_eq!(bounds.logical_to_absolute(300.0, 250.0), (200, 200));
    }

    #[test]
    fn clamps_to_device_edges() {
        let bounds = DesktopBounds {
            x: 0,
            y: 0,
            width: 1920,
            height: 1080,
            scale_milli: 1000,
        };

        assert_eq!(bounds.logical_to_absolute(-50.0, -50.0), (0, 0));
        assert_eq!(bounds.logical_to_absolute(5000.0, 5000.0), (1919, 1079));
    }
}
