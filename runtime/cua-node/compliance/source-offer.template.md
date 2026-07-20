# Corresponding Source Offer: <component name> <version>

Status: `OPEN`

This record is a release artifact, not a claim that the corresponding-source
obligation has been satisfied. Complete every placeholder and attach it to the
exact binary/archive identified by the compliance policy.

## Covered artifact

- Component: `<component id>`
- Version/revision: `<exact version or revision>`
- Target: `linux-x64-glibc`
- Binary/archive URI: `<canonical URI>`
- Binary/archive SHA-256: `<sha256>`
- SHA-256 scope: `<archive, file, or deterministic tree manifest>`

## Corresponding source

- Source URI or archive: `<canonical corresponding-source URI>`
- Source revision: `<exact commit/tag/release>`
- Source SHA-256: `<sha256>`
- Source scope: `<archive or deterministic source tree manifest>`
- Availability window: `<offer validity period>`
- Delivery channel/contact: `<source offer contact or URL>`

## Build and composition record

- Build configuration URI: `<repository-relative record>`
- Toolchain and platform: `<compiler, linker, libc, and host details>`
- Configure flags: `<exact flags>`
- Patches: `<none or exact patch list and hashes>`
- Static/shared composition: `<map shipped objects/libraries to source inputs>`
- Reproduction command: `<deterministic command or documented reason it cannot be reproduced>`

## License and notices

- Declared license: `<SPDX expression>`
- License text path: `<planned notice path>`
- Component notice paths: `<one path per bundled dependency/codec>`
- Notice completeness review: `<reviewer/date/evidence reference>`

## Sign-off

- Prepared by: `<name or automation identity>`
- Reviewed by: `<name>`
- Evidence links: `<links to immutable records>`
- Gate closed in policy: `<commit or change reference>`
