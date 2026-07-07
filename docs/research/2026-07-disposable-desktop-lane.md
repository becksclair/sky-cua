# Disposable desktop lane: productizing the isolated xpra desktop

## Context

The isolated xpra desktop (`docs/features/isolated-xpra-desktop.md`) shipped
as a non-interference feature: one private X11 desktop the computer-use
agent can drive without touching the human's live session. As of 2026-07-07
it is fully live-proven — host leak guard, MCP end-to-end, and the
`isolated-xpra` VM smoke profile all pass (see the feature doc `Status`).
That closes the last verification gap for the feature *as shipped*: one
sandbox, one human host, config-file driven.

This raises a design question the shipped feature does not answer: what
would it take to turn this into a first-class **disposable/headless CUA
lane** — spin up a throwaway desktop, drive it, tear it down, on a machine
with no human session at all (CI, a build box, a fleet of parallel agent
runs)? This doc answers that question from the actual code, not aspiration.

## 1. Session lifecycle

### What create -> use -> destroy looks like today

**Create.** `ServiceClient::connect_or_spawn` (`crates/sky-cua-client/src/service_launcher.rs:64-171`)
resolves the isolated selection via `resolve_isolated_desktop_selection()`
(`crates/sky-cua-platform/src/config.rs:506-509`), which loads the single
machine config file (`load_machine_config`, `config.rs:335-353`, at
`~/.config/sky-cua/sky-cua.toml` or `SKY_CUA_CONFIG_PATH`,
`config.rs:310-330`) and layers per-process env overrides on top
(`resolve_isolated_desktop`, `config.rs:516-542`: env beats file beats
default for every field). If `enabled`, `IsolatedDesktopHandle::ensure(&cfg)`
(`crates/sky-cua-client/src/isolated_desktop.rs:98-185`) does the real work:
probe required binaries (`ensure_dependencies`, referenced at
`isolated_desktop.rs:102`), resolve the display string (`resolve_display`,
`isolated_desktop.rs:414-463` — `"auto"` scans for a free number and
persists the choice to `$XDG_RUNTIME_DIR/sky-cua/isolated-display`; an
explicit `":N"` is used verbatim), resolve the resolution
(`resolve_resolution`/`largest_monitor_dimensions`, `isolated_desktop.rs:514-546`
— probes the *host's* `xrandr` for 3/4-of-largest-monitor sizing, falling
back to `1920x1080` when unreachable), and either reuse a healthy existing
xpra session on that display (`xpra_session_is_healthy`,
`isolated_desktop.rs:118-148, 716-722`) or launch a fresh one
(`start_xpra_desktop`, `isolated_desktop.rs:566-593`, an `xpra
start-desktop ... --daemon=yes` headless virtual-X allocator with no host
graphical-session dependency). Bring-up also resolves the sandbox D-Bus
session bus — preferring xpra's own `--dbus-launch` recorded in `xpra info`
(`xpra_info_dbus_address`, `isolated_desktop.rs:750-759`), falling back to a
client-owned `dbus-daemon` (`owned_bus::start_owned_session_bus`,
`crates/sky-cua-client/src/isolated_desktop/owned_bus.rs:24-51`) persisted
per display number at `$XDG_RUNTIME_DIR/sky-cua/isolated-bus-<N>`
(`owned_bus.rs:70-75`).

The client then redirects `SERVICE_SOCKET_PATH_ENV` at the isolated socket
(`service_launcher.rs:116-118`), builds a
`LaunchEnvironment::for_isolated_daemon(&handle.spawn_env(),
&handle.removed_env())` (`service_launcher.rs:128-133`,
`crates/sky-cua-client/src/launch_environment.rs`) scoping health
expectations to the sandbox's own graphical identity rather than the host's,
spawns (or reuses) the isolated `sky-cua-service` daemon on that socket, and
launches the configured viewer (`launch_isolated_viewer`,
`service_launcher.rs:152-153, 167-168`; warn-only on failure).

**Use.** Every `desktop_*` MCP tool call flows through the normal
`ServiceClient::call` path against the isolated daemon's socket. The one
isolation-specific gate is `desktop_launch_app`: it is advertised in every
session but refuses at call time with `IsolatedDesktopRequired` when
`ServiceClient::is_isolated()` is false (`service_launcher.rs:198-208`; gate
enforced in `crates/sky-cua-client/src/mcp_tools.rs` per the feature doc).

**Destroy.** `lifecycle` (`persistent` default, or `ephemeral`) decides
whether anything happens automatically. `shutdown_isolated_if_ephemeral`
(`service_launcher.rs:223-244`) is a no-op unless `isolated_lifecycle ==
Lifecycle::Ephemeral`, in which case it calls `IsolatedDesktopHandle::stop`
(`isolated_desktop.rs:288-304`): stop the xpra session, reap a persisted
owned bus by pid (`reap_persisted_owned_bus`), `SIGTERM` the dedicated
isolated daemon after verifying its pid is a live `sky-cua-service`
(`terminate_isolated_daemon`, `isolated_desktop.rs:805-824`, filtered
strictly by the resolved display number so the user's real daemon is never
touched), and remove a stale `/tmp/.X<N>-lock`. This runs on both the
normal MCP exit and the pipe-error exit (feature doc, `main.rs` shutdown
seam per `crates/sky-cua-client/src/main.rs:47-105`). It does **not** run on
`SIGKILL`/panic — the feature doc lists this explicitly as a known
limitation, recoverable via `sky-cua-client isolated-desktop stop`.

### What persists where

- **On disk:** the machine config file (global, one `[isolated_desktop]`
  table); `$XDG_RUNTIME_DIR/sky-cua/isolated-display` (the persisted `auto`
  display-number choice — one file, no per-session key);
  `$XDG_RUNTIME_DIR/sky-cua/isolated-bus-<N>` (owned-bus address/pid, keyed
  by display number); `/tmp/.X<N>-lock` (X11's own display lock).
- **As processes:** the xpra `start-desktop` server itself (Openbox inside
  it), an isolated `sky-cua-service` daemon per socket, and — only on the
  owned-bus fallback — a `dbus-daemon`.
- **Nothing** is scoped to a session/request ID today; everything is scoped
  to the display number, which is itself either a fixed default (`:100`) or
  a singly-persisted `auto` choice.

### What a first-class "disposable session" surface would add

Three shapes, weighed:

- **MCP tool** (e.g. `isolated_desktop_create`/`_destroy` alongside
  `desktop_launch_app`): fits the existing tool-gating pattern
  (`IsolatedDesktopRequired`-style structured errors) and needs no new
  transport, but conflates session lifecycle (an infrastructure concern)
  with the tool surface an LLM reasons about mid-task — every agent
  conversation would need to explicitly manage sandbox teardown, which is
  exactly the kind of side-channel bookkeeping the tool surface currently
  avoids by resolving isolation once at `connect_or_spawn` time.
- **Installer/bootstrap mode** (a flag on `install.py`/`bin/` wrappers that
  provisions a disposable sandbox as part of environment setup): natural for
  a CI job that wants "give me an isolated desktop for the duration of this
  run" as a fixture, but it's a heavier lift — it would have to reproduce
  the dependency-presence checks, display allocation, and teardown-on-exit
  semantics that `isolated_desktop.rs` already owns, just from a different
  entry point.
- **Operator CLI** (extending the existing hidden `isolated-desktop
  {ensure|status|stop}` subcommand, `crates/sky-cua-client/src/main.rs:105-157`,
  with a `create`/`destroy` pair that takes an explicit display and a
  `--ttl`/`--ephemeral` flag): the cheapest change — the subcommand already
  wraps `IsolatedDesktopHandle::ensure`/`stop` directly, so a disposable
  variant is mostly plumbing an explicit display argument and a lifecycle
  flag through the existing dispatch (`main.rs:105-157`) rather than new
  machinery. This is the natural v1 surface: it reuses the proven
  ensure/stop code paths verbatim and keeps the MCP tool surface unchanged.

## 2. Concurrency

**Can N isolated desktops coexist today? Only if each is given an explicit,
distinct display.** Walking the actual collision points:

- **Display-number allocation.** An explicit `display` config value that is
  already up is *reused*, not rejected (`isolated_desktop.rs:118-148`): two
  processes both resolving to `:100` (the default) do not error, they end up
  sharing one xpra session — not two isolated desktops, one. Real N-way
  concurrency requires each process to resolve to a *different* display,
  which today means each process needs a distinct explicit
  `SKY_CUA_ISOLATED_DESKTOP_DISPLAY` (or config `display`) value set by
  whatever orchestrates them. The `"auto"` path does not solve this safely:
  `resolve_display` (`isolated_desktop.rs:414-463`) reads a *single* shared
  state file (`$XDG_RUNTIME_DIR/sky-cua/isolated-display`, not keyed by
  session), and the read-decide-scan-persist sequence has no lock — two
  processes calling `ensure()` with `display = "auto"` concurrently can both
  read the same persisted number (or both scan and land on the same first
  free number in `first_available_display_number`,
  `isolated_desktop.rs:470-478`) before either has bound `xpra
  start-desktop`, and then race to start two xpra servers on the same
  display number. This is the first thing that breaks, and it breaks
  silently (whichever `xpra start-desktop` call loses just fails to bind,
  surfacing as an opaque "exited with status" error, not a collision
  diagnostic).
- **Per-display service socket.** Once display numbers *are* distinct, the
  socket layer is fine: `isolated_socket_path` (`isolated_desktop.rs:488-496`)
  names the daemon socket `service-isolated-<N>.sock` purely from the
  display number (confirmed live by the VM proof's `service-isolated-131.sock`),
  so distinct displays get distinct sockets and distinct daemons with no
  extra work.
- **xpra's own socket directory.** Not sky-cua's to manage — xpra keys its
  own IPC socket by display number internally (`xpra list`/`xpra info
  <display>` address a specific display), so distinct display numbers do
  not collide there either. Unverified in this repo's code (xpra is an
  external dependency); noted as an assumption based on xpra's documented
  per-display socket convention, not code read in this repository.
- **The D-Bus bus.** Also keyed correctly once displays differ: the
  xpra-provided bus is read fresh per display via `xpra info :N`
  (`isolated_desktop.rs:750-759`), and the owned-bus fallback persists
  under `isolated-bus-<N>` (`owned_bus.rs:70-75`) — no shared file, no
  collision across distinct display numbers.
- **Config persistence.** The machine config's `[isolated_desktop]` table is
  a single global set of defaults (`window_manager`, `viewer`, `lifecycle`,
  `resolution`, and the *default* `display`) — there is no per-session
  config file. Concurrent sessions that want different window managers or
  resolutions must each set distinct env overrides
  (`ISOLATED_DESKTOP_*_ENV`, `config.rs:517-538`); the file itself cannot
  express "session A gets openbox, session B gets a different WM"
  simultaneously. This is a real limitation for a fleet use case, but not a
  crash — it just pushes all per-session variation onto env vars supplied
  by whatever orchestrates N concurrent clients.

**What breaks first:** the `"auto"` display allocator's shared,
unsynchronized state file — it is the one path in this module explicitly
designed to hand back a *fresh* number per call, and it is also the one
path with no locking. **What a fix would touch:** `resolve_display`
(`isolated_desktop.rs:414-463`) and `isolated_display_state_path`
(`isolated_desktop.rs:480-484`) would need either a file lock around the
scan-persist sequence, or (better for a disposable-lane use case) a
per-session key so each caller's "auto" choice is independent rather than a
single machine-wide persisted value — the latter is a bigger contract
change since the persisted value is currently *meant* to be reused across a
session's restarts, not scoped per-request.

## 3. Headless viability

Verified in code (see the isolated-desktop research spike this doc
consolidates):

- **Capture and input never touch portals in the isolated lane.**
  `infer_session_kind` classifies the sandbox's `XDG_SESSION_TYPE=x11`/
  `DISPLAY=:N` env as `SessionKind::X11`
  (`crates/sky-cua-linux/src/env_probe.rs`), and both
  `select_capture_backend` and `select_input_backend` route `X11` to
  X11-native mechanisms (XShm-based `x11_capture::capture_still` via
  `crates/sky-cua-linux/src/capture_plan.rs`, and XTest input) — the portal/
  PipeWire/RemoteDesktop branches exist only under the `Wayland` arm. A
  headless CI box with no compositor and no portal service running is not a
  blocker for capture or input inside the sandbox.
- **`xpra start-desktop` itself needs no host graphical session** —
  confirmed by `start_xpra_desktop` (`isolated_desktop.rs:566-593`) and the
  live VM proof (COSMIC guest, sandbox brought up over SSH-driven
  provisioning with no interactive host session). D-Bus is satisfied either
  by xpra's own `--dbus-launch` or the owned-bus fallback
  (`owned_bus.rs:24-51`), neither of which depends on a host session bus.
- **What *does* assume a host session, and degrades gracefully:**
  `largest_monitor_dimensions` (`isolated_desktop.rs:539-546`) shells out to
  `xrandr --current` inheriting the *client process's own* `DISPLAY` (the
  host's, not the sandbox's `:N`) when `resolution = "auto"`; with no host
  display it returns `None` and `resolve_resolution` falls back to the
  literal `1920x1080` (`isolated_desktop.rs:507, 514-524`) — non-fatal.
  `launch_viewer(ViewerMode::Attach)` (`isolated_desktop.rs:227-248`) spawns
  `xpra attach :N --readonly` inheriting the client's own environment, so it
  targets the host's real display to render a window; with no host display
  this attach fails, but `launch_isolated_viewer`
  (`service_launcher.rs:152-153, 167-168`) only logs a warning — it never
  blocks or fails session bring-up. `ViewerMode::Html5`
  (`isolated_desktop.rs:249-278`) starts an xpra HTML5 listener and logs a
  URL; it has no host-display dependency at all and is the natural viewer
  choice for a headless preset. `LaunchEnvironment::for_isolated_daemon`
  (used, not `probe()`, on the isolated bring-up path per
  `service_launcher.rs:128-133`) never depends on host session identity —
  `probe()` is reserved for the non-isolated path.

**What a `headless` preset would change:** default `resolution` to an
explicit value (skip the `xrandr` probe, since there is no host monitor to
size against and the fallback already exists) and default `viewer = "none"`
or `"html5"` (skip `attach`, which targets a host display that does not
exist in this preset). Both are config-only changes — no source path in
`isolated_desktop.rs` needs new branches, since the fallback behavior for
"no host display" already exists for both. This is a strong signal the
headless case is close to already-supported by defaults, just not
packaged as an explicit preset.

## 4. Non-goals

- **Wayland-native isolation.** The xpra lane is X11 by design — the
  feature doc's `Behavior` section documents the deliberate
  `XDG_SESSION_TYPE=x11`/`DISPLAY=:N` env recipe that short-circuits
  `env_probe`'s Wayland-voting `/proc` scan
  (`crates/sky-cua-linux/src/env_probe.rs`). A Wayland-native equivalent
  (a private Wayland compositor/session) is a different, unbuilt feature,
  not a variant of this one; nothing in this doc assumes it exists.
- **Multi-tenant security hardening.** The feature doc states plainly: "This
  is a non-interference feature, not a security sandbox: the private
  desktop runs as the same OS user with the same filesystem, network, and
  system D-Bus access." A disposable lane inherits that boundary
  unchanged — it does not add process isolation, filesystem scoping, or
  network restriction between concurrent sandboxes, and per this project's
  explicit performance-over-security stance (`CLAUDE.md` "Security &
  Secrets": "Project tradeoffs prioritize maximum performance over security
  hardening... unless explicitly requested"), building that out is
  out of scope unless a maintainer explicitly asks for it.

## 5. Costs and recommendation

| Option | Effort (coarse) | What it reuses | What it adds |
|---|---|---|---|
| Operator-CLI disposable pair (`isolated-desktop create/destroy --display :N --ephemeral`) | Small | `IsolatedDesktopHandle::ensure`/`stop` verbatim, existing subcommand dispatch | Explicit-display argument, TTL/ephemeral flag plumbing |
| `headless` config preset (`resolution` default off, `viewer=none`/`html5`) | Small | Existing fallback behavior for absent host display | A named preset so operators don't have to know the fallback exists |
| Per-session `auto`-allocation fix (lock or per-caller key) | Medium | `resolve_display`'s existing scan logic | File-lock or key-scoping around the read-decide-persist sequence |
| First-class MCP disposable-session tool | Medium-large | Existing tool-gating pattern (`IsolatedDesktopRequired`) | New tool surface, new lifecycle bookkeeping exposed to the LLM mid-conversation — conflates infra lifecycle with task tools |
| N-concurrent-sandbox fleet support (distinct WM/viewer/resolution per session) | Large | Per-display socket/bus keying (already correct) | Per-session config scoping beyond env-var overrides; the machine-config-is-global assumption threaded through `config.rs` would need revisiting |

**Recommendation:** ship the operator-CLI disposable pair plus the
`headless` preset first. Both are small, additive to code that is already
live-proven, and directly unlock "spin up a throwaway sandbox on a CI box
with no host session" — the concrete headless-lane ask — without touching
the config-is-global assumption or the tool-gating contract. Defer the MCP
tool and the fleet-concurrency fix until there's a real caller: the MCP tool
adds session-lifecycle bookkeeping to the LLM-facing surface that nothing
today needs, and the concurrency fix is speculative work against a
single-sandbox-at-a-time reality that has never been exercised live.

**Open questions for the maintainer, carried forward rather than decided
here:**

1. Is a disposable desktop lane a **sky-cua feature** (shipped, documented,
   config-driven like the rest of `[isolated_desktop]`) or an **operator
   recipe** (a documented `scripts/`/runbook pattern layered on the existing
   `isolated-desktop` subcommand, with no new source contract)? The
   small-effort options in the table above work either way; the answer
   mostly decides whether this doc's proposals graduate into
   `docs/features/isolated-xpra-desktop.md` or into
   `docs/operations/`.
2. Given the `"auto"`-allocation race and the global-config assumption
   documented in Section 2, is **one sandbox at a time** the honest v1
   scope for a disposable lane, with concurrent fleets explicitly
   out-of-scope until there's a real multi-sandbox caller? This doc's
   recommendation assumes yes.

## Implications

If accepted, this narrows the next isolated-desktop work to two small,
additive changes (operator-CLI disposable pair, `headless` preset) instead
of a speculative MCP-tool or concurrency redesign, and defers the harder
fleet-concurrency question until a real caller exists. No source changes
are made by this doc; it is research only.
