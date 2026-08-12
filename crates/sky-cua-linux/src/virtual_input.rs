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
    HelperCommand, HelperResponse, KeyEventCommand, PointerAction, parse_response_line,
    request_line,
};
use sky_cua_input_helper::uinput::DesktopBounds;
use sky_cua_platform::config::INPUT_HELPER_SOCKET_ENV as SKY_CUA_INPUT_HELPER_SOCKET;
use sky_cua_platform::diagnostics::{BackendError, BackendErrorCode};
use sky_cua_platform::model::CuaCancellation;
use xkbcommon::xkb;

use crate::portal::eis_keymap::{
    EisKeyStroke, build_keysym_cache, clear_modifiers_already_present_in_chord,
    find_keycodes_from_cache, keysym_for_char, keysym_for_key_name, required_modifier_keycodes,
    resolve_eis_keystroke,
};
use crate::portal::remote_desktop::MouseButton;
use crate::x11::input_xtest::horizontal_scroll_button;

const COMMAND_TIMEOUT: Duration = Duration::from_secs(3);
const HELPER_COMMAND_TIMEOUT: Duration = Duration::from_secs(3);
/// Extra read-timeout headroom for a helper batch that may rebuild its uinput
/// device on a bounds change (each rebuild costs `UINPUT_SETTLE_DELAY` ~650ms).
const HELPER_DEVICE_REBUILD_ALLOWANCE: Duration = Duration::from_millis(1000);
const DEFAULT_HELPER_SOCKET_PATH: &str = "/run/sky-cua/input-helper.sock";

/// Keycode tapped after a COSMIC helper keyboard batch to flush the
/// compositor's one-event-late text-input pipeline.
///
/// `type_text` uses `KEY_END` (107): the inter-character commits already work
/// reliably under load because each is triggered by the *next character* — a
/// decisive, non-modifier key event — whereas a trailing modifier like `Shift`
/// is treated as a held chord prefix and commits the final character's preedit
/// only flakily. `End` is an equally decisive non-modifier and, with the cursor
/// already at the end after typing, a visual no-op. `press_key` keeps
/// `KEY_LEFTSHIFT` (42): it ships the flush in the same batch as the key (so the
/// soft-modifier race does not apply) and Shift never moves the cursor, which a
/// cursor key would for navigation presses.
const COSMIC_TEXT_FLUSH_KEYCODE: u16 = 107;
const COSMIC_KEY_FLUSH_KEYCODE: u16 = 42;

/// Deterministic gap between the per-character helper commands of a COSMIC
/// `type_text` (and before the trailing flush). The inter-command gap would
/// otherwise be whatever the helper's nonblocking accept poll happens to add
/// (0–25 ms), and when it collapses toward zero cosmic-comp's one-event-late
/// text-input pipeline drops the commit of the preceding character — most
/// visibly the final character, which the flush then fails to commit. A fixed
/// floor keeps every commit reliable.
const COSMIC_KEY_COMMAND_GAP: Duration = Duration::from_millis(30);

/// Why a helper-routed `type_text` failed, distinguishing a failure before any
/// keystroke reached the device (safe to retry through another adapter) from a
/// failure after a partial prefix was already injected (a full retype would
/// duplicate that prefix).
enum HelperTypeError {
    BeforeInjection(BackendError),
    AfterInjection(BackendError),
}

/// Press-to-release dwell for a helper-routed click, in milliseconds.
const HELPER_BUTTON_HOLD_MILLIS: u32 = 120;
/// Settle around the press/release that bracket a helper-routed drag grab.
const HELPER_DRAG_GRAB_SETTLE_MILLIS: u32 = 40;

/// Left/right/middle button index understood by the helper's `PointerAction::Button`.
fn helper_button_index(button: MouseButton) -> u8 {
    match button {
        MouseButton::Left => 0,
        MouseButton::Right => 1,
        MouseButton::Middle => 2,
    }
}

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
    /// When set, COSMIC pointer actions are routed to the privileged helper's
    /// absolute `EV_ABS` device instead of ydotool. Only true on COSMIC with a
    /// pointer-capable helper and detected desktop bounds.
    pub pointer_via_helper: bool,
    /// Desktop-logical bounds the helper maps onto absolute device units. Present
    /// only on the COSMIC helper-pointer route.
    pub desktop_bounds: Option<DesktopBounds>,
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

    /// True when COSMIC pointer actions go through the privileged helper's
    /// absolute `EV_ABS` device. The action layer reads this to skip the ydotool
    /// coordinate fudge and pass raw desktop-logical coordinates.
    pub fn pointer_via_helper(&self) -> bool {
        self.probe.pointer_via_helper
    }

    pub fn move_absolute(&self, x: f64, y: f64) -> Result<(), BackendError> {
        if self.probe.pointer_via_helper {
            return self.run_pointer_actions(vec![PointerAction::MoveAbsolute { x, y }]);
        }
        match self.probe.adapter {
            VirtualInputAdapterKind::PrivilegedHelper => Err(pointer_requires_ydotool_error()),
            VirtualInputAdapterKind::Ydotool => self.run_ydotool(move_absolute_args(x, y)),
        }
    }

    pub fn click(&self, button: MouseButton) -> Result<(), BackendError> {
        if self.probe.pointer_via_helper {
            let index = helper_button_index(button);
            return self.run_pointer_actions(vec![
                PointerAction::Button {
                    button: index,
                    pressed: true,
                },
                PointerAction::Settle {
                    millis: HELPER_BUTTON_HOLD_MILLIS,
                },
                PointerAction::Button {
                    button: index,
                    pressed: false,
                },
            ]);
        }
        match self.probe.adapter {
            VirtualInputAdapterKind::PrivilegedHelper => Err(pointer_requires_ydotool_error()),
            VirtualInputAdapterKind::Ydotool => {
                self.run_ydotool(["click".to_string(), click_code(button, ClickAction::Click)])
            }
        }
    }

    pub fn pointer_button(&self, button: MouseButton, pressed: bool) -> Result<(), BackendError> {
        if self.probe.pointer_via_helper {
            return self.run_pointer_actions(vec![PointerAction::Button {
                button: helper_button_index(button),
                pressed,
            }]);
        }
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
        if self.probe.pointer_via_helper {
            let index = helper_button_index(button);
            return self.run_pointer_actions(vec![
                PointerAction::MoveAbsolute { x, y },
                PointerAction::Button {
                    button: index,
                    pressed: true,
                },
                PointerAction::Settle {
                    millis: HELPER_BUTTON_HOLD_MILLIS,
                },
                PointerAction::Button {
                    button: index,
                    pressed: false,
                },
            ]);
        }
        match self.probe.adapter {
            VirtualInputAdapterKind::PrivilegedHelper => Err(pointer_requires_ydotool_error()),
            VirtualInputAdapterKind::Ydotool => {
                self.move_absolute(x, y)?;
                self.click(button)
            }
        }
    }

    pub fn pointer_mapping_details(&self, x: f64, y: f64) -> String {
        if self.probe.pointer_via_helper {
            let socket = self
                .probe
                .helper_socket_path
                .as_ref()
                .map(|path| path.display().to_string())
                .unwrap_or_else(|| "unknown".to_string());
            return match self.probe.desktop_bounds {
                Some(bounds) => {
                    let (absolute_x, absolute_y) = bounds.logical_to_absolute(x, y);
                    format!(
                        "adapter=privileged_helper_absolute socket={socket} coordinate_plane=desktop_logical requested=({x:.1},{y:.1}) emitted_absolute=({absolute_x},{absolute_y}) bounds=x:{} y:{} width:{} height:{} scale_milli:{}",
                        bounds.x, bounds.y, bounds.width, bounds.height, bounds.scale_milli
                    )
                }
                None => format!(
                    "adapter=privileged_helper_absolute socket={socket} coordinate_plane=desktop_logical requested=({x:.1},{y:.1}) bounds=missing"
                ),
            };
        }
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

    pub fn drag(
        &self,
        waypoints: &[(f64, f64)],
        step_delay: Duration,
        cancellation: Option<&CuaCancellation>,
    ) -> Result<(), BackendError> {
        if cancellation.is_some_and(CuaCancellation::is_cancelled) {
            return Err(virtual_drag_cancelled(false));
        }
        if self.probe.pointer_via_helper {
            // One batch replays the full interpolated path under a single grab.
            // The absolute device has no pointer acceleration so it tracks the
            // requested waypoints exactly; no subsampling is needed.
            let Some((first, rest)) = waypoints.split_first() else {
                return Ok(());
            };
            let step_millis = u32::try_from(step_delay.as_millis()).unwrap_or(u32::MAX);
            let mut actions = vec![
                PointerAction::MoveAbsolute {
                    x: first.0,
                    y: first.1,
                },
                PointerAction::Button {
                    button: helper_button_index(MouseButton::Left),
                    pressed: true,
                },
                PointerAction::Settle {
                    millis: HELPER_DRAG_GRAB_SETTLE_MILLIS,
                },
            ];
            for &(x, y) in rest {
                actions.push(PointerAction::MoveAbsolute { x, y });
                actions.push(PointerAction::Settle {
                    millis: step_millis,
                });
            }
            actions.push(PointerAction::Settle {
                millis: HELPER_DRAG_GRAB_SETTLE_MILLIS,
            });
            actions.push(PointerAction::Button {
                button: helper_button_index(MouseButton::Left),
                pressed: false,
            });
            return self.run_pointer_actions(actions);
        }
        match self.probe.adapter {
            VirtualInputAdapterKind::PrivilegedHelper => Err(pointer_requires_ydotool_error()),
            VirtualInputAdapterKind::Ydotool => {
                // Each ydotool move is a subprocess spawn, so cap the number of
                // intermediate motions rather than spawning one per interpolation
                // step. The first and last points are always preserved.
                let sampled = sample_drag_waypoints(waypoints, YDOTOOL_MAX_DRAG_SEGMENTS);
                let Some((first, rest)) = sampled.split_first() else {
                    return Ok(());
                };
                self.move_absolute(first.0, first.1)?;
                self.pointer_button(MouseButton::Left, true)?;
                thread::sleep(Duration::from_millis(40));
                for &(x, y) in rest {
                    if cancellation.is_some_and(CuaCancellation::is_cancelled) {
                        let _ = self.pointer_button(MouseButton::Left, false);
                        return Err(virtual_drag_cancelled(true));
                    }
                    if let Err(error) = self.move_absolute(x, y) {
                        let _ = self.pointer_button(MouseButton::Left, false);
                        return Err(error);
                    }
                    thread::sleep(step_delay);
                }
                thread::sleep(Duration::from_millis(40));
                self.pointer_button(MouseButton::Left, false)
            }
        }
    }

    pub fn scroll_vertical(&self, steps: i32) -> Result<(), BackendError> {
        if steps == 0 {
            return Ok(());
        }
        if self.probe.pointer_via_helper {
            return self.run_pointer_actions(vec![PointerAction::ScrollVertical { steps }]);
        }
        match self.probe.adapter {
            VirtualInputAdapterKind::PrivilegedHelper => Err(pointer_requires_ydotool_error()),
            VirtualInputAdapterKind::Ydotool => self.run_ydotool(scroll_vertical_args(steps)),
        }
    }

    pub fn scroll_vertical_at(&self, x: f64, y: f64, steps: i32) -> Result<(), BackendError> {
        if self.probe.pointer_via_helper {
            return self.run_pointer_actions(vec![
                PointerAction::MoveAbsolute { x, y },
                PointerAction::ScrollVertical { steps },
            ]);
        }
        match self.probe.adapter {
            VirtualInputAdapterKind::PrivilegedHelper => Err(pointer_requires_ydotool_error()),
            VirtualInputAdapterKind::Ydotool => {
                self.move_absolute(x, y)?;
                self.scroll_vertical(steps)
            }
        }
    }

    pub fn scroll_horizontal_at(&self, x: f64, y: f64, steps: i32) -> Result<(), BackendError> {
        if self.probe.pointer_via_helper {
            return Err(BackendError::new(
                BackendErrorCode::ActionUnsupportedForEnvironment,
                "horizontal scrolling is unavailable on the privileged virtual-input helper",
            ));
        }
        if steps == 0 {
            return Ok(());
        }
        self.move_absolute(x, y)?;
        let button = horizontal_scroll_button(steps);
        self.run_ydotool([
            "click".to_string(),
            "--repeat".to_string(),
            steps.abs().to_string(),
            button.to_string(),
        ])
    }

    pub fn type_text(&self, text: &str) -> Result<(), BackendError> {
        if let Some(socket_path) = self.probe.helper_socket_path.as_deref()
            && socket_is_connectable(socket_path)
        {
            match self.type_text_via_helper(socket_path, text) {
                Ok(()) => return Ok(()),
                // A partial prefix is already in the field; a ydotool retype of
                // the whole string would duplicate it, so surface the error.
                Err(HelperTypeError::AfterInjection(error)) => return Err(error),
                Err(HelperTypeError::BeforeInjection(error))
                    if self.probe.adapter != VirtualInputAdapterKind::Ydotool
                        || !self.probe.supports_keyboard() =>
                {
                    return Err(error);
                }
                Err(HelperTypeError::BeforeInjection(_)) => {}
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

    /// Type `text` through the privileged helper one character at a time.
    ///
    /// cosmic-comp's text-input pipeline drops the tail of a long single
    /// keyboard batch: a multi-character `type_text` injected as one continuous
    /// event stream reliably loses its last few characters even when the events
    /// are paced, while the same characters delivered as discrete per-character
    /// batches all land. Every character is resolved up front so an unsupported
    /// character fails before anything is injected (clean fallback, no
    /// half-typed text); each is then sent as its own helper command, and the
    /// inter-command round trip lets the compositor commit one keystroke before
    /// the next arrives. A trailing flush commits the final character on COSMIC
    /// (see [`Self::push_cosmic_input_flush`]).
    fn type_text_via_helper(&self, socket_path: &Path, text: &str) -> Result<(), HelperTypeError> {
        let resolver =
            LinuxKeyResolver::from_environment().map_err(HelperTypeError::BeforeInjection)?;
        // Resolve every character first so an unsupported character or keymap
        // failure aborts before any keystroke is injected (clean fallback).
        let mut per_character = Vec::with_capacity(text.chars().count());
        for character in text.chars() {
            let mut buffer = [0u8; 4];
            per_character.push(
                resolver
                    .text_events(character.encode_utf8(&mut buffer))
                    .map_err(HelperTypeError::BeforeInjection)?,
            );
        }
        // Detect COSMIC once (a /proc scan): it gates both the per-command
        // pacing and the commit flush, neither of which any other compositor on
        // the helper keyboard path needs.
        let cosmic = desktop_is_cosmic();
        let mut injected = false;
        for events in per_character {
            if events.is_empty() {
                continue;
            }
            // Space the per-character commands on COSMIC so the inter-command gap
            // never collapses below the compositor's commit window.
            if injected && cosmic {
                std::thread::sleep(COSMIC_KEY_COMMAND_GAP);
            }
            // The helper injects a batch before it writes its response, so a
            // failed send may already have applied that keystroke. Treat every
            // send failure as post-injection to keep the ydotool fallback from
            // duplicating an already-typed prefix; only the pre-send resolution
            // failures above stay eligible for the fallback.
            run_helper_command(socket_path, HelperCommand::KeyEvents { events })
                .map_err(HelperTypeError::AfterInjection)?;
            injected = true;
        }
        // Only flush once at least one character landed: the flush commits the
        // final character's preedit on COSMIC, and skipping it keeps an empty
        // `type_text` from injecting a stray modifier tap.
        if injected {
            let mut flush = Vec::new();
            self.push_cosmic_input_flush(&mut flush, cosmic, COSMIC_TEXT_FLUSH_KEYCODE);
            if !flush.is_empty() {
                std::thread::sleep(COSMIC_KEY_COMMAND_GAP);
                run_helper_command(socket_path, HelperCommand::KeyEvents { events: flush })
                    .map_err(HelperTypeError::AfterInjection)?;
            }
        }
        Ok(())
    }

    pub fn press_key_sequence(&self, keys: &[String]) -> Result<(), BackendError> {
        if let Some(socket_path) = self.probe.helper_socket_path.as_deref()
            && socket_is_connectable(socket_path)
        {
            match LinuxKeyResolver::from_environment()
                .and_then(|resolver| resolver.key_sequence_events(keys))
                .and_then(|mut events| {
                    self.push_cosmic_input_flush(
                        &mut events,
                        desktop_is_cosmic(),
                        COSMIC_KEY_FLUSH_KEYCODE,
                    );
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

    pub fn key_state(&self, key: &str, pressed: bool) -> Result<(), BackendError> {
        if let Some(socket_path) = self.probe.helper_socket_path.as_deref()
            && socket_is_connectable(socket_path)
        {
            let mut events =
                LinuxKeyResolver::from_environment()?.key_sequence_events(&[key.to_string()])?;
            let event = if pressed {
                events.first().cloned()
            } else {
                events.last().cloned()
            }
            .ok_or_else(|| {
                BackendError::new(
                    BackendErrorCode::InvalidRequest,
                    format!("unsupported held modifier key {key:?}"),
                )
            })?;
            events.clear();
            events.push(event);
            return run_helper_command(socket_path, HelperCommand::KeyEvents { events });
        }
        match self.probe.adapter {
            VirtualInputAdapterKind::PrivilegedHelper => Err(self.missing_helper_error()),
            VirtualInputAdapterKind::Ydotool => {
                self.require_keyboard_adapter()?;
                let events = key_sequence_events(&[key.to_string()])?;
                let event = if pressed {
                    events.first().cloned()
                } else {
                    events.last().cloned()
                }
                .ok_or_else(|| {
                    BackendError::new(
                        BackendErrorCode::InvalidRequest,
                        format!("unsupported held modifier key {key:?}"),
                    )
                })?;
                self.run_ydotool(["key".to_string(), event])
            }
        }
    }

    fn run_ydotool<I>(&self, args: I) -> Result<(), BackendError>
    where
        I: IntoIterator<Item = String>,
    {
        run_ydotool_command(&self.probe, args)
    }

    /// Append a no-op `Shift` tap to a helper keyboard batch on COSMIC.
    ///
    /// cosmic-comp's `zwp_text_input_v3` pipeline applies each key event's
    /// effect one event late: the final typed character stays in preedit until
    /// the next key arrives, and a trailing `Return` only fires the entry's
    /// activate on the following key. Every key-press event is delivered on
    /// time, so this is not a focus or delivery failure — only the committed
    /// result lags by one event. A bare `Shift` tap is a no-op (no character,
    /// no cursor movement, no chord), so terminating the batch with one pushes
    /// the last real key's effect through immediately while contributing nothing
    /// of its own. The EIS keyboard path on KDE/GNOME commits per keystroke and
    /// never reaches this code.
    ///
    /// Gated on COSMIC rather than `pointer_via_helper`: the text-input lag is a
    /// compositor property, so the flush must fire on every COSMIC helper
    /// keyboard batch even when pointer bounds detection failed (which would
    /// leave `pointer_via_helper` false while keyboard still routes through the
    /// helper). The caller passes the already-computed COSMIC flag so a
    /// multi-command `type_text` does not re-scan `/proc` per character, and the
    /// flush keycode so text and key presses can each use the safest committer.
    fn push_cosmic_input_flush(
        &self,
        events: &mut Vec<KeyEventCommand>,
        cosmic: bool,
        flush_keycode: u16,
    ) {
        if !cosmic {
            return;
        }
        events.push(KeyEventCommand {
            code: flush_keycode,
            pressed: true,
        });
        events.push(KeyEventCommand {
            code: flush_keycode,
            pressed: false,
        });
    }

    fn run_pointer_actions(&self, actions: Vec<PointerAction>) -> Result<(), BackendError> {
        let bounds = self.probe.desktop_bounds.ok_or_else(|| {
            BackendError::new(
                BackendErrorCode::ActionUnsupportedForEnvironment,
                "Linux helper-absolute pointer requires detected desktop bounds",
            )
        })?;
        let socket_path = self.helper_socket_path()?;
        run_helper_command(
            socket_path,
            HelperCommand::PointerActions { bounds, actions },
        )
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

fn virtual_drag_cancelled(after_press: bool) -> BackendError {
    if after_press {
        BackendError::new(
            BackendErrorCode::CuaActionOutcomeUnknown,
            "the CUA drag was cancelled after pointer-down; virtual pointer release completed",
        )
    } else {
        BackendError::new(
            BackendErrorCode::InvalidRequest,
            "the CUA turn was cancelled",
        )
    }
}

pub fn probe_virtual_input() -> Result<VirtualInputProbe, VirtualInputUnavailable> {
    let helper_socket_path = configured_helper_socket_path();
    let helper_capabilities = helper_capabilities(&helper_socket_path);
    let helper_available = helper_capabilities.is_some();
    let ydotool_path = find_executable("ydotool");
    let socket_path = configured_socket_path();

    // COSMIC has no RemoteDesktop portal and no libei, and ydotool's EV_REL
    // device gets distorted by libinput pointer acceleration. When the helper
    // advertises an absolute pointer device and we can detect bounds, route
    // COSMIC pointer actions through it for linear (acceleration-free) mapping.
    // KDE/GNOME never reach this code (they use the portal/EIS backend).
    let pointer_capable_helper = helper_capabilities
        .as_ref()
        .is_some_and(|capabilities| capabilities.pointer);
    let desktop_bounds = if desktop_is_cosmic() && pointer_capable_helper {
        detect_desktop_bounds()
    } else {
        None
    };
    let pointer_via_helper = desktop_bounds.is_some();

    let Some(ydotool_path) = ydotool_path else {
        if helper_available {
            return Ok(helper_keyboard_only_probe(
                helper_socket_path,
                None,
                socket_path,
                pointer_via_helper,
                desktop_bounds,
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
                    pointer_via_helper,
                    desktop_bounds,
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
                    pointer_via_helper,
                    desktop_bounds,
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
        pointer_via_helper,
        desktop_bounds,
    })
}

fn helper_keyboard_only_probe(
    helper_socket_path: PathBuf,
    ydotool_path: Option<PathBuf>,
    socket_path: Option<PathBuf>,
    pointer_via_helper: bool,
    desktop_bounds: Option<DesktopBounds>,
) -> VirtualInputProbe {
    VirtualInputProbe {
        adapter: VirtualInputAdapterKind::PrivilegedHelper,
        coordinate_plane: VirtualInputCoordinatePlane::DesktopLogical,
        ydotool_path,
        socket_path,
        helper_socket_path: Some(helper_socket_path),
        pointer_via_helper,
        desktop_bounds,
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

/// Read timeout for a single helper command. The helper sleeps through every
/// `Settle` (and may add `UINPUT_SETTLE_DELAY` for a device rebuild) before it
/// writes the response, so a long interpolated drag can run for several seconds.
/// Size the timeout to that work plus the base allowance instead of a fixed
/// constant, or the client reports a false failure on a drag the helper is
/// still completing.
fn helper_command_read_timeout(command: &HelperCommand) -> Duration {
    match command {
        HelperCommand::PointerActions { actions, .. } => {
            let settle_millis: u64 = actions
                .iter()
                .filter_map(|action| match action {
                    PointerAction::Settle { millis } => Some(u64::from(*millis)),
                    _ => None,
                })
                .sum();
            HELPER_COMMAND_TIMEOUT
                + Duration::from_millis(settle_millis)
                + HELPER_DEVICE_REBUILD_ALLOWANCE
        }
        _ => HELPER_COMMAND_TIMEOUT,
    }
}

fn run_helper_command(path: &Path, command: HelperCommand) -> Result<(), BackendError> {
    let response = exchange_helper_command(path, command)?;
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

fn exchange_helper_command(
    path: &Path,
    command: HelperCommand,
) -> Result<HelperResponse, BackendError> {
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
        .set_read_timeout(Some(helper_command_read_timeout(&command)))
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
    parse_response_line(response.trim_end()).map_err(|error| {
        BackendError::new(
            BackendErrorCode::ActionUnsupportedForEnvironment,
            format!("failed to parse privileged input helper response: {error}"),
        )
    })
}

fn helper_io_error(context: &str, error: std::io::Error) -> BackendError {
    BackendError::new(
        BackendErrorCode::ActionUnsupportedForEnvironment,
        format!("failed to {context}: {error}"),
    )
}

fn helper_capabilities(path: &Path) -> Option<sky_cua_input_helper::protocol::HelperCapabilities> {
    if !socket_is_connectable(path) {
        return None;
    }
    let response = exchange_helper_command(path, HelperCommand::Hello).ok()?;
    response.ok.then_some(response.capabilities).flatten()
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

/// Maximum number of motion segments the ydotool drag adapter emits. Each move
/// is a subprocess spawn, so an interpolated path is subsampled to this many
/// segments to keep drags responsive while still feeding the compositor enough
/// intermediate motion for slider/DnD tracking.
const YDOTOOL_MAX_DRAG_SEGMENTS: usize = 16;

/// Subsample a waypoint path down to at most `max_segments` segments while
/// always preserving the exact first and last points (press and release land on
/// the requested coordinates).
fn sample_drag_waypoints(waypoints: &[(f64, f64)], max_segments: usize) -> Vec<(f64, f64)> {
    if max_segments == 0 || waypoints.len() <= max_segments + 1 {
        return waypoints.to_vec();
    }
    let last_index = waypoints.len() - 1;
    let mut sampled = Vec::with_capacity(max_segments + 1);
    for step in 0..=max_segments {
        sampled.push(waypoints[step * last_index / max_segments]);
    }
    // Guard against integer rounding leaving the destination off the end.
    if let Some(last) = sampled.last_mut() {
        *last = waypoints[last_index];
    }
    sampled
}

fn move_absolute_args(x: f64, y: f64) -> Vec<String> {
    vec![
        "mousemove".to_string(),
        "--absolute".to_string(),
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
                let _ = child.wait();
                return Err(BackendError::new(
                    BackendErrorCode::ActionUnsupportedForEnvironment,
                    format!("ydotool command timed out: ydotool {}", args.join(" ")),
                ));
            }
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
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

/// True when the live desktop is COSMIC, gating the helper-absolute pointer
/// route. KDE/GNOME report different desktop names and use the portal/EIS
/// backend, so they never enable this path.
fn desktop_is_cosmic() -> bool {
    // The session env vars are the cheap path, but the service is not always
    // launched inside the COSMIC session (e.g. via systemd-run), so they can be
    // empty. Fall back to detecting the running cosmic-comp compositor, which is
    // an authoritative COSMIC signal independent of the launch environment.
    desktop_name_is_cosmic(env::var("XDG_CURRENT_DESKTOP").ok().as_deref())
        || desktop_name_is_cosmic(env::var("DESKTOP_SESSION").ok().as_deref())
        || cosmic_compositor_running()
}

/// True when a `cosmic-comp` process is running for the current user, i.e. the
/// active session is COSMIC. Independent of session environment variables.
fn cosmic_compositor_running() -> bool {
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return false;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        if !name
            .to_string_lossy()
            .bytes()
            .all(|byte| byte.is_ascii_digit())
        {
            continue;
        }
        if let Ok(comm) = std::fs::read_to_string(entry.path().join("comm"))
            && comm.trim() == "cosmic-comp"
        {
            return true;
        }
    }
    false
}

fn desktop_name_is_cosmic(value: Option<&str>) -> bool {
    value
        .unwrap_or_default()
        .split([':', ';', ','])
        .any(|part| part.trim().eq_ignore_ascii_case("cosmic"))
}

/// Resolve desktop-logical bounds for the COSMIC helper-absolute pointer route.
/// Order: explicit `SKY_CUA_VIRTUAL_INPUT_*` env override, then `cosmic-randr
/// list` (scoped to COSMIC by the caller), then `None`.
///
// TODO(cosmic multi-output): this resolves a single output only. Multi-output
// COSMIC should plumb `environment.displays` (point-in-rect selection) through
// to the helper bounds; tracked as an explicit follow-up.
fn detect_desktop_bounds() -> Option<DesktopBounds> {
    desktop_bounds_from_env().or_else(desktop_bounds_from_cosmic_randr)
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

#[cfg(test)]
mod tests {
    use std::os::unix::net::UnixListener;
    use std::path::PathBuf;

    use super::{
        ClickAction, LinuxVirtualInput, VirtualInputAdapterKind, VirtualInputCoordinatePlane,
        VirtualInputProbe, click_code, desktop_name_is_cosmic, helper_keyboard_only_probe,
        key_sequence_events, move_absolute_args, parse_cosmic_randr_bounds, parse_localectl_status,
        parse_scale_milli, parse_setxkbmap_query, preferred_socket_path, scroll_vertical_args,
        type_text_args,
    };
    use crate::portal::remote_desktop::MouseButton;
    use sky_cua_input_helper::uinput::DesktopBounds;

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
    fn helper_read_timeout_scales_with_pointer_settles() {
        use super::{
            HELPER_COMMAND_TIMEOUT, HELPER_DEVICE_REBUILD_ALLOWANCE, helper_command_read_timeout,
        };
        use sky_cua_input_helper::protocol::{HelperCommand, PointerAction};
        use std::time::Duration;

        let drag = HelperCommand::PointerActions {
            bounds: DesktopBounds {
                x: 0,
                y: 0,
                width: 1280,
                height: 800,
                scale_milli: 1000,
            },
            actions: vec![
                PointerAction::Settle { millis: 40 },
                PointerAction::MoveAbsolute { x: 1.0, y: 1.0 },
                PointerAction::Settle { millis: 5000 },
                PointerAction::Settle { millis: 40 },
            ],
        };
        // Base timeout plus every Settle plus a device-rebuild allowance, so a
        // multi-second drag does not trip a fixed read timeout mid-batch.
        assert_eq!(
            helper_command_read_timeout(&drag),
            HELPER_COMMAND_TIMEOUT + Duration::from_millis(5080) + HELPER_DEVICE_REBUILD_ALLOWANCE
        );

        // Keyboard batches carry no settles and keep the base timeout.
        let keys = HelperCommand::KeyEvents { events: Vec::new() };
        assert_eq!(helper_command_read_timeout(&keys), HELPER_COMMAND_TIMEOUT);
    }

    #[test]
    fn builds_ydotool_pointer_and_text_argv_without_shell_escaping() {
        assert_eq!(
            move_absolute_args(10.4, 20.6),
            vec!["mousemove", "--absolute", "10", "21"]
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
                pointer_via_helper: false,
                desktop_bounds: None,
            },
        };

        assert!(input.type_text("hello").is_err());
        assert!(input.probe.supports_keyboard());
    }

    #[test]
    fn helper_pointer_action_requires_ydotool_without_absolute_route() {
        let probe = helper_keyboard_only_probe(
            PathBuf::from("/run/sky-cua/input-helper.sock"),
            None,
            None,
            false,
            None,
        );
        assert_eq!(probe.adapter, VirtualInputAdapterKind::PrivilegedHelper);
        assert!(probe.supports_keyboard());
        assert!(!probe.pointer_via_helper);

        let input = LinuxVirtualInput { probe };
        let error = input
            .move_absolute(10.0, 20.0)
            .expect_err("helper without the absolute route should reject pointer events");

        assert!(error.message.contains("ydotool"));
    }

    #[test]
    fn helper_absolute_route_reports_mapped_pointer_details() {
        let probe = helper_keyboard_only_probe(
            PathBuf::from("/run/sky-cua/input-helper.sock"),
            None,
            None,
            true,
            Some(DesktopBounds {
                x: 0,
                y: 0,
                width: 1600,
                height: 1200,
                scale_milli: 1250,
            }),
        );
        assert!(probe.pointer_via_helper);

        let input = LinuxVirtualInput { probe };
        let details = input.pointer_mapping_details(194.0, 314.0);
        assert!(details.contains("adapter=privileged_helper_absolute"));
        assert!(details.contains("emitted_absolute=(243,393)"));
        assert!(details.contains("scale_milli:1250"));
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
    fn parses_cosmic_randr_scale_for_absolute_bounds() {
        let output = "\u{1b}[1mVirtual-1\u{1b}[0m \u{1b}[1;32m(enabled)\u{1b}[0m\n  Position: 0,0\n  Scale: 125%\n  Modes:\n    1600x1200 @ 60.000 Hz (current)\n";

        let bounds = parse_cosmic_randr_bounds(output).unwrap();
        assert_eq!(bounds.scale_milli, 1250);
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
    fn cosmic_desktop_detection_is_scoped_to_cosmic_names() {
        assert!(desktop_name_is_cosmic(Some("COSMIC")));
        assert!(desktop_name_is_cosmic(Some("pop:COSMIC")));
        assert!(desktop_name_is_cosmic(Some("gnome;cosmic")));
        assert!(!desktop_name_is_cosmic(Some("KDE")));
        assert!(!desktop_name_is_cosmic(Some("GNOME")));
        assert!(!desktop_name_is_cosmic(None));
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
