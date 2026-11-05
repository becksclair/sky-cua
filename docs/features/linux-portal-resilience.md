# Linux portal and EIS resilience

## Status

Shipped. Live-proven across Plasma, GNOME, Hyprland, COSMIC, and i3/X11 on
2026-05-23; source validation re-verified in the current tree on 2026-08-12.

## Summary

The Linux RemoteDesktop/EIS path recovers from stale devices and failed worker
startup, cleans up portal sessions on timeout and reset, and preserves correct
keyboard modifiers and compositor targeting across supported desktops.

## Contract surface

- Existing desktop action and capture APIs are unchanged.
- Portal setup and action failures return structured backend errors instead of
  panicking.
- EIS keyboard resolution supports Shift and AltGr/Level3 from the active XKB
  keymap, including Unicode keysyms.
- `desktop_scroll` accepts all four directions. Exact XWayland targets keep
  pointer movement and vertical or horizontal wheel injection in one XTest
  state; the action fails closed if XTest is unavailable.
- Native Wayland horizontal scrolling stays on EIS for both targeted and
  originless actions. It does not mix EIS pointer motion with legacy
  `NotifyPointerAxis` calls from a session that may forbid that legacy lane.
- Explicit portal denial remains distinct from an unavailable portal and is
  not silently bypassed.
- Portal identity (2026-11-05): before its first ashpd portal request the
  daemon registers its portal connection as `com.heliasar.SkyCua` through
  `org.freedesktop.host.portal.Registry`. The installer projects the hidden
  `com.heliasar.SkyCua.desktop` entry the portal requires to resolve that id.
  Without the entry (checkout runs) registration fails with a warning and the
  portal falls back to the spawning scope's id.
- KDE preauthorization (`kde-authorized` RemoteDesktop and screenshot rows)
  covers `""`, `desktop`, `com.anthropic.Claude` and `com.heliasar.SkyCua`, so
  a fresh grant under any of those ids completes without a dialog. On GNOME the
  seeded restore token carries a permission row for each of those ids, because
  the frontend only restores for an app id with a row.
- Portal activation gate (2026-11-05): the environment probe only queries
  `org.freedesktop.portal.Desktop` when it is already running or the systemd
  user environment has `XDG_CURRENT_DESKTOP` (or that environment cannot be
  read). Doctor's portal interface checks run only against a running portal,
  since introspecting its well-known name D-Bus-activates it. A portal
  activated before the desktop session names itself falls back to gtk and
  exposes no ScreenCast, RemoteDesktop or Screenshot for the whole session.
- With no capture backend, captures fail with a cause-specific code:
  `PortalSessionNotReady` (the probe deferred activation; retry after the
  session finishes starting) or `PortalBackendMissing` (the running portal
  has no capture implementation; when it was started without
  `XDG_CURRENT_DESKTOP` the message names its pid and the
  `systemctl --user restart xdg-desktop-portal` remedy). `doctor` reports the
  same `{code, message}` as `portal.capture_unavailable` and uses the message
  as `readiness.recommended_next_step`.
- `PortalSessionRestoreMiss` is reported when a persisted token was sent but
  the portal returned a different one (it granted a fresh session instead of
  restoring); `PortalSessionRestored` only when the same token came back.

## Behavior

Interactive portal setup creates the session before entering the bounded
selection/start phase. Timeout or setup failure explicitly closes that
session. Capture retry, session reset, and persisted-token reset also close the
old session before dropping it.

EIS worker startup waits asynchronously without holding the RemoteDesktop
state lock. A session generation fences late worker startup, and paused,
stopped, or removed devices trigger reacquisition. EIS failures may fall back
to the legacy path; invalid requests do not force an unnecessary session
reset.

Hyprland discovery keys its cache by display, rejects stale instances, filters
unmapped windows, and avoids inheriting stale compositor environment into
probe commands.

KWin's XWayland bridge needs one narrowly gated wheel retry after an absolute
XTest move. Other compositors receive one injection. The live pointer fixture
checks the resulting widget adjustment cardinality, so one requested scroll
must still produce exactly one observed vertical or horizontal movement.

xdg-desktop-portal consumes a restore token at `SelectDevices` and re-saves
the restore data under the same token at `Start`, and only restores for the
app id that owns the token. The app id of an unsandboxed process comes from
its systemd scope, so before registration the daemon's id changed with
whichever host spawned it (`com.anthropic.Claude`, `claude`, `""`, ...). A
daemon under another id could not restore the stored token, received a fresh
grant, and replaced the file with a token the next daemon could not use; on
KDE, a fresh grant for an id outside `kde-authorized` shows the interactive
dialog, and the combined session start waits for it until its 12 s timeout
(`PortalApprovalPending`). A timed-out `Start` leaves the token consumed, so
the retry prompts again.

## Source paths

- `crates/sky-cua-linux/src/portal/remote_desktop.rs` — session lifecycle
- `crates/sky-cua-linux/src/portal/portal_session.rs` — setup and cleanup,
  restore-outcome classification
- `crates/sky-cua-linux/src/portal/host_app.rs` — portal app id registration
- `crates/sky-cua-linux/src/portal/preauthorize.rs` — preauthorized app ids
- `crates/sky-cua-linux/src/portal/readiness.rs` — activation gate and
  missing-backend explanation
- `scripts/_portal_desktop_entry.py` — installed desktop entry
- `crates/sky-cua-linux/src/portal/eis_fallback.rs` — fallback and worker
  generation fencing
- `crates/sky-cua-linux/src/portal/eis_input.rs` — device lifecycle and worker
- `crates/sky-cua-linux/src/portal/eis_keymap.rs` — XKB inverse mapping
- `crates/sky-cua-linux/src/windowing/hyprland.rs` — compositor selection

## Verification

- Focused portal, EIS keymap/input, session lifecycle, and Hyprland cache
  regression tests cover the repaired paths.
- Full live pointer/input/overlay smokes passed on Plasma, GNOME, Hyprland,
  COSMIC, and i3/X11 on 2026-05-23.
- Plasma re-passed with reduced EIS delays and cached virtual-input devices.
- On 2026-08-12 the installed Plasma runtime passed native Wayland vertical
  and horizontal scrolling through EIS and exact XWayland vertical and
  horizontal scrolling through XTest, with one observed fixture adjustment
  per request.

## Known limitations

- Portal approval remains an interactive desktop boundary when no valid
  restore token exists.
- Environment-specific live smokes are required after changes to portal,
  compositor, or physical input behavior; headless tests do not replace them.

## Related

- [`docs/features/linux-virtual-input.md`](linux-virtual-input.md)
- [`docs/runtime/linux-architecture.md`](../runtime/linux-architecture.md)
- ROADMAP entry: [`ROADMAP.md`](../../ROADMAP.md) under "Linux desktop parity"
- Originating ExecPlan retired into this feature doc; see git history for
  `plans/portal_review_fixes.md`.
