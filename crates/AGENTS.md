# Rust Workspace Guide

`crates/` is the Rust 2024 workspace for the plugin runtime, split into
stable contracts, platform backends, the daemon, and the MCP client.

## Layout

- Shared request/response/data contracts: `sky-cua-platform` (never
  duplicated in client/service/backend crates); platform-neutral traits in
  `crates/sky-cua-platform/src/backend.rs`.
- Shared model-facing screenshot preparation: `crates/sky-cua-capture/src/lib.rs`
  (downscale-never-upscale, WebP-default encoding, format/quality/bounds env
  resolution, and the `logical_to_pixel_scale` derivation). Both desktop
  backends call it so the model image stays identical; the heavy `image`/`webp`
  encoder deps live here, not in `sky-cua-platform`.
- Linux portal, AT-SPI, KWin, and X11 logic: `crates/sky-cua-linux/src/**`.
- Desktop agent-cursor overlay host (Wayland layer-shell + wgpu renderer +
  the vehicle-steering motion driver): `crates/sky-cua-overlay-host/` ->
  [sky-cua-overlay-host/AGENTS.md](sky-cua-overlay-host/AGENTS.md).
- Daemon state and IPC serving: `crates/sky-cua-service/src/**` (request
  dispatch in `src/daemon.rs`).
- MCP JSON-RPC behavior and tool text/structured output:
  `crates/sky-cua-client/src/mcp_server.rs`.
- Workspace members and shared deps: root `Cargo.toml`; crate manifests use
  `foo.workspace = true`.

## Conventions

- Diagnostics use the `DiagnosticEntry`/`DiagnosticBuilder` patterns from
  `crates/sky-cua-platform/src/diagnostics.rs`; socket/path rules live in
  `crates/sky-cua-platform/src/paths.rs`.
- The client must not infer backend state from text; structured fields carry
  the truth.
- Never treat title-only window matches as identity proof; see the scoring
  in `crates/sky-cua-linux/src/app_match.rs`.

## Gotchas

- `capture.backend` is the selected primary lane; `capture.image_backend` is
  the lane that actually produced the image.
- Portal and X11 action success means transport success, not visual proof.
  Live smokes must re-check state.
- Some tests mutate environment variables; keep serialized patterns like
  `serial_test` in `portal/pipewire.rs`.
