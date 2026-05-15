# Linux Virtual Input Backend ExecPlan

This ExecPlan is a living document. Keep `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` current as work proceeds.

## Purpose

Implement a Linux virtual input backend for `sky-cua` so Computer Use can drive Wayland desktops that do not expose `org.freedesktop.portal.RemoteDesktop`, including COSMIC, Hyprland, and other compositor stacks where portal input injection is unavailable. The agent-facing MCP contract must not change: agents continue to request clicks, drags, scrolls, typing, and key presses in the same coordinate system they already use. The native runtime detects the best available capability, translates coordinates correctly for display scale and monitor layout, and chooses the right backend itself.

After this plan is complete, the COSMIC Wayland session in the `testing-vm` can run the visible full-screen computer-use smoke suite where the agent clicks a button, drags a target, and scrolls a list. The backend selected for COSMIC is `LinuxVirtualInput`, using a direct absolute `/dev/uinput` pointer adapter when `/dev/uinput` is writable and desktop bounds are detected, with `ydotool` retained as the subprocess fallback and keyboard/text adapter. KDE and GNOME sessions continue to prefer the compositor RemoteDesktop portal when it is available. X11 sessions continue to prefer XTest.

The implementation intentionally exposes one top-level backend to the rest of the runtime: `LinuxVirtualInput`. `ydotool`, direct `/dev/uinput`, and future compositor-specific adapters are implementation details below that backend.

## Progress

- [x] Established that COSMIC Wayland does not currently expose the RemoteDesktop portal interface in the VM, while ScreenCast and Screenshot are present.
- [x] Established that `ydotoold` is installed and can move/click the pointer in the COSMIC VM through `/run/user/1000/.ydotool_socket`.
- [x] Confirmed that `sky-cua` has ydotool readiness checks in setup/doctor paths, but no ydotool or uinput execution backend.
- [x] Confirmed that current Linux backend selection only returns `PortalRemoteDesktop`, `XTest`, or `None`.
- [x] Patched session detection so SSH/TTY automation with a valid `WAYLAND_DISPLAY` is treated as Wayland instead of being misclassified as X11 because `DISPLAY=:0` exists.
- [x] Created this ExecPlan.
- [x] (2026-05-15) Added the public `InputBackendKind::LinuxVirtualInput` backend kind and a Linux-only `virtual_input` module with an internal `ydotool` adapter model.
- [x] (2026-05-15) Implemented backend auto-detection for Wayland sessions without RemoteDesktop and added the developer override values `auto`, `portal`, `x11`, `linux-virtual`, and `none`.
- [x] (2026-05-15) Implemented the first display-scaling-aware coordinate contract for `LinuxVirtualInput`: screenshot pixels map to desktop logical coordinates through `capture.pixel_size` and `capture.logical_rect`, including monitor offsets, and missing snapshot metadata fails closed.
- [x] (2026-05-15) Implemented the first `ydotool` adapter for pointer movement, click, drag, scroll, typing, and key press.
- [x] (2026-05-15) Routed Linux click, secondary click, scroll, drag, type_text, press_key, and the heuristics-backed set_value fallback through `LinuxVirtualInput`.
- [x] (2026-05-15) Added unit tests for backend selection, command vector construction, key events, click codes, screenshot-to-desktop-logical conversion, and missing logical-rect failure.
- [x] (2026-05-15) Proved that `ydotool` is not acceptable as the precise COSMIC pointer adapter: its virtual device is relative-only in `/proc/bus/input/devices`, and `ydotool mousemove --absolute` landed at accelerated/doubled coordinates in the VM.
- [x] (2026-05-15) Implemented the direct `/dev/uinput` absolute pointer adapter behind `LinuxVirtualInput`, including `cosmic-randr`/`xrandr`/environment bounds detection, click, drag, and hi-res wheel scroll.
- [x] (2026-05-15) Added focused parser/unit coverage for COSMIC and XRandR desktop-bounds detection plus the existing virtual input command/mapping tests.
- [x] (2026-05-15) Added VM smoke proof on COSMIC Wayland. Artifact `/workspace/artifacts/gui-desktop-smoke/wayland-pointer/20260515T091758Z` proves click, drag, and scroll on the fullscreen GTK fixture through `LinuxVirtualInput`.
- [x] (2026-05-15) Extended the real Wayland pointer smoke to cover `type_text` and `press_key`, then proved COSMIC text/key through the ydotool sub-adapter. Artifact `/workspace/artifacts/gui-desktop-smoke/wayland-pointer/20260515T092606Z` proves click, drag, scroll, type, and Enter submission on the fullscreen GTK fixture.
- [x] (2026-05-15) Proved COSMIC scaled-coordinate behavior at 1600x1200 with 125% display scale. Artifact `/workspace/artifacts/gui-desktop-smoke/wayland-pointer/20260515T093335Z` proves click, drag, scroll, type, and Enter submission after direct uinput maps desktop logical coordinates into physical absolute device coordinates.
- [x] (2026-05-15) Added and proved the repeatable VM profile `wayland-pointer-scaled`, which configures COSMIC scale, runs the full input smoke, and restores 1280x800 at 100% scale afterward. Artifact `/workspace/artifacts/gui-desktop-smoke/wayland-pointer/20260515T093737Z`.
- [x] (2026-05-15) Updated operator documentation and the ongoing continuity notes for the implemented local backend slice.

## Surprises & Discoveries

COSMIC currently has an xdg-desktop-portal implementation for screenshot and screen cast, but not RemoteDesktop. That means portal capture can work while portal input injection cannot. This is not a `sky-cua` permission bug; it is a compositor/portal capability gap.

The existing setup already knows about `ydotool`, `ydotoold`, `/dev/uinput`, and the per-user socket, but those checks stop at readiness reporting. They do not create an input backend.

The VM automation can run a Wayland session over SSH where `XDG_SESSION_TYPE=tty`, `WAYLAND_DISPLAY=wayland-1`, and `DISPLAY=:0` are all present. Backend detection must prefer a live Wayland display in that shape, otherwise COSMIC smoke runs are incorrectly routed toward X11.

The exact coordinate plane consumed by `ydotool mousemove --absolute` was tested in the COSMIC VM and rejected for pointer work. The ydotool virtual device appears as relative-only, and absolute motion was affected by pointer acceleration instead of landing in the requested coordinate plane.

The direct uinput pointer adapter creates an absolute tablet-style device and treats requested points as desktop logical coordinates within detected desktop bounds. COSMIC accepted this device for pointer motion, buttons, and wheel events in the headed VM smoke at 1x scale and at 125% display scale. At scale, the adapter must multiply logical points by the output scale before emitting absolute uinput values because the absolute device range is in physical output pixels.

COSMIC direct uinput scroll needed both `REL_WHEEL_HI_RES` and `REL_WHEEL`, with the sign inverted from the portal helper's discrete scroll direction. Sending only the ordinary wheel event did not satisfy the visible smoke.

`ydotool` command construction now inserts `--` before coordinate, wheel, and text payload arguments. This keeps negative wheel values and text beginning with a dash from being interpreted as ydotool flags, and the argv is covered by unit tests rather than shell-escaped strings.

`cosmic-randr list` exposes the current output position and mode in the VM, and is the preferred COSMIC bounds source for the direct uinput absolute device. Environment overrides remain available as a test escape hatch through `SKY_CUA_VIRTUAL_INPUT_X/Y/WIDTH/HEIGHT`; `xrandr` is a fallback for X11-shaped sessions.

The first scaled COSMIC run after adding scale parsing proved click and drag but failed scroll because the old fixture scroll target was too low in the oversized fullscreen GTK allocation. The fixture now records `scroll_safe`, an upper scroller target that remains visible under fractional scaling.

## Decision Log

Decision: expose one top-level Linux fallback backend named `LinuxVirtualInput`.

Rationale: the rest of the runtime should only care that Linux native input is available. `ydotool`, direct uinput, libei/EIS, or any later compositor bridge are adapter details. This keeps MCP tools, action routing, diagnostics, and tests stable while allowing the implementation underneath to improve.

Decision: backend priority is automatic by default.

Rationale: the agent should not pass a backend name. On Wayland, use `PortalRemoteDesktop` when the compositor exposes it and the existing portal flow is usable. When RemoteDesktop is absent and a virtual input adapter is available, use `LinuxVirtualInput`. On X11, keep `XTest` as the first choice. If no supported input path exists, report `None` with structured diagnostics. A developer override may exist for tests, but `auto` remains the default.

Decision: do not silently bypass an explicit portal denial in the first implementation.

Rationale: there is a difference between "the compositor does not offer RemoteDesktop" and "a human denied a RemoteDesktop permission prompt." The first should fall back to virtual input. The second should remain explicit until the product decision is made. This avoids accidentally treating virtual input as a consent bypass.

Decision: the coordinate contract is agent-transparent and backend-specific only at the native boundary.

Rationale: agents operate against model-visible screenshots and stable tool schemas. They should not know whether the runtime is using portal RemoteDesktop, XTest, ydotool, direct uinput, or Windows SendInput. The native runtime owns the mapping from screenshot pixels to the backend coordinate plane.

Decision: `LinuxVirtualInput` prefers direct absolute `/dev/uinput` for pointer actions when available, and falls back to `ydotool` otherwise.

Rationale: live COSMIC proof showed that `ydotool`'s pointer device is relative-only and not coordinate-stable enough for screenshot-targeted actions. Direct uinput gives the runtime an absolute device with explicit bounds and produced the first passing COSMIC click/drag/scroll smoke. `ydotool` remains useful for keyboard/text actions and as a lower-priority fallback when direct uinput is not available.

Decision: snapshot-based `LinuxVirtualInput` actions fail closed when capture metadata lacks a positive desktop-logical `logical_rect` or a nonzero `pixel_size`.

Rationale: the agent supplies screenshot pixels when it acts against a snapshot. Without the capture-to-desktop mapping metadata, passing those pixels straight to `ydotool` would click the wrong place on scaled displays or offset monitors. A specific `InvalidRequest` is safer than a misleading physical action.

Date/Author: 2026-05-15 / Codex.

## Coordinate Contract

The agent-facing contract remains unchanged:

1. If an action includes a snapshot id, coordinates are in model-visible screenshot pixels for that snapshot.
2. If an action does not include a snapshot id, coordinates are interpreted as current desktop input coordinates, matching the existing native runtime behavior.
3. Tool schemas do not expose backend, compositor, monitor scale, or adapter parameters.
4. Tool results and diagnostics may report which backend was actually selected, but this is observation, not an input requirement.

The native runtime has three relevant coordinate spaces:

`StreamPixels` are the pixels in the image sent to the model. These are the coordinates agents naturally target after seeing a screenshot.

`StreamLogical` are logical coordinates inside the compositor stream or portal session. `PortalRemoteDesktop` expects this space for absolute pointer motion tied to a portal screen-cast stream.

`DesktopLogical` are global desktop coordinates in the compositor's logical coordinate plane. This is the intended initial contract for `LinuxVirtualInput`, subject to adapter calibration. It is also the correct conceptual space for a display-scaling-aware fallback: a 3840 by 2160 physical image of a 2x scaled 1920 by 1080 output maps back to 1920 by 1080 logical desktop coordinates.

For snapshot-based actions, the default mapping to `DesktopLogical` is:

    desktop_x = logical_rect.x + screenshot_x / pixel_size.width * logical_rect.width
    desktop_y = logical_rect.y + screenshot_y / pixel_size.height * logical_rect.height

Where:

- `pixel_size` is the size of the screenshot image visible to the model.
- `logical_rect` is the captured region in desktop logical coordinates.
- `logical_rect.x` and `logical_rect.y` include monitor offset in the virtual desktop.
- `logical_rect.width` and `logical_rect.height` represent the logical size of the captured output or region.

This handles fractional or integer display scaling as long as capture metadata is correct. Examples:

- A 2x scaled 3840 by 2160 screenshot of a 1920 by 1080 logical output maps screenshot point `(1920, 1080)` to desktop logical point `(960, 540)`.
- A captured second monitor at logical rect `(1920, 0, 1280, 720)` maps screenshot midpoint `(640, 360)` in a 1280 by 720 model image to desktop logical point `(2560, 360)`.
- A fractional scaled output is handled the same way. The runtime uses ratios from actual capture metadata, not hardcoded scale guesses.

If an adapter proves that it consumes desktop physical pixels instead of desktop logical coordinates, the adapter must declare `DesktopPhysical` and the native boundary converts from `DesktopLogical` to `DesktopPhysical` using display metadata. The runtime must not guess this silently. The COSMIC VM proof must include a scaling test before claiming the coordinate contract is fully validated.

If the runtime cannot obtain a `logical_rect` for a snapshot-based action on `LinuxVirtualInput`, it must fail with a specific diagnostic rather than pretending screenshot pixels are desktop coordinates. A wrong click is worse than an honest unsupported error.

## Implementation Plan

### 1. Model the backend without leaking adapters

Modify `crates/sky-cua-platform/src/model.rs`:

- Add `InputBackendKind::LinuxVirtualInput`.
- Preserve existing serialized names for current backends.
- Add serde and debug coverage if existing tests or snapshots cover backend kinds.

Do not add `Ydotool` as a top-level `InputBackendKind`. If diagnostics need adapter detail, add that as a Linux-specific diagnostic field or message, not as the action routing contract.

Expected result: any diagnostic or setup output can say the selected input backend is `LinuxVirtualInput`, while deeper Linux logs can say the adapter is `ydotool`.

### 2. Add a Linux virtual input module

Create a Linux-only module, likely `crates/sky-cua-linux/src/virtual_input.rs`.

The module owns:

- Adapter probing.
- Adapter selection.
- Command construction and execution for `ydotool`.
- A trait or small enum that hides adapter details from `backend.rs`.
- Coordinate plane declaration for the selected adapter.

Suggested internal model:

    enum VirtualInputAdapterKind {
        Ydotool,
        DirectUinput,
    }

    enum VirtualInputCoordinatePlane {
        DesktopLogical,
        DesktopPhysical,
    }

    struct VirtualInputProbe {
        adapter: VirtualInputAdapterKind,
        coordinate_plane: VirtualInputCoordinatePlane,
        ydotool_path: Option<PathBuf>,
        socket_path: Option<PathBuf>,
    }

    trait VirtualInputAdapter {
        fn kind(&self) -> VirtualInputAdapterKind;
        fn coordinate_plane(&self) -> VirtualInputCoordinatePlane;
        fn move_absolute(&self, point: DesktopPoint) -> Result<()>;
        fn click(&self, button: MouseButton) -> Result<()>;
        fn button(&self, button: MouseButton, pressed: bool) -> Result<()>;
        fn scroll(&self, delta_x: f64, delta_y: f64) -> Result<()>;
        fn type_text(&self, text: &str) -> Result<()>;
        fn press_key(&self, key: KeySpec) -> Result<()>;
    }

This does not need to be over-abstracted on day one. A small enum with methods is fine if it matches the surrounding crate style better.

### 3. Probe ydotool readiness

Implement `probe_virtual_input()` so backend selection can answer "can we inject input here?" cheaply and deterministically.

For `ydotool`, check:

- `ydotool` executable is available.
- A usable ydotool socket exists. Prefer `$YDOTOOL_SOCKET` when set, otherwise check `/run/user/$UID/.ydotool_socket`.
- The socket can be connected or a harmless ydotool command can be run with a tight timeout.
- `/dev/uinput` state is included in diagnostics, but direct access to `/dev/uinput` is not required when `ydotoold` is already running and owns the device.

The probe should return a structured reason when unavailable:

- executable missing
- socket missing
- socket present but unusable
- daemon not running
- command timed out
- unsupported platform/session

Reuse existing doctor/setup logic where practical. If that logic currently lives in scripts, do not duplicate too much in Rust; extract a shared Rust probe only if it is actually used by runtime and doctor. Keep the first implementation focused.

### 4. Update backend auto-detection

Modify `crates/sky-cua-linux/src/env_probe.rs` and the Linux backend initialization path.

Desired selection:

- X11 with XTest available: `XTest`.
- Wayland with RemoteDesktop portal available and not explicitly disabled: `PortalRemoteDesktop`.
- Wayland without RemoteDesktop but with virtual input available: `LinuxVirtualInput`.
- Unsupported or unavailable: `None`.

Optional for later:

- X11 without XTest but with virtual input available may fall back to `LinuxVirtualInput`. Do not make this part of the first acceptance gate unless tests prove coordinate behavior under X11.

Add an operator override only for diagnostics and tests:

- `SKY_CUA_INPUT_BACKEND=auto`
- `SKY_CUA_INPUT_BACKEND=portal`
- `SKY_CUA_INPUT_BACKEND=x11`
- `SKY_CUA_INPUT_BACKEND=linux-virtual`
- `SKY_CUA_INPUT_BACKEND=none`

The default must be `auto`. The MCP tool schema must not gain a backend selector.

### 5. Implement display-scaling-aware coordinate mapping

Review and extend:

- `crates/sky-cua-linux/src/coords.rs`
- `crates/sky-cua-linux/src/backend.rs`

Current coordinate helpers already separate `StreamPixels`, `StreamLogical`, and `DesktopLogical`. Extend that structure instead of adding ad hoc conversions near command execution.

Add or update helpers so action routing asks:

- Portal RemoteDesktop: give me the stream logical point for this action.
- XTest: give me the X11 desktop/root point for this action.
- LinuxVirtualInput: give me the desktop logical point for this action, then let the adapter convert if it consumes physical pixels.

Tests to add:

- 1x single-monitor screenshot maps point to same logical coordinate.
- 2x scaled screenshot maps physical/model midpoint to logical midpoint.
- fractional scale maps by ratio, not by integer scale.
- multi-monitor offset is preserved.
- missing logical rect for snapshot-based `LinuxVirtualInput` actions returns a structured error.
- snapshotless current-coordinate actions keep existing behavior.

The unit tests should exercise pure mapping functions without requiring a compositor.

### 6. Implement ydotool pointer actions

Implement ydotool subprocess execution with:

- Tight timeout.
- Clean stderr capture.
- Structured error mapping.
- No shell string interpolation.
- Integer rounding at the adapter boundary, not earlier in coordinate conversion.

Pointer move:

    ydotool mousemove --absolute <x> <y>

Left click:

    ydotool click 0xC0

Button down/up for drag:

    ydotool click 0x40
    ydotool click 0x80

Before finalizing right/middle click and scroll, verify the exact ydotool button codes in the VM and capture the command transcript in this plan or `NOTES.md`. Do not assume wheel behavior from memory. If ydotool does not provide reliable scroll events, implement scroll with direct uinput earlier than planned or return a specific unsupported diagnostic for scroll until direct uinput exists. The visible smoke suite requires scroll, so this must be resolved before declaring COSMIC complete.

Drag behavior:

- Move to start.
- Button down.
- Move through a small number of interpolated points if the existing backend does that; otherwise follow existing backend semantics.
- Button up.

Click behavior:

- Move absolute to target.
- Click requested button.

Scroll behavior:

- Move to target if target coordinates are present.
- Emit vertical/horizontal wheel events through the selected adapter.
- Preserve existing action semantics for scroll direction and amount.

### 7. Implement keyboard and text actions

Implement the minimal key and text set needed by the current Computer Use action surface:

- Type text.
- Press key.
- Key combinations used by existing tests and fallback flows.

For ydotool:

- Prefer `ydotool type` for plain text.
- Use explicit key codes for non-text keys.
- Keep a local mapping table from action key names to Linux input event codes.

Add tests for:

- command vector construction for plain text
- command vector construction for Enter, Escape, Tab, Backspace, arrows, PageUp, PageDown
- modifier combinations if the action surface supports them

Avoid putting shell-escaped strings into tests. Assert argv arrays.

### 8. Route actions through the new backend

Modify `crates/sky-cua-linux/src/backend.rs`.

For `InputBackendKind::LinuxVirtualInput`:

- Build or retrieve the selected adapter.
- Convert the action point through the new coordinate helper.
- Execute pointer, scroll, keyboard, and text actions through the adapter.
- Return action outcomes consistent with the existing backend outcomes.
- Include backend and adapter details in diagnostics.

Unsupported actions should fail with an action-specific error that names the selected backend and adapter. Do not fall through to generic "no physical input backend" once `LinuxVirtualInput` is selected.

### 9. Update doctor, setup, and provisioning diagnostics

Update Rust and Python diagnostics so the operator can understand why `LinuxVirtualInput` was or was not selected.

Expected doctor shape:

- Session: Wayland, X11, or unsupported.
- Portal RemoteDesktop: available, missing, denied, or untested.
- Linux virtual input: available or unavailable.
- Adapter: ydotool when selected.
- ydotool executable path.
- ydotool socket path and status.
- ydotoold service status when available.
- `/dev/uinput` permissions as a supporting diagnostic.
- Selected input backend.

Update provisioning only if needed. The current VM provisioner already installs and enables `ydotool`; this plan should preserve that and make the runtime consume it.

### 10. Preserve the VM as production-like test surface

The VM remains a clean runtime machine. Build binaries on the host, push the latest build into the VM, then run smoke tests there.

Do not add embedded X servers to the VM smoke flow. X11 tests should run in an actual X11 session. Wayland tests should run in a real Wayland session for the selected desktop.

The COSMIC target is:

- Arch-based `testing-vm`
- Wayland COSMIC session
- portals installed for each desktop environment
- `ydotoold` enabled
- latest host-built `sky-cua` binaries copied into the VM
- headed viewer available so the user can watch the smoke run

## Validation Plan

### Local Rust checks

Run focused checks first:

    cargo fmt --check
    cargo test -p sky-cua-platform
    cargo test -p sky-cua-linux env_probe
    cargo test -p sky-cua-linux coords
    cargo test -p sky-cua-linux virtual_input

If shared contracts changed broadly, run:

    cargo test

### Python and provisioning checks

If scripts or VM provisioning changed:

    uv run ruff format --check scripts
    uv run ruff check scripts
    uv run basedpyright
    uv run pytest scripts

If only VM docs changed, executable Python validation is optional, but any changed script must pass the Python checks.

### Manual adapter proof in COSMIC VM

The original ydotool pointer calibration failed and is recorded as negative proof:

- `/workspace/artifacts/gui-desktop-smoke/cosmic-ydotool-raw/20260515T090442Z`
- `/workspace/artifacts/gui-desktop-smoke/cosmic-ydotool-raw/20260515T090527Z-scaled`
- `/workspace/artifacts/gui-desktop-smoke/cosmic-ydotool-raw/20260515T090628Z-after-session-restart`
- `/workspace/artifacts/gui-desktop-smoke/cosmic-ydotool-raw/20260515T090719Z-relative`

The accepted raw pointer proof is direct uinput:

- `/workspace/artifacts/gui-desktop-smoke/cosmic-uinput-raw/20260515T091042Z-absolute`
- `/workspace/artifacts/gui-desktop-smoke/cosmic-uinput-raw/20260515T091632Z-scroll`

The first artifact proves absolute move plus click; the second proves scroll with both high-resolution and ordinary wheel events.

### Coordinate proof in COSMIC VM

Run the visible smoke target at 1x scale:

    python3 scripts/run_gui_testing_vm_smoke.py --desktop-env COSMIC --profile computer-use

Accepted 1x pointer-only artifact:

    /workspace/artifacts/gui-desktop-smoke/wayland-pointer/20260515T091758Z

Accepted 1x pointer plus text/key artifact:

    /workspace/artifacts/gui-desktop-smoke/wayland-pointer/20260515T092606Z

Accepted scaled pointer plus text/key artifacts:

    /workspace/artifacts/gui-desktop-smoke/wayland-pointer/20260515T093335Z
    /workspace/artifacts/gui-desktop-smoke/wayland-pointer/20260515T093737Z

The repeatable scaled profile is:

    python3 scripts/run_gui_testing_vm_smoke.py --desktop-env COSMIC --profile wayland-pointer-scaled

The profile applies `cosmic-randr mode --scale 1.25 Virtual-1 1600 1200`, runs the smoke, then restores `cosmic-randr mode --scale 1.0 Virtual-1 1280 800` on exit.

If this must be repeated manually, configure a scaled display and repeat the pointer-position proof:

    python3 scripts/run_gui_testing_vm_smoke.py --desktop-env COSMIC --profile wayland-pointer-scaled

The scaled proof must demonstrate that a model-visible screenshot point lands on the intended logical desktop target. Artifact `20260515T093737Z` is the current accepted repeatable-profile proof.

### Full visible smoke acceptance

The acceptance test is the headed VM run where the user can watch the full-screen computer-use smoke app:

- click a button
- drag a target
- scroll a list
- report pass/fail artifacts

Expected artifact shape:

- `artifacts/gui-desktop-smoke/<scenario>/<timestamp>/screenshot-before.png`
- `artifacts/gui-desktop-smoke/<scenario>/<timestamp>/click-result.json`
- `artifacts/gui-desktop-smoke/<scenario>/<timestamp>/drag-result.json`
- `artifacts/gui-desktop-smoke/<scenario>/<timestamp>/scroll-result.json`
- `artifacts/gui-desktop-smoke/<scenario>/<timestamp>/final-state.json`

The final state must show that the UI observed the actual click, drag, and scroll effects. Passing a command exit code alone is not enough.

## Idempotence and Recovery

The implementation must be safe to run repeatedly:

- Probing virtual input does not move the pointer unless explicitly running a smoke test.
- Backend auto-detection does not start or stop user services unless an explicit setup command is running.
- ydotool subprocesses have timeouts and cannot hang the service indefinitely.
- Direct uinput opens a fresh short-lived device for each atomic pointer action and destroys it on drop.
- Failed drag attempts should release the mouse button in a best-effort cleanup path.
- A failed adapter command should produce a structured action error and leave backend selection intact for later diagnostics.

VM provisioning remains idempotent:

- Installing `ydotool` repeatedly is safe.
- Enabling `ydotoold` repeatedly is safe.
- Copying latest host-built binaries into the VM overwrites old test binaries but does not rebuild inside the VM.

## Documentation Plan

Update:

- `docs/gui-desktop-test-harness.md` with the `LinuxVirtualInput` fallback, direct uinput/ydotool prerequisites, and COSMIC acceptance command.
- `CONTINUITY.md` with current status, next command, and any known incomplete backend actions.
- `NOTES.md` only if the direct uinput, ydotool command semantics, or coordinate calibration produce durable facts worth preserving.
- Any existing native cursor overlay plan only if it currently claims COSMIC requires RemoteDesktop for input.

Keep docs factual and professional. Do not claim that all Wayland compositors are supported until each target session has proof.

## Risks and Mitigations

Risk: direct uinput absolute coordinates may not be desktop logical coordinates under scaling.

Mitigation: adapter declares its coordinate plane. COSMIC scaled proof showed the compositor consumes the absolute uinput device range as physical output pixels while the Computer Use contract supplies desktop logical points; the adapter now parses output scale and converts at the uinput boundary.

Risk: ydotool pointer support is incomplete or inconsistent.

Mitigation: keep ydotool as a fallback and keyboard/text helper, but prefer the proven direct uinput pointer adapter when `/dev/uinput` is writable and bounds are available.

Risk: virtual input may bypass compositor permission expectations.

Mitigation: only auto-fallback when RemoteDesktop is absent or unsupported, not when a portal prompt was explicitly denied, unless a later product decision changes that policy.

Risk: subprocess-based input could be slow for drags.

Mitigation: keep the first adapter simple for proof. If drag smoothness is poor, batch events where ydotool supports it or move direct uinput earlier.

Risk: multi-monitor screenshots may lack reliable logical rect metadata.

Mitigation: fail closed for snapshot-based Linux virtual input when logical rect is missing. Add metadata plumbing before enabling that shape.

## Outcomes & Retrospective

The local Rust pointer implementation slice is complete and the 1x COSMIC visible smoke passes. `LinuxVirtualInput` is wired through the platform model, Linux environment selection, action routing, direct uinput pointer execution, ydotool keyboard/text fallback, and coordinate mapping tests.

The live COSMIC Wayland acceptance gate for click, drag, scroll, type, and press-key passed in the Arch `testing-vm` at both 1x and 125% scale. The 1x artifact `/workspace/artifacts/gui-desktop-smoke/wayland-pointer/20260515T092606Z` and scaled profile artifact `/workspace/artifacts/gui-desktop-smoke/wayland-pointer/20260515T093737Z` both report `clicked=true`, `drag_completed=true`, `scroll_events=1`, `entry_text="cosmic-text-smoke"`, and `submitted_text="cosmic-text-smoke"`, with action messages from the Linux virtual input fallback.

Final outcome:

- COSMIC Wayland no longer reports "no physical input backend is available" when Linux virtual input is available.
- The selected backend is `LinuxVirtualInput`.
- The selected pointer adapter is direct absolute `/dev/uinput` when writable and bounded; `ydotool` remains a fallback and keyboard/text adapter.
- The full visible computer-use smoke app passes click, drag, and scroll.
- Scaling-aware coordinate tests pass locally.
- KDE/GNOME portal paths continue to use RemoteDesktop when available.
- X11 continues to use XTest.

2026-05-15 update: implemented the direct uinput pointer adapter after ydotool failed coordinate calibration, extended and proved the headed COSMIC smoke through click, drag, scroll, `type_text`, and `press_key` at `/workspace/artifacts/gui-desktop-smoke/wayland-pointer/20260515T092606Z`, then proved the same input suite at 125% scale. The repeatable scaled acceptance profile is now `python3 scripts/run_gui_testing_vm_smoke.py --desktop-env COSMIC --profile wayland-pointer-scaled`, with accepted artifact `/workspace/artifacts/gui-desktop-smoke/wayland-pointer/20260515T093737Z`.
