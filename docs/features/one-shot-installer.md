# One-shot installer

## Status

Shipped. Last verified: 2026-06-12, full live run on the KDE host
(`python3 install.py --skip-build` -> all phases ok for codex, claude-code,
claude-desktop, opencode, pi, openclaw; doctor and `codex mcp list` /
`claude mcp list` health checks green).

## Summary

`install.py` turns a fresh checkout or an extracted release package into a
fully set-up sky-cua machine in one command: system dependencies, the runtime
(built from source or taken from a prebuilt bundle), the computer-use compat
plugin materialized from the bundled preflight, MCP server registration plus
skills for every detected agent, and a final health check.

## Contract surface

- `python3 install.py` at the repo root (Python 3.12+, stdlib only at the
  shim level; delegates to `scripts/installer.py`). The release package ships
  its own top-level `install.py` that pins bundle mode and its payload path
  before delegating to the same installer.
- Two modes, `--mode {auto,repo,bundle}`:
  - `repo` builds the Rust runtime from a checkout (requires `cargo` and
    `git`), then installs.
  - `bundle` installs a prebuilt release bundle with no build and no cargo.
  - `auto` (default) picks `bundle` when there is no `.git` checkout, so an
    extracted release package resolves to bundle mode automatically. A checkout
    without `cargo` stays in `repo` mode and reports the missing Rust toolchain.
- Flags: `--agents codex,claude-code,claude-desktop,opencode,pi,openclaw`
  (default: auto-detect), `--mode {auto,repo,bundle}`, `--bundle-root`
  (default `dist/plugin/sky-cua`), `--target-dir` (default
  `~/.local/share/sky-cua`), `--codex-home`, `--kwin-effect`,
  `--skip-system-deps`, `--skip-build`, `--skip-health`, `--dry-run`.
- Exit code 0 only when no executed phase failed; the run ends with a
  per-phase summary table.
- Agent detection: `codex`/`opencode`/`openclaw` CLIs on PATH, `claude` CLI
  or `~/.claude`, `~/.config/Claude` (or the macOS equivalent), and
  `~/.pi/agent`.

## Behavior

Phases run in order and delegate to the existing tooling instead of
duplicating it:

1. **system-deps** (Linux only): in `repo` mode requires `cargo` and `git` on
   PATH; `bundle` mode needs only runtime libraries. Installs missing runtime
   packages via `sudo pacman -S --needed` or `sudo apt-get install`
   (GStreamer, AT-SPI, libxkbcommon, wayland, dbus, xdg-desktop-portal,
   ydotool). Skipped with a note on other platforms or unknown package
   managers.
2. **build**: `scripts/build_plugin.py` (cargo release build + staged
   bundle under `dist/plugin/sky-cua`). Skipped in `bundle` mode, which uses
   the prebuilt bundle at `--bundle-root`.
3. **codex**: installs the payload into the local Codex cache
   (`installed_plugin_root`) and materializes the `computer-use` compat plugin
   from the bundled preflight (`resources/chrome_preflight.py`), then applies
   compat-first config enablement. No marketplace and no `codex` CLI
   `plugin/install`. On Linux the compat root is the enabled plugin id; where
   no compat root can be materialized, the installer enables `sky-cua@local`
   directly.
4. **agent:<host>** for each non-Codex agent:
   `scripts/install_mcp_server.py --host <host> --restart-runtime`, which
   installs binaries to the target dir, writes/registers host configs, and
   deploys skills where the host supports them.
5. **kwin-effect** (opt-in via `--kwin-effect`): `_kwin_effect.deploy_kwin_effect`.
6. **health**: `sky-cua-client doctor` against the installed binaries, plus
   `codex mcp list` (expects `computer-use`) and `claude mcp list` (expects
   `sky-cua`) for the selected agents. `--skip-health` is reserved for
   headless validation lanes that perform their own degraded assertions.

A failed system-deps or build phase aborts the run; later phases record
failures but let the remaining agents proceed, so one missing host does not
block the others.

## Source paths

- `install.py` - root shim (interpreter guard, `scripts/` import path).
- `scripts/installer.py` - phase orchestration, detection, mode resolution.
- `scripts/package.py` - builds the release tarball that ships the
  bundle-mode installer subset; see
  [`docs/features/release-package.md`](release-package.md).
- Reused: `scripts/build_plugin.py`, `scripts/install_plugin.py`,
  `scripts/install_mcp_server.py`, `scripts/_kwin_effect.py`,
  `resources/chrome_preflight.py` (payload-side preflight that materializes the
  compat plugin, invoked by the Codex phase).

## Verification

- `uv run pytest scripts/test_installer.py` - detection, selection,
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
- The Codex phase materializes the compat plugin locally from the bundled
  preflight; on platforms without a compat root it enables `sky-cua@local`
  directly (Windows-Codex compat is not yet implemented).

## Related

- `docs/runtime/compat-plugin-contract.md` - compat root the Codex phase
  refreshes.
- `docs/operations/plugin-release.md` - local deploy, release packaging, and
  clean-machine install entrypoints.
- `docs/features/release-package.md` - `package.py` tarball layout the
  bundle-mode installer consumes.
- ROADMAP: Host portability -> one-shot installer entry.
