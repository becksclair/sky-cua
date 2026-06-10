# sky-cua-platform Guide

`sky-cua-platform` owns the platform-neutral contracts shared by the client,
daemon, and backends. It stays small, serializable, and runtime-agnostic:
never pull in Linux crates, ashpd, atspi, GStreamer, KWin, or X11
dependencies here.

## Layout

- Serde-facing structs/enums: `src/model.rs` and focused `src/model/`
  submodules — this is the structured MCP/service contract. Preserve
  `sky_cua_platform::model::*` compatibility with re-exports when splitting.
- Trait seams for platform backends: `src/backend.rs`.
- User-facing backend failures: `src/diagnostics.rs`, mapped to
  `DiagnosticEntry`.
- Socket/state/token path resolution: `src/paths.rs` — the one source of
  truth.
- App instruction key normalization: `src/app_instructions.rs`; snapshot
  IDs: `src/snapshot.rs`.

## Conventions and gotchas

- Public enums use `#[serde(rename_all = "snake_case")]`; optional fields
  use `skip_serializing_if` so JSON stays lean. Do not encode operator prose
  as the only source of truth — add structured fields or diagnostics.
- Changes here ripple through all crates and Python transcript/schema
  validation. Do not remove or rename serialized fields without checking
  `scripts/` and existing artifacts.
- `SERVICE_SOCKET_PATH_ENV` is an operator seam; preserve override behavior.
