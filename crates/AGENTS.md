# Rust Workspace Guide

## Package Identity

`crates/` contains the Rust 2024 workspace for the plugin runtime.
The workspace splits stable contracts, Linux desktop integration, the daemon, and the MCP client into separate crates.

## Setup & Run

```bash
cargo build
cargo test
cargo fmt --check
cargo clippy --workspace --all-targets
cargo test -p sky-cua-platform
cargo test -p sky-cua-linux
cargo test -p sky-cua-service
cargo test -p sky-cua-client
```

## Patterns & Conventions

- Keep shared request/response/data contracts in `sky-cua-platform`, not duplicated in client/service/backend crates.
- Use workspace dependencies from root `Cargo.toml`; prefer `foo.workspace = true` in crate manifests.
- Keep platform-neutral traits in `crates/sky-cua-platform/src/backend.rs`.
- Keep Linux-specific portal, AT-SPI, KWin, and X11 logic in `crates/sky-cua-linux/src/**`.
- Keep daemon state and IPC serving in `crates/sky-cua-service/src/**`.
- Keep MCP JSON-RPC behavior and tool text/structured output in `crates/sky-cua-client/src/mcp_server.rs`.
- DO: Add model fields beside existing serde contracts in `crates/sky-cua-platform/src/model.rs`.
- DO: Add backend diagnostics using `DiagnosticEntry`/`DiagnosticBuilder` patterns from `crates/sky-cua-platform/src/diagnostics.rs`.
- DO: Put environment-specific action routing in `crates/sky-cua-linux/src/backend.rs`.
- DO: Keep socket/path rules in `crates/sky-cua-platform/src/paths.rs`.
- DON'T: Add Linux-only types to `crates/sky-cua-platform/src/model.rs` unless they are part of the public structured contract.
- DON'T: Make the client infer backend state from text; structured fields should carry the truth.
- DON'T: Treat title-only window matches as identity proof; see X11/KWin matching in `crates/sky-cua-linux/src/backend.rs`.

## Touch Points / Key Files

- Workspace members and shared deps: `Cargo.toml`
- Public model and serde contracts: `crates/sky-cua-platform/src/model.rs`
- Backend trait boundary: `crates/sky-cua-platform/src/backend.rs`
- Linux backend orchestration: `crates/sky-cua-linux/src/backend.rs`
- Daemon request handling: `crates/sky-cua-service/src/daemon.rs`
- MCP tool surface: `crates/sky-cua-client/src/mcp_server.rs`

## JIT Index Hints

- Find request/response changes: `rg -n "ServiceRequest|ServiceResponse|ActionRequest" crates`
- Find capture backend handling: `rg -n "CaptureBackendKind|image_backend|screenshot_path" crates`
- Find physical input paths: `rg -n "notify_|XTest|xdotool|MouseButton" crates/sky-cua-linux/src`
- Find portal lifecycle diagnostics: `rg -n "PortalSession|PortalApprovalPending|RemoteDesktop" crates`
- Find window matching: `rg -n "score|title|focused|desktop_file_id|KWinWindowInfo|X11WindowInfo" crates/sky-cua-linux/src`

## Common Gotchas

- `capture.backend` is the selected primary lane; `capture.image_backend` is the lane that actually produced the image.
- Portal and X11 action success means transport success, not visual proof. Live smokes must re-check state.
- Some tests mutate environment variables; keep serialized patterns like `serial_test` in `portal/pipewire.rs`.

## Pre-PR Checks

```bash
cargo fmt --check && cargo clippy --workspace --all-targets && cargo test
```
