# Linux backend architecture

This is a stable architecture description of the `sky-cua` Linux backend.
It documents the runtime layout, the public contracts, and the abstraction
seams. For active design and historical investigation prose, see the
relevant ExecPlan or research extract under `docs/research/`.

## Runtime layout

`sky-cua` is a Rust workspace plus Python harnesses with a split
client / service architecture:

```
host (MCP client, e.g. Codex)
   │
   └─stdio─→ sky-cua-client (mcp)
                 │
                 └─Unix socket─→ sky-cua-service (daemon)
                                    │
                                    └── platform backend
                                            ├── sky-cua-linux
                                            └── sky-cua-windows
```

- The client (`crates/sky-cua-client`) speaks MCP over stdio and
  delegates desktop work to the long-lived service. It also exposes a
  JSON-first operator CLI (`health`, `doctor`, `list-apps`,
  `list-windows`, `focused-window`, `get-app-state`).
- The service (`crates/sky-cua-service`) owns the Unix socket, snapshot
  caching, action routing, and overlay supervision. It selects a
  platform backend through a small factory.
- The Linux backend (`crates/sky-cua-linux`) implements environment
  probing, AT-SPI discovery, portal session management, capture, input,
  windowing registry, action execution, and overlay support.
- The shared platform model (`crates/sky-cua-platform`) defines the
  cross-platform contract: snapshot, action, capability, diagnostic,
  and cursor types.

Hosts other than Codex (OpenCode, Pi, custom) talk to the same MCP
boundary. See [`docs/runtime/mcp-boundary.md`](mcp-boundary.md).

## Crate map

- `crates/sky-cua-platform` — cross-platform shared types and the
  `DesktopBackend` trait.
- `crates/sky-cua-service` — daemon, IPC server, snapshot cache, action
  router, overlay controller.
- `crates/sky-cua-client` — MCP server, operator CLI, service launcher
  (with detached-launch session-env repair).
- `crates/sky-cua-linux` — Linux desktop backend.
  Its public backend entrypoint stays in `src/backend.rs`; Linux action
  execution policy lives under `src/actions/`.
- `crates/sky-cua-windows` — Windows desktop backend (UIA inspection,
  GDI capture, SendInput).
- `crates/sky-cua-overlay-host` — separate process that owns visible
  cursor overlay drawing and system cursor hiding adapters.
- `crates/sky-cua-cosmic-helper` — packaged COSMIC bridge daemon.
- `crates/sky-cua-chrome-host` — Linux native messaging host for the
  Codex Chrome extension.

## Subsystems

### Environment probing

`crates/sky-cua-linux/src/env_probe.rs` answers the questions any
desktop-aware code asks: session kind, compositor, portal versions,
AT-SPI availability, virtual input availability. It treats SSH/TTY
sessions with a valid `WAYLAND_DISPLAY` as Wayland even when
`XDG_SESSION_TYPE=tty`.

`crates/sky-cua-linux/src/session_env.rs` and
`crates/sky-cua-client/src/service_launcher.rs` together repair
detached launches that arrive without a full desktop environment. See
[`docs/features/session-env-repair.md`](../features/session-env-repair.md).

### Apps and AT-SPI

`crates/sky-cua-linux/src/apps/discovery.rs` enumerates AT-SPI
application roots. `crates/sky-cua-linux/src/atspi/tree.rs` flattens a
selected app tree into `ElementNode` values, with rich text and value
readback where the controls expose it. See
[`docs/features/atspi-rich-readback.md`](../features/atspi-rich-readback.md).

Selector matching is score-based, not first-match-wins. PID dominates;
class name, instance name, executable name, desktop-file stem, exact
title, and focused-window status all help rank candidates. Title-only
matches no longer steal correlation from focused-window candidates.

### Windowing registry

`crates/sky-cua-linux/src/windowing/registry.rs` aggregates multiple
environment-appropriate backends and merges their results into
`LinuxWindowInfo`. Supported backends: KWin, X11, GNOME Shell
Introspect, the bundled GNOME Shell extension, COSMIC helper,
Hyprland, and i3. Terminal metadata enrichment from `/proc` feeds
`WindowTarget` selectors `tty`, `terminal_pid`, `terminal_command`,
and `terminal_cwd`. See
[`docs/features/kwin-x11-workspace-metadata.md`](../features/kwin-x11-workspace-metadata.md)
for workspace metadata specifics.

### Capture lanes

The Wayland primary lane is in-process PipeWire frame capture from the
active ScreenCast session, in
`crates/sky-cua-linux/src/portal/screencast.rs` and
`crates/sky-cua-linux/src/portal/pipewire.rs`. The Screenshot portal in
`crates/sky-cua-linux/src/portal/screenshot.rs` is the fallback when
PipeWire fails.

`CaptureInfo` distinguishes:

- `capture.backend` — the selected primary lane.
- `capture.image_backend` — the lane that actually produced the image.

The split exists because a PipeWire snapshot can downgrade to
Screenshot mid-call, and the agent needs both pieces of truth. The
runtime emits `CaptureBackendDowngraded` diagnostics on downgrade. See
[`docs/research/2026-04-pipewire-vs-screenshot-portal.md`](../research/2026-04-pipewire-vs-screenshot-portal.md)
for the original investigation.

X11 and XWayland have a fallback snapshot path with synthetic root
bounds from `xwininfo` plus child-region recovery. The fallback
elements use conservative structural roles (`x11_container`,
`x11_leaf_region`, `x11_action_region`) rather than fabricating real
widget semantics.

### Input lanes

Backend selection in
`crates/sky-cua-linux/src/env_probe.rs::pick_input_backend()`:

- X11 with XTest available → `XTest`.
- Wayland with RemoteDesktop portal available → `PortalRemoteDesktop`.
- Wayland without RemoteDesktop but with virtual input available →
  `LinuxVirtualInput`.
- Otherwise → `None`.

The runtime does not silently bypass an explicit portal denial. See
[`docs/features/linux-virtual-input.md`](../features/linux-virtual-input.md)
for the COSMIC fallback behavior and adapter calibration.

`PortalRemoteDesktop` uses compositor-scoped coordinates for pointer actions.
On GNOME, current RemoteDesktop sessions require EIS for both pointer and
keyboard input: keyboard text and key chords are resolved against the
compositor-provided XKB keymap before the runtime emits EIS key events.

Physical input action routing lives in
`crates/sky-cua-linux/src/actions/`. `actions/targeting.rs` chooses the
effective pointer or keyboard backend for a request and maps model-facing
coordinates into the selected backend's coordinate plane. The backend-specific
side effects stay behind the crate-local `LinuxActionRuntime` facade so the
routing policy can be tested without creating portal, XTest, or uinput state.

### Coordinate spaces

`crates/sky-cua-linux/src/coords.rs` separates three coordinate
spaces:

- `StreamPixels` — pixels in the screenshot the model sees.
- `StreamLogical` — logical coordinates inside the compositor stream
  or portal session. `PortalRemoteDesktop` consumes this for absolute
  pointer motion.
- `DesktopLogical` — global desktop coordinates in the compositor's
  logical plane. `LinuxVirtualInput` targets this.

For snapshot-based actions, the runtime maps `StreamPixels` to
`DesktopLogical` through `capture.pixel_size` and
`capture.logical_rect`, including monitor offsets. If `logical_rect`
is missing, the runtime fails closed with a structured diagnostic
rather than pretending screenshot pixels are desktop coordinates.

The shared math helpers live in `crates/sky-cua-linux/src/coords.rs`; the
action-specific decisions about portal stream coordinates, X11 original pixels,
XWayland fallback elements, and Linux virtual desktop-logical targets live in
`crates/sky-cua-linux/src/actions/targeting.rs`.

### Portal session manager

`crates/sky-cua-linux/src/portal/remote_desktop.rs` owns a long-lived
RemoteDesktop and ScreenCast portal session. It selects keyboard and
pointer devices, requests monitor capture, and persists restore
tokens under the per-user state directory. Lifecycle transitions
(`PortalSessionStarted`, `PortalSessionRebuilt`, `PortalSessionRestored`,
`PortalSessionRestoreMiss`, `PortalSessionTokenRotated`) are surfaced
as first-class diagnostics and MCP text summaries.

When the compositor is still waiting on portal approval, the service
emits `PortalApprovalPending` and the MCP text tells the operator to
approve the dialog and retry.

### Overlay and cursor

The overlay-host process (`crates/sky-cua-overlay-host`) is supervised
by the service. It owns visible cursor overlay drawing and system
cursor hiding adapters. See
[`docs/features/agent-cursor-overlay.md`](../features/agent-cursor-overlay.md)
and
[`docs/features/compositor-cursor-hiding.md`](../features/compositor-cursor-hiding.md).

### App-action policy

`resources/app-instructions/index.json` carries machine-readable
per-app policy. The first shipped use is the Kate-scoped
`set_value_fallback`, which lets the heuristics-backed physical
`set_value` path replace text when semantic AT-SPI editing is
unavailable. The platform model exposes the policy through
`SemanticBackendKind` and `LinuxActionExecutor` consults it before
falling back.

## Trait surface

`DesktopBackend` (in `crates/sky-cua-platform/src/backend.rs`) is the
trait both Linux and Windows implement:

```rust
pub trait DesktopBackend {
    async fn probe_environment(&self) -> Result<EnvironmentInfo, BackendError>;
    async fn list_apps(&self) -> Result<Vec<AppInfo>, BackendError>;
    async fn get_app_state(&self) -> Result<AppStateSnapshot, BackendError>;
    async fn execute_action(&self, request: ActionRequest) -> Result<ActionOutcome, BackendError>;
}
```

The service's `enrich_action_request` populates `resolved_element`,
`resolved_target_element`, and `resolved_capture` from the cached
snapshot before calling the backend, so backends do not re-resolve
selectors per call.

## Linux action execution boundary

`LinuxDesktopBackend::execute_action` is the public Linux backend entrypoint.
It clears stale portal lifecycle events, probes and validates the current
environment, then delegates the request to `LinuxActionExecutor`.

`LinuxActionExecutor` in `crates/sky-cua-linux/src/actions/mod.rs` owns the
Linux-only action policy for choosing between semantic (AT-SPI) and physical
(portal / XTest / virtual input) lanes. Order:

1. If the action is element-targeted and the element has a usable
   semantic backend reference, invoke the semantic action.
2. Else, resolve element bounds through the existing coordinate path
   and use the selected physical input backend.
3. Snapshotless explicit-coordinate actions skip element resolution
   and go straight to the physical lane.

Semantic action tools (`focus_element`, `activate_element`,
`select_element`, `expand_element`, `collapse_element`,
`toggle_element`, `perform_action`) and physical action tools
(`click`, `perform_secondary_action`, `scroll`, `drag`, `type_text`,
`press_key`, `set_value`) are exposed through the MCP tool surface
documented in [`docs/runtime/mcp-boundary.md`](mcp-boundary.md).

The executor is crate-local. It does not introduce a shared Linux/Windows
action abstraction, and it does not change the MCP, service, or platform model
contracts.

## Definitions

- **Snapshot** — a structured representation of the current desktop
  app state: environment facts, focused app metadata, flattened
  AT-SPI elements, capture info, diagnostics. Identified by
  `snapshot_id`.
- **Element** — a single `ElementNode` from a snapshot, identified
  by `element_index` and optionally `element_identifier`.
- **Capture** — the screenshot plus metadata that supports a
  snapshot. The screenshot path lives under
  `$XDG_RUNTIME_DIR/sky-cua/captures/`.
- **Diagnostic** — a structured `DiagnosticEntry` reported through
  snapshots, list responses, or action outcomes. Diagnostics include
  honest fallback states.

## Related

- Runtime contract: [`docs/runtime/mcp-boundary.md`](mcp-boundary.md)
- Operations: [`docs/operations/gui-desktop-test-harness.md`](../operations/gui-desktop-test-harness.md)
- Feature index: [`docs/features/`](../features/)
- Research: [`docs/research/`](../research/)
