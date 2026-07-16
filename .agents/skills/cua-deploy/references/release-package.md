# Release package details

Load this reference only when building or installing a self-contained release
package for a clean target.

Build the tarball under `dist/release/` with `scripts/package.py`. Inspect its
name, archive contents, bundled runtime, and packaged `install.py`. A target
with no checkout or toolchain extracts the archive and runs:

```bash
tar xzf sky-cua-<version>-<platform>.tar.gz
cd sky-cua-<version>
python3 install.py
```

This is bundle mode: the target uses the existing bundled runtime, does not
run Cargo, and materializes the computer-use compatibility plugin from the
bundled preflight. Release packaging does not deploy the local runtime or sync
global skill links; those are separate, explicitly requested lanes.

For exact package flags and installer options, load
`command-and-flag-catalog.md`.
