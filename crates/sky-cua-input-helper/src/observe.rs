use std::{
    fs,
    io::Write,
    path::PathBuf,
    sync::mpsc::{self, Receiver, Sender},
    thread,
    time::Duration,
};

use anyhow::{Context, Result, bail};
use evdev::{Device, EventSummary, RelativeAxisCode};

use crate::{
    protocol::{HelperStreamEvent, stream_event_line},
    uinput::DesktopBounds,
};

const EVENT_DEVICE_DIR: &str = "/dev/input";
const DEVICE_POLL_DELAY: Duration = Duration::from_millis(4);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PointerDelta {
    dx: i32,
    dy: i32,
}

pub fn observe_pointer(mut writer: impl Write, bounds: DesktopBounds) -> Result<()> {
    if bounds.width == 0 || bounds.height == 0 {
        bail!("observe_pointer requires nonzero desktop bounds");
    }
    let devices = pointer_devices();
    if devices.is_empty() {
        bail!("no readable relative evdev pointer devices found");
    }
    let (sender, receiver) = mpsc::channel();
    for device in devices {
        spawn_device_observer(device, sender.clone());
    }
    drop(sender);
    integrate_pointer_deltas(&mut writer, bounds, receiver)
}

fn pointer_devices() -> Vec<Device> {
    let Ok(entries) = fs::read_dir(EVENT_DEVICE_DIR) else {
        return Vec::new();
    };
    entries
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("event"))
        })
        .filter_map(open_pointer_device)
        .collect()
}

fn open_pointer_device(path: PathBuf) -> Option<Device> {
    let device = Device::open(path).ok()?;
    if excluded_device_name(device.name()) || !supports_relative_pointer(&device) {
        return None;
    }
    Some(device)
}

fn excluded_device_name(name: Option<&str>) -> bool {
    let name = name.unwrap_or_default().to_ascii_lowercase();
    ["sky-cua", "ydotool", "uinput"]
        .into_iter()
        .any(|needle| name.contains(needle))
}

fn supports_relative_pointer(device: &Device) -> bool {
    device.supported_relative_axes().is_some_and(|axes| {
        axes.contains(RelativeAxisCode::REL_X) && axes.contains(RelativeAxisCode::REL_Y)
    })
}

fn spawn_device_observer(mut device: Device, sender: Sender<PointerDelta>) {
    thread::spawn(move || {
        let _ = device.set_nonblocking(true);
        loop {
            match device.fetch_events() {
                Ok(events) => {
                    let mut dx = 0;
                    let mut dy = 0;
                    for event in events {
                        match event.destructure() {
                            EventSummary::RelativeAxis(_, RelativeAxisCode::REL_X, value) => {
                                dx += value;
                            }
                            EventSummary::RelativeAxis(_, RelativeAxisCode::REL_Y, value) => {
                                dy += value;
                            }
                            _ => {}
                        }
                    }
                    if (dx != 0 || dy != 0) && sender.send(PointerDelta { dx, dy }).is_err() {
                        return;
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(DEVICE_POLL_DELAY);
                }
                Err(_) => return,
            }
        }
    });
}

fn integrate_pointer_deltas(
    writer: &mut impl Write,
    bounds: DesktopBounds,
    receiver: Receiver<PointerDelta>,
) -> Result<()> {
    let mut x = f64::from(bounds.x) + f64::from(bounds.width) / 2.0;
    let mut y = f64::from(bounds.y) + f64::from(bounds.height) / 2.0;
    let min_x = f64::from(bounds.x);
    let min_y = f64::from(bounds.y);
    let max_x = f64::from(bounds.x) + f64::from(bounds.width.saturating_sub(1));
    let max_y = f64::from(bounds.y) + f64::from(bounds.height.saturating_sub(1));
    let mut sequence = 0_u64;
    for delta in receiver {
        x = (x + f64::from(delta.dx)).clamp(min_x, max_x);
        y = (y + f64::from(delta.dy)).clamp(min_y, max_y);
        sequence = sequence.saturating_add(1);
        let line = stream_event_line(&HelperStreamEvent::PointerMoved {
            x,
            y,
            sequence,
            coordinate_space: "desktop_logical".to_string(),
            exact: false,
        })
        .context("failed to encode observe_pointer event")?;
        writer
            .write_all(line.as_bytes())
            .context("failed to write observe_pointer event")?;
        writer
            .flush()
            .context("failed to flush observe_pointer event")?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{PointerDelta, excluded_device_name, integrate_pointer_deltas};
    use crate::uinput::DesktopBounds;
    use std::sync::mpsc;

    #[test]
    fn excludes_synthetic_pointer_devices_by_name() {
        assert!(excluded_device_name(Some("sky-cua virtual pointer")));
        assert!(excluded_device_name(Some("ydotool virtual device")));
        assert!(!excluded_device_name(Some("Logitech USB Optical Mouse")));
    }

    #[test]
    fn observe_pointer_integrates_deltas_inside_bounds() {
        let (sender, receiver) = mpsc::channel();
        sender.send(PointerDelta { dx: 10, dy: -5 }).unwrap();
        sender.send(PointerDelta { dx: 1000, dy: 1000 }).unwrap();
        drop(sender);
        let mut output = Vec::new();

        integrate_pointer_deltas(
            &mut output,
            DesktopBounds {
                x: 0,
                y: 0,
                width: 100,
                height: 80,
                scale_milli: 1000,
            },
            receiver,
        )
        .unwrap();

        let rendered = String::from_utf8(output).unwrap();
        assert!(rendered.contains("\"event\":\"pointer_moved\""));
        assert!(rendered.contains("\"sequence\":2"));
        assert!(rendered.contains("\"x\":99.0"));
        assert!(rendered.contains("\"y\":79.0"));
    }
}
