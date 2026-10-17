# CUA Node pinned dependency bumps

The packages overlaid onto the `cua_node` migration seed (`sharp`,
`@img/sharp-linux-x64`, `@img/sharp-libvips-linux-x64`, `semver`, `acorn`,
`acorn-walk`) are pinned by content hash and by compliance evidence, not only
by version. A Dependabot PR for them changes `runtime/cua-node/package.json`
and `bun.lock` only. The `cua-node-pinned` Dependabot groups make the set
arrive in one PR, because `sharp` and its `@img` packages must move together.

## Sources of truth

- `runtime/cua-node/production/package.json`: the declared production pins.
  Harnesses, the model-documentation inventory, and the compliance tests
  derive versions from it.
- `runtime/cua-node/prepared-imports.lock.json`: one reviewed record per
  overlaid package with its version, license, npm integrity, raw installed
  tree hash (`source_tree`, checked by every assembly), and assembled tree
  hash (`assembled_tree`, checked by `tools/source-lock-truth.ts` against the
  lock templates).
- Compliance evidence under `runtime/cua-node/compliance/` stays
  hand-maintained.

## Finishing a bump PR

1. Check out the Dependabot branch with the cached migration seed available.
2. Run `uv run python scripts/refresh_cua_node_pins.py`. It runs a frozen Bun
   install, re-pins `prepared-imports.lock.json` from `bun.lock` and the
   installed trees, syncs `production/package.json` and
   `production/package-lock.json`, rebuilds the host bundle, runs a
   development assembly, and splices the assembled hashes into
   `runtime-lock.json`, `native-assets.lock.json`, and the prepared lock.
3. Resolve every `needs human update:` line it prints: policy and provenance
   versions, re-collected version-named notices, `toolbox.md` prose, and for
   a libvips bump a new `sharp-libvips-<version>-source-offer.json` and NOTICE
   researched from the upstream attestation and build recipe. A libvips bump
   cannot be assembled until that evidence exists, so rerun the script after
   adding it.
4. Rerun the script until it exits 0 (it then regenerates the SBOMs), run the
   `runtime/cua-node` and Python suites, and commit.

Never run the refresh script in CI: CI must only compare these pins with
independently produced artifacts (Bun's install-time integrity check, the
assembly tree re-hash, and generated lock records).
