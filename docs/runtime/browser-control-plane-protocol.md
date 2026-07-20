# Browser control-plane runtime contract

This document defines the stable and staged runtime boundaries for unified
browser control. The source-of-truth Rust models are
`crates/sky-cua-platform/src/model/browser_control.rs` and
`crates/sky-cua-platform/src/model/browser.rs`; daemon behavior lives under
`crates/sky-cua-service/src/browser/control_plane/`.

## Runtime endpoints

### Ordinary service IPC

Existing MCP browser tools use `ServiceRequest::Browser` over the ordinary
sky-cua service endpoint. In `legacy`, requests use the compatibility executor.
In `hybrid` or `strict`, non-status requests require `BrowserRequestContext` and
enter `BrowserControlRuntime`; status includes the bounded control-plane
snapshot.

This is the implemented typed ingress for direct MCP, OpenClaw, OpenCode, and
Pi. It preserves the existing MCP tool contract.

### Raw Codex compatibility UDS

`SKY_CUA_CODEX_BROWSER_SOCKET_PATH=<path>` asks the daemon to bind a second Unix
socket. The path must differ from the service socket. If both environment and
machine config leave it unset, no Codex compatibility listener is bound; an
explicit empty environment or machine value is invalid. The daemon creates parent directories,
removes a stale path before bind, sets mode `0600`, verifies same-UID peers,
rebinds an unexpectedly unlinked path on its maintenance tick, and removes the
path at clean shutdown.

The wire is the upstream Browser client protocol: native-endian four-byte frame
length followed by JSON-RPC JSON, with a 100-MiB maximum frame. It does not send
or expect a typed control-plane `hello`. JSON-RPC server requests,
notifications, errors, and results pass through in upstream shape.

The daemon assigns connection and operation IDs, records provenance as
`codex_desktop`, extracts only top-level trusted session/thread/turn metadata,
classifies scope and mutability, fingerprints canonical method/params, and
chooses deadlines. Defaults are 30,000 ms for reads and 15,000 ms for absolute
sets/mutations, with a 120,000-ms cap. Client EOF invokes cancellation or waiter
detach according to dispatch state; it is not a quiescence boundary.

For raw Codex ingress, `getInfo` preserves unrelated host metadata but maps the
reply to `type="iab"`. When available, it adds `metadata.codexSessionId` from
the trusted top-level logical session and `metadata.codexAppBuildFlavor` from
the bounded, UTF-8 `BROWSER_USE_CODEX_APP_BUILD_FLAVOR` value read from the
same-UID socket peer's process environment. The Codex adapter continues exact
build-flavor matching (Option A); sky-cua does not bypass that check.

### Typed control protocol v1

`BrowserControlClientFrame` and `BrowserControlServerFrame` freeze the intended
typed adapter protocol, version `1`:

```text
client: hello | request | cancel | event_ack
server: hello_ok | response | event
```

`hello` carries protocol version, client name/version/adapter version, normalized
caller provenance, logical identity, capabilities, and optional resume token.
`hello_ok` returns client instance, logical session, principal, daemon
generation, and negotiated capabilities.

`request` carries a transport request ID, retry-stable submission ID, upstream
correlation ID, daemon generation, optional logical-identity delta, optional
lease proof, policy-free operation, and optional requested deadline. The
operation is either a high-level `BrowserRequest` or upstream JSON-RPC method
plus params. The daemon, not the caller, derives operation ID, fingerprint,
scope, class, effective deadline, ownership checks, and retry policy.

`response` returns request ID, submission ID, daemon operation ID, completion
certainty, optional result, and optional structured diagnostic. `cancel` names
the request/submission and optional operation. Events carry monotonic sequence,
daemon generation, timestamp, optional principal/group/tab/operation references,
and a typed client, bridge, lease, queue, operation, settlement, lifecycle,
heartbeat, recovery, failover, or migration event.

No daemon listener currently serves these typed frames on a dedicated UDS. The
model is stable for future adapters; claiming it as a live endpoint is an error.

## Configuration contract

Resolution is independent per field: explicit process environment wins over
`[browser_control]` in `~/.config/sky-cua/sky-cua.toml` (or
`SKY_CUA_CONFIG_PATH`), and absence in both preserves legacy/unset behavior.

| Field | Machine field | Unset behavior | Contract |
| --- | --- | --- | --- |
| `SKY_CUA_BROWSER_CONTROL_MODE` | `mode` | legacy | Exact values `legacy`, `hybrid`, `strict`; whitespace is trimmed; empty, invalid, or non-UTF-8 is a hard diagnostic |
| `SKY_CUA_CODEX_BROWSER_SOCKET_PATH` | `codex_socket_path` | no raw listener | Enables the raw Codex UDS; an explicit empty value is invalid; must differ from service socket |
| `SKY_CUA_BROWSER` | top-level `browser` | existing selection default | Narrows Chrome-family actor discovery; does not grant ownership or identify a browser instance |
| `SKY_CUA_BROWSER_REQUEST_TIMEOUT_MS` | none | path-specific defaults | Existing overall browser/CDP override; persistent raw Codex requests still apply their own classified defaults and cap |

Installers forward the two control-plane variables only when supplied. They do
not silently enable `hybrid` or `strict`; install-time seeding can persist
explicit values into `[browser_control]` without erasing an omitted field.
Legacy browser-selection aliases `brave-origin`, `chrome-origin`, and
`chromium-origin` normalize to their canonical names. OpenClaw/OpenCode/Pi
declarations normalize provenance but are not an authorization boundary.

## Identity and authority

The canonical extension session is always
`sky-cua-control-plane-v1`; methods requiring a turn use
`control-plane-lease-v1`. The host role is `control_plane`. These values are
bridge lifecycle identity only.

Caller provenance, logical session, principal, browser instance, bridge
connection, tab key, group, lease/fence/revision, operation, request, and daemon
generation are separate records. In particular:

- extension tab IDs are unique only within a browser instance;
- bridge reconnect changes bridge identity but not a proven surviving browser
  instance;
- when stability is `connection_only` or unavailable, reconnect always creates
  a new generated browser-instance ID and reports the previous browser lost,
  even if the host repeats its proposed ID;
- browser restart invalidates old tab keys;
- upstream JSON-RPC/tool-call IDs are correlation, not operation dedupe IDs;
- declared caller labels are retained diagnostically but cannot choose the
  bridge role;
- URL/title and process/socket metadata cannot restore authority.

## Scheduling and ownership state

Tab lanes are FIFO and one-in-flight. Independent tabs overlap subject to actor
capacity. Bridge-global and daemon-global operations close relevant admission,
drain admitted tab work, execute exclusively, then reopen it. Phase fairness
alternates one global operation with one round of waiting tab heads when both
classes are queued.

Default queue limits are 128 pending submit messages, 128 per client, and 32
per tab. Submit traffic uses a bounded ingress separate from completions,
settlement, lease ticks, and lifecycle commands, so a caller flood cannot
starve terminal bookkeeping. The scheduler retains up to 512 recent internal
operation IDs; status emits at most 64 recent operation summaries. The bridge
permits two ordinary requests or one exclusive large-frame request. Heartbeat
and extension events bypass ordinary width.

A group belongs to one browser instance and one same-UID principal. It carries
one lease ID, monotonically changing fence, membership revision, and member tab
keys. Idle lease duration is 30 minutes. Disconnect moves an active lease to
orphaned grace until the earlier of its existing expiry and ten minutes.
Handoff or force transfer closes admission and requires the current membership
revision. Reads and writes both require current ownership.

## Lifecycle behavior

### Daemon and clients

Every daemon process has a generation. Old-generation submissions are rejected;
operation-result dedupe does not cross generations. In persistent modes, each
MCP connection gets stable normalized provenance and logical identity. Client
disconnect cancels queued work, detaches waiters from dispatched work, and
orphans its leases for grace. Calls that began before MCP EOF finish before
principal cleanup; a call that reaches the daemon after that connection is
closed fails with `BrowserClientDisconnected` and cannot re-register the
principal.

### Bridge actor

The implemented actor snapshot moves through `connecting -> host_handshake ->
ready`, with `reconnecting`, `quarantined`, and `lost` failure states. The
public protocol enum also reserves `discovered`, `probing`, and
`extension_handshake` for broader discovery/lifecycle reporting. The handshake
checks protocol version, requested owner mode, and required capabilities.
Default connect/handshake/write deadlines are three seconds, heartbeat interval one
second, heartbeat response deadline three seconds, and reconnect backoff 100 ms
through three seconds.

Request IDs are monotonic across each actor generation and include daemon and
actor generation. Timed-out IDs are tombstoned for ten minutes, bounded at
2,048. Socket health prefers a known healthy actor over a merely newer socket.

The negotiated capability set includes `owner_release` and `settlement_ack`;
strict mode does not accept an older host that cannot acknowledge safe
ownership teardown or retained settlement receipt. The
native host does not allow a request to self-select `control_plane` before
handshake. A private role marker before `skyCuaHost/hello` gets
`sky_cua_host_hello_required` and leaves the role unknown. A valid hello fixes
the role immutably; subsequent control-plane markers are accepted and stripped
before extension dispatch, while a legacy-selected client cannot upgrade late.
On a clean strict-mode shutdown after request and settlement ledgers drain, the
actor sends generation-checked `skyCuaHost/release` and waits for an
acknowledged transition to `hybrid`. This permits a surviving native host to
accept the documented legacy rollback path. A disconnect or unsafe/unsettled
shutdown does not silently release strict ownership.

Clean strict release also requires the actor's settlement-bearing mutation
tombstones to be resolved. A timed-out mutation that still awaits a retained
late completion therefore keeps ownership strict; sending its exact settlement
acknowledgement resolves the matching actor tombstones before release.

### Operation and settlement

Operation state is queued, dispatched, settlement-pending,
settlement-unknown, or terminal. Completion certainty is pre-dispatch rejected,
definitive success, definitive failure, or ambiguous completion.

Cancellation while queued is definitive and emits no extension frame.
Cancellation after dispatch detaches the caller; execution and settlement
continue. An ambiguous mutation creates a 30-second settlement-pending window,
then remains settlement-unknown/recovery-required until a matching retained
completion or exact target/browser loss settles it. Reconnect, replacement, or
timeout alone never authorizes replay.

The native host retains every mutating completion, including one also returned
on the active direct-response path, across socket writes and reconnects until
the selected actor acknowledges its exact operation, Chrome request, daemon
generation, and actor generation. Duplicate delivery and duplicate or stale
acknowledgements are idempotent; a kernel write alone is never receipt.
Until acknowledgement, the host retransmits the unchanged queue head to the
same selected actor at a bounded interval so a lagged event receiver cannot
strand the ledger. After a daemon restart, an actor may acknowledge only a
currently matching scheduler fence or an exact settlement identity already
reconciled by that daemon; the mere presence of identity fields is insufficient.

Ready actors are canonicalized deterministically by stable browser-instance
identity and socket path. Duplicate sockets for the same stable browser do not
make routing ambiguous. Distinct browser instances remain distinct, and an
instance-less request fails honestly when more than one distinct eligible
browser remains after explicit browser selection.

Tab-claim reservations follow the same certainty boundary. Pre-dispatch or
definitive failure releases a reservation. Ambiguity keeps the tab reserved to
the original group; matching late success commits membership, while matching
failure or target/browser loss releases it. Periodic ownership reconciliation
preserves reservations for every non-released group.

Raw event-driven Codex continuations are the one same-tab reentrancy exception.
When a `Fetch.continueRequest`, `Fetch.continueResponse`,
`Fetch.continueWithAuth`, `Fetch.failRequest`, `Fetch.fulfillRequest`, or
`Runtime.runIfWaitingForDebugger` request matches the connection/tab of an
in-flight parent, it dispatches directly through the actor as a correlated
child. Queuing it behind the parent would deadlock a parent waiting on the
event response. Other same-tab work remains FIFO; ambiguous child completion
is attributed to the parent rather than replayed.

A raw `executeCdp` command rejected up front because the debugger is unattached
uses the same owner-side session recovery as high-level browser actions and may
replay only because that rejection proves the command did not execute. Timeouts
and mid-execution detach remain ambiguous and are not replayed.

### Restart recovery

Production persistent modes capture an authority-free v1 journal at
`$XDG_STATE_HOME/sky-cua/browser-control-recovery-v1.json` (falling back to
`~/.local/state/sky-cua/`). Writes use a private directory/file, fsync, atomic
rename, bounded schema, and remove the file when no groups remain. Startup
loads valid hints and immediately checkpoints their suspended form; malformed,
unsupported, oversized, or unavailable journal state is ignored with a
recovery diagnostic and no restored authority.

Every recovered group has admission closed, a suspended lease, and a fence one
higher than the recorded fence. An unresolved mutation restores as
`recovery_required`. The journal excludes lease IDs/expiry, operation IDs and
payloads, and handoff offers, so restart never replays work or revives live
authority. Resumption requires exact browser instance, principal, members, and
membership revision; connection-only browser identity cannot satisfy
cross-reconnect recovery.

Both ordinary browser IPC and raw Codex requests mark browser activity. This
prevents the daemon's five-minute idle exit for 30 minutes after the latest
browser request, so an active Codex discovery/action sequence cannot tear down
the canonical actor and heartbeat between operations.

## Diagnostics contract

`status(component="browser")` is the supported MCP introspection entry. In
persistent modes its structured `control_plane` object reports
protocol/generation/mode/readiness,
bounded clients and actors, scheduler/group/operation state, and a sequenced
256-event window with dropped-count accounting. Omitted counts distinguish
truncation from absence.

The one-second lease timer records an event only when it changes lease/group
state; idle ticks must not consume the bounded event window. Daemon health
advertises `browser_control.v1` and the effective migration mode so clients can
reject incompatible shared daemons without displacing a healthy owner.

Snapshot labels and declared provenance are operational diagnostics, not
credentials. Status is point-in-time state; daemon logs are the durable forensic
surface. See
[`browser-control-plane-migration.md`](../operations/browser-control-plane-migration.md).

## Compatibility and versioning

- Protocol v1 rejects a mismatched version; there is no version guessing.
- `legacy` remains the absence/default behavior for the first migration window.
- `strict` does not silently fall back when startup or handshake fails.
- Existing MCP browser tool arguments/results remain the compatibility surface
  while opaque public tab/group/lease handles are incomplete.
- Legacy rollback remains packaged for at least one accepted release after the
  first strict release; removal requires a later explicit decision.
