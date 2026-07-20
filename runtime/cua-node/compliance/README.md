# Linux `cua_node` compliance scaffold

This directory is the compliance/provenance seam for the `linux-x64-glibc`
`cua_node` runtime. `../runtime-lock.json` and `../native-assets.lock.json`
bind the assembled production tree; `../tools/source-lock-truth.ts` verifies
those locks against the canonical first-party source build, the production
dependency graph, and this evidence before release assembly.

The machine-readable source of truth is [`policy.json`](./policy.json). It
contains every planned component, its settled disposition, notice references,
canonical source, external runtime requirements, and any open blocking gate.
[`provenance.json`](./provenance.json) records exact artifact evidence and what
still needs to be proven.

## Settled dispositions

- `routine-notice-clearance`: Node 24.14.0, npm 11.9.0, Corepack 0.34.6,
  Playwright 1.57.0 and core, PDF.js 5.4.624 plus CMaps/fonts, Tesseract.js
  and core 7.0.0, pixelmatch 7.1.0, sharp and its Linux addon, canvas's npm
  package, and first-party `@heliasar/sky-cua`.
- `provenance-only-gate`: tessdata `eng` and `osd`.
- `canvas-linux-x64-gnu` 0.1.91 is cleared as routine notice work by the locked,
  self-built Linux x64 glibc artifact and its exact composition evidence.

The policy is currently `clear` with no open release gate. sharp-libvips 1.2.4
is cleared by npm SLSA provenance plus an operational corresponding-source
record that pins commit `20b5e899954907a3039d6e3d4c200aaa0ec52c4c`, the Linux
build scripts, libvips 8.17.3, patches, 29 source components, and notices.
Tessdata `eng` and `osd` provenance is also resolved. Canvas is self-built from
the exact source and original Skia archives with a preserved Cargo.lock,
84-package Rust inventory, an LLD link map, full notices, bundled Node 24 raw
and prepared-package smokes, a GLIBC 2.28 ceiling, and a byte-identical second
clean build. The accepted final4/final5 builds preserve upstream release
defaults and do not use the earlier experimental deduplicated archives.
The Canvas JavaScript package declaration and shipped license are MIT; the
native lock separately binds the MIT package license plus the complete native,
Rust, and composition notices. PDF.js CMaps are BSD-3-Clause. The standard
fonts combine BSD-3-Clause Foxit data with OFL-1.1 Liberation fonts. Pixelmatch
7.1.0 and its ISC license are pinned in both the production graph and runtime
lock.

Upstream `cua_node` ships the Playwright JavaScript packages only. On Linux, a
supported system-installed Chromium-family executable is an external runtime
requirement. It is not redistributed and therefore is not a CycloneDX or SPDX
distribution component. Video recording and its media payload are outside this
runtime's scope.

`@heliasar/sky-cua` is represented as
`LicenseRef-Heliasar-Proprietary/UNLICENSED`. The SPDX document uses the valid
custom identifier `LicenseRef-Heliasar-Proprietary` and preserves the exact
project designation in `licenseComments` and extracted licensing information.

## Files and workflow

- [`notice-inventory.json`](./notice-inventory.json) records planned and
  collected notices. The collected sharp, Canvas, PDF data, and pixelmatch
  records live under [`notices/`](./notices/).
- [`sharp-libvips-1.2.4-source-offer.json`](./sharp-libvips-1.2.4-source-offer.json)
  is the operational corresponding-source/build/component record.
- [`canvas-0.1.91-evidence.json`](./canvas-0.1.91-evidence.json) inventories the
  source, Cargo/Yarn inputs, Skia archives/components, binary evidence, notices,
  and clearance result. The adjacent build record, source offer, Cargo.lock,
  crate inventory, and notice files preserve the complete rebuild evidence.
- [`provenance.template.json`](./provenance.template.json) is the per-component
  provenance record template.
- [`source-offer.template.md`](./source-offer.template.md) and
  [`source-offer-record.template.json`](./source-offer-record.template.json)
  are the corresponding-source/source-offer templates for LGPL and other
  evidence-gated components.
- [`generate.ts`](./generate.ts) deterministically generates
  [`sbom.cdx.json`](./sbom.cdx.json) and [`sbom.spdx.json`](./sbom.spdx.json).
  It uses a fixed timestamp, stable component sorting, and no network access.
- [`policy.schema.json`](./policy.schema.json) defines the policy contract;
  [`compliance.test.ts`](./compliance.test.ts) checks the cross-file invariants
  that the JSON schema alone cannot express.

From the repository root:

```sh
bun runtime/cua-node/compliance/generate.ts
bun test runtime/cua-node/compliance/compliance.test.ts
bun --cwd runtime/cua-node run verify:source-locks
bunx tsc -p runtime/cua-node/compliance/tsconfig.json --noEmit
```

The generated SBOMs reflect the current policy and immutable evidence records.
