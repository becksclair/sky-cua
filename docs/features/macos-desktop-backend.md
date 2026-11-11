# macOS desktop backend

## Status

Partial. The desktop lane is implemented and live-proven on one machine
(macOS 27.2, Apple Silicon, single built-in Retina display) on 2026-11-11
from an uncommitted checkout of branch `bex/macos-backend`. The agent cursor
overlay, session presence, application launch, browser-use, the installer,
and the CUA action surface are not ported; see "Known limitations".

## Summary

`sky-cua-macos` is the native macOS implementation of `DesktopBackend`. An
agent can list applications and windows, observe one window as an
Accessibility tree plus a ScreenCaptureKit screenshot, act on elements
semantically, and fall back to CGEvent pointer and keyboard input, through
the same MCP tools the Linux and Windows backends serve.

## Contract surface

Public model in `crates/sky-cua-platform`:

- `SessionKind::Macos` (`macos`).
- `SemanticBackendKind::MacosAccessibility` (`macos_accessibility`).
- `CaptureBackendKind::ScreenCaptureKit` (`screen_capture_kit`).
- `InputBackendKind::CgEvent` (`cg_event`).
- `BackendErrorCode::PermissionRequired` — the operating system has not
  granted a privacy permission the operation needs.

Each backend kind is reported only while its permission is granted;
otherwise the kind is `none` and the doctor report names the missing grant.

Doctor checks added on macOS: `accessibility_permission`,
`screen_capture_permission`, `post_event_permission`, `windowing_backend`,
`display_topology`.

Identifiers callers can rely on:

- `WindowInfo.window_id` is the decimal `CGWindowID`; `DisplayInfo.display_id`
  is the decimal `CGDirectDisplayID`; both report backend `macos-coregraphics`.
- `AppInfo.app_id` is the bundle identifier when the process has one, else
  the owner name.
- Element `backend_ref` values have the form `ax:pid=<pid>;gen=<n>;idx=<n>`
  and are valid only for recent tree reads of that process. The format is not
  stable.

Diagnostics specific to this backend: `AccessibilityTreeTruncated` (the
element or depth cap stopped the walk), `WindowFocusVerified` (the target was
already focused, so a capture did not activate it), and `WindowFocusRequested`.

MCP tool schemas are unchanged. `setup_desktop` with operation
`accessibility` requests every missing permission.

## Behavior

**Coordinates.** Accessibility, CGEvent, and the CoreGraphics window list
share one space: global points, top-left origin. That is `desktop_logical` on
macOS. A capture carries the desktop rectangle it covers and one scale factor
(display backing scale and model-image downscale folded together), so
converting between model-image pixels and desktop points is a subtraction and
a multiplication in `coords.rs`. AppKit geometry is never used.

**Windows and applications.** Windows come from the CoreGraphics window list:
on-screen, normal layer, non-zero alpha. Untitled windows smaller than
100 x 60 points are treated as transient popups (text-suggestion bubbles,
tooltips) and omitted, so they cannot take the focused flag or make an
application selector ambiguous. The application list is derived from window
owner process ids. An application selector must match on every field it
supplies; a `name` that equals an application's name, bundle identifier, or
executable name outranks one that is only contained in them, a `name` matches
a window title only when no window matches it by application identity, and a
selector that matches no on-screen window is an
`InvalidRequest`, never a fallback to the focused application. The focused window is the front-most listed window of the
front process, determined without AppKit run-loop state, which goes stale in
a daemon that has no main run loop.

**Window identity.** macOS has no public call that maps an Accessibility
window to a `CGWindowID`, and the backend uses no private symbols. A window
is correlated by exact frame within one process, then by title, then by the
application's main-window flag for the front window. A frame is only a key
when no other window of the process shares it; a frame twin requires the
title or main-window proof. Anything still ambiguous fails closed with `AccessibilityCoverageLimited`; a title alone is
never identity. A window that moved between the two reads is retried once
against a fresh window list.

**Accessibility.** Every AX call is synchronous IPC into the target
application, so none runs on a tokio runtime thread. Each target process has
its own worker thread, created on first use and exiting after 30 s idle. A
1 s AX messaging timeout bounds each call, and the tree read honours a
wall-clock budget, returning the partial tree when it expires. A lane that
does not answer within its deadline plus a grace period is abandoned and the
next request for that process gets a fresh lane, so an unresponsive
application stalls only itself. Trees are capped at 1500 elements and
250 children per node, and either cap is reported as
`AccessibilityTreeTruncated`; text readback is capped at 4096 characters.
Attributes are read in batches, with role-specific attributes in a second
role-gated batch. Scroll bars report their position as a numeric value on
the unit interval.

Chromium- and Electron-based applications expose no tree until asked. When
such an application's tree is dormant the backend sets
`AXManualAccessibility` and `AXEnhancedUserInterface` on the application
element, once per process, and re-reads.

**Capture.** ScreenCaptureKit captures one window's own content (independent
of occlusion, no shadow, no cursor) or one whole display. The reported
`logical_rect` is the frame ScreenCaptureKit captured. Shareable content is
reused for up to 5 s, and only when a fresh CoreGraphics read confirms the
window is still on screen, owned by the same process, and at the same frame.
A capture of an already-focused window does not activate it.

**Actions.** Semantic AX actions are tried first: focus, press, toggle,
expand and collapse, select, value writes, the context-menu action, and named
actions. A plain single left click on an element is an AX press. Other
requests fall through to CGEvent input posted at the HID event tap, after
the target window is brought forward. If the target window is gone the
action fails with `SnapshotStale`, and if it cannot be brought to the front
it fails with `ActionRequiresPhysicalInput`; input is never posted to
whatever happens to be in front instead. A reply lost after a write was sent is
`CuaActionOutcomeUnknown`, because an AX press that opens a modal sheet does
not reply until the sheet closes.

An element's action point is the centre of the part of it that is visible in
the capture. AX reports a scrolled view's full document frame, so the raw
centre can lie far outside the window; an element with no visible part is
rejected with `InvalidRequest` instead of acting on whatever is at that
point. The same holds for a scroll that names an element or a point: only a
scroll that names neither is posted at the current pointer position. The
physical `set_value` fallback clicks its target first, so the select-all and
typing that follow replace that element's content and not the content of
whatever control already held key focus.

Physical typing goes through the system text-input path, so application
features such as automatic capitalisation and autocorrection apply to it.

**Permissions.** macOS attributes the daemon to the application that launched
it (the terminal, Codex, or Claude host), and the daemon keeps that
attribution after it detaches. Three grants are involved: Accessibility,
Screen & System Audio Recording, and event posting. A missing grant produces
`PermissionRequired` with the System Settings path. `setup_desktop` registers
the host application for each missing grant through the standard system
prompt and opens the matching Privacy & Security pane. A Screen Recording
grant takes effect after the host application is restarted.

**Daemon lifecycle.** The client verifies the socket peer's process id and
start time before it will signal a daemon, and only a freshly verified
process generation is ever signalled. On macOS the client also treats a
daemon as stale when the service binary's modification time is later than the
daemon's start, and replaces it, so a rebuilt binary takes over on the next
client launch without manual cleanup. The inode change time is not used
because cargo re-clones an unchanged binary on invocations that do not
relink, which would restart a daemon serving live sessions. An install that
preserves a modification time older than the running daemon's start is
therefore not detected; an installer must restart the daemon itself. The daemon exits after 300 s idle.

## Source paths

- `crates/sky-cua-macos/src/backend.rs`, `backend/` — `MacosDesktopBackend`.
- `crates/sky-cua-macos/src/ax/` — worker lanes, tree reads, vocabulary,
  semantic actions, retained element handles, Chromium wake-up.
- `crates/sky-cua-macos/src/windows.rs`, `windows/`, `displays.rs`,
  `window_match.rs`.
- `crates/sky-cua-macos/src/capture.rs`, `capture/`.
- `crates/sky-cua-macos/src/input.rs`, `input/`, `keymap.rs`, `keymap/`.
- `crates/sky-cua-macos/src/permissions.rs`, `permissions/`.
- `crates/sky-cua-macos/src/coords.rs`.
- `crates/sky-cua-service/src/backend_factory.rs` — backend selection.
- `crates/sky-cua-client/src/service_launcher.rs`,
  `crates/sky-cua-platform/src/process/darwin.rs` — peer identity, process
  generation, and rebuild staleness.
- `justfile` — `verify-rust` dispatches to `verify-rust-linux` or
  `verify-rust-macos`.

## Verification

- `just verify-rust` on macOS: `cargo fmt --check`, clippy with
  `-D warnings`, and `cargo nextest run` over every workspace member except
  `sky-cua-linux` and `sky-cua-cosmic-helper`.
- Linux compile coverage of the shared changes: `cargo check --workspace
  --all-targets` in an arm64 Linux container with the packages
  `.github/workflows/rust.yml` installs.
- Read-only live probes are unit tests named `live_*`, ignored by default:
  `cargo nextest run -p sky-cua-macos --run-ignored only <name> --no-capture`.
- Live proof, 2026-11-11, through `sky-cua-client mcp` against an isolated
  daemon and a scratch TextEdit document: `doctor`; window listing; `observe`
  with screenshot (about 160-200 ms on a debug build); `desktop_set_value`
  through AX; `desktop_keyboard` chord and text; `desktop_pointer` click at
  screenshot pixel coordinates, drag selection, and secondary click;
  `desktop_scroll` in both directions verified by scroll-bar position; an
  element-targeted click as an AX press; and `activate_window` in both
  directions. The daemon was replaced automatically across four rebuilds with
  one daemon running afterwards.

An isolated daemon for testing uses a private `HOME` and XDG directories and
a relative `SKY_CUA_SERVICE_SOCKET_PATH`, because an absolute path under a
deep directory exceeds the 104-byte Unix socket path limit. Swap a rebuilt
service binary into place with a rename, not an in-place overwrite.

## Known limitations

- Not ported: agent cursor overlay (the overlay host is an intentional no-op
  on macOS), session presence, `desktop_launch_app`, browser-use, the
  installer and standalone release, and the CUA action surface.
- The window model is the CoreGraphics normal layer only. An untitled child
  window of 100 x 60 points or more that an application attaches to a
  document (a rename or suggestion popover, a sheet) is listed as its own
  window and takes the focused flag from its parent, and it has no entry in
  the application's AX window list to correlate with. App-modal panels and
  floating panels live on other window levels and are not listed at all, so
  they are visible only through a display capture. Observed live with a
  TextEdit rename popover on 2026-11-11.
- `activate_window` reports success with a `WindowFocusUnverified`
  diagnostic when macOS accepts the request but the window does not become
  focused.
- Physical input moves the user's real pointer and requires the target
  window to be frontmost.
- Element bounds are `desktop_logical` on the `observe` path and
  `stream_pixels` on the `get_app_state` path; both are tagged and both are
  accepted by actions.
- A tree stopped by its deadline is reported as
  `AccessibilityCoverageLimited`, which the AppShot path treats as no
  accessibility text.
- The Chromium and Electron tree wake-up has unit coverage only; no dormant
  application was available to trigger it live.
- Not exercised: multiple displays, mixed display scale factors, and the
  missing-permission paths (all three grants were present).
- The permission grant belongs to the host application, so a daemon started
  by one host and reused by another runs with the first host's grants.
- Rebuild staleness is not detected when `SKY_CUA_SERVICE_PATH` points at a
  wrapper script, and there is no hosted macOS CI job.

## Related

- ROADMAP: "Phase: macOS parity".
- [`windows-uia-automation.md`](windows-uia-automation.md) — the backend this
  one mirrors for contract vocabulary.
- [`service-desktop-request-lane.md`](service-desktop-request-lane.md) —
  server-side request deadlines.
