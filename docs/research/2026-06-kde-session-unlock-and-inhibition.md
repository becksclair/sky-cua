# How sky-cua unlocks and keeps awake a KDE Plasma 6 session (June 2026)

Question: how can a user-owned daemon unlock a locked KDE Plasma 6 Wayland
session without a stored credential, and what actually blocks auto-lock and
auto-suspend while an agent works?

## Unlock without a credential

`org.freedesktop.login1.Manager.UnlockSession` (the **singular** method,
system bus) is authorized by logind's owner short-circuit: when the caller's
uid equals the session owner's uid, the call succeeds with no polkit prompt
and no PAM interaction (`bus_message_check_good_user` in systemd's
`method_lock_session`). kscreenlocker honors the resulting logind `Unlock`
signal via `KSldApp::doUnlock()` with no password check. The plural
`UnlockSessions` is different: it hits polkit `auth_admin` and must not be
used. Equivalent: `Session.Unlock` on the session object.

Live proof (2026-06-12, KDE Plasma 6 Wayland, systemd-logind): a locked
session (`LockedHint=true`) unlocked within ~1 s of `UnlockSession("3")`,
no prompt. Windows has no programmatic unlock counterpart (LockWorkstation
exists; unlock does not; the secure desktop is LocalSystem-only).

## Two inhibitors on two buses

On Plasma 6, kscreenlocker runs its own KIdleTime timer and **ignores**
logind's `idle` inhibitor. Blocking the two automatic behaviors therefore
takes two different handles from two different daemons:

- Auto-lock: session-bus `org.freedesktop.ScreenSaver.Inhibit(app, reason)`
  returns a `u32` cookie; release with `UnInhibit(cookie)`. The cookie is
  auto-released if the requesting D-Bus connection drops, so the connection
  must be held alive for the duration. GNOME proxies the same interface, so
  it doubles as the portable Linux lock-blocker.
- Auto-suspend: system-bus `org.freedesktop.login1.Manager.Inhibit("sleep",
  app, reason, "block")` returns a Unix fd; closing the fd is the release
  (an `OwnedFd` whose `Drop` releases the lock). It shows up in
  `systemd-inhibit --list`.

Both auto-release when the holding process dies, so a crashed daemon never
leaves a desktop permanently unlockable or awake.

## Session resolution and lock state

Resolve the caller's own session with `Manager.GetSession("auto")` (or
`XDG_SESSION_ID`, or `GetSessionByPID`). Lock state is the boolean
`LockedHint` property on `org.freedesktop.login1.Session`; kscreenlocker
sets it on every lock/unlock, making it the canonical signal on Plasma 6.
`LockedHint` lags `UnlockSession` by under a second (D-Bus propagation).

## Windows power requests

`PowerCreateRequest` + `PowerSetRequest` with `PowerRequestSystemRequired`
and `PowerRequestExecutionRequired` block idle suspend; release with
`PowerClearRequest` + `CloseHandle`. Preferred over
`SetThreadExecutionState`, whose semantics changed on Windows 11.
`PowerRequestDisplayRequired` fails with `ERROR_NOT_SUPPORTED` (50) in a
session with no interactive display (e.g. SSH service sessions), so display
inhibition can only be live-proven from a logged-on console session.
Requests are visible in `powercfg /requests` (admin).

macOS primitives, if a backend is ever added: `IOPMAssertionCreateWithName`
with `kIOPMAssertionTypePreventUserIdleDisplaySleep` /
`kIOPMAssertionTypePreventUserIdleSystemSleep`, released via
`IOPMAssertionRelease`; no programmatic unlock.

Shipped implementation: `docs/features/session-presence.md`.
