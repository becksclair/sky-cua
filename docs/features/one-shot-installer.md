# Fixed-root standalone installer

## Status

Shipped for Linux x86-64 glibc. Last verified against the standalone producer
implementation on 2026-07-24.

## Summary

`python3 install.py install` installs one complete sky-cua payload at one fixed
user-data root. The same command works from a repository checkout and from the
extracted standalone archive; checkout mode first refreshes the durable build
outputs.

## Contract surface

The public CLI contains exactly two commands:

```bash
python3 install.py build
python3 install.py install
```

`install` accepts no generation, rollback, install-root, or manifest-hash
arguments. Its destination is always:

```text
${XDG_DATA_HOME:-~/.local/share}/sky-cua
```

The installer projects these stable user-facing surfaces:

- launchers in `~/.local/bin` for `sky-cua-client`, `sky-cua-service`,
  `sky-cua-overlay-host`, `node_repl`, and `sky-cua-chrome-host`;
- Chrome, Chromium, Brave, and Brave Origin native-messaging manifests;
- `computer-use`, `browser-use`, and `phone-use` skill links for detected
  agent homes;
- fixed-root Codex compatibility plugins `computer-use@openai-bundled` and
  `browser@openai-bundled`, plus native install requests when Codex is detected;
- the global OpenClaw `node_repl` registration when OpenClaw is detected.

Consumer configuration points at stable paths under the fixed root. It does not
pin an artifact hash or trust a Browser client by hash.

The installed Codex marketplace is
`codex/openai-bundled/.agents/plugins/marketplace.json`. Its Browser source is
`codex/openai-bundled/plugins/browser/`; there is no `browser-use` plugin alias.
That plugin carries a `scripts/browser-client.mjs` adapter which resolves the
shared client through the `RELEASE.json` `paths.browser_client` semantic path.
The shared client accepts Codex Desktop's task-scoped
`type="iab"`/`transport="host_provided_iab"` backend and retains the distinct
`extension_native_host` transport used by non-IAB consumers.

## Behavior

From a checkout, `install` runs the same durable build used by
`python3 install.py build`; compilation and assembly stay under `target/`,
`out/components/`, and `dist/`. From an extracted archive, the packaged payload
is already complete and no source build occurs.

The installer validates the payload, removes the fixed destination, and copies
the payload directly into its place before projecting integrations. A second
install converges to the same tree. Replacing the payload also removes files
that existed only in the previous install, so stale contents cannot accumulate.

The native-host manifests target the stable `~/.local/bin/sky-cua-chrome-host`
launcher. The installed standalone payload carries exactly one Chrome extension:
the latest version selected during build.

The payload retains its private `bin/node` runtime for `node_repl`, but the
installer does not project it as the user's `~/.local/bin/node`. Upgrading
removes that legacy launcher only when it is a symlink into the sky-cua install
root; a user-owned file or unrelated symlink at that path is preserved.

## Source paths

- `install.py` — Python-version guard and public dispatcher.
- `scripts/standalone_release.py` — fixed-root install transaction, launchers,
  native manifests, skills, and detected consumer integration.
- `scripts/test_standalone_release.py` — isolated-home and convergence tests.

## Verification

```bash
uv run pytest scripts/test_standalone_release.py
```

The focused tests use disposable `HOME` and `XDG_DATA_HOME` values. They prove
fixed-root replacement, stable launcher and native-manifest targets, skill links,
idempotence, stale-file removal, the exact `computer-use`/`browser` marketplace
inventory, the Browser client adapter and IAB routing skill, and detected
Codex/OpenClaw calls without a Browser trust-hash environment contract.

Clean-artifact validation extracts
`dist/sky-cua-linux-x64-glibc.tar.gz` into a disposable directory and invokes:

```bash
python3 install.py install
```

with isolated user directories. Producer validation must never install onto the
live user environment.

## Known limitations

- The packaged standalone target is currently Linux x86-64 glibc.
- The installer does not install system packages or privileged input helpers.
- Consumer repositories own their final installed/live acceptance after they
  adopt this fixed-root contract.

## Related

- [`docs/features/release-package.md`](release-package.md)
- [`docs/operations/plugin-release.md`](../operations/plugin-release.md)
- [`docs/runtime/mcp-boundary.md`](../runtime/mcp-boundary.md)
