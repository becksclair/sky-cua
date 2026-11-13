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
- macOS Accessibility, ScreenCaptureKit, CoreGraphics window-list, and
  CGEvent logic: `crates/sky-cua-macos/src/**` ->
  [sky-cua-macos/AGENTS.md](sky-cua-macos/AGENTS.md).
- Desktop agent-cursor overlay host (platform-neutral surface host with
  Wayland layer-shell and macOS panel platforms, the wgpu renderer, and the
  vehicle-steering motion driver): `crates/sky-cua-overlay-host/` ->
  [sky-cua-overlay-host/AGENTS.md](sky-cua-overlay-host/AGENTS.md).
- Chrome native-messaging host (extension to daemon bridge):
  `crates/sky-cua-chrome-host/`.
- COSMIC toplevel helper (Wayland `cosmic-protocols` client for window
  discovery, focus, and activation on COSMIC, plus the cursor bridge):
  `crates/sky-cua-cosmic-helper/`.
- Root uinput helper (systemd system service; keyboard and absolute-pointer
  injection plus pointer observation over `/run/sky-cua/input-helper.sock`):
  `crates/sky-cua-input-helper/`.
- Daemon state and IPC serving: `crates/sky-cua-service/src/**` (request
  dispatch in `src/daemon/mod.rs`; per-surface handlers live beside it in
  `src/daemon/`).
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
  `serial_test` in `src/portal/pipewire.rs`.
