# Profile matrix

The runner and the project operation docs are authoritative if this catalog
drifts. These are the current semantics needed to select and classify a run.

## `--profile all`

`all` is the routine full VM smoke gate. Its remote sequence is:

1. `isolated-xpra` (exit 67 is an allowed prerequisite skip)
2. `wayland-pointer`
3. `targeted-screenshot`
4. `display-screenshot`
5. `session-env`
6. `text-readback`
7. `codex-desktop`
8. `opencode-mcp`
9. `pi-mcp`
10. `codex-cua`
11. `kde-kwin-effect`

When `HOST_WAYLAND_DISPLAY` is non-empty, it then adds the headed nested
visual-debug profiles `kde-plasma`, `gnome`, `cosmic`, and `hyprland` in that
order. Without it, those four are intentionally skipped. They are debug
lanes, not proof that the VM has passed real-session acceptance for each
desktop.

The `codex-cua` entry in `all` runs the deterministic coverage gate inside the
VM. It does not run `scripts/live_agent_perf_judge.py`; use
`--profile codex-cua` when the host-side judge is required and host auth is
available.

## `--profile curated`

`curated` is the session-agnostic pre-merge set, in registry order:

`codex-desktop` -> `wayland-pointer` -> `session-env` -> `text-readback`.

It is not `all`, does not include host framebuffer proofs, scaled COSMIC
output, desktop-smoke, or i3, and does not provide complete cross-desktop
coverage. It preauthorizes required portals once and resets guest sky-cua
processes between members.

## Lane catalog

- `wayland-pointer`: visible pointer and input proof on the selected real
  desktop; `wayland-layer-shell-overlay`: service-backed layer-shell overlay
  and screenshot proof on a real Wayland socket.
- `targeted-screenshot` and `display-screenshot`: window-targeted and
  display-targeted capture; `session-env` and `text-readback`: environment and
  text-readback checks.
- `codex-desktop`: visible installed Codex Desktop launch; `opencode-mcp` and
  `pi-mcp`: installed-MCP wiring checks, with settings sync still opt-in.
- `cosmic-helper`: real COSMIC helper protocol; `wayland-pointer-scaled`:
  scaled cursor proof; `cosmic-patched-cursor-host-proof` and
  `cosmic-transparent-xcursor-host-proof`: specialized COSMIC host-framebuffer
  proofs requiring their matching boot mode.
- `kde-kwin-effect`: KWin build/load/IPC proof; the `-system-install` variant
  installs the exact production package path, captures host frames, and
  removes it. `kde-plasma`, `gnome`, `cosmic`, and `hyprland` are nested headed
  debug profiles; `i3` is real X11 session proof.
