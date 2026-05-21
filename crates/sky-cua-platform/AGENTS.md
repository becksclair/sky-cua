# sky-cua-platform Guide

## Package Identity

`sky-cua-platform` owns platform-neutral contracts shared by the client, daemon, and Linux backend.
This crate should stay small, serializable, and runtime-agnostic.

## Setup & Run

```bash
cargo test -p sky-cua-platform
cargo fmt --check
cargo clippy -p sky-cua-platform --all-targets
```

## Patterns & Conventions

- Put serde-facing structs/enums in `src/model.rs` or focused `src/model/` submodules; this is the structured MCP/service contract. Preserve `sky_cua_platform::model::*` compatibility with re-exports when splitting.
- Keep trait seams in `src/backend.rs` so platform backends can implement behavior without leaking details.
- Put user-facing backend failures in `src/diagnostics.rs`, then map them to `DiagnosticEntry`.
- Keep path resolution in `src/paths.rs`; this is the one source for socket/state/token paths.
- Use `#[serde(rename_all = "snake_case")]` for public enums like `ActionName` and `CaptureBackendKind`.
- Use `#[serde(skip_serializing_if = "Option::is_none")]` for optional fields that should not bloat JSON.
- DO: Mirror `AppSelector` and `CaptureInfo` style in `src/model.rs` for new optional structured fields.
- DO: Add tests beside parsing/path helpers like `src/app_instructions.rs`.
- DON'T: Pull in Linux crates, ashpd, atspi, GStreamer, KWin, or X11 dependencies here.
- DON'T: Encode operator prose as the only source of truth; add structured fields or diagnostics.

## Touch Points / Key Files

- Shared model: `src/model.rs` and focused `src/model/` submodules
- Backend traits: `src/backend.rs`
- Diagnostic builder: `src/diagnostics.rs`
- App instruction key normalization: `src/app_instructions.rs`
- Socket/state paths: `src/paths.rs`
- Snapshot IDs: `src/snapshot.rs`

## JIT Index Hints

- Find public model fields: `rg -n "pub struct|pub enum" src/model.rs`
- Find path contracts: `rg -n "XDG|HOME|SERVICE_SOCKET_PATH|portal_tokens" src/paths.rs`
- Find app-guidance key logic: `rg -n "normalize|focused_app_instruction_keys|index_path" src/app_instructions.rs`
- Find diagnostics: `rg -n "BackendError|BackendErrorCode|DiagnosticBuilder" src`

## Common Gotchas

- Changes here ripple through all crates and Python transcript/schema validation.
- Do not remove or rename serialized fields without checking scripts and existing artifacts.
- `SERVICE_SOCKET_PATH_ENV` is an operator seam; preserve override behavior.

## Pre-PR Checks

```bash
cargo test -p sky-cua-platform && cargo clippy -p sky-cua-platform --all-targets
```
