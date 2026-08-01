# Standalone release package

## Status

Shipped for Linux x86-64 glibc. Last verified against the fixed-root standalone
implementation on 2026-08-01.

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

Initiate a release from a clean, synchronized `main` checkout:

```bash
python3 install.py release                 # next minor, patch reset to zero
python3 install.py release --patch         # next patch
python3 install.py release --minor         # next minor, patch reset to zero
python3 install.py release --major         # next major, minor and patch reset
python3 install.py release --version 2.3.4 # explicit increasing stable version
```

The selectors are mutually exclusive and `--version` accepts only canonical
stable `X.Y.Z` values. Release updates only the standalone `PRODUCT_VERSION`;
crate, Python, plugin, and Node package versions remain independent.

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

Release requires a clean `main` worktree whose configured remote `main` is
identical to local `HEAD`, requires exactly one push destination, and rejects
existing local or remote target tags. It
rewrites only `PRODUCT_VERSION`, runs `just verify`, commits only that file,
creates an annotated `standalone-vX.Y.Z` tag, and atomically pushes the branch
and tag without force. Successful local completion means the refs were pushed;
Gitea publication and Saga deployment remain asynchronous. Failures preserve
the visible local modification, commit, or tag for inspection and never reset,
clean, delete, or force-push state.

## Source paths

- `install.py` — build, install, and checkout-only release entrypoint.
- `scripts/standalone_release.py` — durable build, payload assembly, archive,
  fixed-root install, consumer projection, and release CLI dispatch.
- `scripts/_standalone_release_command.py` — guarded version and Git release
  transaction.
- `scripts/assemble_cua_node.py` — durable CUA Node component assembly.
- `scripts/build_plugin.py` — durable core plugin staging.
- `scripts/test_standalone_release.py` — payload and install convergence tests.
- `scripts/test_standalone_release_command.py` — focused release-command tests.

## Verification

Focused producer validation:

```bash
uv run pytest scripts/test_standalone_release.py scripts/test_standalone_release_command.py
python3 install.py build
tar tzf dist/sky-cua-linux-x64-glibc.tar.gz
```

Release tests use isolated temporary repositories and bare remotes. Producer
verification must not invoke `python3 install.py release` against the real
checkout because that command intentionally commits, tags, and pushes.

Install validation must use an isolated `HOME`/`XDG_DATA_HOME`; it must not
overwrite the live user's install during producer tests. Verify first install,
repeat install, replacement of an old-only marker, stable launchers, native-host
manifest targets, skills, and detected consumer configuration.

## Known limitations

- The standalone target is currently Linux x86-64 glibc.
- System package installation remains an operator prerequisite; this installer
  owns the sky-cua payload and user-level integration, not OS package managers.
- Codex Desktop and OpenClaw live acceptance remain consumer-owned. The
  producer installer owns stable OpenClaw integration projection, including
  global `node_repl` and the no-prompt native Codex policy.

## Related

- [`docs/features/complete-cua-stack-ownership.md`](complete-cua-stack-ownership.md)
- [`docs/features/one-shot-installer.md`](one-shot-installer.md)
- [`docs/operations/plugin-release.md`](../operations/plugin-release.md)
- [`docs/runtime/mcp-boundary.md`](../runtime/mcp-boundary.md)
