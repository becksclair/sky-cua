# Canvas native 0.1.91 notices and composition evidence

Artifacts: `@napi-rs/canvas@0.1.91` and
`@napi-rs/canvas-linux-x64-gnu@0.1.91`.

Canvas is MIT licensed. The pinned license text is:
https://raw.githubusercontent.com/Brooooooklyn/canvas/6661e25b9520bfc2df1e4c9820717fee5dd304fd/LICENSE

The addon statically contains Skia from submodule commit
`ee20d565acb08dece4a32e3f209cdd41119015ca`. Skia is BSD-3-Clause licensed;
the pinned license is:
https://raw.githubusercontent.com/google/skia/ee20d565acb08dece4a32e3f209cdd41119015ca/LICENSE

The pinned Skia build enables bundled expat, FreeType, HarfBuzz, ICU,
libjpeg-turbo, JPEG XL decode, libpng, libwebp, Wuffs, and zlib. Their exact
Skia revisions are recorded in the pinned `DEPS` file:
https://raw.githubusercontent.com/google/skia/ee20d565acb08dece4a32e3f209cdd41119015ca/DEPS

Rust `libavif`/`libavif-sys` statically supply AVIF support with AOM; the binary
identifies AOM 3.11.0. AOM is BSD-2-Clause with the Alliance for Open Media
Patent License 1.0:
https://aomedia.googlesource.com/aom/+/refs/tags/v3.11.0/LICENSE
https://aomedia.org/license/patent-license/

The locked build resolves the following direct runtime Rust crates:
`anyhow`, `base64-simd`, `cssparser`, `cssparser-color`, `gif`, `imagesize`,
`infer`, `libavif`, `libavif-sys`, `mimalloc-safe`, `napi`, `napi-derive`,
`nom`, `num_cpus`, `regex`, `rgb`, `serde`, `serde_derive`, `serde_json`, and
`thiserror`; build crates are `cc` and `napi-build`. Exact versions, checksums,
features, dependencies, and license declarations for all 84 target-resolved
packages are in `../canvas-0.1.91-rust-crates.json`; their collected license
texts are in `canvas-0.1.91-RUST-NOTICES.md`.

The pinned `yarn.lock` inventories JavaScript build/test tooling. Those packages
are not present in the three-file native npm package and are not represented as
runtime composition. Exact source, workflow, archive list, toolchain evidence,
and composition proof are in `../canvas-0.1.91-evidence.json`. Exact native
license and patent texts are in `canvas-0.1.91-NATIVE-NOTICES.md`; the locked
build, linker-map, smoke, and reproducibility record is
`../canvas-0.1.91-build-record.json`.
