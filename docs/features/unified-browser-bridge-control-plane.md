# Unified browser bridge control plane

## Status

Shipped on 2026-07-19. The daemon scheduler, persistent bridge actor,
native-host control-plane role, MCP provenance normalization, raw Codex ingress,
persistent recovery journal, migration modes, and bounded introspection are
implemented, reviewed, locally deployed, and live-proven. Installed acceptance
covered exact Codex Browser Use plus direct MCP, OpenClaw, OpenCode, and Pi on
separate tabs through one stable daemon. The aggregate VM `all` profile remains
blocked by its pre-existing Wayland-pointer scroll fixture before browser lanes;
the individual Codex Desktop, OpenCode, and Pi profiles pass, while the
deterministic `codex-cua` profile is externally blocked by a revoked Codex
refresh token.

## Summary

The unified control plane makes `sky-cua-service` the single admission,
ownership, scheduling, and bridge-lifecycle authority for browser work. In
`hybrid` or `strict` mode, Codex Browser Use, direct sky-cua MCP, OpenClaw,
OpenCode, and Pi converge on daemon-owned policy while each live browser/profile
keeps one persistent canonical connection to the native host.

This replaces the legacy split between short-lived operation sockets, a
separate heartbeat connection, and direct primary clients. It does not replace
the upstream extension, create a managed browser, or make Chrome's visible tab
groups part of the ownership contract.

## Contract surface

The operator selects `legacy|hybrid|strict` per field with this precedence:
process environment, `[browser_control]` in the machine config, then
legacy/unset behavior. `SKY_CUA_BROWSER_CONTROL_MODE` overrides
`[browser_control].mode`; `SKY_CUA_CODEX_BROWSER_SOCKET_PATH` overrides
`[browser_control].codex_socket_path`. Invalid, empty explicit, or non-UTF-8
values are hard errors. The Codex path optionally binds the raw compatibility
UDS and must differ from the ordinary service socket. `SKY_CUA_BROWSER` still
narrows browser-family selection.

Existing MCP browser tools remain the public high-level API. Their requests
carry a `BrowserRequestContext` containing independently normalized caller
provenance, logical identity, and operation identity.
`status(component="browser")` returns the browser status report, including a
`control_plane` snapshot in persistent modes.

The protocol-v1 constants visible to the extension are:

- canonical session: `sky-cua-control-plane-v1`
- canonical turn: `control-plane-lease-v1`
- native-host role: `control_plane`

Caller provenance, caller session/thread/turn, operation ID, daemon generation,
and private `_sky_cua_*` routing fields never become extension ownership
identity. The native host strips private fields before Chrome dispatch.

The detailed wire and state contract is
[`browser-control-plane-protocol.md`](../runtime/browser-control-plane-protocol.md).

## Behavior

### Implemented topology

```text
Codex nativePipe-compatible client -> raw compatibility UDS --+
direct MCP / OpenClaw / OpenCode / Pi -> service IPC ----------+-> sky-cua-service
                                                               |   identity normalizer
                                                               |   scheduler + leases/groups
                                                               |   operation/settlement registry
                                                               +-> persistent BridgeActor
                                                                     |
                                                                     | framed JSON-RPC UDS
                                                                     v
                                                               sky-cua-chrome-host
                                                                     |
                                                                     | Chrome native messaging
                                                                     v
                                                               browser extension
```

`legacy` retains the old per-operation connection and heartbeat path. `hybrid`
and `strict` route high-level MCP requests and configured raw Codex ingress into
the persistent runtime. One actor is maintained per discovered responsive
native-host socket; actor snapshots select at most one canonical ready actor per
stable browser instance.

The typed non-Codex `BrowserControlClientFrame`/`BrowserControlServerFrame`
model is frozen, but no dedicated typed control-socket listener consumes it in
the current source. Non-Codex production ingress is therefore the ordinary
service IPC/MCP path today.

### Exact identity separation

The following identities are deliberately non-interchangeable:

| Identity | Authority and lifetime |
| --- | --- |
| Caller provenance | Normalized lane: `codex_desktop`, `codex_cli`, `open_claw`, `open_code`, `pi`, `direct_mcp`, or `legacy_unknown`; stable for one ingress connection |
| Logical identity | Caller attribution and continuity; session plus optional thread/turn; never a bridge role |
| Principal | Same-UID ownership holder derived from ingress connection and normalized logical identity |
| Daemon generation | Fences requests and dedupe state across service restarts |
| Browser instance | Stable only while one browser/profile instance survives |
| Bridge connection | Changes on native-host/actor reconnect; includes peer PID/start ticks and actor generation |
| Tab key | Browser-instance ID plus extension tab ID; a bare tab ID is not global identity |
| Group | Daemon-local, browser-instance-scoped ownership unit with one lease, fence, membership revision, and member set |
| Operation | Daemon/adapter allocation plus canonical fingerprint; upstream JSON-RPC IDs are correlation only |

MCP provenance is chosen from the actual ingress, installer declaration,
`initialize.clientInfo`, and trusted Codex turn metadata. Installer declarations
are advisory same-user attribution, not authorization. Unknown declarations
normalize to `legacy_unknown` and remain visible diagnostically. OpenClaw,
OpenCode, Pi, and direct MCP no longer intentionally share the old
`sky-cua-mcp` principal. Install-time machine-config seeding normalizes legacy
browser aliases `brave-origin`, `chrome-origin`, and `chromium-origin` to
`brave`, `chrome`, and `chromium`.

### Canonical actor and Codex ingress

The bridge actor owns discovery, handshake, peer identity, one persistent
connection, request-ID allocation, pending replies, timeout tombstones,
heartbeat, extension events, reconnect/backoff, quarantine, and settlement
notifications. Its host hello requires control-plane, heartbeat,
extension-event, private-field-stripping, settlement, settlement-ack,
side-panel, and owner-release capabilities. Requiring owner release and
settlement acknowledgement prevents negotiation with an older host that cannot
complete rollback or durable completion delivery. A control-plane
role marker before `skyCuaHost/hello` is rejected
without selecting the role; after a valid hello the same private marker is
accepted and stripped before extension dispatch. Once any legacy role has been
selected, a late control-plane hello is rejected because role is immutable.

Raw Codex ingress preserves the upstream native-endian length-prefixed JSON-RPC
shape and exact server messages. The socket is owner-only (`0600`), accepts
same-UID peers, caps frames at 100 MiB, normalizes identity/class/scope/deadline
server-side, and detaches or cancels work when the connection closes. Default
read and mutation deadlines are 30 seconds and 15 seconds, capped at 120
seconds. Each connection retains at most 64 concurrent requests and 64
outbound messages; overflow fails closed instead of growing daemon memory.

Option A remains the Codex compatibility rule. For raw ingress, sky-cua maps
the canonical host reply to `type="iab"`, adds `metadata.codexSessionId` when
the request has a trusted logical session, and adds
`metadata.codexAppBuildFlavor` when the same-UID Codex peer exposes a bounded
`BROWSER_USE_CODEX_APP_BUILD_FLAVOR` environment value. Other host metadata is
preserved. Codex retains its exact build-flavor filter; sky-cua adds no flavor
exemption.

### Scheduling, leases, and completion

Accepted tab operations are FIFO with at most one in flight per tab.
Independent tabs can overlap. Bridge-global and daemon-global work use explicit
barriers; phase fairness permits at most one queued global operation before a
head-of-line round across waiting tab lanes. No scheduler lock is held across
browser or socket I/O.

Actors with the same stable browser-instance identity are canonicalized by
socket path before routing and status reporting. Multiple distinct browser
instances are preserved; instance-less operations reject genuine ambiguity
instead of selecting one by map iteration order.

One narrow reentrancy exception prevents an event-driven deadlock: a raw Codex
`Fetch` continuation (or `Runtime.runIfWaitingForDebugger`) associated with the
same connection/tab as an in-flight parent bypasses that tab's FIFO and enters
the actor as a correlated child. Its ambiguous completion is attributed to the
parent; unrelated same-tab work remains serialized.

Default bounds are 128 pending scheduler submissions, 128 queued operations per client, 32 per tab, 512 recent
operation records, two ordinary actor requests per bridge, and one exclusive
large-frame request. Heartbeat and extension events are out of band. Timed-out
bridge IDs are tombstoned for ten minutes, bounded to 2,048 entries.

Groups use exclusive fenced leases for reads and writes. The defaults are a
30-minute idle lease and a disconnect grace no longer than ten minutes or the
remaining lease lifetime. Group membership and admission close during handoff;
stale lease/fence/revision proofs are rejected. Discovery does not grant
ownership.

The daemon ticks lease expiry once per second without recording no-op events.
MCP connection close is ordered after calls that already began, while calls
arriving after close fail closed; this prevents a detached tool task from
re-registering a released principal. Claim reservations are operation-scoped:
definitive failure releases them, ambiguous completion retains them until a
matching settlement or browser/group loss, and definitive late success commits
the tab to group membership.

Retained settlements stay in the native host until the selected actor sends an
identity-fenced acknowledgement after scheduler handling. Reconnect after a
write-before-consume failure therefore replays the settlement, while duplicate
delivery and stale acknowledgement remain harmless. The host also retries an
unacknowledged settlement to the same actor at a bounded interval, covering a
lagged actor event receiver without requiring reconnect. A replacement daemon
does not acknowledge a retained settlement unless it has a matching live fence;
only the daemon that already reconciled an exact identity may acknowledge its
post-fence duplicate. This retention applies even when the direct response was
successfully written: kernel-buffer acceptance is not application receipt.
Direct and late terminal completions record a bounded operation-generation
marker so the following retained settlement can be acknowledged after its
active fence is cleared.

Raw Codex `executeCdp` receives debugger-session recovery only for an upfront
unattached rejection that proves no command ran. The daemon reuses its existing
recovery policy and replays that one safe class; timeout or mid-command detach
never authorizes replay.

Cancellation before dispatch guarantees no browser action. After dispatch it
detaches the waiter while shared execution drains. A dispatched mutation that
cannot be classified becomes settlement-pending, then settlement-unknown after
30 seconds. The affected group cannot expire or transfer until matching
terminal-safe completion or exact target/browser loss proves settlement. Lost
correlation while the target survives produces non-transferable
`recovery_required`; the mutation is never automatically replayed.

### Migration and rollback

- `legacy`: compatibility default; persistent control runtime is not created.
- `hybrid`: persistent actor and scheduler route sky-cua work while the native
  host permits legacy coexistence for measured migration and rollback.
- `strict`: persistent routing is selected and the host rejects competing
  legacy operation clients with structured diagnostics.

Strict is opt-in for its first accepted release. Rollback closes admission,
drains what can settle, marks unresolved mutations ambiguous, preserves open
tabs, and changes mode only after state is inspectable. Queued or ambiguous
mutations are not replayed. An idle clean shutdown sends a generation-checked
host release and requires its acknowledgement before the surviving native host
returns to hybrid compatibility. Abrupt or unsettled shutdown stays strict and
requires explicit recovery. See
[`browser-control-plane-migration.md`](../operations/browser-control-plane-migration.md)
for the operator procedure.

### Failure and ambiguity rules

- Missing `BrowserRequestContext` in `hybrid`/`strict` is a hard
  `BrowserRequestContextRequired` error.
- An invalid migration mode is a hard configuration error; there is no silent
  fallback.
- No ready persistent actor produces an unavailable diagnostic, not a direct
  legacy bypass.
- A protocol, owner-mode, generation, or required-capability mismatch prevents
  actor readiness.
- A bare tab ID, URL, title, socket mtime, PID, or connection generation is
  never sufficient to recover ownership.
- A `connection_only` or unavailable browser ID is regenerated from the
  host/peer/actor connection on every reconnect. Even a repeated host-provided
  ID is not evidence that browser lifetime survived; the prior browser is
  reported lost.
- Old-generation operation retries are rejected; daemon restart is not proof
  that a mutation stopped.
- Browser or exact target loss may settle an ambiguous operation; actor,
  native-host, extension, debugger, client, or daemon replacement alone may
  not.
- The authority-free journal is atomically persisted under the per-user state
  directory as `browser-control-recovery-v1.json`. Restart restores groups only
  as suspended with a fresh fence; unresolved mutations restore as
  `recovery_required`. It never restores live lease authority or operation
  payloads and never replays an operation.

### Performance model

The persistent path removes filesystem scan, probe, connect, and hello work
from every operation. Expected scheduler admission is constant-time mailbox and
hash-map work. Cross-tab overlap is bounded by the actor width, while same-tab
serialization prevents double mutation.

The remaining physical bottleneck is the single ordered native-messaging frame
stream. A large screenshot can delay small replies even when admission is
concurrent, so large frames are exclusive. Base64 image copies remain across
the extension, native host, service, and MCP layers. Queue depth, heartbeat RTT,
reconnect count, settlement counts, frame pressure, peak RSS, and unrelated-tab
latency are the relevant measurements; widening beyond two ordinary requests
requires evidence.

### Diagnostics and introspection

`status(component="browser")` structured content exposes:

- protocol version, daemon generation, migration mode, readiness, and client
  count;
- normalized client summaries and provenance source;
- actor socket/state/identity/stability, peer PID/start ticks, capability and
  canonical-selection flags, heartbeat RTT, reconnects, and quarantine reason;
- queue/in-flight/settlement counts, bounded group/member state, recent
  operations, and omitted counts;
- a 256-event ring with sequence window and dropped-event count.

Client results are bounded to 64, actors to 32, groups to 64, group members to
32, and recent operations to 64. These are snapshots for diagnosis, not a
persistent audit log. Daemon logs remain the forensic record.

Daemon health advertises `browser_control.v1` plus the effective
`legacy|hybrid|strict` mode. Strict requires a valid Codex listener; hybrid
reports listener degradation structurally. The listener owns an independent
singleton lock so a competing daemon cannot unlink or replace the live Codex
socket.

### Ownership boundaries

sky-cua owns protocol contracts, ingress normalization, scheduling,
deduplication, cancellation, leases/groups, canonical bridge identity,
heartbeat, socket selection, reconnect/failover, recovery policy,
introspection, installers, and smokes.

Codex Desktop owns only an upstream-compatible endpoint selector/transport
adapter for `nodeRepl.nativePipe`, metadata forwarding, reconnect/refresh, and
presentation of sky-cua errors. It must not own scheduling, leases, ownership,
heartbeat, or failover. OpenClaw, OpenCode, and Pi require only sky-cua-owned
installer/provenance adapters unless a future host limitation is proven.

## Source paths

- `crates/sky-cua-platform/src/model/browser.rs`
- `crates/sky-cua-platform/src/model/browser_control.rs`
- `crates/sky-cua-service/src/browser/control_plane/`
- `crates/sky-cua-service/src/browser/control_plane/scheduler/state/{admission,dispatch,introspection,actor}.rs`
- `crates/sky-cua-service/src/browser/transport.rs`
- `crates/sky-cua-service/src/codex_browser_compat/`
- `crates/sky-cua-service/src/daemon/browser.rs`
- `crates/sky-cua-service/src/ipc_server.rs`
- `crates/sky-cua-chrome-host/src/host/control_plane/`
- `crates/sky-cua-client/src/mcp_server.rs`
- `scripts/deploy_plugin.py`
- `scripts/install_mcp_server.py`
- `scripts/_openclaw_install.py`

## Verification

Final local source gates on 2026-07-20 passed workspace Rust formatting and
clippy with warnings denied, all 1,321 nextest cases, Rust doctests, Ruff,
basedpyright, all 730 Python tests, the 15-case process acceptance matrix, and
`git diff --check`. Focused tests cover tab FIFO/cross-tab overlap, global
barriers, fairness, dispatch width, lease/handoff/expiry/reconnect,
cancellation, ambiguous settlement/no replay, restart hints, stale
actors/sockets, heartbeat, full-duplex string and numeric IDs, strict/hybrid
listener behavior, and the two review-discovered disconnect/reservation races.
Settlement coverage additionally proves restart-safe acknowledgement fencing
and bounded same-actor retransmit after a lost event delivery. It also proves
active-response retention, late-response identity preservation, and refusal to
release strict ownership while mutation tombstones remain unresolved.

The reviewed runtime was locally deployed on 2026-07-20 with
`scripts/deploy_plugin.py`, followed by `scripts/sync_agent_skills.py` only
after deployment succeeded. `scripts/deploy_freshness.py` then reported the
installed client fresh against current runtime source. The deployed hybrid
daemon kept one canonical ready actor, zero queued/in-flight/pending/unknown
settlements, and owner-only `0600` service and Codex sockets under one stable
PID. Direct installed MCP opened a real Brave tab, observed it, typed
`FRESH CONTROL PLANE CLOSEOUT`, clicked the proof control, scrolled, and emitted
a browser screenshot. Raw framed Codex `getInfo` concurrently returned
`type="iab"`, `codexSessionId="closeout-codex-session"`, and
`codexAppBuildFlavor="prod"` with no JSON-RPC error.

Deployment now invokes Codex Desktop's installed `browser-use-cache-sync.cjs`
publisher through the installed `ChatGPT` executable in Node mode. The
installed `sky-cua-release.cjs` consumer resolver verifies one immutable
generation before cache mutation; deployment then fails closed unless the
resolved canonical Browser hash, node_repl trusted hash, packaged Browser
projection, and cache `latest` bytes are identical. One accepted 2026-07-21
matched-consumer gate repointed stale `browser-use/latest` from `0.1.0-alpha2`
to Codex Desktop client
`26.707.72221`, SHA-256
`085ba347a047473272cafc9f024b59c35dca4b29e44dab8b22eaa80e81e7c60d`,
from standalone release `f82b61b4962f318b5121464223ba5911d1f66adfed9511ecc42f909fa8b67c11`.
Later standalone rollovers fail before plugin or config mutation until Codex
Desktop repins its consumer contract; deployment never silently falls back past
a present, mismatched standalone `current` generation.
That exact trusted client bootstrapped through `setupBrowserRuntime`, selected
`iab`, typed `TRUSTED CUA NODE CLOSEOUT`, clicked the proof control, read the
committed value, emitted a screenshot, and closed its tab. After the updated
deployment procedure itself completed, a fresh connection repeated the
ordinary workflow with `POST DEPLOY TRUSTED CUA NODE` and emitted a second
screenshot. sky-cua invokes and verifies this Codex-owned materializer; it does
not vendor or rewrite Browser client bytes, the release verifier, or the Web
Store extension.

Installed Brave acceptance used stable service PID `2346805`, service socket
inode `104724`, and Codex socket inode `104725`. Raw `getInfo` returned
`type="iab"` with the trusted Codex session and canonical extension metadata.
The exact installed Codex Browser client then navigated to a local fixture,
clicked, typed `WP-CDX LIVE ACCEPTANCE`, read it back, keyboard-scrolled to
`scrollY=519.33`, and emitted JPEG screenshots with SHA-256
`914f5db4721764f5655d3561408750e4adc65ebf5c73d97b592a709e27726b54`
and `04699fd055d79d6a617a1087b16237f61839bc06cf68a761f9e24c6794988c66`.
It closed its tab and `js_reset` disconnected without autonomous reconnect.

During a Codex window, direct MCP independently navigated, clicked, typed,
scrolled, captured, and read back `DIRECT FINAL CONCURRENT`. Final installed
direct MCP repeated the path with `FINAL INSTALLED DIRECT`. Three simultaneous
installed clients declared as OpenClaw, OpenCode, and Pi opened and observed
separate tabs `379563083`, `379563086`, and `379563089`. Restarting only the
native host replaced PID `2346742` with `2349223` while the daemon PID and both
socket inodes stayed unchanged; raw Codex `getInfo` recovered immediately.

VM results with normal host build/checkout sync were:

- `codex-desktop`: pass, `/workspace/artifacts/gui-desktop-smoke/codex-desktop/20260719T105929Z`;
- `opencode-mcp`: pass with tool evidence, `/workspace/artifacts/opencode-wiring-smoke/20260719T110033Z`;
- `pi-mcp`: pass with tool evidence, `/workspace/artifacts/pi-wiring-smoke/20260719T110132Z`;
- `codex-cua`: product setup completed, then Codex exited before any tool call
  because its synced refresh token was revoked; evidence is
  `/workspace/artifacts/codex-e2e/codex-cua/20260719T110232Z`;
- aggregate `all`: two runs stopped at the unrelated `wayland-pointer` visible
  scroll acknowledgement before browser members, with artifacts
  `/workspace/artifacts/gui-desktop-smoke/wayland-pointer/20260719T091329Z`
  and `20260719T091720Z`.

The remaining measurement work is optional tuning rather than a correctness
gate: characterize unrelated-tab tail latency/RSS for larger screenshots before
changing actor width or transport framing.

## Known limitations

- The typed external control protocol is a model contract, not a bound typed
  UDS service.
- Restart recovery is intentionally suspended and non-authoritative; resumption
  requires exact browser/group/principal/member reconciliation and a fresh
  fence. A connection-only browser identity cannot be resumed across reconnect.
- The public high-level tool surface still uses legacy bare `tab_id` responses;
  opaque public tab handles and explicit lease/group operations are not exposed
  as a complete external API.
- Large frames still cause structural head-of-line blocking.
- Visible Chrome tab-group mirroring is intentionally deferred.
- The VM aggregate is not green because its Wayland-pointer prefix currently
  fails independently of browser control, and the VM `codex-cua` agent gate
  requires refreshed Codex credentials before it can execute tools.

## Related

- [`Browser control-plane runtime contract`](../runtime/browser-control-plane-protocol.md)
- [`Browser control-plane migration runbook`](../operations/browser-control-plane-migration.md)
- [`2026-07 research and decision summary`](../research/2026-07-unified-browser-bridge-control-plane.md)
- [`Browser MCP tools`](browser-mcp-tools.md)
- [`Codex Desktop compatibility`](codex-desktop-compat.md)
- [`ROADMAP.md`](../../ROADMAP.md)
