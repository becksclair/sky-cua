# Detached Linux Session-Env Repair

## Goal

Make Linux `computer-use` usable when an MCP host launches `sky-cua-client mcp`
without the full desktop session environment. The runtime should repair the
missing state, report what it repaired, and let agents continue only after they
can see that recovery signal.

## Status

Code complete and live-smoked on the current KDE/Plasma host through direct
MCP, Codex exec, and rich app-server lanes.

## Progress Ledger

Complete:

- `crates/sky-cua-client/src/service_launcher.rs` normalizes `PATH`, probes
  `/run/user/<uid>`, X11 sockets, logind, and `systemctl --user
  show-environment`, and forwards repaired values when spawning
  `sky-cua-service`.
- `crates/sky-cua-linux/src/session_env.rs` hydrates the service process from
  the process tree, systemd user manager, runtime dir, and session bus path.
- `crates/sky-cua-platform/src/model.rs` exposes
  `DoctorSessionEnvReport` and `DoctorSessionEnvRepair`.
- `doctor` returns `session_env`; MCP summaries and `list_apps` diagnostics
  surface `SessionEnvRepaired` when repair changed runtime state.
- The bundled workflow skill tells agents to inspect `doctor.session_env` and
  treat `SessionEnvRepaired` as recovered context rather than failure.
- `scripts/live_session_env_smoke.py`,
  `scripts/live_app_server_session_env_smoke.py`, and
  `scripts/live_codex_exec_session_env_smoke.py` prove direct and agent-visible
  recovery from stripped desktop variables.

Partial:

- The proven live environment is the current KDE/Plasma host. The same runtime
  seam should be exercised under OpenCode/Pi and additional VM desktops as
  those harness lanes mature.

Pending:

- Add stripped-env session repair to the curated testing-VM profile set.
- Add a non-Codex host smoke once OpenCode MCP registration is live in the VM.
- Keep host adapters forwarding the desktop env allowlist even though runtime
  repair exists; repair is the fallback path, not the preferred contract.

## Invariants

- Host adapters should still forward desktop env variables when available.
- The client must repair enough environment before spawning an existing or new
  service that startup health can reject stale services missing repaired values.
- The service must rehydrate before desktop probing so direct service runs and
  already-running daemons report the same repair state.
- Repair must be observable through structured data, not only prose. Agents
  should be able to find `doctor.session_env` or `SessionEnvRepaired` in their
  transcript before operating the desktop.
- A repaired `PATH` must include normal system command directories such as
  `/usr/bin` and `/bin`; helper-command lookup failure is a launch-environment
  problem, not evidence that the desktop backend is unavailable.

## Verification

Focused checks:

```bash
cargo fmt --check
cargo test -p sky-cua-linux session_env --lib
cargo test -p sky-cua-client service_launcher --bin sky-cua-client
python3 -m py_compile scripts/_session_env_smoke.py scripts/live_session_env_smoke.py scripts/live_app_server_session_env_smoke.py scripts/live_codex_exec_session_env_smoke.py
uv run ruff check scripts/_session_env_smoke.py scripts/live_session_env_smoke.py scripts/live_app_server_session_env_smoke.py scripts/live_codex_exec_session_env_smoke.py
uv run basedpyright scripts/_session_env_smoke.py scripts/live_session_env_smoke.py scripts/live_app_server_session_env_smoke.py scripts/live_codex_exec_session_env_smoke.py
```

Live proof:

- Direct MCP: `artifacts/session-env-smoke/20260517T080206Z`.
- Rich app-server: `artifacts/codex-e2e/app-server-session-env-smoke/20260517T060242Z`.
- Codex exec: `artifacts/codex-e2e/codex-session-env-smoke/20260517T060439Z`.

The direct smoke starts `zenity`, strips desktop env keys, uses a deliberately
minimal `PATH`, requires `doctor.session_env` to show repair and normalized
path defaults, finds the dialog through MCP, submits `session-env-ok`, and then
verifies the actual dialog stdout. The app-server and Codex exec smokes use the
same fixture and additionally require transcript-visible repair evidence before
accepting the final schema result.
