# sky-cua-linux Guide

## Package Identity

`sky-cua-linux` implements the Linux desktop backend: AT-SPI, portals, PipeWire capture, KWin discovery, and X11/XTest fallback.
It is the runtime-sensitive crate; validate claims against source and live smokes when behavior touches the desktop.

## Setup & Run

```bash
cargo test -p sky-cua-linux
cargo clippy -p sky-cua-linux --all-targets
python3 scripts/live_desktop_smoke.py
python3 scripts/live_portal_downgrade_smoke.py
python3 scripts/run_gui_testing_vm_smoke.py --host testing-vm --profile computer-use
```

## Patterns & Conventions

- `src/backend.rs` is the orchestration layer; keep low-level portal/X11/KWin details in their modules.
- Environment detection belongs in `src/env_probe.rs`; do not infer from one env var only.
- Portal session/capture/input behavior belongs under `src/portal/**`.
- X11 discovery/input/capture belongs under `src/x11/**`.
- KWin native Wayland background-window discovery belongs in `src/kwin.rs`.
- AT-SPI discovery/tree/actions belong under `src/atspi/**`.
- DO: Use blunt fallback roles like existing `x11_container`, `x11_leaf_region`, and `x11_action_region` in `src/backend.rs`.
- DO: Preserve `CaptureBackendDowngraded` and `PortalApprovalPending` diagnostic honesty.
- DO: Cache expensive connections following `LinuxDesktopBackend::accessibility_connection` in `src/backend.rs`.
- DO: Put app-specific action policy lookup in `src/app_policy.rs` and source metadata from `resources/app-instructions/index.json`.
- DON'T: Invent semantic roles from geometry alone; real bounds with conservative labels are enough.
- DON'T: Re-enable blocking KWin active-window query without proving it cannot wedge Codex-launched services.
- DON'T: Treat portal keyboard success on XWayland as proof that text changed; prefer X11/XTest for matched X11 keyboard paths.

## Touch Points / Key Files

- Backend orchestration and action routing: `src/backend.rs`
- Portal RemoteDesktop/ScreenCast session manager: `src/portal/remote_desktop.rs`
- PipeWire frame capture: `src/portal/pipewire.rs`
- X11 metadata fallback: `src/x11/windowing.rs`
- X11 input fallback: `src/x11/input_xtest.rs`
- KWin window enumeration: `src/kwin.rs`
- Coordinate mapping: `src/coords.rs`

## JIT Index Hints

- Find backend action cases: `rg -n "ActionName|execute_action|SetValue|XTest|portal" src/backend.rs`
- Find portal lifecycle: `rg -n "PortalSession|restore|token|ensure_started" src/portal`
- Find PipeWire downgrade path: `rg -n "capture_frame|CaptureBackendDowngraded|portal_screenshot" src`
- Find X11 fallback roles: `rg -n "x11_container|x11_leaf_region|x11_action_region" src`
- Find KWin queries: `rg -n "WindowsRunner|getWindowInfo|query_active_window|gdbus" src/kwin.rs`

## Common Gotchas

- Remote shells can lie about the real session; corroborate Wayland/X11/portal state.
- Portal prompts are operator-facing and may block; surface `PortalApprovalPending`, do not let socket timeouts explain it badly.
- Full live smokes are environment-dependent; name exactly which ones were or were not run.

## Pre-PR Checks

```bash
cargo test -p sky-cua-linux && cargo clippy -p sky-cua-linux --all-targets
```
