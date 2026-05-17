---
name: vm-tests
description: Use when running, selecting, debugging, or reporting sky-cua Arch testing-vm smoke profiles through scripts/run_gui_testing_vm_smoke.py, including KDE/Plasma, GNOME, COSMIC, Hyprland, i3/X11, KWin effect, layer-shell overlay, pointer/input, Codex Desktop, OpenCode prep, and VM artifact evidence.
---

# VM Tests

Use this skill for the Arch `testing-vm` smoke lane. It is for real guest desktop sessions, not local-only live smokes and not the retired nested Docker/Xvfb path.

## Read First

Before running or interpreting a VM profile, read the current project sources of truth:

- `docs/operations/gui-desktop-test-harness.md` for provisioning, runner behavior, profiles, session switching, current proof status, and artifact expectations.
- `skills/sky-cua-isolated-daemon/references/testing-vm-desktop-smokes.md` for the current SSH port-forward form, session/display names, known-good commands, and false trails.
- `scripts/run_gui_testing_vm_smoke.py --help` or the script source when adding flags, choosing a profile, or investigating runner behavior.

Treat those files as authoritative over this skill if they drift.

## Core Rules

- Use `scripts/run_gui_testing_vm_smoke.py` as the accepted VM matrix runner.
- Run profiles against the visible VM desktop session. A nested compositor, Docker GUI image, or old nested-Xvfb smoke is historical evidence, not acceptance proof.
- Let the runner build and sync by default. Use `--skip-host-build` or `--skip-sync` only when you have confirmed the VM already has the exact artifacts under test.
- Select or confirm the guest session before real-session profiles. Stale compositors and Wayland sockets produce misleading failures.
- Use the port-forward SSH form when `testing-vm` does not resolve:

```bash
--host 127.0.0.1 --port 22222 --user skycua \
--ssh-option StrictHostKeyChecking=no \
--ssh-option UserKnownHostsFile=artifacts/testing-vm/known_hosts
```

- Do not copy Codex credentials into the VM unless the profile needs an authenticated Codex lane; when needed, use the runner's `--sync-codex-settings` flag and say so.
- If a run touches portal, input, or overlay behavior, report the selected desktop, `WAYLAND_DISPLAY` or X11 display, profile, command, and artifact directory.

## Session Selection

Switch sessions with the guest helper, then confirm the actual compositor/socket state:

```bash
ssh -p 22222 \
  -o StrictHostKeyChecking=no \
  -o UserKnownHostsFile=artifacts/testing-vm/known_hosts \
  skycua@127.0.0.1 'cd /workspace && sudo scripts/testing-vm/select-session.sh plasma'

ssh -p 22222 \
  -o StrictHostKeyChecking=no \
  -o UserKnownHostsFile=artifacts/testing-vm/known_hosts \
  skycua@127.0.0.1 'pgrep -a "kwin_wayland|gnome-shell|Hyprland|cosmic-session|cosmic-comp|i3|Xorg"; ls -l /run/user/1000/wayland-* 2>/dev/null || true'
```

Known real-session display defaults:

- Plasma/KWin: `--desktop-env KDE --wayland-display wayland-0`
- GNOME: `--desktop-env GNOME --wayland-display wayland-0`
- COSMIC: `--desktop-env COSMIC --wayland-display wayland-1`
- Hyprland: `--desktop-env Hyprland --wayland-display wayland-1`
- i3/X11: `--desktop-env i3`

## Profile Selection

- Use `computer-use` or `wayland-pointer` for visible real-session pointer/input proof.
- Use `kde-kwin-effect-system-install` for VM-only KWin production package-path proof. Include `--vm-name testing-vm --libvirt-uri qemu:///session` and confirm cleanup state.
- Use `wayland-layer-shell-overlay` for Hyprland or other layer-shell cursor overlay proof.
- Use `cosmic-helper` for the COSMIC protocol helper lane.
- Use `cosmic-patched-cursor-host-proof` or `cosmic-transparent-xcursor-host-proof` only when the VM was booted into the matching COSMIC mode described in the docs.
- Use `codex-desktop` for Codex Desktop launch smoke. Add `--sync-codex-settings` only when the test needs authenticated Codex state.
- Use `all` only for the fast non-session-specific set; do not report it as complete cross-desktop coverage.

## Command Templates

Prefer `uv run python` when the Python environment matters; raw `python3` is acceptable for scripts that already run that way in the docs.

Plasma pointer/input proof:

```bash
uv run python scripts/run_gui_testing_vm_smoke.py \
  --host 127.0.0.1 --port 22222 --user skycua \
  --ssh-option StrictHostKeyChecking=no \
  --ssh-option UserKnownHostsFile=artifacts/testing-vm/known_hosts \
  --profile wayland-pointer \
  --desktop-env KDE --wayland-display wayland-0
```

Hyprland layer-shell proof:

```bash
uv run python scripts/run_gui_testing_vm_smoke.py \
  --host 127.0.0.1 --port 22222 --user skycua \
  --ssh-option StrictHostKeyChecking=no \
  --ssh-option UserKnownHostsFile=artifacts/testing-vm/known_hosts \
  --profile wayland-layer-shell-overlay \
  --desktop-env Hyprland --wayland-display wayland-1
```

KWin system-install proof:

```bash
uv run python scripts/run_gui_testing_vm_smoke.py \
  --host 127.0.0.1 --port 22222 --user skycua \
  --ssh-option StrictHostKeyChecking=no \
  --ssh-option UserKnownHostsFile=artifacts/testing-vm/known_hosts \
  --profile kde-kwin-effect-system-install \
  --vm-name testing-vm --libvirt-uri qemu:///session \
  --desktop-env KDE --wayland-display wayland-0
```

COSMIC scaled pointer proof:

```bash
uv run python scripts/run_gui_testing_vm_smoke.py \
  --host 127.0.0.1 --port 22222 --user skycua \
  --ssh-option StrictHostKeyChecking=no \
  --ssh-option UserKnownHostsFile=artifacts/testing-vm/known_hosts \
  --profile wayland-pointer-scaled \
  --desktop-env COSMIC --wayland-display wayland-1
```

## Failure Triage

- If SSH says `Could not resolve hostname testing-vm`, rerun with the `127.0.0.1:22222` port-forward form.
- If a profile fails on the wrong Wayland socket, switch the guest session with `scripts/testing-vm/select-session.sh`, then confirm compositor processes and `/run/user/1000/wayland-*`.
- If portal behavior looks wrong after switching desktops, rerun without `--skip-sync`; the runner refreshes the user portal stack and imports the requested desktop environment.
- If cleanup looks contaminated, remember the active cleanup target is `sky-cua-overlay-host` plus `service.sock` and `agent-cursor.sock`; stale `sky-cua-overlay` references are historical.
- If a local smoke points at nested X11/Xvfb, Docker GUI, or retired TIDAL flows, treat it as stale guidance unless the user explicitly asks for historical archaeology.

## Reporting

For a useful closure note, include:

- the selected guest session and display, such as Plasma `wayland-0` or Hyprland `wayland-1`
- the exact runner command and whether build, sync, or Codex settings sync was skipped
- the profile name
- the artifact directory or host summary path
- any cleanup residue, especially for KWin system-install proof
- any live-smoke gates not run
