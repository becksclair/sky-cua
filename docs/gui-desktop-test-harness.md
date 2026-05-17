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
- smoke tools: `gst-plugins-good`, ImageMagick, `grim`, `jq`, `libinput`,
  `openbox`, `slurp`, `socat`, `strace`, `wev`, `weston`, `wl-clipboard`,
  `wmctrl`, `xdotool`, `ydotool`/`ydotoold`, Xorg, xauth, xdpyinfo, xev,
  xmessage, and xwininfo
- browser-use smoke browser: Google Chrome installed from Google's stable
  Linux package
- Codex Desktop: installed from the local CodexDesktop-Rebuild Arch package
  when `CODEX_DESKTOP_PACKAGE` is set
- OpenCode CLI: installed from npm with `OPENCODE_NPM_SPEC`, defaulting to the
  host-proven `opencode-ai@1.14.51`, so future non-Codex harness work can run
  in the same production-like VM

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

If the viewer appears blank, first capture the libvirt framebuffer with
`virsh --connect qemu:///session screenshot testing-vm <path>.png` and check
the guest session processes over SSH. A blanked or locked guest display can
look like an overlay failure even when the overlay is not drawing anything.

## Runner

Run profiles from the host with:

```bash
python3 scripts/run_gui_testing_vm_smoke.py --host testing-vm --profile computer-use
```

The runner:

- builds host artifacts with
  `cargo build --release -p sky-cua-client -p sky-cua-service -p sky-cua-overlay-host`
  plus a debug `sky-cua-overlay-host` build
- syncs the checkout into `/workspace` with `rsync`
- excludes heavy/generated host state such as `.git/`, `.venv/`, `dist/`,
  `artifacts/`, and irrelevant `target/` subtrees
- copies selected `~/.codex` settings, auth, browser config, plugins, and
  skills into the VM user account only when `--sync-codex-settings` is set
- runs `scripts/testing-vm/profiles/run-profile.sh` over SSH

Useful commands:

```bash
python3 scripts/run_gui_testing_vm_smoke.py --host testing-vm --profile cosmic
python3 scripts/run_gui_testing_vm_smoke.py --host testing-vm --profile kde-kwin-effect
python3 scripts/run_gui_testing_vm_smoke.py --host testing-vm --profile kde-kwin-effect-system-install --vm-name testing-vm --libvirt-uri qemu:///session
python3 scripts/run_gui_testing_vm_smoke.py --host testing-vm --profile computer-use --desktop-env COSMIC
python3 scripts/run_gui_testing_vm_smoke.py --host testing-vm --profile codex-desktop --desktop-env COSMIC
python3 scripts/run_gui_testing_vm_smoke.py --host testing-vm --profile cosmic-helper --desktop-env COSMIC
python3 scripts/run_gui_testing_vm_smoke.py --host testing-vm --profile cosmic-patched-cursor-host-proof --desktop-env COSMIC
python3 scripts/run_gui_testing_vm_smoke.py --host testing-vm --profile cosmic-transparent-xcursor-host-proof --desktop-env COSMIC
python3 scripts/run_gui_testing_vm_smoke.py --host testing-vm --profile all
```

Detached session-env repair is not yet a VM runner profile, but it is now a
first-class Linux launch seam. Use the local live smokes when changing client
startup, service health checks, Linux environment probing, or Codex harness
env-scrubbing:

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
and then runs `npm install --omit=dev` inside the VM config. It does not copy
the host OpenCode database, logs, snapshots, or tool-output history.

This prepares the VM for the non-Codex harness lane. Registering the sky-cua
MCP runtime inside OpenCode still follows the plain MCP host instructions in
`docs/mcp-runtime.md`; the VM prep here only installs OpenCode itself and
copies the user's OpenCode config/auth safely.

For the current QEMU user-networking VM:

```bash
scripts/testing-vm/sync-opencode-to-vm.sh
```

Then verify:

```bash
ssh -p 22222 \
  -o StrictHostKeyChecking=no \
  -o UserKnownHostsFile=artifacts/testing-vm/known_hosts \
  skycua@127.0.0.1 'opencode --version && opencode models openai | head'
```

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
`LinuxVirtualInput`; pointer actions prefer the direct absolute `/dev/uinput`
adapter when `/dev/uinput` is writable and desktop bounds are detected, while
`ydotool` remains the keyboard/text adapter and lower-priority fallback. KDE and
GNOME continue to prefer their `RemoteDesktop` portals in their own real
sessions.

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
  active Wayland session and drives click, drag, and scroll through
  `sky-cua-client mcp`.
- `wayland-pointer`: explicit name for the same visible real-session pointer
  smoke used by `computer-use`.
- `wayland-layer-shell-overlay`: real Wayland session proof for the native
  layer-shell cursor overlay. It uses the active session socket, draws the
  copied Chrome cursor asset through `sky-cua-overlay-host`, captures with the
  compositor screenshot tool (`grim -o <output>` on Hyprland), and proves visible
  then hidden cursor pixels.
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
  installs the compiled effect under `/usr` with `sudo`, restarts Plasma, proves
  KWin discovery/load and overlay-host `kwin_effect` IPC, then uninstalls the
  system files and restarts Plasma again. The host runner owns pixel proof:
  it captures before/after VM framebuffers with `virsh screenshot`, probes the
  cursor diff locally, and writes `host-summary.json`.
- `kde-plasma`, `gnome`, `cosmic`, and `hyprland`: legacy nested visual-debug
  profiles retained for targeted compositor debugging. They are not acceptance
  proof for the VM session matrix. For COSMIC/GNOME/Plasma/Hyprland acceptance,
  boot the VM into that desktop and run the app/plugin smoke against the real
  guest session.
- `all`: runs the fast non-session-specific profiles. It does not claim that
  every desktop session has been proved.

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

After the Linux virtual input pass, COSMIC pointer input is accepted through the
direct absolute `/dev/uinput` adapter. Artifact
`/workspace/artifacts/gui-desktop-smoke/wayland-pointer/20260515T091758Z`
proves the fullscreen GTK fixture received click, drag, and scroll
(`clicked=true`, `drag_completed=true`, `scroll_events=1`). The ydotool pointer
calibration artifacts immediately before that were useful negative proof:
ydotool's VM pointer device is relative-only and `mousemove --absolute` landed
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
`/workspace/artifacts/gui-desktop-smoke/wayland-pointer/20260515T084643Z`.
It proves click, drag, and scroll through the GNOME RemoteDesktop portal
(`clicked=true`, `drag_completed=true`, `scroll_events=2`). This run depends on
two GNOME-specific fixes: session switching must stop the inactive display
manager so Plasma and GNOME do not run together, and the GTK fixture adjusts
GNOME fullscreen coordinates when the reported allocation is taller than the
visible framebuffer.

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

Progress ledger:

| Area | Status | Current proof | Remaining proof |
| --- | --- | --- | --- |
| VM provisioner | First live proof | `scripts/testing-vm/provision-arch-testing-vm.sh` provisioned a QEMU/libvirt Arch guest with COSMIC Wayland, Chrome, OpenCode, Codex Desktop, SSH, rsync, matching terminal apps, and the desktop matrix. | Re-run from a fully fresh guest after future package-list changes. |
| VM runner | Accepted matrix runner | `scripts/run_gui_testing_vm_smoke.py` now owns the accepted real-session matrix for COSMIC helper/input, patched COSMIC cursor bridge, transparent COSMIC, KDE/KWin system-install, GNOME, Hyprland, and i3/X11 cursor proof profiles. | Add host-side artifact pullback or index generation if repeated VM runs need easier local browsing. |
| OpenCode | Config/auth prep proof | `scripts/testing-vm/sync-opencode-to-vm.sh` synced host OpenCode config/auth without DB/log/snapshot state; VM `opencode --version` returned `1.14.51`, and `opencode models openai` succeeded. | Register and smoke the sky-cua MCP runtime under OpenCode when the non-Codex harness lane starts. |
| Text readback | Direct MCP plus agent harness accepted on Plasma | In the Plasma `testing-vm`, `scripts/live_desktop_smoke.py` proved initial `zenity` entry value readback, post-`set_value` readback, and post-`type_text` readback through fresh `get_app_state` snapshots. `scripts/live_codex_exec_text_readback_smoke.py` produced `/workspace/artifacts/codex-e2e/codex-text-readback-smoke/20260517T041212Z`, and `scripts/live_app_server_text_readback_smoke.py` produced `/workspace/artifacts/codex-e2e/app-server-text-readback-smoke/20260517T041242Z`; both transcript checks require one `get_app_state` result with `stale-readback` and a later one with `verified-readback`. | Add this lane to the automated VM runner when the broader pre-merge profile set is curated; extend native readback proof to Windows/UIA only after that backend extracts equivalent metadata. |
| COSMIC | Helper, app launch, pointer, text/key, patched cursor bridge, and transparent no-patch mode accepted | Real COSMIC Wayland guest session was active with `cosmic-session`, `cosmic-comp`, and `/run/user/1000/wayland-1`; `cosmic-helper` proved helper listing, activation, and focused-window readback at `/workspace/artifacts/gui-desktop-smoke/cosmic-helper/20260515T034206Z/`. Full input artifact `/workspace/artifacts/gui-desktop-smoke/wayland-pointer/20260515T092606Z` proves `LinuxVirtualInput` direct uinput click, drag, scroll plus ydotool-backed `type_text`/`press_key`; repeatable scaled profile artifact `/workspace/artifacts/gui-desktop-smoke/wayland-pointer/20260515T093737Z` proves the same path at 125%. Patched compositor proof `artifacts/cosmic-framebuffer-cursor-proof/20260515T142538562074Z/host-summary.json` reports `ok=true` with `system_cursor_backend=cosmic_comp_bridge`. No-patch transparent session proof `artifacts/cosmic-transparent-xcursor-cursor-proof/20260516T073232164704Z/host-summary.json` reports `ok=true` with `system_cursor_backend=cosmic_transparent_xcursor`. | Keep this in the session-matrix gate; broaden later to multi-output and richer list/focus coverage when the VM exposes more than one real output. |
| KDE/KWin | Layer-shell, pointer input, and KWin effect system path accepted | Real Plasma Wayland VM proofs: clean cursor sequence `/workspace/artifacts/codex-e2e/agent-cursor-kde/0515100302670580-syn`, `/workspace/artifacts/codex-e2e/agent-cursor-kde/0515100303845615-vis`, `/workspace/artifacts/codex-e2e/agent-cursor-kde/0515100305142807-hide`, `/workspace/artifacts/codex-e2e/agent-cursor-kde/0515100306568235-click`, full pointer `/workspace/artifacts/gui-desktop-smoke/wayland-pointer/20260515T100113Z/`, user-level effect discovery blocker `/workspace/artifacts/codex-e2e/agent-cursor-kde/0515075621741796-kwin`, and automated system-install proof `artifacts/kde-framebuffer-cursor-proof/kwin-system-install/20260515T132649888064Z/host-summary.json`. | Keep this profile in the pre-merge/live-smoke gate for future KWin effect changes; broader registry/list/focus proof is still a separate seam. |
| GNOME | Pointer input and Shell-extension cursor proof accepted | Real GNOME Wayland VM pointer artifact `/workspace/artifacts/gui-desktop-smoke/wayland-pointer/20260515T084643Z` proves click, drag, and scroll through GNOME RemoteDesktop. The GNOME Shell extension cursor artifact `artifacts/gnome-framebuffer-cursor-proof/20260515T140437893805720Z/host-summary.json` reports `ok=true` with `backend=gnome_shell_extension` and `system_cursor_backend=gnome_shell_extension`. | Broaden GNOME registry/listing/focus proof beyond the current cursor and pointer seams, and re-run after Shell or session-launch changes. |
| Hyprland | Layer-shell overlay and compositor cursor hide accepted | Real Hyprland VM artifact `/workspace/artifacts/codex-e2e/agent-cursor-wayland-layer-shell/20260515T142710878162Z` proves `wayland_layer_shell`, `system_cursor_backend=hyprland_config`, visible overlay capture, click-through capability, hide-for-capture, and restore of `cursor:invisible`. The same slice fixed the unconfigured layer-surface buffer attach protocol bug. | Broaden Hyprland registry/list/focus/terminal-enrichment proof and full pointer-input matrix as those paths mature. |
| i3/X11 | X11 overlay and XFixes system cursor hide accepted | Real i3/X11 VM artifact `/workspace/artifacts/codex-e2e/agent-cursor-x11-overlay/20260515T142731049499Z` proves visible overlay capture, hide, re-show, click-through, and XFixes system cursor hide/show. | Broaden the i3 profile later for `i3-msg -t get_tree`, app focus activation, terminal enrichment, and X11/XTest input beyond the cursor overlay proof. |

Do not mark any live-smoke gap complete until the command, desktop profile, and
artifact directory are recorded.

## Retired Docker Path

The earlier `docker/gui-test` image and
`scripts/run_gui_desktop_docker_smoke.py` runner were useful for package
discovery, but containers could only prove nested compositor behavior. They
could not provide the real standalone COSMIC/GNOME/Plasma/Hyprland sessions
needed for Computer Use acceptance. Keep future Linux desktop proof centered on
the Arch testing VM.
