# Complete CUA stack ownership

## Status

Shipped for the Linux x86-64 glibc producer. The fixed-root standalone
distribution contract replaced the former generation-selected release model on
2026-07-22; consumer-repository convergence is tracked separately.

## Summary

sky-cua owns one complete standalone artifact: the native direct MCP server,
persistent Node 24 `node_repl`, Browser/Computer/Phone JavaScript facades, the
latest bundled Chrome extension, native messaging host, Codex compatibility
plugins, skills, and model-facing documentation. Codex Desktop and OpenClaw
consume stable paths from the same fixed install root.

## Contract surface

The producer exposes one build command, one install command, one archive, and
one install root:

```bash
python3 install.py build
python3 install.py install
```

```text
dist/sky-cua-linux-x64-glibc.tar.gz
${XDG_DATA_HOME:-~/.local/share}/sky-cua
```

`sky_cua` is the direct native MCP server. `node_repl` preserves `js`,
`js_reset`, and `js_add_node_module_dir` with a persistent Node 24 VM. Installed
JavaScript exposes the Browser, Computer, and Phone APIs from the same payload.
Its fixed lazy-loader surface also provides Acorn 8.16.0 and Acorn Walk 8.3.5
for structural inspection of generated or minified JavaScript without
repository-specific module registration.

The standalone artifact contains one `browser/extension` tree selected from the
latest bundled Chrome extension source version. Browser code trust derives from
the installed fixed-root package layout; consumers do not receive or maintain a
Browser-client hash allowlist.

There is no public generation identity, `current` link, rollback operation,
manifest-hash install argument, or consumer trust-hash contract.

## Behavior

Both MCP servers share the daemon and daemon-owned Browser scheduler; direct
`sky_cua` has no Node hop. Browser bridge caller provenance and separate tab
groups remain runtime concerns. Codex in-app Browser remains host-provided;
Chrome-family browsers use the packaged extension/native-host path.

The build owns all inputs and writes reusable outputs under `dist/` and
`out/components/`. Installation replaces the single fixed root, then projects
stable launchers, native-host manifests, skills, and detected consumer
registrations. Reinstall is convergent and removes stale files from an older
payload rather than retaining prior generations.

The Acorn packages are prepared by the frozen runtime Bun lock, validated
against exact package identity and tree hashes, and copied into the assembled
module root without network access or mutation of the immutable migration
seed. `nodeRepl.loaders.acorn()` and `nodeRepl.loaders.acornWalk()` share module
identity with ordinary package imports in each runtime generation.

## Source paths

- `install.py`
- `scripts/standalone_release.py`
- `scripts/assemble_cua_node.py`
- `scripts/build_plugin.py`
- `runtime/cua-node/`
- `packages/browser-use/`
- `packages/sky-cua-js/`
- `skills/browser-use/`, `skills/computer-use/`, and `skills/phone-use/`

## Verification

Producer acceptance includes focused standalone build/install tests, full CUA
Node tests, archive-shape inspection, and checkout-versus-extracted-artifact
convergence in isolated homes. The proof must establish:

- the build is self-sufficient and keeps reusable artifacts in durable paths;
- the archive contains every runtime, integration, skill, and documentation
  surface and only the latest Chrome extension;
- checkout and extracted installs converge on the fixed root;
- a replacement install removes old-only files;
- no generated consumer configuration contains a Browser trust hash.
- Acorn parses module syntax, Acorn Walk visits the expected nodes, and both
  convenience loaders preserve ordinary-import namespace identity from the
  installed fixed-root runtime.

Codex Desktop and OpenClaw perform their own repository-owned acceptance against
the producer artifact. Those changes are not made from the sky-cua repository.

## Known limitations

- The standalone target is currently Linux x86-64 glibc. macOS, arm64, musl,
  and Windows `node_repl` remain follow-ups.
- No npm packages are published; JavaScript packages are artifact components.
- Consumer live acceptance is separate from producer-safe local validation.

## Related

- [`docs/features/release-package.md`](release-package.md)
- [`docs/features/codex-desktop-compat.md`](codex-desktop-compat.md)
- [`docs/features/unified-browser-bridge-control-plane.md`](unified-browser-bridge-control-plane.md)
- [`docs/research/2026-07-model-facing-cua-docs-phone-js.md`](../research/2026-07-model-facing-cua-docs-phone-js.md)
