# GUI Desktop Test Harness

This is the preferred Linux test path for the `sky-cua` plugin, MCP server,
desktop backends, and agent-cursor overlay work. Unit tests cover parsers,
serialization, and routing, but the important Linux behavior lives inside real
desktop sessions: KWin, GNOME Shell, COSMIC, Hyprland, i3, portals, AT-SPI,
PipeWire capture, X11 input, and compositor-rendered overlays.

The preferred environment is an Arch Linux `testing-vm` managed through
QEMU/libvirt/virt-manager. The VM should boot into the target desktop session
as its own guest display, not a nested compositor in a container. Build
`sky-cua` runtime binaries on the host and push them into the VM; the VM is a
clean production-like smoke environment, not a Rust build worker.

For agent-run VM smoke work, use the local `$vm-tests` skill at
`.agents/skills/vm-tests/`. It points agents at this document, the current
runner, and the testing-VM desktop-smoke reference before choosing commands, so
the workflow starts from the active matrix instead of stale nested-X11 or Docker
paths.

## Provisioning

Provision a fresh Arch guest by copying this repository into the VM or scp-ing
the provisioner, then run it as root:

```bash
sudo SKY_CUA_TESTING_VM_USER=skycua \
  SKY_CUA_TESTING_VM_SESSION=cosmic \
  CODEX_DESKTOP_PACKAGE=/path/to/codex-desktop.pkg.tar.zst \
  bash scripts/testing-vm/provision-arch-testing-vm.sh
```

`SKY_CUA_TESTING_VM_SESSION` selects the autologin session that greetd starts on
the VM display. Supported values are `cosmic`, `cosmic-blank`,
`cosmic-transparent`, `kde`, `plasma`, `gnome`, `hyprland`, and `i3`.

The provisioner was retargeted from the retired Arch Docker image and keeps the
same dependency intent:

- base/runtime: `bash`, `git`, `grep`, `openssh`, `python`, `rsync`, `sudo`
- build/proof tools: `base-devel`, `clang`, `gcc`, `cmake`,
  `extra-cmake-modules`, `ninja`, `pkgconf`, `rust`
- GUI runtime libraries: AT-SPI, DBus, GTK, Qt6, X11 libraries, Mesa, NSS,
  PipeWire, Wayland, and the portal backends for COSMIC, GNOME, Hyprland,
  KDE/Plasma, wlroots, and GTK fallback
- desktop stacks: KWin/Plasma, GNOME Shell, COSMIC, Hyprland, i3, Xwayland,
  and their portal backends
- terminal apps: COSMIC Terminal, Konsole, GNOME Terminal, GNOME Console,
  foot, xterm, Alacritty, Kitty, WezTerm, and Ghostty, so every installed
  desktop has a native or practical terminal target for launch/list/focus tests
- smoke tools: `gst-plugins-good`, ImageMagick, `grim`, `jq`, `kdialog`,
  `libinput`, `openbox`, `slurp`, `socat`, `strace`, `wev`, `weston`,
  `wl-clipboard`, `wmctrl`, `xdotool`, `ydotool`/`ydotoold`, Xorg, xauth,
  xdpyinfo, xev, xmessage, xwininfo, and `zenity`
- browser-use smoke browser: Google Chrome installed from Google's stable
  Linux package
- Codex Desktop: installed from the local CodexDesktop-Rebuild Arch package
  when `CODEX_DESKTOP_PACKAGE` is set
- OpenCode CLI: installed from npm with `OPENCODE_NPM_SPEC`, defaulting to
  `opencode-ai@latest`, so future non-Codex harness work can run in the same
  production-like VM

The VM should have a visible virt-manager/virt-viewer console and SSH access.
If libvirt's default network is absent, direct QEMU user networking with an SSH
forward is acceptable for automation, but visual proof still comes from the VM
display.

When opening the viewer from an agent shell, detach it from the command session
so the viewer process survives after the command returns:

```bash
setsid -f virt-viewer --connect qemu:///session testing-vm \
  >/tmp/sky-cua-virt-viewer.log 2>&1
```

For a persistent background viewer that auto-restarts and survives the agent
session, use the systemd user service:

```bash
# Install the service
cp scripts/testing-vm/virt-viewer-testing-vm.service \
  ~/.config/systemd/user/virt-viewer-testing-vm.service
systemctl --user daemon-reload
systemctl --user enable virt-viewer-testing-vm.service
systemctl --user start virt-viewer-testing-vm.service
```

Control it later with:

```bash
systemctl --user status virt-viewer-testing-vm.service
systemctl --user stop virt-viewer-testing-vm.service
systemctl --user start virt-viewer-testing-vm.service
```

If the viewer appears blank, first capture the libvirt framebuffer with
`virsh --connect qemu:///session screenshot testing-vm <path>.png` and check
the guest session processes over SSH. A blanked or locked guest display can
look like an overlay failure even when the overlay is not drawing anything.

## Runner

Run profiles from the host with:

```bash
python3 scripts/run_gui_testing_vm_smoke.py --host testing-vm --profile cosmic-helper --desktop-env COSMIC
python3 scripts/run_gui_testing_vm_smoke.py --host testing-vm --profile cosmic-patched-cursor-host-proof --desktop-env COSMIC
python3 scripts/run_gui_testing_vm_smoke.py --host testing-vm --profile cosmic-transparent-xcursor-host-proof --desktop-env COSMIC
python3 scripts/run_gui_testing_vm_smoke.py --host testing-vm --profile computer-use --desktop-env COSMIC
python3 scripts/run_gui_testing_vm_smoke.py --host testing-vm --profile targeted-screenshot --desktop-env COSMIC
python3 scripts/run_gui_testing_vm_smoke.py --host testing-vm --profile display-screenshot --desktop-env COSMIC
python3 scripts/run_gui_testing_vm_smoke.py --host testing-vm --profile codex-desktop --desktop-env COSMIC
python3 scripts/run_gui_testing_vm_smoke.py --host testing-vm --profile kde-kwin-effect
python3 scripts/run_gui_testing_vm_smoke.py --host testing-vm --profile kde-kwin-effect-system-install --vm-name testing-vm --libvirt-uri qemu:///session
python3 scripts/run_gui_testing_vm_smoke.py --host testing-vm --profile opencode-mcp --sync-opencode-settings
python3 scripts/run_gui_testing_vm_smoke.py --host testing-vm --profile pi-mcp --sync-pi-settings
python3 scripts/run_gui_testing_vm_smoke.py --host testing-vm --profile all --sync-opencode-settings --sync-pi-settings
```

List the profile registry without a VM, including dispatch type, curated-set
membership, and host-framebuffer-proof routing:

```bash
python3 scripts/run_gui_testing_vm_smoke.py --list-profiles
```

## Curated pre-merge profile set

`--profile curated` runs the trimmed pre-merge profile set in registry order
against the currently selected guest session:

```bash
python3 scripts/run_gui_testing_vm_smoke.py --host testing-vm --profile curated --desktop-env COSMIC
```

The curated members are the profiles flagged `curated` in `--list-profiles`:
`codex-desktop`, `wayland-pointer`, `session-env`, and `text-readback`.
Together they cover host-app launch, the portal/EIS
pointer-keyboard-scroll matrix, stripped-env detached session repair, and
direct MCP text readback in one command.

The curated set is deliberately session-agnostic: every member must be able
to pass headless on whichever real desktop session the VM is booted into.
That criterion excludes the stricter `desktop-smoke` lane, which requires live
PipeWire snapshot frames that COSMIC does not deliver headless. That lane, the
cursor pixel host proofs (`kde-kwin-effect-system-install`, the COSMIC
cursor host proofs), `wayland-pointer-scaled`, and `i3` remain per-session
feature and release gates run through the full matrix.

The runner preauthorizes each required portal once for the whole curated
sequence, resets guest sky-cua processes between members so a leaky lane
cannot poison the next one, runs every member even after a failure, and
prints a per-profile summary before exiting nonzero if any member failed.

The runner:

- builds host artifacts with
  `cargo build --release -p sky-cua-client -p sky-cua-service -p sky-cua-overlay-host`
  plus a debug `sky-cua-overlay-host` build
- syncs the checkout into `/workspace` with `rsync`
- excludes heavy/generated host state such as `.git/`, `.venv/`, `dist/`,
  `artifacts/`, and irrelevant `target/` subtrees
- copies selected non-auth `~/.codex` settings, browser config, plugins, and
  skills into the VM user account only when `--sync-codex-settings` is set;
  Codex authentication must be created inside the VM with
  `/opt/codex-desktop/resources/codex login --device-auth`
- runs `scripts/testing-vm/profiles/run-profile.sh` over SSH

Detached session-env repair runs in the VM as the `session-env` profile (a
curated-set member). The local live smokes remain the fastest loop when
changing client startup, service health checks, Linux environment probing, or
Codex harness env-scrubbing:

```bash
python3 scripts/live_session_env_smoke.py
python3 scripts/live_codex_exec_session_env_smoke.py
python3 scripts/live_app_server_session_env_smoke.py
```

These smokes intentionally strip desktop variables and put a minimal `PATH` in
front of the runtime. Passing means the agent or direct MCP client saw
`doctor.session_env` / `SessionEnvRepaired`, found the visible `zenity` dialog,
submitted `session-env-ok`, and the harness observed that exact value from the
dialog process.

Use `--skip-host-build` only when the synced runtime artifacts are already the
ones under test. Use `--skip-sync` only for remote debugging after confirming
the VM checkout is current.

The runner defaults to `WAYLAND_DISPLAY=wayland-0` and the remote user's
`/run/user/<uid>` runtime directory when a real desktop session is active. Use
`--wayland-display` only when the guest session uses a different socket name.
Use `--desktop-env` for SSH-launched real-session smokes when the graphical
session did not import `XDG_CURRENT_DESKTOP` into the SSH environment. For the
current COSMIC VM, pass `--desktop-env COSMIC` so `xdg-desktop-portal` loads
`cosmic-portals.conf` instead of falling through to generic GTK choices.

Plasma must run on the normal user DBus bus, not a private `dbus-run-session`
bus. The provisioner exports `DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/<uid>/bus`,
imports the desktop environment into DBus/systemd activation, and then execs
`startplasma-wayland`. This keeps KWin DBus discovery, portals, and SSH-run
smokes in the same session world. The provisioner also disables the KDE
screen locker and masks PowerDevil for the test image; visual framebuffer
proofs are not useful if the guest has dimmed or locked itself.

## OpenCode Harness Prep

OpenCode is installed by the VM provisioner, but user config and auth are
deliberately synced from the host as a separate operator step because they
contain live credentials. The sync copies `~/.config/opencode` into
`~/.agents/opencode` on the VM, recreates `~/.config/opencode` as a symlink,
copies only `~/.local/share/opencode/auth.json` from OpenCode's data directory,
and then updates OpenCode to the latest version via npm. It does not copy
the host OpenCode database, logs, snapshots, or tool-output history.

This prepares the VM for the non-Codex harness lane. The runner can then
install sky-cua as an MCP server and exercise it through OpenCode:

```bash
# Sync config and update to latest
scripts/testing-vm/sync-opencode-to-vm.sh

# Run OpenCode MCP smoke tests
python3 scripts/run_gui_testing_vm_smoke.py \
  --host 127.0.0.1 --port 22222 --user skycua \
  --ssh-option StrictHostKeyChecking=no \
  --ssh-option UserKnownHostsFile=artifacts/testing-vm/known_hosts \
  --profile opencode-mcp \
  --sync-opencode-settings
```

The `opencode-mcp` profile installs sky-cua as an MCP server for OpenCode,
deploys the `computer-use` skill, and runs a single **wiring check** through
OpenCode's tool-calling loop: the agent must see the sky-cua tool schema and
call one read-only tool (`doctor`/`observe`) without error. It runs on the free
model `opencode/deepseek-v4-flash-free` (override with
`SKY_CUA_SMOKE_OPENCODE_MODEL`). Substantive tool-use coverage is the
`codex-cua` profile, not this lane.

Verify OpenCode is functional before the smoke:

```bash
ssh -p 22222 \
  -o StrictHostKeyChecking=no \
  -o UserKnownHostsFile=artifacts/testing-vm/known_hosts \
  skycua@127.0.0.1 'opencode --version && opencode models openai | head'
```

## Pi Harness Prep

Pi (`pi.dev`) is supported through the `pi-mcp-adapter` extension. The sync
copies the host's `~/.pi` directory into the VM, excluding runtime state
(sessions, cache, memory), then updates Pi and `pi-mcp-adapter` to latest via npm.

```bash
# Sync config and update to latest
scripts/testing-vm/sync-pi-to-vm.sh

# Run Pi MCP smoke tests
python3 scripts/run_gui_testing_vm_smoke.py \
  --host 127.0.0.1 --port 22222 --user skycua \
  --ssh-option StrictHostKeyChecking=no \
  --ssh-option UserKnownHostsFile=artifacts/testing-vm/known_hosts \
  --profile pi-mcp \
  --sync-pi-settings
```

The `pi-mcp` profile installs sky-cua as an MCP server for Pi, merges the
`sky_cua` entry into Pi's `~/.pi/agent/mcp.json`, deploys the
`computer-use` and `browser-use` skills to `~/.pi/agent/skills/`, and runs the
same single **wiring check** through Pi's tool-calling loop (schema visible +
one read-only tool call, no error) on `opencode/deepseek-v4-flash-free`
(override with `SKY_CUA_SMOKE_PI_MODEL`).

## Codex CUA full tool-use profile and the performance judge

The `codex-cua` profile is the substantive codex-CLI gate. In **one** `codex
exec` run it drives the entire computer-use and browser-use surface against live
fixtures:

- a GTK pointer fixture (click / secondary-click / drag / scroll / text entry /
  a check button / a combo box) for the desktop tools, and
- a live Chrome tab — the profile opens Chrome at `chrome://extensions` and
  registers the native-messaging host, then the **agent installs the Codex
  extension itself** with computer-use (Developer mode → Load unpacked → the
  folder chooser); that unlocks the browser tools, including reading a pixels-only
  token rendered on a page `<canvas>` (the model-image vision proof).

```bash
python3 scripts/run_gui_testing_vm_smoke.py \
  --host 127.0.0.1 --port 22222 --user skycua \
  --ssh-option StrictHostKeyChecking=no \
  --ssh-option UserKnownHostsFile=artifacts/testing-vm/known_hosts \
  --profile codex-cua --sync-codex-settings
```

The run exercises the **production `computer-use@openai-bundled` compat surface**.
The runner stages the openai-bundled marketplace into the VM from
`--openai-bundled-resource-root` (default the host's
`codex-desktop-linux/codex-app/resources`); when that source is absent it falls
back to the `sky-cua@local` dev surface (the tool surface is identical either
way — same `sky-cua-client` server and skills). The resolved surface is recorded
as `plugin_surface` in `coverage-summary.json`. Pass `--skip-openai-bundled-sync`
to force the dev fallback.

Two gates apply:

1. **Deterministic coverage gate (in the VM):**
   `scripts/live_codex_cua_smoke.py` parses the codex transcript and fails unless
   every required tool/operation/surface was called (see `scripts/_cua_coverage.py`),
   no tool call errored, and the fixtures' ground truth confirms the actions
   landed. It writes `coverage-summary.json` next to the transcript.
2. **Performance judge (on the host):** because the VM lacks host gpt-5.5 auth,
   the runner pulls the transcript + `coverage-summary.json` + `last-message.json`
   back to the host and runs `scripts/live_agent_perf_judge.py` (gpt-5.5, high
   reasoning). The judge scores tool-use 0-100 across tool-selection,
   error-recovery, efficiency, and task-completion, **hard-fails below the
   threshold** (default 70, override `--threshold` / `SKY_CUA_JUDGE_THRESHOLD`),
   and **always** writes `judge-verdict.json` and `judge-triage.json`. The judge
   runs even when the deterministic gate failed, so a triage list is always
   produced. Overall success requires both gates to pass.

`--profile all` runs the deterministic `codex-cua` gate in-sequence but **not**
the host judge (the `all` sequence is VM-local); invoke `--profile codex-cua`
for the judged run.

## Portal Selection

Portal backend selection is environment-driven. `xdg-desktop-portal` looks at
`XDG_CURRENT_DESKTOP`, then loads the matching
`/usr/share/xdg-desktop-portal/<desktop>-portals.conf` or an override in
`~/.config/xdg-desktop-portal/`. The VM session launcher therefore exports
explicit desktop values for `cosmic`, `kde`, `gnome`, `hyprland`, and `i3`.
The runner can also import those values into the user systemd environment with
`--desktop-env`.

COSMIC is correctly selected when `XDG_CURRENT_DESKTOP=COSMIC`: the router
activates `org.freedesktop.impl.portal.desktop.cosmic`, and the public
`org.freedesktop.portal.Desktop` object exposes COSMIC `ScreenCast` and
`Screenshot`. Current upstream and Arch `xdg-desktop-portal-cosmic` do not
advertise `org.freedesktop.impl.portal.RemoteDesktop`; their
`cosmic.portal` interface list is `Access`, `FileChooser`, `Screenshot`,
`Settings`, and `ScreenCast`. That means COSMIC RemoteDesktop absence is an
upstream capability gap, not a local package-selection bug.

Do not force GNOME or KDE `RemoteDesktop` as the COSMIC answer. Those backends
are session/compositor-specific and make a misleading production proof. COSMIC
Wayland physical input should use the Linux virtual input backend when
virtual input is available. That backend exposes one runtime selection,
`LinuxVirtualInput`; pointer actions use ydotool, while the privileged helper
is reserved for keyboard/text injection and pointer observation. KDE and GNOME
continue to prefer their `RemoteDesktop` portals in their own real sessions.

## Session Switching

Switch visible VM sessions with the guest helper, not raw display-manager
restarts:

```bash
ssh -p 22222 \
  -o StrictHostKeyChecking=no \
  -o UserKnownHostsFile=artifacts/testing-vm/known_hosts \
  skycua@127.0.0.1 'cd /workspace && sudo scripts/testing-vm/select-session.sh hyprland'
```

Use `plasma`, `cosmic`, `gnome`, `hyprland`, or `i3`. The helper rewrites the
greetd/GDM target, kills stale compositor and sky-cua processes for the VM user,
and restarts the display manager. This matters because raw session switches can
leave stale sockets, such as old KWin on `wayland-0` while Hyprland is active on
`wayland-1`.

## Profiles

Profiles live under `scripts/testing-vm/profiles/`.

- `computer-use`: real-session visible pointer smoke for the installed Computer
  Use plugin path. It opens the fullscreen GTK pointer fixture on the VM's
  active Wayland session and drives click, secondary-click, drag, and scroll through
  `sky-cua-client mcp`.
- `wayland-pointer`: explicit name for the same visible real-session pointer
  smoke used by `computer-use`.
- `targeted-screenshot`: real-session window-targeted screenshot smoke. It
  opens a target dialog plus an occluder, calls `screenshot` with `window_id`,
  asserts focus/crop metadata, and clicks the target through the returned
  cropped `snapshot_id`. The profile supports Wayland sessions and i3/X11.
- `display-screenshot`: real-session display-targeted screenshot smoke. It
  asserts `environment.displays`, main-display default capture, explicit
  display capture, rejection of the retired `capture_all_displays` selector,
  structured secondary-output skip when only one monitor exists, and snapshot
  click landing through a display crop. The profile supports Wayland sessions
  and i3/X11.
- `session-env`: real-session stripped-env proof for detached session-env
  repair. It runs `scripts/live_session_env_smoke.py` on the guest: the MCP
  client starts with graphical session variables stripped and a minimal
  `PATH`, must show `doctor.session_env` repair, then finds and submits the
  visible `zenity` dialog.
- `text-readback`: real-session focused direct MCP readback proof. It runs
  `scripts/live_text_readback_smoke.py` on the guest: initial `zenity` entry
  readback of the stale value, `set_value` replacement, fresh-snapshot
  verification, and dialog submission. It does not require live PipeWire
  frame capture, so it passes headless on COSMIC.
- `desktop-smoke`: real-session full direct MCP desktop smoke. It runs
  `scripts/live_desktop_smoke.py` on the guest: semantic actions, pointer
  fixtures, and strict capture requirements (snapshots must not downgrade
  from PipeWire). Run it on sessions with preauthorized PipeWire capture,
  such as Plasma.
- `curated`: host-side run mode, not a remote profile. Runs the trimmed
  pre-merge curated set in sequence; see "Curated pre-merge profile set".
- `wayland-layer-shell-overlay`: real Wayland session proof for the native
  layer-shell cursor overlay. It runs the service-backed KDE cursor smoke in
  non-KDE mode, captures through sky-cua's screenshot request, and proves
  visible cursor pixels on the display containing the fixture point.
- `opencode-mcp`: real-session OpenCode MCP **wiring check**. Installs sky-cua as
  an MCP server for OpenCode, deploys skills, and runs one read-only tool call
  (schema visible + `doctor`/`observe`, no error) on
  `opencode/deepseek-v4-flash-free`. It proves MCP is wired for the agent;
  substantive tool-use coverage is `codex-cua`.
- `pi-mcp`: real-session Pi MCP **wiring check**. Installs sky-cua as an MCP
  server for Pi via `pi-mcp-adapter`, deploys skills, and runs the same
  single read-only wiring check through Pi's tool-calling loop.
- `codex-cua`: the substantive single-run codex tool-use gate. Brings up Chrome +
  the sky-cua extension + native host, then drives the full computer-use and
  browser-use surface in one `codex exec` run against the GTK pointer fixture and
  a live browser page. A deterministic coverage/no-error gate runs in the VM
  (`scripts/live_codex_cua_smoke.py` + `scripts/_cua_coverage.py`); the host-side
  performance judge (`scripts/live_agent_perf_judge.py`, gpt-5.5/high) scores
  tool-use and emits a triage list. Dispatch routes through the host so the judge
  has gpt-5.5 auth. See "Codex CUA full tool-use profile and the performance
  judge".
- `codex-desktop`: real-session launch smoke for the installed
  CodexDesktop-Rebuild package. It requires a visible Codex window on the VM's
  active desktop session and records Codex and Chrome versions.
- `cosmic-helper`: real COSMIC Wayland guest-session proof for the
  `sky-cua-cosmic-helper` protocol path. It launches a Wayland client on the
  guest session socket, proves helper `probe`, `list-windows`,
  `activate-window`, and `focused-window`, and records the JSON replies.
- `cosmic-patched-cursor-host-proof`: patched COSMIC compositor proof for the
  host-visible one-cursor invariant. It requires a `cosmic-comp` build with the
  repo patch applied, drives `sky-cua-overlay-host` over the real guest session,
  and verifies the framebuffer/host-summary contract.
- `cosmic-transparent-xcursor-host-proof`: no-patch COSMIC proof for the
  dedicated transparent native-cursor session mode. Boot the VM into
  `cosmic-blank` or `cosmic-transparent`; the profile verifies
  `system_cursor_backend=cosmic_transparent_xcursor`, visible overlay proof, and
  the absence of a native cursor in the hidden frame.
- `i3`: real X11 session proof. Boot the VM into i3/X11 first; the profile
  refuses Wayland and runs the X11 overlay/current-display smoke against the
  guest session display.
- `kde-kwin-effect`: KWin effect build/load/IPC and agent-cursor overlay proof.
  Run this from a VM booted into Plasma Wayland when testing production KWin
  behavior.
- `kde-kwin-effect-system-install`: VM-only production package-path proof. It
  installs the compiled effect under `/usr` with `sudo`, proves KWin
  discovery/load and overlay-host KWin-shim IPC, then uninstalls the exact
  system files. The host runner owns pixel proof:
  it captures before/after VM framebuffers with `virsh screenshot`, probes the
  cursor diff locally, and writes `host-summary.json`.
- `kde-plasma`, `gnome`, `cosmic`, and `hyprland`: legacy nested visual-debug
  profiles retained for targeted compositor debugging. They are not acceptance
  proof for the VM session matrix. For COSMIC/GNOME/Plasma/Hyprland acceptance,
  boot the VM into that desktop and run the app/plugin smoke against the real
  guest session.
- `all`: runs the standard VM smoke gate: direct computer-use profiles, Codex
  Desktop launch proof, OpenCode/Pi installed-MCP agent harnesses, and
  KWin-effect proof. When `HOST_WAYLAND_DISPLAY` is set it also runs the
  legacy nested compositor debug profiles with `--headed`; otherwise those
  headed profiles are skipped. Treat `all` as the routine full smoke gate for
  agent closeout, but keep per-desktop real-session acceptance separate by
  booting the VM into the target desktop and running that profile against the
  real guest session.

## Current Verification Status

The Docker GUI harness has been retired as the preferred path. Its package list
and profile ideas were folded into the Arch testing-VM provisioner and runner.
The accepted VM matrix now covers COSMIC helper/input, patched COSMIC cursor
bridge, no-patch transparent COSMIC session mode, KDE/KWin system-install
effect proof, GNOME Shell extension cursor proof, Hyprland compositor cursor
hide, i3/X11 overlay proof, and Plasma text-readback proof for both direct
MCP and agent-driven Codex harnesses.

Early COSMIC bring-up proofs:

```bash
python3 scripts/run_gui_testing_vm_smoke.py \
  --host 127.0.0.1 \
  --port 22222 \
  --user skycua \
  --profile computer-use \
  --wayland-display wayland-1 \
  --desktop-env COSMIC \
  --ssh-option StrictHostKeyChecking=no \
  --ssh-option UserKnownHostsFile=artifacts/testing-vm/known_hosts

python3 scripts/run_gui_testing_vm_smoke.py \
  --host 127.0.0.1 \
  --port 22222 \
  --user skycua \
  --profile cosmic-helper \
  --wayland-display wayland-1 \
  --desktop-env COSMIC \
  --ssh-option StrictHostKeyChecking=no \
  --ssh-option UserKnownHostsFile=artifacts/testing-vm/known_hosts

python3 scripts/run_gui_testing_vm_smoke.py \
  --host 127.0.0.1 \
  --port 22222 \
  --user skycua \
  --profile codex-desktop \
  --skip-host-build \
  --skip-sync \
  --desktop-env COSMIC \
  --ssh-option StrictHostKeyChecking=no \
  --ssh-option UserKnownHostsFile=artifacts/testing-vm/known_hosts
```

The VM was booted through greetd into COSMIC Wayland with `cosmic-session`,
`cosmic-comp`, and `/run/user/1000/wayland-1` active. The earlier
`mcp-x11`/embedded-X11 VM smoke was retired; Computer Use proof on this VM now
means a visible real-session pointer smoke.
The COSMIC helper artifact is
`/workspace/artifacts/gui-desktop-smoke/cosmic-helper/20260515T031400Z/` in the
VM; it proves `probe`, `list-windows`, `activate-window`, and `focused-window`
against `org.freedesktop.weston.flower` on the real `wayland-1` session.
The Codex Desktop launch artifact is
`/workspace/artifacts/gui-desktop-smoke/codex-desktop/20260515T030818Z/` in the
VM, with `codex-desktop 26.506.31421-1`, Google Chrome
`148.0.7778.167`, and a visible `codex.Codex` X11 window.
After the COSMIC portal-selection pass, fresh COSMIC session smokes produced:
`/workspace/artifacts/gui-desktop-smoke/cosmic-helper/20260515T034206Z/`,
`/workspace/artifacts/gui-desktop-smoke/codex-desktop/20260515T034206Z/`, and
the expected visible pointer blocker
`/workspace/artifacts/gui-desktop-smoke/wayland-pointer/20260515T034151Z/`.
The pointer smoke now fails honestly with
`ActionUnsupportedForEnvironment: no physical input backend is available for click fallback`
instead of falsely routing through the X11/XWayland input fallback.

The old direct absolute `/dev/uinput` pointer adapter was tried and then
retired because it was not a reliable compositor-delivered pointer path. Older
artifact
`/workspace/artifacts/gui-desktop-smoke/wayland-pointer/20260515T091758Z`
proved the fullscreen GTK fixture received click, drag, and scroll in that VM
run (`clicked=true`, `drag_completed=true`, `scroll_events=1`), but later live
KDE validation made it non-production. The ydotool pointer calibration
artifacts immediately before that were useful negative proof: ydotool's VM
pointer device is relative-only and `mousemove --absolute` landed
at accelerated coordinates, so ydotool is not the COSMIC pointer adapter.
`ydotool` remains useful for keyboard/text fallback.
The extended COSMIC artifact
`/workspace/artifacts/gui-desktop-smoke/wayland-pointer/20260515T092606Z`
also proves `type_text` and `press_key` through the same Linux virtual input
backend: the fixture recorded `entry_text="cosmic-text-smoke"` and
`submitted_text="cosmic-text-smoke"`.
The scaled COSMIC artifact
`/workspace/artifacts/gui-desktop-smoke/wayland-pointer/20260515T093335Z`
repeats that full input proof at 1600x1200 with `Scale: 125%`, after the direct
uinput adapter converts desktop logical coordinates into physical absolute
device coordinates.
The repeatable profile proof is
`/workspace/artifacts/gui-desktop-smoke/wayland-pointer/20260515T093737Z`,
from `--profile wayland-pointer-scaled`; the profile restores COSMIC to
1280x800 at 100% scale after the smoke.

Fresh Plasma VM cursor proof after fixing the session bus:

```bash
ssh -p 22222 \
  -o StrictHostKeyChecking=no \
  -o UserKnownHostsFile=artifacts/testing-vm/known_hosts \
  skycua@127.0.0.1 \
  'cd /workspace && env XDG_CURRENT_DESKTOP=KDE XDG_SESSION_DESKTOP=KDE DESKTOP_SESSION=plasma XDG_SESSION_TYPE=wayland WAYLAND_DISPLAY=wayland-0 DISPLAY=:0 QT_QPA_PLATFORM=wayland GDK_BACKEND=wayland SKY_CUA_SKIP_LOCAL_BUILD=1 SKY_CUA_SERVICE_BIN=/workspace/target/debug/sky-cua-service SKY_CUA_OVERLAY_HOST_BIN=/workspace/target/debug/sky-cua-overlay-host SKY_CUA_LAYER_SHELL_LAYER=overlay python scripts/live_agent_cursor_kde_smoke.py --mode layer-shell-hide-for-capture --request-timeout 180'

ssh -p 22222 \
  -o StrictHostKeyChecking=no \
  -o UserKnownHostsFile=artifacts/testing-vm/known_hosts \
  skycua@127.0.0.1 \
  'cd /workspace && env XDG_CURRENT_DESKTOP=KDE XDG_SESSION_DESKTOP=KDE DESKTOP_SESSION=plasma XDG_SESSION_TYPE=wayland WAYLAND_DISPLAY=wayland-0 DISPLAY=:0 QT_QPA_PLATFORM=wayland GDK_BACKEND=wayland SKY_CUA_SKIP_LOCAL_BUILD=1 SKY_CUA_SERVICE_BIN=/workspace/target/debug/sky-cua-service SKY_CUA_OVERLAY_HOST_BIN=/workspace/target/debug/sky-cua-overlay-host SKY_CUA_LAYER_SHELL_LAYER=overlay python scripts/live_agent_cursor_kde_smoke.py --mode layer-shell-click-through --request-timeout 180'
```

The latest accepted VM artifacts are
`/workspace/artifacts/codex-e2e/agent-cursor-kde/0515100302670580-syn`,
`/workspace/artifacts/codex-e2e/agent-cursor-kde/0515100303845615-vis`,
`/workspace/artifacts/codex-e2e/agent-cursor-kde/0515100305142807-hide`, and
`/workspace/artifacts/codex-e2e/agent-cursor-kde/0515100306568235-click`.
The host-side headed framebuffer proof is
`artifacts/kde-framebuffer-cursor-proof/cursor-overlay-clean/after.png`.
When comparing `virsh screenshot` output, convert to RGB before diffing; RGBA
diffs can hide real pixel changes because of the screenshot alpha channel.

The fuller KDE real-session pointer smoke artifact is
`/workspace/artifacts/gui-desktop-smoke/wayland-pointer/20260515T100113Z`. It
proves the fullscreen GTK fixture received the portal-driven click, drag,
scroll, `type_text`, and `press_key` events.

Two cleanup details are part of the harness contract. First, the runner imports
the target desktop environment and refreshes the user portal stack before
preauthorization/profile startup; otherwise a previous COSMIC session can leave
`xdg-desktop-portal` selecting the wrong implementation for Plasma. Second,
cleanup kills stale `cosmic-randr` probes as well as sky-cua service and overlay
processes; Linux virtual input scopes `cosmic-randr` probing to COSMIC desktops
so KDE portal smokes cannot hang behind an irrelevant display helper.

Fresh GNOME VM pointer proof:

```bash
python3 scripts/run_gui_testing_vm_smoke.py \
  --host 127.0.0.1 \
  --port 22222 \
  --user skycua \
  --profile wayland-pointer \
  --wayland-display wayland-0 \
  --desktop-env GNOME \
  --ssh-option StrictHostKeyChecking=no \
  --ssh-option UserKnownHostsFile=artifacts/testing-vm/known_hosts
```

The accepted GNOME artifact is
`/workspace/artifacts/gui-desktop-smoke/wayland-pointer/20260518T025611Z`.
This artifact was produced during initial development of the EIS GNOME path;
re-run the automated VM profile to confirm it independently before relying on it.
It proves click, secondary-click, drag, scroll, `type_text`, and `press_key`
through the GNOME RemoteDesktop portal EIS path, with keyboard injection backed
by the compositor-provided XKB keymap
(`clicked=true`, `secondary_clicked=true`, `drag_completed=true`, `scroll_events=1`, and
`submitted_text="cosmic-text-smoke"`). This run depends on GNOME-specific
automation fixes: session switching must stop the inactive display manager so
Plasma and GNOME do not run together, stale pointer fixtures must be cleaned
before each profile run, the GTK fixture must publish points from realized
allocations inside the visible framebuffer, and the EIS adapter must keep
pointer emulation session-scoped while translating the tool convention
`delta_y=-180` into libei's positive-down `scroll_delta_y=180`.

Fresh i3/X11 VM proof:

```bash
python3 scripts/run_gui_testing_vm_smoke.py \
  --host 127.0.0.1 \
  --port 22222 \
  --user skycua \
  --profile i3 \
  --desktop-env i3 \
  --ssh-option StrictHostKeyChecking=no \
  --ssh-option UserKnownHostsFile=artifacts/testing-vm/known_hosts
```

Boot or switch the VM to `SKY_CUA_TESTING_VM_SESSION=i3` first. The i3 profile
derives the real Xorg display and Xauthority from the active `Xorg` process,
because `startx` may choose `:1` while the user systemd environment still has
stale Wayland or Xwayland values from an earlier session. The accepted X11
artifact is
`/workspace/artifacts/codex-e2e/agent-cursor-x11-overlay/20260515T075301057704Z`;
it proves visible capture, hide, re-show, click-through, and XFixes cursor
hide/show state transitions.

The production KWin effect discovery blocker was rechecked after the Plasma
session-bus fix. User-level artifact
`/workspace/artifacts/codex-e2e/agent-cursor-kde/0515075621741796-kwin` shows
KWin DBus reachable on the normal user bus, the C++ effect building and
installing user-level files, and cleanup succeeding, but KWin still reports
`listed=false`, `effect_supported=false`, and `load_stdout="false"`.

The system path is viable and automated in the disposable VM: installing under
`/usr`, restarting Plasma, and loading the effect makes KWin list
`sky-cua-agent-cursor`; `sky-cua-overlay-host` returns `backend=kwin_effect`
with `system_cursor_hidden=true`; host libvirt framebuffer diff finds the cursor
at `(420,260)`; cleanup removes the system files and KWin no longer lists the
effect after restart. Latest proof:
`artifacts/kde-framebuffer-cursor-proof/kwin-system-install/20260515T100852814643Z/host-summary.json`.

Do not mark any live-smoke gap complete until the command, desktop profile, and
artifact directory are recorded.

## Retired Docker Path

The earlier `docker/gui-test` image and
`scripts/run_gui_desktop_docker_smoke.py` runner were useful for package
discovery, but containers could only prove nested compositor behavior. They
could not provide the real standalone COSMIC/GNOME/Plasma/Hyprland sessions
needed for Computer Use acceptance. Keep future Linux desktop proof centered on
the Arch testing VM.
