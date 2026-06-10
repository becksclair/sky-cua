# sky-cua-linux Guide

`sky-cua-linux` implements the Linux desktop backend: AT-SPI, portals,
PipeWire capture, KWin discovery, and X11/XTest fallback. It is the
runtime-sensitive crate; validate claims against source and live smokes when
behavior touches the desktop. Relevant live smokes:
`scripts/live_desktop_smoke.py`, `scripts/live_portal_downgrade_smoke.py`,
`scripts/run_gui_testing_vm_smoke.py --host testing-vm --profile computer-use`.

## Layout

- `src/backend.rs` — orchestration, fallback snapshot geometry, and the
  cached `accessibility_connection` pattern for expensive connections.
- `src/actions/` — action execution: `mod.rs` owns `LinuxActionExecutor`,
  `runtime.rs` the fakeable runtime facade, `targeting.rs` physical backend
  selection and coordinate planning.
- `src/env_probe.rs` — environment detection; never infer from one env var.
- `src/portal/**` — portal session/capture/input; `src/x11/**` — X11
  discovery/input/capture; `src/kwin.rs` — KWin native Wayland
  background-window discovery; `src/atspi/**` — AT-SPI discovery/tree/
  actions; `src/coords.rs` — coordinate mapping.
- `src/app_match.rs` — app/window matching policy; `src/app_policy.rs` —
  app-specific action policy, sourced from
  `resources/app-instructions/index.json`.

## Conventions

- Use blunt fallback roles (`x11_container`, `x11_leaf_region`,
  `x11_action_region`); do not invent semantic roles from geometry alone.
- Preserve `CaptureBackendDowngraded` and `PortalApprovalPending` diagnostic
  honesty.
- Do not re-enable blocking KWin active-window query without proving it
  cannot wedge Codex-launched services.
- Do not treat portal keyboard success on XWayland as proof that text
  changed; prefer X11/XTest for matched X11 keyboard paths.

## Gotchas

- Remote shells can lie about the real session; corroborate Wayland/X11/
  portal state.
- Portal prompts are operator-facing and may block; surface
  `PortalApprovalPending`, do not let socket timeouts explain it badly.
- Full live smokes are environment-dependent; name exactly which ones were
  or were not run.
