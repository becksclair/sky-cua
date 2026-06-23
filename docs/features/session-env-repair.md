# Detached Linux session-env repair

## Status

Shipped on Linux. Code complete and live-smoked on the current KDE/Plasma
host through direct MCP, Codex exec, and rich app-server lanes. Last
verified: 2026-05-15. VM matrix coverage and a
non-Codex host (OpenCode/Pi) smoke remain follow-up work.

## Summary

When an MCP host launches `sky-cua-client mcp` without a reliable local
desktop session environment, the runtime selects the active non-remote
graphical logind session, repairs the process state, reports what was
repaired, and lets agents continue only after they can see the recovery
signal. From SSH, tmux, detached shells, or other non-graphical launch
contexts, repaired graphical values replace stale shell values such as an
SSH-forwarded `DISPLAY=localhost:10.0`. The repair is observable through
`doctor.session_env`, `SessionEnvRepaired` diagnostics, and `list_apps`
diagnostics.

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
- Client-side pre-spawn repairs are reported with source `client-launch`,
  so a daemon that starts with already-repaired variables still exposes how
  it attached to the desktop.
- Detached client launches also mark graphical keys intentionally cleared,
  preventing the daemon from restoring stale parent/systemd display values.
- `list_apps` and snapshot diagnostics may include `SessionEnvRepaired`.
  Agents should treat that as recovered context, not as an error.

## Behavior

Two-stage repair:

1. **Client-side repair** in
   `crates/sky-cua-client/src/launch_environment.rs`: normalizes `PATH`,
   chooses the best local graphical logind session for the current user,
   hydrates display/session identity from the selected session and its
   leader process when readable, uses `systemctl --user show-environment`
   only for support values such as runtime dir and session bus, then
   forwards repaired values when spawning `sky-cua-service`.
2. **Service-side rehydration** in
   `crates/sky-cua-linux/src/session_env.rs`: hydrates the service process
   from the process tree, systemd user manager, runtime dir, and session
   bus path before probing portals, AT-SPI, KWin, and other desktop
   backends.

Invariants:

- Host adapters should still forward desktop env variables when available.
  Repair is a fallback, not the preferred contract.
- A remote, tty, detached, empty-env, or runtimeless launch is treated as
  untrusted for graphical session variables; the selected local graphical
  session wins. A missing session bus alone repairs the bus path without
  overriding an otherwise valid local graphical identity.
- Remote logind sessions and user-manager sessions are never selected as
  the graphical target.
- In detached launches, systemd user-manager values are not allowed to
  supply display/session identity; they may only fill support values.
- The client repairs enough environment before spawning a service that
  startup health can reject stale services missing repaired values.
- When startup health rejects a reachable stale Unix daemon, the client
  terminates the daemon that owns the singleton lock and starts a fresh one
  with the repaired launch environment.
- The service rehydrates before desktop probing, so direct service runs
  and already-running daemons report the same repair state.
- Repair is observable through structured data, not only prose.
- A repaired `PATH` includes normal system command directories
  (`/usr/bin`, `/bin`).

## Source paths

- `crates/sky-cua-client/src/launch_environment.rs` — client startup repair
- `crates/sky-cua-client/src/service_launcher.rs` — repaired service launch
  and startup health enforcement
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
cargo test -p sky-cua-client launch_environment
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

- Stripped-env session repair runs in the VM as the `session-env` profile, a
  member of the curated pre-merge set (`--profile curated`); first VM proof
  passed on COSMIC 2026-06-12. Additional desktops are covered as curated
  runs happen against other guest sessions.
- This is a recovery path, not a reason for host adapters to omit the
  desktop environment allowlist.

## Related

- ROADMAP entry: [`ROADMAP.md`](../../ROADMAP.md) under "Linux desktop parity"
- Runtime contract: [`docs/runtime/mcp-boundary.md`](../runtime/mcp-boundary.md)
  describes the desktop environment allowlist host adapters should still
  forward.
- Originating ExecPlan (retired into this feature doc; see git history for `plans/detached_session_env_repair.md`).
