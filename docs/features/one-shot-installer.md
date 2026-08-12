# Fixed-root standalone installer

## Status

Shipped for Linux x86-64 glibc. Last verified against the standalone producer
implementation on 2026-08-01.

## Summary

`python3 install.py install` installs one complete sky-cua payload at one
XDG-aware physical user-data root. `~/.local/share/sky-cua` is the stable public
rendezvous: it is the payload directory under the default layout and a symlink
to the physical tree under a custom `XDG_DATA_HOME`. The same command works from
a repository checkout and from the extracted standalone archive; checkout mode
first refreshes the durable build outputs.

## Contract surface

The public CLI contains exactly two commands:

```bash
python3 install.py build
python3 install.py install
```

`install` accepts no generation, rollback, install-root, or manifest-hash
arguments. Its physical destination is always:

```text
${XDG_DATA_HOME:-~/.local/share}/sky-cua
```

Every agent-facing path uses this stable ABI regardless of physical storage:

```text
~/.local/share/sky-cua
```

The custom-XDG case uses one relative rendezvous symlink; it does not copy the
payload into a second tree.

The installer projects these stable user-facing surfaces:

- launchers in `~/.local/bin` for `sky-cua-client`, `sky-cua-service`,
  `sky-cua-overlay-host`, `node_repl`, and `sky-cua-chrome-host`;
- Chrome, Chromium, Brave, and Brave Origin native-messaging manifests;
- `computer-use`, `browser-use`, and `phone-use` skill links for detected
  agent homes. Shared `~/.agents/skills` links use relative targets such as
  `../../.local/share/sky-cua/skills/computer-use`;
- fixed-root Codex compatibility plugins `computer-use@openai-bundled` and
  `browser@openai-bundled`, plus native install requests when Codex is detected;
- the global OpenClaw `node_repl` registration when OpenClaw is detected;
- no-prompt OpenClaw Codex policy: the native app-server is set to `yolo`,
  `approvalPolicy: never`, and `sandbox: danger-full-access`, while every
  existing agent Codex home is converged to `approval_policy = "never"` and
  `sandbox_mode = "danger-full-access"`.
- no-prompt Hermes policy when an existing Hermes configuration is detected,
  covering command approvals, write approvals, delegated workers, hook
  registration, MCP reload, and destructive slash-command confirmation.

Consumer configuration points at stable paths under the public rendezvous. It
does not expose custom XDG storage, pin an artifact hash, or trust a Browser
client by hash.

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

The installer validates the payload, validates any distinct public rendezvous,
removes the physical destination, and copies the payload directly into its
place before projecting integrations. Before replacement it stops current-user
sky-cua runtime processes executing from the physical root or a prior managed
rendezvous target; MCP hosts then respawn from the stable public paths. A second
install converges to the same tree and byte-identical shared skill-link text.
Replacing the payload also removes files that existed only in the previous
physical install, so stale contents cannot accumulate. An unrelated object at a
custom-XDG public rendezvous is refused rather than overwritten.

The native-host manifests target the stable `~/.local/bin/sky-cua-chrome-host`
launcher. The installed standalone payload carries exactly one Chrome extension:
the latest version selected during build.

The OpenClaw policy projection is intentionally convenience-first and applies
to the whole native Codex runtime, not only Computer Use calls. This is the
installer contract: an OpenClaw deployment detected during installation must
not pause for Codex command, file, permission, Browser, desktop-input, or phone
approval prompts. OpenClaw's global app-server policy covers agents created
after installation; existing agent `codex-home/config.toml` files are also
updated so already-created runtimes converge immediately. Unrelated TOML and
OpenClaw configuration are preserved.

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
physical-root replacement, public-rendezvous behavior, stable launcher and
native-manifest targets, canonical relative skill links, migration of managed
absolute links, idempotence, stale-file removal, durable surface convergence,
unmanaged-entry preservation, the exact `computer-use`/`browser` marketplace
inventory, the Browser client adapter and IAB routing skill, detected
Codex/OpenClaw calls without a Browser trust-hash environment contract, and
idempotent no-prompt OpenClaw policy convergence across multiple agent homes.

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
