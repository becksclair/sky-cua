# sky-cua-service Guide

## Package Identity

`sky-cua-service` is the long-lived daemon behind the MCP client.
It owns IPC serving, request dispatch, snapshot storage, approval/session startup, and backend lifetime.

## Setup & Run

```bash
cargo test -p sky-cua-service
cargo clippy -p sky-cua-service --all-targets
cargo run -p sky-cua-service -- daemon
```

## Patterns & Conventions

- Keep daemon request handling in `src/daemon.rs`.
- Keep Unix-socket server behavior in `src/ipc_server.rs`.
- Keep snapshot cache behavior in `src/snapshot_manager.rs`.
- Keep approval/token startup effects explicit through `src/approval_store.rs`.
- Convert backend errors through `src/diagnostics.rs` rather than formatting ad hoc response text.
- DO: Keep per-client failures from killing the server, following the resilient loop in `src/ipc_server.rs`.
- DO: Resolve service paths through `sky-cua-platform::paths`; do not duplicate fallback order.
- DO: Store and resolve snapshots through `SnapshotManager` so action calls can target prior state.
- DON'T: Chmod operator-supplied `SKY_CUA_SERVICE_SOCKET_PATH` parent directories.
- DON'T: Smuggle initialization side effects into unused stored fields.

## Touch Points / Key Files

- Entrypoint and module wiring: `src/main.rs`
- Daemon request dispatch: `src/daemon.rs`
- IPC server: `src/ipc_server.rs`
- Snapshot storage: `src/snapshot_manager.rs`
- Approval store setup: `src/approval_store.rs`
- Action wrapper: `src/action_router.rs`

## JIT Index Hints

- Find service requests: `rg -n "ServiceRequest|ServiceResponse|match request" src`
- Find socket behavior: `rg -n "UnixListener|accept|SERVICE_SOCKET_PATH|chmod|permissions" src`
- Find snapshot handling: `rg -n "SnapshotManager|snapshot_id|get_app_state|ActionRequest" src`
- Find error conversion: `rg -n "error_response|BackendError|diagnostic" src`

## Common Gotchas

- The daemon must keep serving after malformed client requests or transient accept failures.
- Socket path override behavior is part of the operator contract.
- Do not make cleanup read child process pipes before the child can exit; Python harnesses document that trap too.

## Pre-PR Checks

```bash
cargo test -p sky-cua-service && cargo clippy -p sky-cua-service --all-targets
```
