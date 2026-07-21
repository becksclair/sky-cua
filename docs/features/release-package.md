# Complete release package

## Status

Shipped. The immutable complete-release builder and release-root activation
controller are the normal distribution and machine-install path.
`scripts/package.py` and its generic bundle installer remain compatibility
artifacts only; they do not perform complete activation.

## Summary

`scripts/build_complete_release.py` binds the core runtime, CUA Node runtime,
Browser JavaScript, Codex compatibility resources, documentation, compliance,
and installer into one content-addressed release. It prints the full
`release_id`, `manifest_sha256`, `release_root`, and `fat_archive` paths.

The release-root `install.py install` is the sole normal activation command.
It verifies and promotes the generation, installs native-messaging manifests,
projects compatibility commands through the stable `current` link, drains
obsolete sky-cua runtimes, writes `activation-receipt.json`, and prunes only
after those producer-owned surfaces succeed. A failure restores the prior
generation, manifests, links, and receipt.

## Build and inspect

Build from the repository root after the core plugin and CUA Node component
inputs are current:

```bash
python3 scripts/build_complete_release.py
```

Inspect the JSON-selected release root and archive. The release root must
contain `RELEASE.json`, `SHA256SUMS`, `install.py`, the installer component,
and every profile-bound component. The archive name is:

```text
dist/complete-release/sky-cua-<release-id>-linux-x64-glibc.tar.gz
```

Useful builder options are `--output-root`, `--core-source`,
`--cua-node-component`, `--producer-commit`, and `--no-fat-archive`.

## Clean-machine activation

Copy the reported fat archive to the target and use the reported full manifest
hash:

```bash
tar xzf sky-cua-<release-id>-linux-x64-glibc.tar.gz
cd sky-cua-<release-id>
python3 install.py verify --manifest-sha256 <manifest-sha256>
python3 install.py install --manifest-sha256 <manifest-sha256>
python3 install.py verify-activation --manifest-sha256 <manifest-sha256>
```

The default standalone store is `~/.local/share/sky-cua`. Known command names
in `~/.local/bin` and the compatibility `store/bin` directory are symlinks
through `current`; they are not independent mutable copies. Native-host
manifests point to the exact installed generation. Unknown entries in either
bin directory are preserved.

Public release-root operations:

- `verify` checks the extracted immutable release without mutating the machine.
- `install` performs the complete activation transaction.
- `ensure` verifies artifact-derived state and performs the same activation
  transaction only when repair is required.
- `verify-activation` checks the current generation, exact native manifests,
  stable links, receipt, and current-user live sky-cua process paths without
  mutation.
- `rollback` activates the retained prior generation and reprojects consumers.

`scripts/release_generation.py install` is an internal promotion primitive and
requires `--internal-generation-only`; normal workflows must not invoke it.

## Codex Desktop integration

Codex Desktop packages a verified complete release as an immutable fallback.
Its user-run Linux installer invokes the packaged release-root `install`
operation before synchronizing Browser Use resources. Its launcher invokes
`ensure`, then resolves runtime paths through the exact standalone active
generation. Installed verification checks activation before and after
consumer acceptance, plus packaged resources, cache projections, skills, and
trusted Browser client identity.

Replacing Electron's ASAR still requires relaunching Electron. No second
generic sky-cua install command is required.

## Source paths

- `scripts/build_complete_release.py` — complete immutable release builder.
- `scripts/complete_release_cli.py` — checkout-free release-root dispatcher.
- `scripts/install_complete_release.py` — complete activation transaction.
- `scripts/_release_activation.py` — receipt, stable links, process proof, and
  artifact-derived activation verification.
- `scripts/release_generation.py` — internal generation store and release
  integrity verification.
- `scripts/_native_messaging_install.py` — exact browser native-host manifest
  projection and rollback.

## Verification

```bash
uv run pytest \
  scripts/test_release_activation.py \
  scripts/test_install_complete_release.py \
  scripts/test_release_generation.py \
  scripts/test_native_messaging_install.py \
  scripts/test_build_complete_release.py
```

The tests cover idempotent ensure, receipt and manifest skew, mutable-copy
retirement, rollback, deferred pruning, obsolete/deleted process detection,
the internal raw-promotion guard, and the checkout-free release controller.

## Related

- [`docs/features/complete-cua-stack-ownership.md`](complete-cua-stack-ownership.md)
- [`docs/operations/plugin-release.md`](../operations/plugin-release.md)
- [`docs/runtime/mcp-boundary.md`](../runtime/mcp-boundary.md)
