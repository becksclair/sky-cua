# One-shot installer

## Status

Shipped. Last verified: 2026-06-12, full live run on the KDE host
(`python3 install.py --skip-build` → all phases ok for codex, claude-code,
claude-desktop, opencode, pi, openclaw; doctor and `codex mcp list` /
`claude mcp list` health checks green).

## Summary

`install.py` turns a fresh clone into a fully set-up sky-cua machine in one
command: system dependencies, the Rust runtime build, the Heliasar Codex
marketplace install (with the computer-use compat plugin root), MCP server
registration plus skills for every detected agent, and a final health check.

## Contract surface

- `python3 install.py` at the repo root (Python 3.12+, stdlib only at the
  shim level; delegates to `scripts/installer.py`).
- Flags: `--agents codex,claude-code,claude-desktop,opencode,pi,openclaw`
  (default: auto-detect), `--target-dir` (default `~/.local/share/sky-cua`),
  `--codex-home`, `--marketplace-root`, `--marketplace-source`,
  `--kwin-effect`, `--skip-system-deps`, `--skip-build`, `--dry-run`.
- Exit code 0 only when no executed phase failed; the run ends with a
  per-phase summary table.
- Agent detection: `codex`/`opencode`/`openclaw` CLIs on PATH, `claude` CLI
  or `~/.claude`, `~/.config/Claude` (or the macOS equivalent), and
  `~/.pi/agent`.

## Behavior

Phases run in order and delegate to the existing tooling instead of
duplicating it:

1. **system-deps** (Linux only): requires `cargo` and `git` on PATH, then
   installs missing runtime packages via `sudo pacman -S --needed` or
   `sudo apt-get install` (GStreamer, AT-SPI, libxkbcommon, wayland, dbus,
   xdg-desktop-portal, ydotool). Skipped with a note on other platforms or
   unknown package managers.
2. **build**: `scripts/build_plugin.py` (cargo release build + staged
   bundle under `dist/plugin/sky-cua`).
3. **codex**: `scripts/setup_heliasar_marketplace.py` — marketplace
   checkout/add, `codex` plugin install, compat plugin root refresh through
   the payload preflight, compat-first config enablement, MCP reload.
4. **agent:<host>** for each non-Codex agent:
   `scripts/install_mcp_server.py --host <host> --restart-runtime`, which
   installs binaries to the target dir, writes/registers host configs, and
   deploys skills where the host supports them.
5. **kwin-effect** (opt-in via `--kwin-effect`): `_kwin_effect.deploy_kwin_effect`.
6. **health**: `sky-cua-client doctor` against the installed binaries, plus
   `codex mcp list` (expects `computer-use`) and `claude mcp list` (expects
   `sky-cua`) for the selected agents.

A failed system-deps or build phase aborts the run; later phases record
failures but let the remaining agents proceed, so one missing host does not
block the others.

## Source paths

- `install.py` — root shim (interpreter guard, `scripts/` import path).
- `scripts/installer.py` — phase orchestration, detection, package planning.
- Reused: `scripts/build_plugin.py`, `scripts/setup_heliasar_marketplace.py`,
  `scripts/install_mcp_server.py`, `scripts/_kwin_effect.py`,
  `resources/chrome_preflight.py` (payload-side sync engine, invoked by the
  Codex flow).

## Verification

- `uv run pytest scripts/test_installer.py` — detection, selection,
  package planning, phase wiring, dry-run output.
- Live: full `python3 install.py --skip-build` run on the KDE host
  (2026-06-12), summary all ok; compat root `.mcp.json` mtime unchanged
  across the rerun (idempotent re-deploy preserves per-plugin state).

## Known limitations

- System dependency installation covers pacman and apt; other package
  managers get a skip note and manual instructions.
- Windows/macOS runs skip the system-deps phase entirely; the build and
  agent phases work but are not the primary tested path.
- `claude-desktop` and `opencode` configs are written for manual merge where
  the host has no registration CLI (unchanged from `install_mcp_server.py`).
- The Codex phase requires the `codex` CLI and reachable marketplace source;
  there is no offline/local-only Codex fallback lane in the installer.

## Related

- `docs/runtime/compat-plugin-contract.md` — compat root the Codex phase
  refreshes.
- `docs/operations/plugin-release.md` — release/publish flow the installer
  consumes via the Heliasar marketplace.
- ROADMAP: Host portability → one-shot installer entry.
