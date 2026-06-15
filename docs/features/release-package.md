# Release package

## Status

Shipped. Last verified: 2026-06-15 (`scripts/package.py` builds a tarball; the
package-root `install.py` resolves to bundle mode and installs without a build
toolchain). Headless Docker validation of the clean-machine flow is tracked
under "Known limitations".

## Summary

`scripts/package.py` builds a self-contained release tarball that installs
sky-cua on a clean machine with no checkout, no Rust toolchain, and no
marketplace. The package carries the plugin bundle, a pure-Python installer
subset, mirrored skills, and a top-level `install.py` that runs the existing
installer in bundle mode.

## Contract surface

- `python3 scripts/package.py` builds
  `dist/release/sky-cua-<version>-<platform>.tar.gz`. Flags:
  - `--no-build`: package the existing `dist/plugin/sky-cua` bundle as-is.
  - `--platform`: target platform id (default: current host platform).
    Packaging fails loudly when the bundle lacks that platform's runtime
    binaries.
  - `--version-from-tag [TAG]`: set the bundle version from a `vX.Y.Z` git tag
    before packaging. When `TAG` is omitted, use the current CI/git tag.
  - `--release-dir`: output directory (default `dist/release`).
- Tarball layout (single top-level `sky-cua-<version>/` directory):

  ```text
  sky-cua-<version>/
  |-- install.py            # package-root installer (pins bundle mode + payload)
  |-- VERSION               # version stamp
  |-- plugin/sky-cua/       # full plugin bundle (binaries, resources, manifests)
  |-- scripts/              # pure-Python installer subset (no cargo, no build)
  `-- skills/               # mirrored skills (computer-use, browser-use)
  ```

- The package `scripts/` subset is import-closed: `installer.py` ->
  `install_mcp_server.py` / `install_plugin.py` -> `_install_shared.py`,
  `_openclaw_install.py`, `_kwin_effect.py`, `_plugin_bundle.py`. It does not
  ship `build_plugin.py` or any cargo-dependent tooling.
- The package-root `install.py` is generated, not copied: it pins
  `--mode bundle` and `--bundle-root <package>/plugin/sky-cua`, then forwards
  any user-supplied installer flags after those defaults.

## Behavior

`package.py` builds `dist/plugin/sky-cua` (unless `--no-build`), validates the
bundle structure, optionally rewrites the bundle version from a git tag, and
asserts the target platform's runtime binaries are present. It then stages the
package tree in a temporary directory and writes the gzip tarball.

On a clean machine the flow is:

```bash
tar xzf sky-cua-<version>-<platform>.tar.gz
cd sky-cua-<version>
python3 install.py
```

The package-root `install.py` delegates to the bundled `scripts/installer.py`
in bundle mode, which:

- skips the build phase and installs the prebuilt bundle from `--bundle-root`;
- runs the Codex phase by installing the payload into the local Codex cache and
  materializing the `computer-use` compat plugin from the bundled preflight
  (`resources/chrome_preflight.py`) - no marketplace, no `codex` CLI
  `plugin/install`. On Linux the compat root is the enabled plugin id; where no
  compat root can be materialized it enables `sky-cua@local` directly;
- registers the MCP server and skills for every detected agent (Codex, Claude
  Code, Claude Desktop, OpenCode, Pi, OpenClaw);
- ends with a per-phase health summary.

Mode resolution: `--mode auto` (the installer default) picks `bundle` when
there is no `.git`, so an extracted package resolves to bundle mode; the
package-root `install.py` additionally pins `--mode bundle` explicitly. From a
full checkout, `python3 install.py` resolves to `repo` mode and builds the
runtime from source first; a checkout without `cargo` stays in `repo` mode so
the installer reports the missing Rust toolchain instead of using a stale
bundle. See
[`docs/features/one-shot-installer.md`](one-shot-installer.md) for the full
phase list and agent detection.

## Source paths

- `scripts/package.py` - release tarball builder (staging, version handling,
  platform-binary assertion, tarball write).
- `scripts/installer.py` - phase orchestration shared by repo and bundle modes;
  `mode auto` resolution.
- `install.py` - repo-root shim (repo mode by default through `auto`).
- `scripts/_plugin_bundle.py` - bundle paths, structure checks, version-from-tag
  helpers consumed by `package.py`.
- `resources/chrome_preflight.py` - preflight that materializes the compat
  plugin during the Codex phase.

## Verification

- `uv run pytest scripts/test_installer.py scripts/test_package.py` - installer
  mode resolution + the bundle-mode Codex/agent phases, plus the package staging
  tree, script allowlist, skills mirror, version stamp, and platform gate.
- `uv run pytest scripts/test_runtime_packaging.py` - runtime bundle helper
  coverage consumed by `package.py`.
- `python3 scripts/package.py --no-build` produces a tarball under
  `dist/release/`.
- `docker/validate/run.sh --tarball dist/release/sky-cua-<version>-<platform>.tar.gz`
  builds a headless container, runs `install.py` from the tarball on a clean
  archlinux base (no repo, no cargo), and asserts the Codex compat plugin is
  enabled, OpenCode is registered, and the installed binary runs. Host auth is
  not mounted by default; pass `--with-host-auth` only for trusted tarballs that
  need live host credentials.

## Known limitations

- The headless Docker harness (`docker/validate/`) proves install + config +
  binary execution, but not live desktop control: the MCP server needs a
  desktop session, so the live-MCP handshake is informational there and
  tool-execution checks stay on the GUI VM.
- `package.py` packages one platform per invocation; cross-platform tarballs
  require the target platform's binaries to be pre-staged in the bundle before
  packaging.
- The Codex phase materializes the compat plugin only where a compat root is
  available (Linux today); other platforms enable `sky-cua@local` directly and
  Windows-Codex compat is not yet implemented.

## Related

- [`docs/features/one-shot-installer.md`](one-shot-installer.md) - the shared
  installer the package runs in bundle mode.
- [`docs/operations/plugin-release.md`](../operations/plugin-release.md) -
  local deploy, packaging, and clean-machine install runbook.
- [`docs/runtime/compat-plugin-contract.md`](../runtime/compat-plugin-contract.md)
  - compat root the Codex phase materializes.
- ROADMAP entry: [`ROADMAP.md`](../../ROADMAP.md) under "Host portability".
