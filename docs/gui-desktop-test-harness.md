# GUI Desktop Test Harness

This project needs live desktop sessions to validate Linux Computer Use backends. Unit tests cover parsers and routing, but GNOME Shell, KWin, COSMIC, Hyprland, i3, portals, AT-SPI, and rendered windows need a real compositor session.

## Host Findings

Current CachyOS/KDE6 host checks found these installed: `at-spi2-core`, `xdg-desktop-portal`, `xdg-desktop-portal-kde`, `gdbus`, `xprop`, `xdotool`, `ydotool`, `pipewire`, `gsettings`, `cosmic-session`, Rust, `chromium`, and `brave`.

Missing for local live-testing outside containers:

```bash
sudo pacman -S gnome-shell xdg-desktop-portal-gnome
sudo pacman -S hyprland i3-wm
```

## Intended Harness Shape

Use a GUI-enabled container with all target desktop environments installed. Each run selects one profile and starts a full nested graphical session, not a headless process tree.

Profiles:

- `kde`: KWin, KDE portal, PipeWire capture, X11/XWayland fallback, AT-SPI.
- `gnome`: GNOME Shell Introspect, Codex GNOME Shell extension, GNOME portal.
- `cosmic`: `sky-cua-cosmic-helper`, COSMIC window listing/focus, portal behavior.
- `hyprland`: `hyprctl clients -j`, focus dispatch, terminal enrichment.
- `i3`: `i3-msg -t get_tree`, `xprop` PID hydration, X11/XTest input.

The harness should expose VNC/noVNC for visual debugging and preserve logs under `artifacts/gui-desktop-smoke/<profile>/`.

## Current Verification Status

The new cross-desktop windowing code has compile/unit-test coverage, but the desktop-specific live smokes are still pending. They are blocked until the runner can start or attach to the real sessions listed below. Parser tests and a KDE host session are not enough proof for compositor-local IPC, focus, portal, AT-SPI, or extension behavior.

The current runner is deliberately not a passing smoke: it records the selected profile under `artifacts/gui-desktop-smoke/<profile>/result.json` with `"implemented": false` and exits non-zero. Treat that artifact as a blocked-status marker only, not proof that the desktop profile works.

Progress ledger:

| Area | Status | Current proof | Remaining proof |
| --- | --- | --- | --- |
| Runner contract | Partial | `scripts/gui_desktop_smoke.py` accepts target profiles and writes blocked-status artifacts. | Start or attach to real nested sessions and run MCP actions inside them. |
| KDE/KWin | Partial | Existing KDE host proofs cover earlier live flows; KWin parser/registry code exists. | Re-smoke through the unified registry path and record `artifacts/gui-desktop-smoke/kde/`. |
| GNOME | Pending | GNOME Introspect and extension code paths exist. | Real GNOME Shell session proving Introspect, extension install/enable, DBus listing, and activation. |
| COSMIC | Pending | `sky-cua-cosmic-helper` exists and is packaged for Linux. | Real COSMIC session proving listing, focused-window detection, and activation. |
| Hyprland | Pending | `hyprctl` parser/focus code exists. | Real Hyprland session proving listing, focus dispatch, terminal enrichment, and portal behavior. |
| i3 | Pending | `i3-msg` parser/focus code exists. | Real i3/X11 session proving listing, PID hydration, focus activation, terminal enrichment, and X11/XTest input. |
| Browser companion | Complete for host compatibility | Codex Desktop Settings proof shows Browser Use and Chrome visible beside Computer Use. `artifacts/chrome-host-smoke/20260514T154125Z/result.json` proves the official extension using the sky-cua host binary for `getInfo`, `getTabs`, heartbeat, and rollout `turnEnded`. | First-class `browser_*` MCP tools are a separate future product decision, not a host-compatibility gate. |

Separate from this matrix, Codex Desktop Settings proof for the bundled
`computer-use`/`browser-use`/`chrome` compatibility lane exists under
`artifacts/desktop-ui-proof/20260513T100515Z-browser-settings-patched-profile/`.
That proof confirms the Desktop UI sees Computer Use and Browser Use with the
Chrome companion plugin after the narrow CodexDesktop-Rebuild patch. It does
not replace the compositor-specific live smoke matrix below.

Pending gaps:

- `gnome`: needs a real GNOME Shell session to prove GNOME Introspect listing, bundled extension install/enable, extension DBus listing, and exact activation.
- `cosmic`: needs a real COSMIC session to prove `sky-cua-cosmic-helper` listing, focused-window detection, and activation.
- `hyprland`: needs a real Hyprland session to prove `hyprctl clients -j`, focus dispatch, terminal enrichment, and portal behavior.
- `i3`: needs a real i3/X11 session to prove `i3-msg -t get_tree`, `xprop` PID hydration, focus activation, terminal enrichment, and X11/XTest input.
- `kde`: should be re-smoked after registry changes to prove KWin/X11 fallback behavior still passes through the new probe-order path.
- Browser companion: host compatibility is live-proven through the official
  extension and sky-cua host binary. Browser automation outside Codex Desktop's
  existing Browser Use plugin flow remains a separate future MCP-tooling scope.

Do not mark any of these live-smoke gaps complete until the command, desktop profile, and artifact directory are recorded.

## Runner Contract

The runner is `scripts/gui_desktop_smoke.py`:

```bash
python3 scripts/gui_desktop_smoke.py --profile kde
python3 scripts/gui_desktop_smoke.py --profile gnome
python3 scripts/gui_desktop_smoke.py --profile cosmic
python3 scripts/gui_desktop_smoke.py --profile hyprland
python3 scripts/gui_desktop_smoke.py --profile i3
python3 scripts/gui_desktop_smoke.py --profile kde --include-browser-smoke
```

For now the runner records the contract and creates the artifact directory. `--include-browser-smoke` is only part of that recorded contract until profile startup and browser-extension smoke are implemented. The full Docker profile implementation should add `docker/gui-test/Dockerfile` plus `docker/gui-test/profiles/*.sh`.

## Container Requirements

The eventual Docker run may need:

- `/dev/dri` passthrough for hardware rendering when available.
- A software-rendering fallback for CI or restricted hosts.
- Semi-privileged portal tests for RemoteDesktop behavior.
- A session bus, PipeWire/WirePlumber, desktop-specific portal backend, and deterministic test apps inside the container.

Document exact Docker flags once the first profile is implemented.
