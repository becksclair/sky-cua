---
name: verify-sky-cua
description: Launch a self-owned, isolated sky-cua instance (second daemon on a private xpra/Openbox X11 display, built from this checkout) on a Linux host and drive its MCP desktop tools the way an agent would, capturing evidence. Use to prove a sky-cua desktop change works in the real runtime (launch app, observe, set value, activate, type, pointer, doctor) without touching the user's running daemon, MCP registrations, config, or desktop session. Not for unit tests, the Arch testing-vm lanes (use vm-tests), browser-use, or phone-use.
---

# Verify sky-cua

Primary surface: the MCP server `sky-cua-client mcp` (stdio), backed by a
`sky-cua-service` daemon. This skill builds both from the current checkout into
a scratch tree on a Linux host, starts them on the shipped isolated-desktop lane
(`docs/features/isolated-xpra-desktop.md`), and drives the MCP tools with the
repo's own stdio harness (`scripts/_mcp_stdio.py`).

Everything goes through one script: `.agents/skills/verify-sky-cua/scripts/verify.sh`
(below: `verify.sh`). Run it from the checkout on any machine with SSH to the host.

## Host and isolation

- `VERIFY_HOST` (default `asgard`, the CachyOS KDE workstation) runs the instance.
  macOS cannot: the service has no macOS backend. `VERIFY_HOST=local` runs on the
  current machine, which must be Linux with a user session (`XDG_RUNTIME_DIR`).
- Host requirements: `cargo` (toolchain from `rust-toolchain.toml`), `python3`,
  `rsync`, `xpra`, `openbox`, `xdotool`, `/usr/lib/at-spi-bus-launcher`,
  `/usr/lib/at-spi2-registryd`, GStreamer `ximagesrc`, and `zenity` for the
  default journeys.
- Everything the skill writes on the host lives under `~/.cache/sky-cua-verify/`:
  `src/` (synced checkout), `target/` (cargo target), `state/` and `config/`
  (`XDG_STATE_HOME`/`XDG_CONFIG_HOME` for the instance, so the user's
  `~/.config/sky-cua/sky-cua.toml` is never read), `browser-sockets/` (empty
  browser socket dir), and `runs/<run-id>/` (evidence).
- The instance: private display `VERIFY_DISPLAY` (default `:147`), its own socket
  `$XDG_RUNTIME_DIR/sky-cua/service-isolated-147.sock`, private D-Bus and AT-SPI
  buses, `SKY_CUA_SURFACES=desktop`, `SKY_CUA_PHONE=0`, viewer `none`, lifecycle
  `persistent`. Every inherited `SKY_CUA_*` key is stripped first.
- The user's daemon (`~/.local/share/sky-cua/.../sky-cua-service daemon`, on
  `service.sock`) can coexist. `launch` records it; `doctor` and `teardown` fail
  if the set of foreign daemons changed.

Hard rules:

- Never stop, restart, signal, or reconfigure a `sky-cua-service` that is not
  `~/.cache/sky-cua-verify/target/release/sky-cua-service`. Never `pkill -f`;
  kill by recorded PID only.
- Never run `install.py install`, `scripts/install_mcp_server.py`, or the `bin/`
  wrappers for verification. `bin/sky-cua-service` falls back to the user's
  `target/release` build, and the installers rewrite live registrations.
- Never point `VERIFY_DISPLAY` at a display you did not create. `launch` refuses
  a display held by a non-xpra server.
- Only the isolated lane is safe here. Driving the Wayland/portal lane would act
  on the user's real screen.

## Launch

```bash
verify.sh sync     # copy tracked + untracked-unignored files to the host's src/
verify.sh build    # cargo build --release -p sky-cua-client -p sky-cua-service (first build ~6 min, then incremental)
verify.sh launch   # isolated-desktop ensure, then `health` spawns the isolated daemon
```

Ready means `launch` exits 0 and its last line reads
`ready: display=:147 daemon_pid=<pid> socket=... run=~/.cache/sky-cua-verify/runs/<run-id>`.
That line requires a daemon from the scratch binary holding the isolated socket
lock and answering `health`. `launch` starts a new run id and writes it to
`~/.cache/sky-cua-verify/current-run`. Later commands use it, or `VERIFY_RUN_ID`.
Run `sync` and `build` again after every source change. `doctor` flags a stale build.

## Doctor

```bash
verify.sh doctor
```

This is read-only and starts nothing. It requires `verdict=drivable`, which means:
binaries present, `build_fresh=true`, `status.up=true` on the expected display
and geometry, every `status.dep_*=true`, `isolated_daemon_pid` whose `exe` is the
scratch binary, `isolated_daemon_health=ok`, and `user_daemons_unchanged=true`.
Run it first whenever anything looks off.

## Drive

```bash
verify.sh drive                      # all journeys: smoke, semantic, keyboard
verify.sh drive --journey semantic   # one journey (smoke | semantic | keyboard)
verify.sh cli list-windows           # operator CLI against the isolated instance
verify.sh cli get-app-state --window-title <title> --detail compact
```

`drive` runs `scripts/drive_isolated.py`, which talks MCP over stdio to a fresh
`sky-cua-client mcp`. That client attaches to the running isolated daemon. Each
journey launches its own app with `desktop_launch_app`, so journeys are
independent. The feature map lists the journeys and their exact tool calls:
`features/README.md`.

Tool contract to follow when adding a journey (see `skills/computer-use/SKILL.md`):

- Observe first: `observe {surface: "desktop", window_title: <title>}`. Read
  `structuredContent.result.appshot_id`,
  `structuredContent.result.action_snapshot.snapshot_id`, and
  `structuredContent.result.semantic_projection.elements[]`. Each element has
  `element_index`, `role`, `name`, `value`, `semantic_actions`, and `backend_ref`.
- Every state-changing call needs the current `appshot_id`. Element targets pair
  `element_index` with the `snapshot_id` from the same observation.
- For a state change, record the initial structured value, act once, re-observe,
  and require `final == requested` and `final != initial`.
- Prove side effects outside sky-cua too (a file the app wrote, its exit code),
  not only sky-cua's `success: true`.
- `list_resources {surface: "desktop", resource: "windows"}` takes no title
  filter (`title_contains` is browser-only and returns `InvalidRequest`); match
  titles client-side.
- Give launched apps a unique title marker so window and host-leak checks are
  exact.

## Evidence

Each run writes `~/.cache/sky-cua-verify/runs/<run-id>/` on the host. `pull`
(also the last step of `all`) copies it to
`<checkout>/artifacts/verify-sky-cua/<run-id>/` on the operator machine, which is
gitignored. Contents:

- `summary.json`: per-journey verdicts and every check with its detail. `drive`
  exits non-zero if any check fails.
- `steps.jsonl`: every MCP call (arguments, full response, image refs) in order.
- `images/`: AppShot window crops (`*.webp`) and full-display source captures
  (`<snapshot_id>.png`) copied out of `$XDG_RUNTIME_DIR/sky-cua/{appshots,captures}`
  before teardown deletes them.
- `<journey>-zenity-output.txt`, `<journey>-zenity-exit.txt`: app-side proof.
- `cli-<time>-<command>.out`: stdout of every `verify.sh cli` call.
- `tools-list.json`, `launch-ensure.txt`, `launch-health.json`, `src-stamp.txt`
  (synced git rev and dirty-path count), `isolated-daemon.pid`,
  `daemon-service-isolated-147.log` (daemon tracing), `foreign-daemons-before.txt`,
  `runtime-dir-before.txt`, `teardown.txt`.

Report the check lines from `summary.json` and point at one before and one after
image. A screenshot alone is not proof.

## Cleanup

```bash
verify.sh teardown   # then: verify.sh pull
```

`teardown` copies the daemon log into the run dir. It then runs
`sky-cua-client isolated-desktop stop`, which reaps xpra, the private buses, the
AT-SPI owners, and the isolated daemon (identity-checked by `/proc/<pid>/exe`).
It also deletes only this run's AppShot and capture files and their sibling
variants (by ids found in `steps.jsonl` and `cli-*.out`), plus unheld lock files
for its own display. It removes the
`appshots/` and `captures/` dirs only if this run created them and they are empty.
It must print `teardown: residue=none`, and the user's daemons must be unchanged.
`teardown.txt` also lists `runtime_dir_new_entries=` (entries in
`$XDG_RUNTIME_DIR/sky-cua` that did not exist at launch). That list should be
empty. Report anything listed instead of deleting it, because it may belong to
the user's daemon.
It is idempotent: run it after every failed or interrupted attempt.

Evidence in `runs/` and `artifacts/verify-sky-cua/` survives teardown. To reclaim
host disk later (about 1.2 GB of target), remove everything except evidence:
`ssh $VERIFY_HOST 'cd ~/.cache/sky-cua-verify && rm -rf src target state config browser-sockets src-files.txt src-stamp.txt'`.

One-shot: `verify.sh all` runs sync, build, launch, doctor, drive, teardown, and
pull. Teardown runs even if an earlier step failed, and the exit status is
non-zero if any step failed.

## Status

2026-11-08, host asgard: smoke, semantic, and keyboard PASS (33 of 33 checks),
`residue=none`, user daemon unchanged. The keyboard journey caught a product
defect at creation: the X11 input fallback typed with `xdotool --window`, which
sends `XSendEvent` events that GTK ignores, so `desktop_keyboard` reported
`success: true` while nothing reached the app. The fallback now focuses the
target window and uses XTest (`crates/sky-cua-linux/src/x11/input_xtest.rs`,
`focus_target`). Do not weaken a journey to make it pass.

## Gotchas

- `AgentCursorHostUnavailable` (overlay exit 127) appears in action diagnostics
  on this lane. It is expected: the agent-cursor overlay is Wayland layer-shell
  only. Ignore it.
- `doctor`/`list-windows` report `compositor: kde-kwin-wayland` while
  `session_kind=x11` and `display=:147`. The compositor field reflects the host
  session, not the private display. Judge the lane by `session_kind`, `display`,
  and `input_backend=x_test`.
- `desktop_launch_app` returns `DestinationAppShotUnavailable` diagnostics for
  `sh -c` launches. Poll `list_resources` windows for the title marker instead.
- The host leak probe reads each X server's `-auth` from its cmdline. Some
  displays stay `unqueryable` (for example a second Xwayland with
  per-client auth). The check fails only on `present`. The authoritative
  containment proof is the launched app's `/proc/<pid>/environ` (private
  `DISPLAY`, no `WAYLAND_DISPLAY`, private session bus).
- `xpra stop did not report success (exit status 18)` during a repeated
  teardown means the session was already gone. That is fine when
  `residue=none`.
- `cargo` must not use the user's checkout or `target/` on the host. The skill
  always builds `~/.cache/sky-cua-verify/src` into `~/.cache/sky-cua-verify/target`.
- Not covered here: browser-use (not routed into the isolated desktop), phone-use
  (needs a device; see `phone-use`), Wayland portal/KWin/overlay lanes (see
  `vm-tests`), and Windows.
