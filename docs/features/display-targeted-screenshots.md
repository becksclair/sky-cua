# Display-targeted screenshots

## Status

Shipped. Last verified: 2026-06-15 local focused Rust checks, local deploy,
installed MCP surface proof, and agent-loop proof. KDE Wayland targeted-window
and display screenshot live-smoke proof remains the 2026-06-13 artifact set.
Linux VM matrix and an interactive-session Windows live smoke gate are tracked
in Verification.

## Summary

Desktop screenshots are display-aware. A no-selector `screenshot` captures the
primary display, explicit display selectors capture one monitor, window
selectors activate and focus-verify the window before returning an unoccluded
crop, and full virtual-desktop capture requires `capture_all_displays=true`.

## Contract surface

- `EnvironmentInfo.displays: Vec<DisplayInfo>` exposes monitor topology from
  existing state surfaces; there is no separate `list_displays` tool.
- `DoctorReport.display_topology` records which provider was attempted, which
  provider supplied displays, stdout size, timeout/failure state, and display
  count so agents can distinguish unavailable topology from a successful
  fallback.
- `WindowInfo.display` and `WindowInfo.display_intersections` expose the chosen
  display and spanning-window intersections.
- `FocusedApp.display` carries the current app/window display when known.
- `CaptureInfo.capture_scope`, `CaptureInfo.display`,
  `CaptureInfo.logical_rect`, and `CaptureInfo.source_logical_rect` describe
  what the screenshot represents and how snapshot coordinates map back to the
  backend. If a Screenshot-portal fallback is outside the active RemoteDesktop
  stream, `source_logical_rect` is omitted and portal actions against that
  snapshot fail closed.
- `ServiceRequest::Screenshot` carries `target`, `display_target`, and
  `capture_all_displays`.
- MCP `screenshot` accepts flat display selector fields:
  `display_id`, `display_name`, `display_index`, and `capture_all_displays`.
- Targeted screenshots that cannot map crop pixels because capture source
  geometry is unavailable report `CaptureSourceGeometryMissing`; MCP error
  payloads include `code`, `message`, and an explicit retry/fallback
  `suggestion`.

Exactly one screenshot selector may be provided: a window target, a display
selector, or `capture_all_displays=true`. Omitted selectors resolve to the
primary display when display topology is available.

## Behavior

Window-targeted screenshots resolve the target through native window state,
activate the window first, verify focus when the backend can prove it, then crop
the captured image to the window bounds. The resulting snapshot has
`capture_scope=window` and screenshot-pixel actions must pass that
`snapshot_id`. If a RemoteDesktop/Screencast frame lacks source geometry for a
targeted capture, Linux resets the RemoteDesktop capture session and retries the
targeted screenshot once before surfacing the failure.

Display-targeted screenshots resolve the requested display from
`environment.displays`. Explicit display target failures are hard failures:
the backend does not silently return another monitor. When no selector is
provided, the primary display is captured. If topology is unavailable only for
that omitted-selector path, Linux preserves the legacy raw capture with a
diagnostic instead of breaking old callers.

Linux diagnostics distinguish two display-topology caveats:
`DisplayTopologyUnavailable` means no provider supplied display geometry, and
display-targeted screenshots should not be treated as authoritative until state
is refreshed; `DisplayTopologyInferred` means XRandR fallback supplied geometry
in a Wayland session, so agents should prefer window-targeted screenshots and
use the returned `snapshot_id` for any pixel action.

`capture_all_displays=true` returns the virtual desktop union with
`capture_scope=all_displays`. Agents should use it only when the whole desktop
is actually required. On Linux Wayland, a PipeWire frame is accepted for this
scope only when the active RemoteDesktop stream covers the virtual desktop
union; otherwise the planner falls back to the Screenshot portal.

For snapshot actions, screenshot pixels first map through
`capture.logical_rect` into desktop-logical coordinates. Linux virtual input
and XTest dispatch that desktop point directly. Portal RemoteDesktop subtracts
`capture.source_logical_rect` before dispatching stream-local coordinates.
Windows `SendInput` continues to use virtual desktop metrics so negative-origin
monitor layouts remain valid.

## Source paths

- `crates/sky-cua-platform/src/model.rs`
- `crates/sky-cua-platform/src/model/service.rs`
- `crates/sky-cua-client/src/mcp_tools.rs`
- `crates/sky-cua-client/src/mcp_tools/definitions.rs`
- `crates/sky-cua-linux/src/displays.rs`
- `crates/sky-cua-linux/src/backend.rs`
- `crates/sky-cua-linux/src/capture_plan.rs`
- `crates/sky-cua-linux/src/actions/targeting.rs`
- `crates/sky-cua-windows/src/backend.rs`
- `scripts/live_targeted_screenshot_smoke.py`
- `scripts/live_display_screenshot_smoke.py`

## Verification

- Rust contract/client tests cover display contract serialization,
  screenshot request fields, MCP schema fields, and selector parsing.
- Linux unit tests cover KWin/KScreen, GNOME DisplayConfig, Hyprland, COSMIC,
  and xrandr parsers; window-to-display assignment; capture-region planning;
  and portal, XTest, and Linux virtual-input coordinate mapping.
- Windows target checks cover monitor selection, window intersection, display
  capture source origins, and screenshot-pixel to desktop-coordinate mapping.
- VM smoke profiles:
  `targeted-screenshot` proves window activation, crop metadata, focus
  verification where available, and snapshot-click landing;
  `display-screenshot` proves primary default, explicit primary, explicit
  secondary when present, all-displays opt-in, and display-snapshot click
  landing. Current unit/client tests cover the `doctor.display_topology`
  contract; refresh the canonical VM artifacts before treating live
  `doctor.display_topology` output as proven by artifact.

Accepted local validation for this feature:

```bash
cargo fmt --check && cargo test
uv run ruff format --check scripts && uv run ruff check scripts && uv run basedpyright && uv run pytest
python3 scripts/build_plugin.py
python3 scripts/live_agentic_loop_smoke.py
python3 scripts/live_targeted_screenshot_smoke.py
python3 scripts/live_display_screenshot_smoke.py
cargo check -p sky-cua-windows --target x86_64-pc-windows-gnu
cargo check -p sky-cua-windows --target x86_64-pc-windows-gnu --tests
```

Accepted DevBox validation for this feature:

```bash
ssh devbox 'cd /c/Users/bex/sky-cua-windows-display-test && RUSTC_WRAPPER= cargo test -p sky-cua-windows'
ssh devbox 'cd /c/Users/bex/sky-cua-windows-display-test && RUSTC_WRAPPER= cargo test -p sky-cua-platform -p sky-cua-client -p sky-cua-service -p sky-cua-windows'
```

Accepted canonical live-smoke artifacts:

- Agentic-loop smoke:
  `artifacts/pi-agentic-loop-smoke/20260615T211914Z`
- Targeted screenshot smoke:
  `artifacts/gui-desktop-smoke/targeted-screenshot/20260613T134618Z`
- Display screenshot smoke:
  `artifacts/gui-desktop-smoke/display-screenshot/20260613T134628Z`
- Note: the 2026-06-13 VM artifacts predate `doctor.display_topology`
  readback in these profiles.

Registered VM profile commands:

```bash
python3 scripts/run_gui_testing_vm_smoke.py --profile targeted-screenshot --desktop-env KDE --wayland-display wayland-0
python3 scripts/run_gui_testing_vm_smoke.py --profile display-screenshot --desktop-env KDE --wayland-display wayland-0
```

Recent local proof on 2026-06-15 covered the
`CaptureSourceGeometryMissing` retry/error contract, display-topology doctor
diagnostics, the display-probe large-stdout regression, installed MCP
`tools/list` readback, and a Pi agent-loop smoke with successful sky-cua tool
evidence. The listed 2026-06-13 targeted/display artifacts remain the current
canonical live proof for KDE Wayland window and display screenshot flows.

The same VM profiles are registered for Plasma/KWin, GNOME, Hyprland, COSMIC,
and i3/X11 by selecting the corresponding guest session before invoking the
profile. Windows live proof is a host/VM gate because this repository does not
yet have a Windows VM runner equivalent to the Linux testing-VM harness.
A throwaway DevBox MCP probe verified that `list_windows` exposes
`environment.displays` over SSH, but the OpenSSH service session had no logged
in interactive desktop (`query user` reported none) and GDI screenshot capture
failed with `ERROR_INVALID_HANDLE`, so screenshot/window activation proof still
needs an interactive DevBox session.

## Known limitations

- The KWin topology provider currently uses the available `kscreen-doctor`
  command path rather than a separate direct KScreen DBus client.
- Multi-output VM proof depends on the selected guest exposing more than one
  monitor; the display smoke writes a structured skip artifact for the
  secondary-display assertion when only one output exists. Set
  `SKY_CUA_DISPLAY_SCREENSHOT_REQUIRE_SECONDARY=1` when the VM profile is
  intended to fail unless a secondary output is present.
- Windows GDI is still the active capture lane; the WGC/DXGI ladder remains a
  separate roadmap item.

## Related

- [`docs/runtime/linux-architecture.md`](../runtime/linux-architecture.md)
- [`docs/runtime/mcp-boundary.md`](../runtime/mcp-boundary.md)
- [`docs/features/kwin-window-targeting.md`](kwin-window-targeting.md)
- [`ROADMAP.md`](../../ROADMAP.md)
