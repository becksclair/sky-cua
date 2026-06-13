# Testing VM Desktop Smokes

Use this reference before running the real desktop matrix through `scripts/run_gui_testing_vm_smoke.py`.

## Connection

The libvirt domain is named `testing-vm`, but SSH may not resolve that hostname from the host. The session VM exposes SSH through the user-mode port forward:

```bash
--host 127.0.0.1 --port 22222 --user skycua \
--ssh-option StrictHostKeyChecking=no \
--ssh-option UserKnownHostsFile=artifacts/testing-vm/known_hosts
```

If a command fails with `Could not resolve hostname testing-vm`, rerun with the port-forward form instead of debugging the smoke itself.

## Session Selection

Select the intended desktop inside the guest before every real-session profile:

```bash
ssh -p 22222 skycua@127.0.0.1 'cd /workspace && sudo scripts/testing-vm/select-session.sh hyprland'
ssh -p 22222 skycua@127.0.0.1 'cd /workspace && sudo scripts/testing-vm/select-session.sh cosmic'
ssh -p 22222 skycua@127.0.0.1 'cd /workspace && sudo scripts/testing-vm/select-session.sh plasma'
```

The selector rewrites the greetd/GDM target, kills stale compositor and sky-cua processes for `skycua`, and restarts the display manager. Skipping it can leave stale Wayland sockets or the previous compositor alive, which turns profile failures into misleading environment noise.

Confirm the active session cheaply:

```bash
ssh -p 22222 skycua@127.0.0.1 'pgrep -a "kwin_wayland|Hyprland|cosmic-session|cosmic-comp"; ls -l /run/user/1000/wayland-*'
```

Observed display names:

- Plasma/KWin: `--desktop-env KDE --wayland-display wayland-0`
- Hyprland: `--desktop-env Hyprland --wayland-display wayland-1`
- COSMIC: `--desktop-env COSMIC --wayland-display wayland-1`

## Known Good Profiles

Hyprland layer-shell overlay:

```bash
uv run python scripts/run_gui_testing_vm_smoke.py \
  --host 127.0.0.1 --port 22222 --user skycua \
  --ssh-option StrictHostKeyChecking=no \
  --ssh-option UserKnownHostsFile=artifacts/testing-vm/known_hosts \
  --profile wayland-layer-shell-overlay \
  --desktop-env Hyprland --wayland-display wayland-1
```

COSMIC scaled pointer:

```bash
uv run python scripts/run_gui_testing_vm_smoke.py \
  --host 127.0.0.1 --port 22222 --user skycua \
  --ssh-option StrictHostKeyChecking=no \
  --ssh-option UserKnownHostsFile=artifacts/testing-vm/known_hosts \
  --profile wayland-pointer-scaled \
  --desktop-env COSMIC --wayland-display wayland-1
```

KWin effect system-install proof:

```bash
uv run python scripts/run_gui_testing_vm_smoke.py \
  --host 127.0.0.1 --port 22222 --user skycua \
  --ssh-option StrictHostKeyChecking=no \
  --ssh-option UserKnownHostsFile=artifacts/testing-vm/known_hosts \
  --profile kde-kwin-effect-system-install \
  --vm-name testing-vm --libvirt-uri qemu:///session \
  --desktop-env KDE --wayland-display wayland-0
```

Window-targeted and display-targeted screenshot proof:

```bash
uv run python scripts/run_gui_testing_vm_smoke.py \
  --host 127.0.0.1 --port 22222 --user skycua \
  --ssh-option StrictHostKeyChecking=no \
  --ssh-option UserKnownHostsFile=artifacts/testing-vm/known_hosts \
  --profile targeted-screenshot \
  --desktop-env KDE --wayland-display wayland-0

uv run python scripts/run_gui_testing_vm_smoke.py \
  --host 127.0.0.1 --port 22222 --user skycua \
  --ssh-option StrictHostKeyChecking=no \
  --ssh-option UserKnownHostsFile=artifacts/testing-vm/known_hosts \
  --profile display-screenshot \
  --desktop-env KDE --wayland-display wayland-0
```

For GNOME, Hyprland, and COSMIC, switch the guest session first and use that
session's `--desktop-env` and `--wayland-display`. For i3/X11, switch to `i3`
and run the same profiles with `--desktop-env i3`; the profile derives the
real Xorg display inside the guest.

Use `--skip-host-build` only when host release/dev artifacts were already rebuilt after the code under test. Otherwise let the runner rebuild and rsync the current checkout.

## False Trails

- `scripts/live_wayland_pointer_smoke.py --help` is not an inert help path; it can launch the GTK fixture. On hosts without Python `gi`, it fails before proving anything useful.
- Local layer-shell overlay proof may fail if the current compositor does not expose the screen capture protocol to `grim`. Treat `grim: compositor doesn't support the screen capture protocol` as a local capture limitation; use the VM profile for layer-shell proof.
- A Hyprland profile run while the guest is still in Plasma can fail with `grim: failed to create display` against a stale `wayland-1`; switch the guest session first.
- The KWin system-install profile must run from Plasma. If the guest is still COSMIC, it can exit before `host-framebuffer-ready.json` appears and report remote exit 67.
- The ydotool control socket can be a Unix datagram socket. Do not use `UnixStream::connect` as the readiness test; check that the path is a socket or prove it with a real `ydotool key ...` command.

## Closure Evidence

For a useful final report, include the profile, selected session, display name, and artifact directory. For KWin system install, also include whether cleanup left system effect files behind.
