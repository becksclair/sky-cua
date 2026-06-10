# sky-cua-service Guide

`sky-cua-service` is the long-lived daemon behind the MCP client: IPC
serving, request dispatch, snapshot storage, approval/session startup, and
backend lifetime. Run locally with `cargo run -p sky-cua-service -- daemon`.

## Layout

- `src/daemon.rs` — request dispatch; `src/ipc_server.rs` — Unix-socket
  server; `src/snapshot_manager.rs` — snapshot cache; `src/approval_store.rs`
  — approval/token startup effects; `src/action_router.rs` — action wrapper.
- Backend errors convert through `src/diagnostics.rs`, never ad hoc response
  text. Service paths resolve through `sky-cua-platform::paths`; do not
  duplicate fallback order.

## Conventions and gotchas

- The daemon must keep serving after malformed client requests or transient
  accept failures (see the resilient loop in `src/ipc_server.rs`).
- Store and resolve snapshots through `SnapshotManager` so action calls can
  target prior state.
- Socket path override (`SKY_CUA_SERVICE_SOCKET_PATH`) is part of the
  operator contract; never chmod operator-supplied parent directories.
- Do not smuggle initialization side effects into unused stored fields.
- Do not make cleanup read child process pipes before the child can exit.
