# Session presence

## Status

Shipped. Linux unlock and lock/suspend inhibition, the shared contract, daemon
lifecycle, MCP tools, CLI surfaces, and Windows power-request inhibition are
all implemented. Last verified: 2026-06-12 (live KDE Plasma 6 Wayland
lock/unlock/inhibit/decay cycle; Windows power requests observed in
`powercfg /requests` on a Windows 11 host). macOS and other targets
intentionally report unsupported status until a backend lands.

## Summary

Session presence lets a remote agent deliberately prepare an interactive
desktop session before operating it. On supported Linux desktops it can unlock
the current logind session, block automatic screen lock, block suspend, and
release those holds after the daemon has been idle long enough.

## Contract surface

Platform model in `crates/sky-cua-platform/src/model.rs`:

- `SessionPresenceIntent` carries `unlock`, `inhibit_lock`, and
  `inhibit_suspend`.
- `SessionPresenceStatus` returns `backend`, `supported`,
  `unlock_supported`, `locked`, `lock_inhibited`, `suspend_inhibited`, and
  `detail`.
- `DoctorSessionPresenceReport` returns `backend` plus four `DoctorCheck`
  fields: `unlock`, `inhibit_lock`, `inhibit_suspend`, and
  `lock_state_readable`.
- `DoctorReadiness` includes `can_inhibit_presence` and
  `can_unlock_session`.
- `DoctorReport.session_presence` is optional. Direct status defaults use
  `SessionPresenceStatus::unsupported("none")`; backends that emit an
  unsupported doctor report use `DoctorSessionPresenceReport::unsupported`.
- `ServiceRequest::SessionPresence` carries `SessionPresenceAction::Ensure`,
  `Release { relock }`, or `Status`; `ServiceResponse::SessionPresence`
  carries the status.

MCP tools in `crates/sky-cua-client/src/mcp_tools/definitions.rs`:

| Tool | Arguments | Behavior |
| --- | --- | --- |
| `hold_session` | `unlock` default `false`; `inhibit_lock` default `true`; `inhibit_suspend` default `true` | Ensures the requested holds through the daemon. |
| `unlock_session` | `inhibit_lock` default `true`; `inhibit_suspend` default `true` | Ensures presence with `unlock` forced to `true`. |
| `release_session` | `relock` default `false` | Releases held inhibitors and optionally requests a session lock. |
| `session_presence_status` | none | Reports current backend support and held state. |

CLI surfaces:

| Binary | Command | Notes |
| --- | --- | --- |
| `sky-cua-client` | `session-presence ensure|hold [--unlock|--no-unlock] [--inhibit-lock|--no-inhibit-lock] [--inhibit-suspend|--no-inhibit-suspend]` | Sends a persistent ensure request to the long-lived daemon. |
| `sky-cua-client` | `session-presence release [--relock|--no-relock]` | Releases the daemon-held resources; release defaults to `--no-relock`. |
| `sky-cua-client` | `session-presence status` | Reports daemon status as JSON. |
| `sky-cua-service` | `session-presence ensure|hold|release|status` with the same flags | Operates on a direct backend instance. A direct `ensure` holds resources only for that process lifetime. |

Automatic daemon configuration in `crates/sky-cua-service/src/daemon.rs`:

| Environment variable | Default | Meaning |
| --- | --- | --- |
| `SKY_CUA_PRESENCE_ENABLED` | `false` | Enables automatic presence acquisition before desktop-affecting requests and idle watchdog release. |
| `SKY_CUA_PRESENCE_IDLE_RELEASE_SECS` | `90` | Idle duration before the watchdog releases a held session. Invalid or unset values use the default. |
| `SKY_CUA_PRESENCE_UNLOCK` | `true` | Whether automatic acquisition requests unlock. |
| `SKY_CUA_PRESENCE_RELOCK` | `true` | Whether automatic idle release requests a lock after releasing inhibitors. |
| `SKY_CUA_PRESENCE_INHIBIT_LOCK` | `true` | Whether automatic acquisition holds the desktop lock/screensaver inhibitor. |
| `SKY_CUA_PRESENCE_INHIBIT_SUSPEND` | `true` | Whether automatic acquisition holds the system suspend inhibitor. |

Boolean env parsing accepts `1`, `true`, `yes`, and `on` as true, and `0`,
`false`, `no`, and `off` as false. Unknown values fall back to the default.
`SKY_CUA_PRESENCE_ENABLED` is a hard gate for state-changing requests: when it
is off, the daemon rejects explicit `Ensure` and `Release` session-presence
requests (MCP tools and CLI alike) with `ActionUnsupportedForEnvironment`, so
no local socket client can unlock or inhibit the desktop while the feature is
disabled. `Status` requests remain available and report support honestly. `.mcp.json`, `scripts/install_mcp_server.py`, and
`resources/chrome_preflight.py` include the six `SKY_CUA_PRESENCE_*` names in
the installed MCP env allowlists so host-launched clients can forward the
opt-in.

Per-platform capability matrix:

| Platform | Backend name | Unlock | Lock inhibit | Suspend inhibit | Status |
| --- | --- | --- | --- | --- | --- |
| Linux with systemd-logind and `org.freedesktop.ScreenSaver` | `systemd-logind+screensaver` | Supported through logind `UnlockSession` when `LockedHint` is readable | Supported through the session-bus ScreenSaver inhibitor | Supported through a system-bus logind `sleep` fd inhibitor | Shipped on this branch |
| Windows | `windows-power-request` | Unsupported; Windows has `LockWorkstation` but no equivalent user-session unlock API | Supported through `PowerRequestDisplayRequired` (fails honestly in sessions with no interactive display) | Supported through `PowerRequestSystemRequired` and `PowerRequestExecutionRequired` | Shipped |
| macOS | `none` until a backend is added | Unsupported | Placeholder primitive would be `IOPMAssertionCreateWithName(kIOPMAssertionTypePreventUserIdleDisplaySleep)` | Placeholder primitive would be `IOPMAssertionCreateWithName(kIOPMAssertionTypePreventUserIdleSystemSleep)` | Intentionally unsupported placeholder |
| Other targets | `none` | Unsupported | Unsupported | Unsupported | Default unsupported backend |

## Behavior

The explicit tools and CLI commands are direct control surfaces. `hold_session`
can request unlock and either inhibitor; `unlock_session` always requests
unlock first; `release_session` drops inhibitors and locks only when `relock`
is true; `session_presence_status` is read-only. Responses always use
`SessionPresenceStatus` so callers can branch on structured fields instead of
parsing text.

The automatic daemon lifecycle is default-off. When
`SKY_CUA_PRESENCE_ENABLED` is true, `ServiceDaemon::handle` tries to acquire
presence before desktop-affecting requests. Health, doctor,
`agent_cursor_status`, explicit session-presence requests, browser status, and
browser tab listing do not trigger the automatic hold. A daemon-held boolean
keeps acquisition to once per held window, and
`spawn_session_presence_watchdog` releases after
`SKY_CUA_PRESENCE_IDLE_RELEASE_SECS` of inactivity.

On Linux, `SessionPresenceManager` owns the live OS resources. Unlock and
re-lock use the caller's resolved logind session id and the system-bus
`org.freedesktop.login1.Manager` `UnlockSession` and `LockSession` methods.
Suspend inhibition holds the fd returned by
`org.freedesktop.login1.Manager.Inhibit("sleep", "sky-cua",
"automation session active", "block")`. Lock inhibition holds a session-bus
connection plus the cookie returned by
`org.freedesktop.ScreenSaver.Inhibit("sky-cua", "automation session active")`,
then releases it with `UnInhibit(cookie)`.

The KDE distinction is load-bearing: Plasma's screen locker is blocked by the
session-bus `org.freedesktop.ScreenSaver` inhibitor, while suspend is blocked
by the system-bus logind `sleep` fd inhibitor. A logind idle inhibitor alone
does not block KDE auto-lock.

Linux `doctor` derives readiness from the structured report:
`can_inhibit_presence` is true when either `inhibit_lock` or
`inhibit_suspend` is ok, and `can_unlock_session` is true when both `unlock`
and `lock_state_readable` are ok.

## Source paths

- `crates/sky-cua-platform/src/model.rs` and
  `crates/sky-cua-platform/src/model/service.rs` - shared model and service
  variants
- `crates/sky-cua-platform/src/backend.rs` - default unsupported trait methods
- `crates/sky-cua-linux/src/session_presence/` - Linux logind and ScreenSaver
  manager
- `crates/sky-cua-linux/src/backend.rs` and
  `crates/sky-cua-linux/src/doctor.rs` - Linux backend delegation and doctor
  readiness
- `crates/sky-cua-service/src/daemon.rs` and
  `crates/sky-cua-service/src/ipc_server.rs` - env-gated lifecycle and idle
  watchdog
- `crates/sky-cua-service/src/main.rs` - direct service CLI
- `.mcp.json`, `resources/chrome_preflight.py`, and
  `scripts/install_mcp_server.py` - installed host env allowlists
- `crates/sky-cua-client/src/mcp_tools.rs`,
  `crates/sky-cua-client/src/mcp_tools/definitions.rs`, and
  `crates/sky-cua-client/src/operator_cli.rs` - MCP tools and operator CLI
- `skills/computer-use/SKILL.md` - runtime agent guidance for remote launch
  sessions

## Verification

Focused validation recorded in `plans/session-presence.md`:

```bash
cargo test -p sky-cua-linux session_presence -- --nocapture
cargo test -p sky-cua-service automatic_session_presence_acquires_once_and_releases_after_idle -- --nocapture
cargo test -p sky-cua-client session_presence -- --nocapture
cargo test -p sky-cua-service session_presence -- --nocapture
cargo fmt --check
cargo build
cargo test
```

Safe live status proof recorded there:

```bash
cargo run -p sky-cua-service -- session-presence status
```

The accepted Linux status used `backend = "systemd-logind+screensaver"`,
`supported = true`, `unlock_supported = true`, `locked = false`, and both
inhibitors released. Live lock, unlock, re-lock, and aggressive idle-timeout
flows were reserved for the orchestrator because the implementation worker was
not allowed to lock or unlock the active desktop session.

## Known limitations

- The automatic lifecycle is opt-in through `SKY_CUA_PRESENCE_ENABLED`; default
  behavior remains unchanged.
- The Linux implementation depends on systemd-logind and a session-bus
  `org.freedesktop.ScreenSaver` owner for full capability.
- Windows uses the `windows-power-request` backend, with inhibition only and
  no unlock support. `PowerRequestDisplayRequired` fails with
  `ERROR_NOT_SUPPORTED` in sessions without an interactive display (for
  example SSH service sessions); the failure is reported in `detail` and does
  not abort the other holds.
- macOS and other platforms intentionally report unsupported status. A future
  macOS backend would use IOPM assertions for idle display/system sleep
  inhibition and would still have no unlock primitive.
- Explicit `sky-cua-service session-presence ensure` is useful for status and
  manual diagnostics, but a direct process cannot persist inhibitors after it
  exits; use the client/operator path for daemon-held resources.

## Related

- ROADMAP entry: [`ROADMAP.md`](../../ROADMAP.md) under "Linux desktop parity"
- Research notes: [`docs/research/2026-06-kde-session-unlock-and-inhibition.md`](../research/2026-06-kde-session-unlock-and-inhibition.md)
