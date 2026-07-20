# Unified browser bridge control-plane decision summary

## Context

The browser path had three competing lifecycle/identity mechanisms: extension
session ownership, native-host primary/heartbeat/ephemeral roles, and daemon
tab-to-socket affinity. Per-operation connections repeated discovery and could
not provide one queue, lease table, completion registry, or reliable
multi-caller attribution. The research question was: which topology can unify
Codex Browser Use and MCP hosts without moving browser policy into Codex or
changing the upstream extension?

This summary is dated 2026-07-19. Current source, the runtime protocol doc, and
the shipped feature doc are authority for implementation state. Source/test
evidence and installed live evidence are separated below.

## Investigation

### Source-confirmed findings

1. **Legacy topology was fragmented.** Ordinary operations opened short-lived
   native-host streams while heartbeat used a separate persistent connection.
   Native-host routing selected primary/heartbeat/ephemeral roles independently
   from daemon affinity. Evidence:
   `crates/sky-cua-service/src/browser/{executor,keepalive,probe,transport}.rs`,
   `crates/sky-cua-chrome-host/src/host.rs`.
2. **Caller identity and bridge identity cannot be one field.** Codex metadata,
   MCP client identity, native-host role, browser instance, tab ownership, and
   operation correlation have different lifetimes. Evidence:
   `crates/sky-cua-platform/src/model/{browser,browser_control}.rs`.
3. **MCP hosts can be normalized in sky-cua.** Connection lifetime,
   `initialize.clientInfo`, installer declarations, trusted Codex metadata, and
   tool-call correlation provide stable separate lanes for direct MCP,
   OpenClaw, OpenCode, and Pi. Evidence:
   `crates/sky-cua-client/src/mcp_server.rs`, `scripts/install_mcp_server.py`,
   `scripts/_openclaw_install.py`.
4. **Raw Codex compatibility is transport, not policy.** The implemented UDS
   accepts upstream framed JSON-RPC and normalizes it into the daemon. It does
   not expose the typed control protocol to Codex. Evidence:
   `crates/sky-cua-service/src/codex_browser_compat/`.
5. **The daemon is the only coherent scheduler owner.** The scheduler enforces
   tab FIFO, cross-tab overlap, bridge/daemon barriers, queue bounds,
   cancellation, fencing, and settlement without holding I/O locks. Evidence:
   `crates/sky-cua-service/src/browser/control_plane/{control,scheduler,group,lease,operation}.rs`.
6. **One actor per browser instance centralizes lifecycle.** Actor code owns
   host handshake, persistent connection, heartbeat, request IDs, tombstones,
   reconnect, peer identity, quarantine, events, and settlements. Evidence:
   `crates/sky-cua-service/src/browser/control_plane/bridge_actor/`.
7. **Status can expose bounded truth.** Structured
   `status(component="browser")` includes
   mode/generation/readiness, normalized clients, actors, scheduler/groups,
   recent operations, and a sequenced event window. Evidence:
   `crates/sky-cua-platform/src/model/browser_control.rs`,
   `crates/sky-cua-service/src/browser/control_plane/{integration,introspection}.rs`.
8. **Restart recovery is persistent but never authoritative.** Production
   persistent modes atomically write and load bounded authority-free recovery
   hints. Restart restores a fresh fence and suspended admission; unresolved
   mutations restore as `recovery_required`, with no operation replay. Evidence:
   `crates/sky-cua-service/src/browser/control_plane/{persistence,control,integration}.rs`.
9. **Connection-only IDs end at reconnect.** The actor generates a new browser
   ID for every connection-only/unavailable handshake and reports the previous
   browser lost, even if a host repeats its proposed value. Evidence:
   `crates/sky-cua-service/src/browser/control_plane/bridge_actor.rs`.
10. **Codex compatibility adapts `getInfo`.** The daemon emits `type="iab"`,
    maps trusted logical session to `codexSessionId`, and derives
    `codexAppBuildFlavor` from the same-UID peer's bounded environment value.
    Evidence: `crates/sky-cua-service/src/browser/control_plane/integration/codex.rs`,
    `crates/sky-cua-service/src/codex_browser_compat/connection.rs`.
11. **Event-driven continuations require bounded reentrancy.** Allowlisted
    Fetch continuations tied to an in-flight same-connection/tab parent bypass
    same-tab FIFO as correlated children; otherwise a parent waiting on
    `Fetch.requestPaused` deadlocks. Evidence:
    `crates/sky-cua-service/src/browser/control_plane/integration/codex.rs` and
    `crates/sky-cua-service/src/browser/control_plane/integration_tests.rs`.
12. **One target piece remains staged.** The typed v1 frame model has no
    dedicated listener. Usages of `BrowserControlClientFrame` remain model/test
    only; ordinary non-Codex callers use service IPC/MCP.

### Experiments recorded by the plan

- Stable canonical extension session/turn identity preserved ownership across
  same-session reconnect; caller `turnEnded`/`finalizeTabs` must not be
  forwarded as canonical lifecycle.
- The extension/native host accepted concurrent requests. Independent tabs
  overlapped; same-tab operations also overlapped unless the daemon serialized
  them, proving the need for tab lanes. Large screenshot frames caused
  structural head-of-line delay.
- Restart probes showed PID, process start ticks, and socket inode turnover.
  Those values can fence connections but cannot alone identify a surviving
  browser/profile or restore tab authority.
- Delayed-mutation probes showed client EOF, actor/native-host/daemon generation
  replacement, debugger reattach, timeout, and successful same-target
  diagnostics are not quiescence boundaries. A late mutation may still succeed.
- Codex source/installed inspection established nativePipe as a trusted opaque
  byte transport with generation-wide close cancellation and refresh reconnect.
  The selected adapter is therefore an endpoint selector to raw sky-cua UDS.

These experiments support implementation decisions but do not replace the
explicit live gates below.

### Installed live evidence

Installed acceptance on 2026-07-19 confirms exact Codex navigation, click,
typing, keyboard scroll, screenshots, tab cleanup, and reset/disconnect through
the control-plane path. Direct MCP performed the same ordinary workflow during
a Codex window. Three simultaneous installed clients declared as
OpenClaw/OpenCode/Pi opened separate tabs. Native-host PID replacement recovered
raw Codex dispatch while the daemon PID and both listener inodes remained
stable. Focused VM Codex Desktop, OpenCode, and Pi profiles passed with tool
evidence where applicable.

The focused closeout review found and fixed two final races: ambiguous claims
now retain an operation-scoped reservation through settlement, and MCP EOF is
ordered against already-started detached calls so a closed connection cannot
re-register its principal. Live introspection also exposed no-op lease-tick
event flooding; idle ticks no longer consume the bounded event ring.

### Inferences

- Persistent actors should remove per-operation scan/connect/hello overhead;
  final latency and RSS improvement still require measurement.
- Width two is a conservative initial bridge cap: it permits proven cross-tab
  overlap while limiting ordered-stream and screenshot pressure. It is not
  proven optimal.
- Same-UID installer-declared provenance is sufficient for attribution in the
  current trust model, but it is not authentication.
- Logical daemon groups are sufficient for atomic ownership. Mirroring them
  into Chrome's visible `tabGroups` UI would add side effects without helping
  the control contract.

### Remaining unknowns and external gates

- A dedicated typed external control UDS is explicitly deferred. The v1 frame
  model stays frozen, but no listener or framing work should be added until a
  real non-MCP adapter demonstrates demand.
- Public opaque tab/group/lease handle evolution without breaking current MCP
  results.
- The aggregate VM `all` profile cannot reach browser members until the
  unrelated Wayland-pointer visible-scroll acknowledgement is repaired.
- The VM deterministic `codex-cua` lane requires a refreshed Codex token; its
  2026-07-19 run exited before any tool call for that reason.
- Large-screenshot unrelated-tab tail latency, peak RSS, and whether widths
  four/eight are worthwhile remain optional performance measurements.

## Conclusion

The selected architecture is a daemon-owned control plane with independent
caller/logical/ownership/bridge/tab/group/operation identities, one canonical
persistent actor per browser instance, tab-lane scheduling, exclusive fenced
group leases, explicit completion certainty, and observable migration modes.
Codex remains a thin raw-transport adapter; MCP hosts remain ordinary sky-cua
clients with normalized provenance.

The original design output is durably represented by these twelve points:

1. current failure topology and one-daemon target topology;
2. exact identity separation and canonical extension actor;
3. raw Codex versus typed non-Codex ingress;
4. daemon-owned classification, fingerprints, dedupe, and generation fencing;
5. tab FIFO, cross-tab overlap, global barriers, fairness, and queue bounds;
6. group ownership, leases, fencing, membership revisions, and handoff;
7. cancellation, ambiguity, settlement, and no-replay rules;
8. daemon/client/bridge/tab/group/operation lifecycle behavior;
9. strict/hybrid/legacy migration, rollback, and release windows;
10. performance model, physical transport hotspot, and measurement gates;
11. diagnostics, introspection, artifact expectations, and acceptance matrix;
12. repository/host ownership boundaries, rejected alternatives, and unresolved
    decisions.

## Implications

### Accepted decisions

- `SKY_CUA_BROWSER_CONTROL_MODE` defaults to `legacy`; strict is initially
  opt-in and does not silently fall back.
- Canonical extension identity is fixed to
  `sky-cua-control-plane-v1` / `control-plane-lease-v1`; caller metadata remains
  private.
- Groups are browser-instance-scoped and exclusively leased for reads and
  writes, with 30-minute idle and up-to-ten-minute disconnect grace defaults.
- Operations are daemon/adapter allocated; upstream IDs are correlation only.
- Ambiguous mutations fence group transfer until retained settlement or exact
  target/browser loss.
- Initial actor width is two ordinary requests plus one exclusive large frame;
  heartbeat/events are out of band.
- Option A preserves Codex's exact filter while sky-cua supplies
  `type="iab"`, trusted-session `codexSessionId`, and peer-derived
  `codexAppBuildFlavor` in the compatibility reply.
- Legacy rollback stays packaged through at least one accepted release after
  strict first ships.
- Keep non-Codex production callers on ordinary service IPC/MCP. Do not bind a
  dedicated typed control UDS until a real adapter demonstrates that need.

### Rejected alternatives

- **Scheduler or lease policy in Codex Desktop:** rejected because it cannot
  arbitrate MCP hosts and would duplicate daemon state.
- **One extension session per caller/turn:** rejected because it exposes
  multiplexing to extension lifecycle and destabilizes ownership/heartbeat.
- **Role inferred from caller session or sentinel:** rejected because caller
  provenance must never select native-host authority.
- **Bare tab ID, URL/title, PID, socket path/mtime, or bridge generation as tab
  authority:** rejected because each can collide or be recycled.
- **Generic retry counter for mutations:** rejected because timeout/reconnect
  does not prove non-execution.
- **Process-wide mutex across browser I/O:** rejected because it destroys safe
  independent-tab overlap and risks reentrancy deadlock.
- **Silent strict-to-legacy fallback:** rejected because it hides the competing
  path the migration exists to remove.
- **Build-flavor exemption or fixed synthesis for sky-cua:** rejected in favor
  of peer-derived Option-A metadata and Codex's exact filter.
- **Visible Chrome tab-group mirroring in v1:** rejected as unnecessary UI side
  effect.
- **Replacing the upstream extension or launching a managed browser:** rejected
  as outside the real-user-browser product goal.

### Unresolved decisions

- Public opaque tab/group/lease handle evolution while preserving current MCP
  response compatibility.
- Default-mode promotion timing after measured hybrid/strict release windows.
- Actor-width changes and any future image chunking/shared-memory transport,
  pending performance evidence.

Durable behavior is described in
[`unified-browser-bridge-control-plane.md`](../features/unified-browser-bridge-control-plane.md),
the wire/state boundary in
[`browser-control-plane-protocol.md`](../runtime/browser-control-plane-protocol.md),
and operations in
[`browser-control-plane-migration.md`](../operations/browser-control-plane-migration.md).
