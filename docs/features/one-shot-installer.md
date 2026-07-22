# Fixed-root standalone installer

## Status

Shipped for Linux x86-64 glibc. Last verified against the standalone producer
implementation on 2026-07-22.

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
  `sky-cua-overlay-host`, `node`, `node_repl`, and `sky-cua-chrome-host`;
- Chrome, Chromium, Brave, and Brave Origin native-messaging manifests;
- `computer-use`, `browser-use`, and `phone-use` skill links for detected
  agent homes;
- native Codex compatibility plugins when Codex is detected;
- the global OpenClaw `node_repl` registration when OpenClaw is detected.

Consumer configuration points at stable paths under the fixed root. It does not
pin an artifact hash or trust a Browser client by hash.

## Behavior

From a checkout, `install` runs the same durable build used by
`python3 install.py build`; compilation and assembly stay under `target/`,
`out/components/`, and `dist/`. From an extracted archive, the packaged payload
is already complete and no source build occurs.

The installer validates the payload, copies it into a sibling staging directory,
and recoverably replaces the fixed destination. The sibling stage is only an
install transaction; it is not a build directory and does not discard compiler
caches. A second install converges to the same tree. Replacing the payload also
removes files that existed only in the previous install, so stale generation
contents cannot accumulate.

The native-host manifests target the stable `~/.local/bin/sky-cua-chrome-host`
launcher. The installed standalone payload carries exactly one Chrome extension:
the latest version selected during build.

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
idempotence, stale-file removal, and detected Codex/OpenClaw calls without a
Browser trust-hash environment contract.

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
