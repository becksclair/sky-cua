# Detached Linux session-env repair

## Status

Shipped on Linux. Code complete and live-smoked on the current KDE/Plasma
host through direct MCP, Codex exec, and rich app-server lanes. Last
verified: per `CONTINUITY.md` 2026-05-15. VM matrix coverage and a
non-Codex host (OpenCode/Pi) smoke remain follow-up work.

## Summary

When an MCP host launches `sky-cua-client mcp` without the full desktop
session environment, the runtime repairs the missing state, reports what
was repaired, and lets agents continue only after they can see the recovery
signal. The repair is observable through `doctor.session_env`,
`SessionEnvRepaired` diagnostics, and `list_apps` diagnostics.

## Contract surface

Public model in `crates/sky-cua-platform/src/model.rs`:

- `DoctorSessionEnvReport` — full repair report for `doctor` results.
- `DoctorSessionEnvRepair` — per-key repair record (`source`, `previous`,
  `current`).
- `DiagnosticEntry` variant `SessionEnvRepaired` — emitted on snapshots and
  list responses when the runtime repaired session state for the call.

Tool surface:

- `doctor` returns `session_env` with `repaired` (the keys filled and from
  which source), `path_changed` (whether `PATH` was normalized), and
  `final_path` (the effective path after repair).
- `list_apps` and snapshot diagnostics may include `SessionEnvRepaired`.
  Agents should treat that as recovered context, not as an error.

## Behavior

Two-stage repair:

1. **Client-side repair** in
   `crates/sky-cua-client/src/service_launcher.rs`: normalizes `PATH`,
   probes `/run/user/<uid>`, X11 sockets, logind, and
   `systemctl --user show-environment`, then forwards repaired values
   when spawning `sky-cua-service`.
2. **Service-side rehydration** in
   `crates/sky-cua-linux/src/session_env.rs`: hydrates the service process
   from the process tree, systemd user manager, runtime dir, and session
   bus path before probing portals, AT-SPI, KWin, and other desktop
   backends.

Invariants:

- Host adapters should still forward desktop env variables when available.
  Repair is a fallback, not the preferred contract.
- The client repairs enough environment before spawning a service that
  startup health can reject stale services missing repaired values.
- The service rehydrates before desktop probing, so direct service runs
  and already-running daemons report the same repair state.
- Repair is observable through structured data, not only prose.
- A repaired `PATH` includes normal system command directories
  (`/usr/bin`, `/bin`).

## Source paths

- `crates/sky-cua-client/src/service_launcher.rs` — client startup repair
- `crates/sky-cua-linux/src/session_env.rs` — service rehydration
- `crates/sky-cua-platform/src/model.rs` — `DoctorSessionEnvReport`,
  `DoctorSessionEnvRepair`
- `skills/computer-use/SKILL.md` — agent guidance for inspecting
  `doctor.session_env` and treating `SessionEnvRepaired` as recovered
  context
- `scripts/live_session_env_smoke.py`,
  `scripts/live_app_server_session_env_smoke.py`,
  `scripts/live_codex_exec_session_env_smoke.py` — direct and
  agent-visible session-env smokes

## Verification

Focused tests:

```bash
cargo test -p sky-cua-client service_launcher
cargo test -p sky-cua-linux session_env
```

Direct and agent-visible smokes from a stripped environment:

```bash
python3 scripts/live_session_env_smoke.py
python3 scripts/live_app_server_session_env_smoke.py
python3 scripts/live_codex_exec_session_env_smoke.py
```

Each smoke proves that the harness inspects `doctor.session_env` and
`SessionEnvRepaired` before operating a `zenity` dialog from a stripped
environment.

## Known limitations

- Live proof exists on the current KDE/Plasma host only. The same runtime
  seam should be exercised under OpenCode/Pi and additional VM desktops as
  those harness lanes mature.
- Stripped-env session repair is not yet a `scripts/run_gui_testing_vm_smoke.py`
  profile. Tracked in `ROADMAP.md`.
- This is a recovery path, not a reason for host adapters to omit the
  desktop environment allowlist.

## Related

- ROADMAP entry: [`ROADMAP.md`](../../ROADMAP.md) under "Linux desktop parity"
- Runtime contract: [`docs/runtime/mcp-boundary.md`](../runtime/mcp-boundary.md)
  describes the desktop environment allowlist host adapters should still
  forward.
- Originating ExecPlan (retired into this feature doc; see git history for `plans/detached_session_env_repair.md`).
