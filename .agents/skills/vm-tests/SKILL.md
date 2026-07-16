---
name: vm-tests
description: Use when running, selecting, debugging, or reporting an Arch testing-vm smoke through scripts/run_gui_testing_vm_smoke.py, including profile choice, guest-session/socket verification, and artifact evidence. Do not trigger for config or credential sync, provisioning, viewer control, local smokes, or unit tests unless a smoke operation is also requested.
---

# VM Tests

Operate the real Arch `testing-vm` smoke lane through
`scripts/run_gui_testing_vm_smoke.py`. This skill is not for local-only smokes,
the retired nested Docker/Xvfb path, or a configuration-sync task by itself.

## Mandatory plan contract

- Every plan/report must show the fully expanded runner command, exact profile membership/order, selected guest session/display, sync choices, artifact path, every outcome/conditional skip, and live-smoke gates not run.
- `all` is exactly `isolated-xpra`, `wayland-pointer`, `targeted-screenshot`, `display-screenshot`, `session-env`, `text-readback`, `codex-desktop`, `opencode-mcp`, `pi-mcp`, `codex-cua`, `kde-kwin-effect`; add `kde-plasma`, `gnome`, `cosmic`, `hyprland` in that order only when `HOST_WAYLAND_DISPLAY` is set. The VM-local `codex-cua` gate is not the host performance judge or real-session cross-desktop acceptance.
- `curated` is the session-agnostic sequence `codex-desktop`, `wayland-pointer`, `session-env`, `text-readback`; preauthorize portals once and reset guest sky-cua processes between members.
- The exact port-forwarded runner prefix is `uv run python scripts/run_gui_testing_vm_smoke.py --host 127.0.0.1 --port 22222 --user skycua --ssh-option StrictHostKeyChecking=no --ssh-option UserKnownHostsFile=artifacts/testing-vm/known_hosts`; copy it literally and do not substitute `/dev/null` or invent transport flags. A Hyprland overlay run appends `--profile wayland-layer-shell-overlay --desktop-env Hyprland --wayland-display wayland-1` after selecting and verifying the real Hyprland session/socket.
- KWin production proof must name `--profile kde-kwin-effect-system-install --vm-name testing-vm --libvirt-uri qemu:///session --desktop-env KDE --wayland-display wayland-0`, require host framebuffer/`host-summary.json`, verify cleanup of `sky-cua-overlay-host`, `service.sock`, and `agent-cursor.sock`, and classify `sky-cua-overlay` as historical residue only.

## Route

Use the compact router below for known lanes. Consult
`docs/operations/gui-desktop-test-harness.md`,
`docs/operations/testing-vm-desktop-smokes.md`, and the runner's `--help` or
source only when adding flags, diagnosing drift/failure, or resolving a
router mismatch; those sources are authoritative. Load the linked references
only when the lane needs their catalog, command, or troubleshooting.

| Task | Profile or action | Guest session | Evidence to preserve |
| --- | --- | --- | --- |
| Routine full gate | `--profile all` | Current visible session; headed legacy additions need host `HOST_WAYLAND_DISPLAY` | Exact command, fixed sequence, conditional skips, artifacts; VM-local `codex-cua` gate only, not the host judge |
| Trimmed pre-merge gate | `--profile curated` | Current real session | `codex-desktop`, `wayland-pointer`, `session-env`, `text-readback` outcomes in order, artifacts, aggregate status |
| Real-session desktop acceptance | Select target, then run its real-session lane | KDE `wayland-0`; GNOME `wayland-0`; COSMIC/Hyprland `wayland-1`; i3 derives X11 | Explicit target session/display, profile result, artifacts; never count `all`'s nested debug lanes as acceptance |
| Pointer/input | `wayland-pointer` | Target real desktop/display | Profile result, session/display, artifact directory |
| Layer-shell overlay | `wayland-layer-shell-overlay` | Usually Hyprland `wayland-1`; use the selected real socket | Profile result, session/display, overlay/screenshot artifacts |
| Screenshot/readback | `targeted-screenshot`, `display-screenshot`, `text-readback` | Target real session | Exact profile, session/display, capture/readback artifacts |
| COSMIC helper or scaled cursor | `cosmic-helper`, `wayland-pointer-scaled` | COSMIC `wayland-1` | Helper replies or scaled-pointer evidence plus artifacts |
| KWin production package path | `kde-kwin-effect-system-install`, with `--vm-name testing-vm --libvirt-uri qemu:///session` | Plasma/KDE `wayland-0` | Host framebuffer/`host-summary.json`, exact cleanup result; current targets are `sky-cua-overlay-host`, `service.sock`, and `agent-cursor.sock` |
| Agent harness | `codex-desktop`, `opencode-mcp`, or `pi-mcp` | Current visible session | Tool/launch result, artifact path, and whether settings were explicitly synced |
| Profile inventory or selection | `--list-profiles` | None | Current registry and the selected lane; do not imply a smoke ran |

For the complete `all`/`curated` order and lane catalog, read
`references/profile-matrix.md` before planning, running, or reporting either
aggregate profile. Before writing or running any SSH/runner command, read
`references/commands.md` and copy its flag names; never invent transport
flags. For a failed or misleading run, read
`references/troubleshooting.md`.

## Execution boundaries

1. For a real-session lane, select or confirm the guest session first, then
   verify the compositor and `/run/user/1000/wayland-*` sockets (or the X11
   display). The runner does not select a compositor session; use the helper
   only when the task explicitly requires changing it. Do not paper over a
   stale socket with nested Docker/Xvfb guidance.
2. Use the runner's default host build and checkout sync. Pass
   `--skip-host-build` or `--skip-sync` only after confirming that the exact
   artifacts under test are already present in the VM.
3. If `testing-vm` does not resolve, use the documented `127.0.0.1:22222`
   port-forward and known-hosts options. This changes transport, not the
   profile or acceptance target.
4. Settings and credentials sync is opt-in: use `--sync-codex-settings`,
   `--sync-opencode-settings`, or `--sync-pi-settings` only when the requested
   authenticated lane needs it. Do not copy credentials merely to prepare a
   later run.

## Stop and report

Stop when the requested profile or sequence returns, including an intentional
conditional skip, or at the first concrete preflight blocker after preserving
its logs and artifact paths. Do not continue into unrelated lanes or invent a
fallback acceptance path. Report the exact runner command; selected guest
session and display; profile and per-member outcomes or conditional skips;
host/checkout/settings sync choices; artifact directory or host summary;
cleanup residue (especially KWin system-install); and live-smoke gates not
run. For KWin system-install, name the current cleanup targets
`sky-cua-overlay-host`, `service.sock`, and `agent-cursor.sock`, and classify
`sky-cua-overlay` as historical. For `all`, explicitly state that the VM-local deterministic `codex-cua`
gate ran while the host performance judge did not, that headed legacy
compositor lanes were conditional, and that real-session per-desktop
acceptance remains separate.
