# Cross-platform session presence: unlock, keep-awake, and decaying wake-lock

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds. This document is maintained in accordance with `/home/bex/.agents/PLANS.md` (the repository does not check that file in; its rules are summarized where they matter below). Plan lifecycle within this repo follows `plans/AGENTS.md`.

## Purpose / Big Picture

Today, if a sky-cua user launches an automation thread remotely (for example, from a phone) and their desktop has auto-locked or is about to suspend, the agent is dead in the water: a locked KDE session routes all input to the password greeter, screen capture shows the greeter, and a suspend drops the whole machine. Earlier exploration confirmed there is currently no code that detects or handles a locked or idle session.

After this change, sky-cua can do three things, gated behind explicit opt-in configuration and reported honestly through `doctor`:

1. Unlock a locked session (Linux/systemd only) without storing or typing any password.
2. Hold the desktop awake — blocking both the automatic screen lock and automatic suspend — for as long as the agent is actively working.
3. Let that hold decay: every tool call refreshes an activity timestamp, and a background task releases the wake-lock (and optionally re-locks the session) once the agent has been quiet for a configurable interval.

You can see it working like this: enable the feature, lock your KDE session, then issue any sky-cua tool call from a second machine. The screen unlocks within a few hundred milliseconds, the session stops auto-locking and auto-suspending while the agent works, and after the agent goes idle past the timeout the machine returns to a locked state on its own. `sky-cua doctor` shows a new `session_presence` report with concrete backend names and honest available/unavailable reasons on every platform.

The single most important non-obvious fact this plan is built on: **on KDE Plasma 6, the screen locker (kscreenlocker) runs its own idle timer and ignores systemd-logind's `idle` inhibitor.** Blocking auto-lock therefore requires the session-bus `org.freedesktop.ScreenSaver` inhibitor, while blocking suspend requires the system-bus `org.freedesktop.login1` inhibitor. These are two different daemons, two different buses, and two different handle types. Unlock is a third, separate, Linux-only capability. The cross-platform design models these as distinct capabilities, never as one.

## Progress

- [x] (2026-06-12 12:00Z) M1 platform contracts landed: session-presence types, default trait methods, readiness booleans, service request/response variants, daemon dispatch for the explicit request path, and an unsupported Linux doctor report until the real Linux backend lands.
- [x] (2026-06-12 12:04Z) M2 Linux inhibitor spike landed in `crates/sky-cua-linux/examples/session_presence_probe.rs`; `status`, `inhibit-suspend 15`, and `inhibit-lock 15` ran on the live KDE Plasma 6 Wayland box. `unlock` was deliberately not run because the orchestrator owns live lock/unlock verification for this task.
- [x] (2026-06-12 12:10Z) M3 Linux backend implementation landed: `SessionPresenceManager` owns logind and ScreenSaver handles, `LinuxDesktopBackend` delegates the trait methods, snapshots and direct doctor calls include the live session-presence report, and focused manager tests plus workspace validation pass.
- [ ] (M4) Daemon lifecycle: env-gated, activity-driven acquire-on-request and timer-driven release, with optional re-lock, reusing `SessionStore`.
- [ ] (M5) Tool surface: a `hold_session` MCP tool (and `unlock_session` alias), the service request/response plumbing, and a CLI subcommand.
- [ ] (M6) Windows backend: suspend/display inhibition via the Windows power-request API; unlock reported as unsupported.
- [ ] (M7) Documentation: `docs/features/session-presence.md`, a section in the `computer-use` skill, and a `ROADMAP.md` entry.

Use timestamps as steps complete, e.g. `- [x] (2026-06-12 14:00Z) M1 done.`

## Surprises & Discoveries

- Observation: The activity-timer primitive is already half-built. `ServiceDaemon` (long-lived) owns `SessionStore`, which already exposes `touch()` and `idle_for()`, and `ServiceDaemon::handle` already calls `self.sessions.touch().await` on every request.
  Evidence: `crates/sky-cua-service/src/session_store.rs:28-37`, `crates/sky-cua-service/src/daemon.rs:88`.
- Observation: A background-watchdog pattern already exists and is the exact shape needed for the decay task.
  Evidence: `ServiceDaemon::spawn_overlay_idle_watchdog` at `crates/sky-cua-service/src/daemon.rs:56-73`.
- Observation: KDE auto-lock is gated by `org.freedesktop.ScreenSaver` inhibition, not by logind idle inhibition; kscreenlocker increments its own `m_inhibitCounter` and consults its own KIdleTime timer.
  Evidence: research against KDE/kscreenlocker `ksldapp.cpp` (`isInhibited()`, `updateIdleTimeout()`) and `interface.cpp` + `dbus/org.freedesktop.ScreenSaver.xml`.
- Observation: The platform default doctor and the Linux doctor builder are separate seams. Adding the optional report field to the trait default does not affect the live Linux `sky-cua-service doctor` output because Linux overrides `doctor()`.
  Evidence: M1 added `session_presence: None` in `crates/sky-cua-platform/src/backend.rs` and a temporary unsupported `DoctorSessionPresenceReport` in `crates/sky-cua-linux/src/doctor.rs`; `cargo run -p sky-cua-service -- doctor` showed `session_presence.backend = "none"`.
- Observation: On this box, logind's own session object path encodes numeric session `3` as `/org/freedesktop/login1/session/_33`; reading the session `Id` property is safer than deriving the id from the object path.
  Evidence: `cargo run -p sky-cua-linux --example session_presence_probe -- status` printed `session_id: 3` and `session_path: /org/freedesktop/login1/session/_33`.
- Observation: The logind sleep inhibitor registers under the example binary name as a block lock while the returned fd is held.
  Evidence: During `inhibit-suspend 15`, `systemd-inhibit --list` showed `sky-cua 1000 bex ... session_presenc sleep automation session active block`.
- Observation: The live KDE box exposes all Linux session-presence capabilities expected by this plan.
  Evidence: After M3, `cargo run -p sky-cua-service -- doctor` reported `can_inhibit_presence: true`, `can_unlock_session: true`, `session_presence.backend = "systemd-logind+screensaver"`, and `unlock.ok`, `inhibit_lock.ok`, `inhibit_suspend.ok`, and `lock_state_readable.ok` all true.

## Decision Log

- Decision: Use the logind `UnlockSession` D-Bus path for unlock rather than typing a stored password through a virtual keyboard.
  Rationale: systemd's `method_lock_session` authorizes the call via a uid-equality short-circuit (`bus_message_check_good_user`) before polkit is consulted, so a process running as the session owner can unlock its own session with no polkit prompt, no PAM, and no stored secret. kscreenlocker honors the resulting logind `Unlock` signal via `KSldApp::doUnlock()` with no password check. This stores no replayable credential and is strictly safer than password injection. The password/uinput path is retained only as documented fallback knowledge for hardened or non-systemd distros.
  Date/Author: 2026-06-12, design phase.
- Decision: Model unlock and inhibition as separate capabilities with independent availability, not a single "presence" toggle.
  Rationale: Unlock is Linux/systemd-only; inhibition is available on Linux, Windows, and macOS by entirely different APIs. Honest per-capability reporting is required by the repo convention (structured diagnostics, concrete backend names, honest fallback states).
  Date/Author: 2026-06-12, design phase.
- Decision: On Linux, hold two inhibitors — a session-bus `org.freedesktop.ScreenSaver` cookie (blocks auto-lock) and a system-bus `org.freedesktop.login1` `sleep` fd (blocks suspend) — rather than the single xdg-desktop-portal `Inhibit` call.
  Rationale: kscreenlocker ignores the logind idle inhibitor, so the ScreenSaver path is mandatory for auto-lock on KDE; the portal route adds a backend dependency and is only needed when sandboxed. The two-handle approach has fewer moving parts for a non-sandboxed daemon and the logind fd is a trivial owned-resource lifetime in Rust.
  Date/Author: 2026-06-12, design phase.
- Decision: Gate the entire feature behind environment variables, default off, and acquire presence only as a side effect of real tool activity (not on daemon startup).
  Rationale: When armed, any MCP client could otherwise unlock the desktop; defaulting off and tying acquisition to activity keeps the blast radius to "an explicitly-armed daemon, while actively working."
  Date/Author: 2026-06-12, design phase.
- Decision: During M1, populate Linux `doctor` with an unsupported `session_presence` report even though the trait default keeps the optional field absent.
  Rationale: The live Linux backend has a custom doctor implementation, so without a temporary Linux-side report the M1 acceptance check would not show the new structured field on this machine. M3 will replace this placeholder with real logind/screensaver checks.
  Date/Author: 2026-06-12, implementation.

## Outcomes & Retrospective

M1 completed the shared contract only. All backends still report unsupported behavior, and the explicit service request path returns the structured unsupported status rather than an error. This does not yet unlock, inhibit, or decay a hold; it makes those behaviors representable and keeps `doctor` honest until the Linux implementation arrives.

M2 proved the safe Linux primitives that this worker is allowed to exercise: the current logind session resolves, `LockedHint` reads, a logind `sleep` fd inhibitor appears in `systemd-inhibit --list`, and the session-bus ScreenSaver inhibitor returns and releases a cookie. This worker did not lock the session and did not run `probe unlock`, by task constraint.

M3 moved the proven Linux primitives into the backend. `SessionPresenceManager` now owns the fd and ScreenSaver cookie lifetimes, exposes idempotent `ensure`, `release`, and `status`, and reports best-effort failures through `SessionPresenceStatus.detail` instead of aborting other presence operations. Live `doctor` now reports concrete logind/screensaver capability checks.

## Context and Orientation

sky-cua is a Rust workspace plus Python harnesses. The runtime is split across crates under `crates/`:

- `sky-cua-platform` holds the shared contracts: the `DesktopBackend` trait (`crates/sky-cua-platform/src/backend.rs`) and the data model including all `Doctor*` report structs (`crates/sky-cua-platform/src/model.rs`, and the service request/response enums under the model module). "Contract" here means the types and trait that every OS backend must speak; clients depend only on these, never on a specific OS crate.
- `sky-cua-linux` and `sky-cua-windows` are the OS backends. Each defines a struct (`LinuxDesktopBackend` at `crates/sky-cua-linux/src/backend.rs:38`, `WindowsDesktopBackend` at `crates/sky-cua-windows/src/backend.rs`) implementing `DesktopBackend`.
- `sky-cua-service` is the long-lived daemon. "Daemon" means a single background process per user that persists across many tool calls; it is launched as `sky-cua-service daemon` and serves requests over a Unix socket (Linux) or TCP (Windows). Its core type is `ServiceDaemon` (`crates/sky-cua-service/src/daemon.rs:22`). The concrete backend is chosen at compile time by `create_backend()` in `crates/sky-cua-service/src/backend_factory.rs` using `#[cfg(target_os = ...)]`.
- `sky-cua-client` hosts the MCP server. "MCP" (Model Context Protocol) is the JSON protocol the agent host speaks; this process is spawned per agent session, reads requests on stdin, and forwards them to the daemon over the socket. Tool calls are dispatched in `crates/sky-cua-client/src/mcp_tools.rs` (`handle_tool_call`) after being routed in `crates/sky-cua-client/src/mcp_server.rs` (`handle_message`).

How a tool call flows: the agent host sends a `tools/call` to `sky-cua-client`, which translates it into a `ServiceRequest` and sends it over the socket to `sky-cua-service`. Every request enters `ServiceDaemon::handle` (`crates/sky-cua-service/src/daemon.rs:87`), which already calls `self.sessions.touch().await` to record activity, then dispatches to the backend. The backend object lives inside the daemon process, so a backend can hold live OS resources (file descriptors, D-Bus connections) in its own fields across many requests — there is no serialization boundary between the daemon and the backend.

Key existing pieces this plan reuses rather than reinvents:

- `SessionStore` (`crates/sky-cua-service/src/session_store.rs`): an `Arc<RwLock<…>>` with `touch()` (updates `last_activity`, bumps a counter) and `idle_for()` (returns `last_activity.elapsed()`). This is the activity clock.
- `ServiceDaemon::spawn_overlay_idle_watchdog` (`crates/sky-cua-service/src/daemon.rs:56-73`): the template for a background task — clone the `Arc<ServiceDaemon>`, `tokio::spawn` a loop on a `tokio::time::interval`. The decay task copies this shape exactly.
- The `DesktopBackend` default-method pattern (`crates/sky-cua-platform/src/backend.rs:82-124`): capability-specific methods (`setup_accessibility`, `reset_portal_tokens`) default to returning `BackendError` with code `ActionUnsupportedForEnvironment`. New capabilities hang off the trait the same way, so backends that do not support them inherit a correct, honest default.
- The doctor report shape (`crates/sky-cua-platform/src/model.rs`, `DoctorReport` and its `Doctor*Report` members, plus the atomic `DoctorCheck { name, ok, detail }`): each subsystem gets an `Option<DoctorXReport>` field, populated by the backend.

The repository already uses `zbus` for D-Bus, but only on the **session** bus (`crates/sky-cua-linux/src/portal/session.rs` calls `zbus::Connection::session()`). logind lives on the **system** bus, which is not connected anywhere yet; `zbus::Connection::system()` provides it with no new dependency. The session-bus `org.freedesktop.ScreenSaver` interface is likewise new to this codebase.

Terms used below:

- "Inhibitor": a handle that, while held, asks a daemon not to perform an automatic action (lock or suspend). Releasing the handle re-enables the action.
- "Cookie": the `u32` token returned by `org.freedesktop.ScreenSaver.Inhibit`; you release it by passing it back to `UnInhibit`. It is also auto-released if the D-Bus connection that requested it drops — so the requesting connection must be kept alive for the whole hold.
- "fd inhibitor": the Unix file descriptor returned by `org.freedesktop.login1.Manager.Inhibit`. There is no uninhibit call; closing the fd (and every duplicate of it) is the release. In Rust this is an `OwnedFd` whose `Drop` releases the lock.

## Plan of Work

The work proceeds in seven milestones. M1 establishes the cross-platform contract with everything unsupported, so the workspace stays green and other platforms get correct defaults immediately. M2 is a deliberate de-risking spike that proves the Linux mechanisms on the real box before any of them are wired into the daemon. M3–M5 build the Linux feature end to end. M6 adds the Windows inhibitor. M7 documents.

### Milestone 1 — Platform contracts (everything unsupported)

In `crates/sky-cua-platform/src/model.rs`, add the session-presence value types and doctor report:

- `SessionPresenceIntent { unlock: bool, inhibit_lock: bool, inhibit_suspend: bool }` — what a caller wants ensured. Serializable.
- `SessionPresenceStatus { backend: String, supported: bool, unlock_supported: bool, locked: Option<bool>, lock_inhibited: bool, suspend_inhibited: bool, detail: String }` — the honest current state, returned from ensure/release/status and embedded in responses. `backend` carries a concrete name such as `"systemd-logind+screensaver"`, `"windows-power-request"`, or `"none"`. `locked` is `None` when lock state is unknowable on this platform.
- `DoctorSessionPresenceReport { backend: String, unlock: DoctorCheck, inhibit_lock: DoctorCheck, inhibit_suspend: DoctorCheck, lock_state_readable: DoctorCheck }`. Add an `Option<DoctorSessionPresenceReport>` field named `session_presence` to `DoctorReport` (with the same `#[serde(default, skip_serializing_if = "Option::is_none")]` treatment as the existing optional reports), and remember to set it to `None` in the default `doctor()` implementation at `crates/sky-cua-platform/src/backend.rs:69-75` so the trait default still compiles.
- Add two readiness booleans to `DoctorReadiness`: `can_inhibit_presence` and `can_unlock_session`, both `#[serde(default)]` so existing serialized reports remain compatible.

In `crates/sky-cua-platform/src/backend.rs`, add three default methods to `DesktopBackend`:

    async fn ensure_session_presence(
        &self,
        _intent: SessionPresenceIntent,
    ) -> Result<SessionPresenceStatus, BackendError> {
        Ok(SessionPresenceStatus::unsupported("none"))
    }

    async fn release_session_presence(
        &self,
        _relock: bool,
    ) -> Result<SessionPresenceStatus, BackendError> {
        Ok(SessionPresenceStatus::unsupported("none"))
    }

    async fn session_presence_status(&self) -> SessionPresenceStatus {
        SessionPresenceStatus::unsupported("none")
    }

Note the deliberate choice: the defaults return `Ok(unsupported)`, not `Err`. The daemon will call `ensure_session_presence` frequently when the feature is armed; an unsupported backend must degrade quietly, not emit an error on every request. Provide a `SessionPresenceStatus::unsupported(backend: &str)` constructor that sets `supported: false`, `unlock_supported: false`, `locked: None`, both inhibited flags false, and a `detail` of `"session presence is not available for this backend"`.

Add a service request/response pair in the service model (the enums `ServiceRequest`/`ServiceResponse` referenced by `crates/sky-cua-service/src/daemon.rs`): `ServiceRequest::SessionPresence { action: SessionPresenceAction }` where `SessionPresenceAction` is `Ensure(SessionPresenceIntent) | Release { relock: bool } | Status`, and `ServiceResponse::SessionPresence { status: SessionPresenceStatus }`. This is the explicit path used by the tool and CLI in M5; the automatic path in M4 calls the backend directly.

Acceptance for M1: `cargo build && cargo test` is green; `sky-cua doctor --json` (however doctor is currently invoked) emits a `session_presence` object whose three checks are `ok: false` with the unsupported detail on the current machine, because no backend overrides the defaults yet.

### Milestone 2 — Linux inhibitor spike (prove it on the real box)

Before integrating anything, validate every Linux mechanism in isolation. Create `crates/sky-cua-linux/examples/session_presence_probe.rs` — a small binary that takes a subcommand argument and exercises one mechanism at a time, printing what it did. This is a throwaway proof, kept additive; it depends only on `zbus` and std.

Embed the following exact D-Bus knowledge in the example (and later reuse it in M3):

Lock state and unlock (system bus, service `org.freedesktop.login1`):

- Resolve the caller's own session: call `org.freedesktop.login1.Manager.GetSession` with the string `"auto"` on object `/org/freedesktop/login1`, or read the `XDG_SESSION_ID` environment variable and pass it, or call `GetSessionByPID` with the current pid. Any of these yields the session object path, e.g. `/org/freedesktop/login1/session/3`.
- Read lock state: the property `LockedHint` (boolean) on interface `org.freedesktop.login1.Session` of that session object. kscreenlocker sets this on every lock and unlock, so it is the canonical signal on Plasma 6.
- Unlock: call `org.freedesktop.login1.Manager.UnlockSession` (the **singular** method) with the session id string. This must be the singular call: the plural `UnlockSessions` passes an invalid uid and hits polkit `auth_admin`, whereas the singular call is authorized by logind's owner short-circuit (the caller's uid equals the session owner's uid) with no polkit prompt and no PAM. Equivalent: `Session.Unlock` on the session object.
- Re-lock (for the decay path): `org.freedesktop.login1.Manager.LockSession` with the session id, or `Session.Lock`.

Suspend inhibitor (system bus): call `org.freedesktop.login1.Manager.Inhibit("sleep", "sky-cua", "automation session active", "block")`. The reply is a Unix file descriptor; hold it to block suspend, close it to release. (Use `what = "sleep:idle:handle-lid-switch"` if also fighting logind's own idle-suspend and lid close; `"sleep"` alone is the minimum to block suspend-on-idle.)

Lock inhibitor (session bus, service `org.freedesktop.ScreenSaver`, object `/org/freedesktop/ScreenSaver`, interface `org.freedesktop.ScreenSaver`): call `Inhibit("sky-cua", "automation session active")`; the reply is a `u32` cookie. Release with `UnInhibit(cookie)`. The requesting connection must stay alive for the duration, since dropping it auto-releases the cookie. This is the call that actually suppresses KDE's auto-lock; it is also honored by GNOME (proxied into gnome-session), so it doubles as the portable Linux lock-blocker.

The example's subcommands and what to observe:

- `probe status` — print `LockedHint` and the resolved session id.
- `probe unlock` — lock your session by hand first (or from another terminal), then run this; the screen must unlock with no prompt. This is the load-bearing live check; if it fails, the whole unlock capability is unavailable on this box and the rest of the plan still stands for inhibition only.
- `probe inhibit-suspend 60` — acquire the logind `sleep` fd, hold 60 seconds, then release. During the hold, `systemd-inhibit --list` must show a `sky-cua` `sleep` block lock.
- `probe inhibit-lock 60` — acquire the ScreenSaver cookie, hold 60 seconds, release. During the hold, set your auto-lock timeout very low (e.g. via System Settings) and confirm the screen does **not** auto-lock; after release, confirm it does.

Acceptance for M2: each subcommand behaves as described on the live KDE Plasma 6 Wayland box. Record the observed transcripts in `Artifacts and Notes`. If `probe unlock` fails, note it in `Surprises & Discoveries` and set `unlock_supported` expectations accordingly; do not block the rest of the plan.

### Milestone 3 — Linux backend implementation

Create a `logind` and screensaver client module set under `crates/sky-cua-linux/src/session_presence/`:

- `mod.rs` — public `SessionPresenceManager` and exports.
- `logind.rs` — system-bus connection (`zbus::Connection::system()`), session resolution, `LockedHint`, `UnlockSession`, `LockSession`, and the `sleep` fd inhibitor.
- `screensaver.rs` — session-bus `org.freedesktop.ScreenSaver` `Inhibit`/`UnInhibit`, keeping the connection alive while the cookie is held.

`SessionPresenceManager` mirrors the existing `RemoteDesktopSessionManager` ownership style (`crates/sky-cua-linux/src/portal/remote_desktop.rs`): an `Arc<RwLock<SessionPresenceState>>` where the state holds `Option<OwnedFd>` (sleep inhibitor), `Option<(zbus::Connection, u32)>` (the screensaver connection plus cookie), the cached system-bus connection, and the resolved session path. It exposes async `ensure(intent)`, `release(relock)`, and `status()`:

- `ensure(intent)` is idempotent. It lazily connects the buses on first use; if `intent.unlock` and `LockedHint` is true, it calls `UnlockSession`; if `intent.inhibit_suspend` and no fd is held, it acquires the `sleep` fd; if `intent.inhibit_lock` and no cookie is held, it acquires the ScreenSaver cookie. Already-held resources are left untouched, so the hot path after the first acquisition is cheap (a couple of in-memory checks plus, when unlock is requested, a fast `LockedHint` read). Every step is best-effort: a failure to acquire one handle is recorded in the returned `SessionPresenceStatus.detail` but does not abort the others or error the call.
- `release(relock)` closes the fd (drop), calls `UnInhibit` and drops the screensaver connection, and if `relock` is true calls `LockSession`. Idempotent.
- `status()` reads `LockedHint` and reports which handles are currently held.

Wire it into `LinuxDesktopBackend` (`crates/sky-cua-linux/src/backend.rs:38`) as a new field, lazily initialized like `virtual_input` (an `Arc<OnceLock<…>>` or an always-constructed manager — the manager itself defers bus connections, so it can be constructed eagerly and cheaply in `new()`). Implement the three trait methods to delegate to the manager. Populate `DoctorSessionPresenceReport` in the Linux doctor builder (`crates/sky-cua-linux/src/doctor.rs`): `unlock` ok if the system bus connects and a session resolves; `inhibit_suspend` ok if the system bus connects; `inhibit_lock` ok if `org.freedesktop.ScreenSaver` is owned on the session bus (check with `org.freedesktop.DBus.NameHasOwner`); `lock_state_readable` ok if `LockedHint` reads. Set `backend` to `"systemd-logind+screensaver"`. Set the readiness booleans accordingly.

Acceptance for M3: with the feature not yet armed, `sky-cua doctor` on the KDE box shows `session_presence.backend = "systemd-logind+screensaver"` and the appropriate checks green; a focused unit/integration test that constructs the manager and calls `status()` returns a status with `supported: true`.

### Milestone 4 — Daemon activity-driven lifecycle

Add env-gated configuration, read once at daemon construction (`ServiceDaemon::new`, `crates/sky-cua-service/src/daemon.rs:32`):

- `SKY_CUA_PRESENCE_ENABLED` (default unset/false) — master switch.
- `SKY_CUA_PRESENCE_IDLE_RELEASE_SECS` (default `90`) — release the hold after this much inactivity.
- `SKY_CUA_PRESENCE_UNLOCK` (default `true`) — whether to unlock a locked session when acquiring.
- `SKY_CUA_PRESENCE_RELOCK` (default `true`) — whether to re-lock on release.
- `SKY_CUA_PRESENCE_INHIBIT_LOCK` / `SKY_CUA_PRESENCE_INHIBIT_SUSPEND` (default `true`) — which inhibitors to hold.

This mirrors the existing `SKY_CUA_VIRTUAL_INPUT_*` env convention already in the Linux crate.

Acquire on the hot path, release on the timer:

- In `ServiceDaemon::handle` (`crates/sky-cua-service/src/daemon.rs:87`), after the existing `self.sessions.touch().await`, if the feature is enabled and the request is a desktop-affecting one (everything except `Health` and pure status queries), call `self.backend.ensure_session_presence(intent).await` and ignore the value for control flow (it is idempotent and fast once held). Do this before dispatching the action so the screen is unlocked and inhibited before the action executes — this avoids the race where the first action would otherwise land on the lock screen. Acquiring before the action costs the unlock+inhibit latency only on the first call after an idle release; log the returned `detail` at debug level.
- Add `spawn_session_presence_watchdog`, copied from `spawn_overlay_idle_watchdog` (`crates/sky-cua-service/src/daemon.rs:56-73`). On each tick (every ~1s), if the feature is enabled and `self.sessions.idle_for().await >= idle_release`, call `self.backend.release_session_presence(relock).await` and remember that presence is released so the next `ensure` re-acquires. Spawn it next to the overlay watchdog at the daemon startup site (`crates/sky-cua-service/src/ipc_server.rs`, where `spawn_overlay_idle_watchdog` is currently spawned).

Track a small "presence held" boolean (in a `tokio::sync::Mutex` field on `ServiceDaemon`, or inside the manager) so `ensure` after a release correctly re-runs the full acquisition and the watchdog does not repeatedly call `release` once already released.

Acceptance for M4: with `SKY_CUA_PRESENCE_ENABLED=1` and a low auto-lock timeout, lock the KDE session, issue one tool call from a second machine, and observe the screen unlock and stay awake while you issue more calls; stop issuing calls and observe the machine re-lock after `SKY_CUA_PRESENCE_IDLE_RELEASE_SECS`. With the variable unset, none of this happens and behavior is identical to today.

### Milestone 5 — Tool surface

Add an MCP tool `hold_session` (with `unlock_session` as an alias that implies `unlock: true`) in `crates/sky-cua-client/src/mcp_tools.rs`, dispatched like the other tools. It maps to `ServiceRequest::SessionPresence { action: Ensure(intent) }` and returns the `SessionPresenceStatus` as structured tool output. Handle the request in `ServiceDaemon::handle` by delegating to the backend. Add a `release_session` tool mapping to `Release { relock }`, and surface `Status` for diagnostics. Add a `sky-cua session-presence <ensure|release|status>` subcommand in `crates/sky-cua-service/src/main.rs` for manual operation, following the existing CLI subcommand pattern.

The explicit tool exists for the phone-launch flow: the agent calls `hold_session` (or `unlock_session`) as its deliberate first step. The automatic M4 path is the safety net so even an agent that forgets the explicit call still unlocks before its first action.

Acceptance for M5: from a connected MCP client, calling `hold_session` returns a status object with the held inhibitors and the unlocked state; calling `release_session` with `relock: true` re-locks. The CLI subcommand does the same from a shell.

### Milestone 6 — Windows backend

Implement the three trait methods on `WindowsDesktopBackend` (`crates/sky-cua-windows/src/backend.rs`). Use the Windows power-request API as primary (`PowerCreateRequest` then `PowerSetRequest` with `PowerRequestExecutionRequired`, `PowerRequestSystemRequired`, and `PowerRequestDisplayRequired`; release with `PowerClearRequest` then `CloseHandle`), preferring it over `SetThreadExecutionState` because the latter's behavior changed on Windows 11. Report `inhibit_lock`/`inhibit_suspend` as available, `backend = "windows-power-request"`, and `unlock_supported: false` with a `detail` explaining that Windows has no programmatic unlock (there is `LockWorkstation` but no unlock counterpart; the secure desktop is LocalSystem-only). `ensure_session_presence` with `unlock: true` simply skips the unlock step and records that in `detail`; it does not error.

Acceptance for M6: on a Windows host (or in CI cross-compile plus a manual check), `doctor` shows the Windows power-request backend with unlock unsupported and inhibition supported; holding presence prevents idle sleep/display-off; releasing restores normal behavior.

macOS and any other platform are intentionally left to the default-unsupported trait methods (the `UnsupportedDesktopBackend` fallback in the factory covers a missing backend). For completeness, the macOS primitives, if a backend is ever added, are `IOPMAssertionCreateWithName` with `kIOPMAssertionTypePreventUserIdleDisplaySleep`/`kIOPMAssertionTypePreventUserIdleSystemSleep`, released via `IOPMAssertionRelease`, and no unlock is possible. Note this in the feature doc so the placeholder is intentional, not an oversight.

### Milestone 7 — Documentation

Create `docs/features/session-presence.md` following `docs/AGENTS.md`'s template: what the feature does, the per-platform capability matrix (Linux: unlock + lock-inhibit + suspend-inhibit; Windows: inhibit only; macOS/other: placeholder), the env configuration, the `doctor` fields, and the KDE-specific fact that auto-lock is blocked by the ScreenSaver inhibitor while suspend is blocked by logind. Add a section to the `computer-use` skill (`skills/computer-use/`) telling the agent to call `hold_session`/`unlock_session` as the first step of a remotely-launched thread when `doctor` reports a locked or lockable session, and that it is opt-in. Add a `ROADMAP.md` entry linking the feature doc.

## Concrete Steps

Run all commands from the repository root `/home/bex/projects/sky-cua` unless noted.

Build and test the whole workspace after each milestone:

    cargo fmt --check
    cargo build
    cargo test

Run the M2 spike (after locking your session from a second terminal or the menu):

    cargo run -p sky-cua-linux --example session_presence_probe -- status
    cargo run -p sky-cua-linux --example session_presence_probe -- unlock
    cargo run -p sky-cua-linux --example session_presence_probe -- inhibit-suspend 60
    cargo run -p sky-cua-linux --example session_presence_probe -- inhibit-lock 60

While an inhibitor is held, in another terminal confirm it is registered:

    systemd-inhibit --list

Exercise the integrated feature (M4), from the desktop machine:

    SKY_CUA_PRESENCE_ENABLED=1 SKY_CUA_PRESENCE_IDLE_RELEASE_SECS=30 sky-cua-service daemon

Then lock the session and issue a tool call from the second machine through the normal MCP path; observe unlock, sustained wake, and re-lock after 30 seconds of inactivity.

Inspect doctor output:

    sky-cua doctor --json

Expect a `session_presence` object; on the KDE box its `backend` is `systemd-logind+screensaver` and its checks are green once M3 lands.

When this plan's harness work touches Python (it should not, but if packaging changes), also run:

    uv run ruff format --check scripts && uv run ruff check scripts && uv run basedpyright && uv run pytest

## Validation and Acceptance

The feature is done when, on the live KDE Plasma 6 Wayland box with `SKY_CUA_PRESENCE_ENABLED=1`:

1. Locking the session and issuing any sky-cua tool call from a second machine unlocks the screen within roughly one timer tick, with no password prompt and no stored credential anywhere in the codebase.
2. While tool calls continue, the session neither auto-locks nor auto-suspends, even with both timeouts set aggressively low. `systemd-inhibit --list` shows a `sky-cua` `sleep` lock, and the screensaver does not fire.
3. After tool calls stop for `SKY_CUA_PRESENCE_IDLE_RELEASE_SECS`, the inhibitors release and (with re-lock enabled) the session returns to locked on its own.
4. With `SKY_CUA_PRESENCE_ENABLED` unset, none of the above happens; behavior is byte-for-byte today's behavior. This is the regression guard.
5. `sky-cua doctor` reports `session_presence` with concrete backend names and honest checks on Linux and Windows; on a platform with no backend it reports unsupported, not an error.
6. `cargo fmt --check && cargo test` is green; new unit tests for the manager's idempotent ensure/release and for the status mapping pass, and they fail if the default-unsupported behavior is accidentally turned into an `Err`.

Phrase any new automated test so it fails before the change and passes after: e.g. a service-level test that arms the feature with a fake backend recording ensure/release calls, drives a request plus an idle interval, and asserts exactly one `ensure` on the request and one `release` after the idle threshold.

## Idempotence and Recovery

Every acquire and release is idempotent: `ensure` holds at most one of each handle and re-acquires only what was released; `release` tolerates being called with nothing held. The fd inhibitor releases automatically if the daemon dies (the kernel closes the fd), and the ScreenSaver cookie releases automatically if the daemon's session-bus connection drops — so a crashed daemon never leaves the desktop permanently un-lockable or awake. Re-running the spike example is safe and self-contained. The env-gated default-off design means a partially-implemented or misbehaving feature cannot affect users who have not opted in. If unlock proves unavailable on a given box (the M2 `unlock` check fails), the inhibition half still functions and `doctor` reports unlock as unsupported with a reason.

## Artifacts and Notes

Record M2 spike transcripts here as indented blocks as they are produced, for example:

    $ cargo run -p sky-cua-linux --example session_presence_probe -- status
    session: /org/freedesktop/login1/session/3
    LockedHint: true

    $ cargo run -p sky-cua-linux --example session_presence_probe -- unlock
    requested UnlockSession(3); LockedHint now: false

Keep these concise and focused on proving the mechanism.

M1 validation:

    $ cargo fmt --check
    success

    $ cargo build
    Finished `dev` profile [unoptimized + debuginfo] target(s)

    $ cargo test
    test result: ok. Workspace unit and doc tests passed.

    $ cargo run -p sky-cua-service -- doctor
    "session_presence": {
      "backend": "none",
      "unlock": { "ok": false, "detail": "session presence is not available for this backend" },
      "inhibit_lock": { "ok": false, "detail": "session presence is not available for this backend" },
      "inhibit_suspend": { "ok": false, "detail": "session presence is not available for this backend" },
      "lock_state_readable": { "ok": false, "detail": "session presence is not available for this backend" }
    }

M2 safe live probe transcripts:

    $ cargo run -p sky-cua-linux --example session_presence_probe -- status
    session_id: 3
    session_path: /org/freedesktop/login1/session/_33
    LockedHint: false

    $ cargo run -p sky-cua-linux --example session_presence_probe -- inhibit-suspend 15
    holding logind sleep inhibitor for 15s

    $ systemd-inhibit --list
    sky-cua        1000 bex  3207106 session_presenc sleep  automation session active  block

    # same inhibit-suspend command after the hold elapsed:
    released logind sleep inhibitor

    $ cargo run -p sky-cua-linux --example session_presence_probe -- inhibit-lock 15
    holding ScreenSaver inhibitor cookie 637 for 15s
    released ScreenSaver inhibitor cookie 637

M2 validation:

    $ cargo fmt --check
    success

    $ cargo build
    Finished `dev` profile [unoptimized + debuginfo] target(s)

    $ cargo test
    test result: ok. Workspace unit and doc tests passed.

M3 live doctor evidence:

    $ cargo run -p sky-cua-service -- doctor
    "readiness": {
      "can_inhibit_presence": true,
      "can_unlock_session": true
    }
    "session_presence": {
      "backend": "systemd-logind+screensaver",
      "unlock": { "ok": true, "detail": "logind session 3 LockedHint=false" },
      "inhibit_lock": { "ok": true },
      "inhibit_suspend": { "ok": true },
      "lock_state_readable": { "ok": true, "detail": "logind session 3 LockedHint=false" }
    }

M3 focused and workspace validation:

    $ cargo test -p sky-cua-linux session_presence -- --nocapture
    test result: ok. 2 passed.

    $ cargo fmt --check
    success

    $ cargo build
    Finished `dev` profile [unoptimized + debuginfo] target(s)

    $ cargo test
    test result: ok. Workspace unit and doc tests passed.

## Interfaces and Dependencies

In `crates/sky-cua-platform/src/backend.rs`, extend `DesktopBackend`:

    async fn ensure_session_presence(
        &self,
        intent: SessionPresenceIntent,
    ) -> Result<SessionPresenceStatus, BackendError>;
    async fn release_session_presence(
        &self,
        relock: bool,
    ) -> Result<SessionPresenceStatus, BackendError>;
    async fn session_presence_status(&self) -> SessionPresenceStatus;

with default bodies returning `Ok(SessionPresenceStatus::unsupported("none"))` for the first two and the unsupported status for the third.

In `crates/sky-cua-platform/src/model.rs`, define `SessionPresenceIntent`, `SessionPresenceStatus` (with `unsupported(backend: &str) -> Self`), `DoctorSessionPresenceReport`, the `session_presence: Option<DoctorSessionPresenceReport>` field on `DoctorReport`, the `can_inhibit_presence`/`can_unlock_session` fields on `DoctorReadiness`, and the `ServiceRequest::SessionPresence`/`ServiceResponse::SessionPresence` variants with `SessionPresenceAction`.

In `crates/sky-cua-linux/src/session_presence/mod.rs`, define:

    pub struct SessionPresenceManager { /* Arc<RwLock<SessionPresenceState>> */ }
    impl SessionPresenceManager {
        pub fn new() -> Self;
        pub async fn ensure(&self, intent: SessionPresenceIntent) -> SessionPresenceStatus;
        pub async fn release(&self, relock: bool) -> SessionPresenceStatus;
        pub async fn status(&self) -> SessionPresenceStatus;
    }

Dependencies: reuse the workspace `zbus` (already a dependency of `sky-cua-linux`) for both system and session buses; no new crate is needed on Linux. On Windows, use the existing `windows`/`windows-sys` binding the Windows crate already pulls in for the `Power` functions (`PowerCreateRequest`, `PowerSetRequest`, `PowerClearRequest`, `CloseHandle`), adding the `Win32_System_Power` feature if not already enabled. Do not add the xdg-desktop-portal `Inhibit` path; it is reserved as a documented sandboxed fallback only.

---

Change note (2026-06-12, initial authoring): Created from a design discussion that established, through source-level research, that (a) logind `UnlockSession` unlocks a KDE session via a uid-equality short-circuit with no polkit/PAM/password, (b) KDE auto-lock is blocked only by the session-bus `org.freedesktop.ScreenSaver` inhibitor, not the logind idle inhibitor, and (c) the existing `SessionStore`/`spawn_overlay_idle_watchdog` infrastructure already provides the activity clock and background-task pattern the decaying wake-lock needs. The plan deliberately separates unlock (Linux-only) from inhibition (cross-platform) and gates everything behind default-off env configuration.
