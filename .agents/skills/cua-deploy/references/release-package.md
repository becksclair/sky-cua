# Release package details

Load this reference only when building or installing a self-contained release
package for a clean target.

Build the immutable release under `dist/complete-release/` with
`scripts/build_complete_release.py`. Capture its JSON result, especially
`release_root`, `release_id`, `manifest_sha256`, and `fat_archive`. Verify the
release-root `install.py`, `RELEASE.json`, component archives, and fat archive
before handoff. A target with no checkout extracts the archive and runs:

```bash
tar xzf sky-cua-<release-id>-linux-x64-glibc.tar.gz
cd sky-cua-<release-id>
python3 install.py verify --manifest-sha256 <manifest-sha256>
python3 install.py install --manifest-sha256 <manifest-sha256>
python3 install.py verify-activation --manifest-sha256 <manifest-sha256>
```

`install` is the single normal activation transaction. It promotes the
immutable generation, installs native-messaging manifests, replaces known
mutable compatibility copies with stable links through `current`, drains
obsolete runtime processes, writes `activation-receipt.json`, and prunes only
after those steps succeed. `ensure` performs the same repair only when
artifact-derived verification fails. `verify-activation` is read-only.

Never substitute `scripts/release_generation.py install`; it is an internal
generation-store primitive and intentionally refuses normal operator use.
`scripts/package.py` and its generic top-level installer are legacy
compatibility packaging, not the complete release workflow. Release packaging
does not deploy the local development runtime or sync global skill links.

For exact package flags and installer options, load
`command-and-flag-catalog.md`.
