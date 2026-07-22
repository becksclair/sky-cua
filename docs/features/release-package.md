# Standalone release package

## Status

Shipped for Linux x86-64 glibc. Last verified against the fixed-root standalone
implementation on 2026-07-22.

## Summary

`python3 install.py build` owns every generated input and emits one complete,
checkout-free sky-cua artifact. The payload contains the native CUA runtime,
Node 24 and `node_repl`, Browser/Computer/Phone JavaScript, the latest bundled
Chrome extension, native messaging host, Codex compatibility plugins, skills,
model documentation, and its installer.

The distribution deliberately has no release generations, `current` selector,
rollback operation, manifest-hash selection, or consumer trust-hash contract.

## Contract surface

Build from the repository root:

```bash
python3 install.py build
```

Durable build outputs are:

- `dist/plugin/sky-cua` — staged core plugin.
- `out/components/cua-node-linux-x64-glibc` — assembled CUA Node runtime.
- `dist/standalone/sky-cua-linux-x64-glibc` — complete unpacked payload.
- `dist/sky-cua-linux-x64-glibc.tar.gz` — distributable archive.

These paths are stable on purpose: repeated builds can reuse precompiled Rust,
JavaScript, and Node artifacts instead of compiling inside a temporary tree.
The archive expands to `sky-cua-linux-x64-glibc/` and ships its own `install.py`.

Install from either the checkout or extracted artifact:

```bash
python3 install.py install
```

The install root is exactly:

```text
${XDG_DATA_HOME:-~/.local/share}/sky-cua
```

There are no install-root, generation, rollback, or hash arguments.

## Behavior

The builder refreshes the durable CUA Node component and core plugin, flattens
them into the standalone tree, selects only the highest-version bundled Chrome
extension, validates required files, and writes the archive. Multiple historical
extension source directories may exist in the checkout, but the artifact carries
one extension tree under `browser/extension`.

Installation stages beside the destination for recoverable replacement, then replaces
the one fixed root. It creates stable launchers under `~/.local/bin`, writes
Chrome/Chromium/Brave native-host manifests that target the stable launcher,
projects skills into detected agent skill roots, and configures detected Codex
and OpenClaw consumers. Repeating install converges to the same layout and
removes files that existed only in the previous installed payload.

The artifact's `RELEASE.json` describes semantic paths and target identity. It
does not expose per-component trust hashes or select an installed generation.

## Source paths

- `install.py` — two-command public entrypoint.
- `scripts/standalone_release.py` — durable build, payload assembly, archive,
  fixed-root install, and consumer projection.
- `scripts/assemble_cua_node.py` — durable CUA Node component assembly.
- `scripts/build_plugin.py` — durable core plugin staging.
- `scripts/test_standalone_release.py` — payload and install convergence tests.

## Verification

Focused producer validation:

```bash
uv run pytest scripts/test_standalone_release.py
python3 install.py build
tar tzf dist/sky-cua-linux-x64-glibc.tar.gz
```

Install validation must use an isolated `HOME`/`XDG_DATA_HOME`; it must not
overwrite the live user's install during producer tests. Verify first install,
repeat install, replacement of an old-only marker, stable launchers, native-host
manifest targets, skills, and detected consumer configuration.

## Known limitations

- The standalone target is currently Linux x86-64 glibc.
- System package installation remains an operator prerequisite; this installer
  owns the sky-cua payload and user-level integration, not OS package managers.
- Codex Desktop and OpenClaw consumer-side convergence are owned and validated
  in their respective repositories.

## Related

- [`docs/features/complete-cua-stack-ownership.md`](complete-cua-stack-ownership.md)
- [`docs/features/one-shot-installer.md`](one-shot-installer.md)
- [`docs/operations/plugin-release.md`](../operations/plugin-release.md)
- [`docs/runtime/mcp-boundary.md`](../runtime/mcp-boundary.md)
