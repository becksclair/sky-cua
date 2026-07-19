# Browser control-plane migration and recovery

Use this runbook to inspect, migrate, roll back, or troubleshoot the shipped
unified browser bridge. Installed acceptance proves exact Codex Browser Use,
direct MCP overlap, simultaneous OpenClaw/OpenCode/Pi clients, and native-host
replacement on one stable daemon. The aggregate VM `all` profile remains
blocked by its unrelated Wayland-pointer prefix, and VM `codex-cua` needs a
refreshed Codex credential. Do not infer readiness from source markers or a log
line alone; require wire `getInfo`, structured status, and an ordinary task.

## Preconditions

- Use a current `sky-cua-service`, `sky-cua-client`, and
  `sky-cua-chrome-host` from the same build.
- Keep the ordinary service socket and raw Codex socket distinct.
- Preserve user tabs. Use a disposable browser profile for destructive
  restart/failover drills.
- Record source revision, binary/install identity, selected mode, socket paths,
  browser family/profile, native-host/extension version, and sanitized env.
- Treat an outstanding or unknown mutation as non-replayable.

For checkout validation, follow
[`isolated-daemon-smokes.md`](isolated-daemon-smokes.md) and pass a run-specific
`SKY_CUA_SERVICE_SOCKET_PATH`. A raw Codex lane additionally needs a distinct
run-specific `SKY_CUA_CODEX_BROWSER_SOCKET_PATH`.

## Establish a legacy baseline

1. Set `SKY_CUA_BROWSER_CONTROL_MODE=legacy`, or set
   `[browser_control].mode = "legacy"` with no environment override. Leaving
   the variable unset selects legacy only when the machine field is also unset.
2. Leave both the Codex socket environment and machine field unset unless
   testing raw ingress; in legacy mode a configured listener reports
   incompatibility rather than bypassing into a direct bridge.
3. Run `status(component="browser")` and one read-only operation, then one
   ordinary operation whose outcome is visually or state verified.
4. Record existing native-host sockets, selected browser family, diagnostics,
   and daemon log path.

The absence of `control_plane` in structured status is expected in `legacy`.

## Enter hybrid

Configure the machine-owned default:

```toml
[browser_control]
mode = "hybrid"
codex_socket_path = "/run/user/1000/sky-cua/codex-browser.sock"
```

The default file is `~/.config/sky-cua/sky-cua.toml`; `SKY_CUA_CONFIG_PATH`
selects another file. A process environment value overrides the matching
machine field independently, so this remains useful for an isolated run:

```bash
export SKY_CUA_BROWSER_CONTROL_MODE=hybrid
export SKY_CUA_CODEX_BROWSER_SOCKET_PATH="${XDG_RUNTIME_DIR}/sky-cua/codex-browser.sock"
```

Restart only the isolated or intentionally targeted daemon so it reads the new
configuration. Absence in both layers preserves legacy/unset behavior. An
explicit empty or invalid field is an error, not an instruction to clear the
machine value. Then call `status(component="browser")` and inspect its
structured content.
Required source-level health is:

- `migration_mode` is `hybrid`;
- `ready` is true;
- at least one actor is `ready`, `protocol_capable`, `selected`, and canonical
  for its browser instance;
- actor peer PID/start ticks, browser-instance stability, and heartbeat RTT are
  present or their absence is explained;
- queue, in-flight, and settlement counts are understood;
- clients show the expected normalized lanes and provenance sources;
- no quarantine, dropped-event growth, or unknown settlement is ignored.

Exercise direct MCP first. Then add OpenClaw, OpenCode, and Pi one lane at a
time, confirming distinct connection IDs/provenance and separate owned tab
groups. The current public API does not expose every group operation directly,
so acceptance harness evidence may be required for ownership assertions.

For Codex, point the trusted nativePipe/browser adapter explicitly at the raw
socket. Confirm `getInfo.type == "iab"`,
`getInfo.metadata.codexSessionId` matches the trusted request session, and
`getInfo.metadata.codexAppBuildFlavor` matches the exact value derived from the
same-UID Codex peer's `BROWSER_USE_CODEX_APP_BUILD_FLAVOR`. Option A forbids a
flavor exemption or weakening Codex's filter. Installed acceptance currently
covers the operation set and two-Codex-connection case listed above; it does
not cover overlap with direct MCP or another agent host.

## Promote to strict

Promote only after hybrid has all of the following evidence:

- same-tab FIFO and independent-tab overlap;
- concurrent Codex plus at least two non-Codex callers without primary eviction
  or cross-session claiming;
- heartbeat continuity beyond multiple extension heartbeat intervals;
- queued cancellation and ambiguous-mutation behavior;
- legacy-client counts identified and reduced to zero for the target lane;
- a successful rollback drill that preserves open tabs;
- installed and VM gates required by the release.

Set `SKY_CUA_BROWSER_CONTROL_MODE=strict`, restart the targeted daemon, and
repeat structured status plus read/mutation verification. A competing legacy
operation client should receive a structured migration rejection. Strict
startup or handshake failure is a failure; do not mask it by silently routing
through legacy.

## Health and introspection checklist

Use `status(component="browser")` and retain its `structuredContent`, not only
the text summary. Inspect:

- `daemon_generation` changes only when expected;
- `actors[].bridge_connection_id` may change on reconnect while a stable
  surviving `browser_instance_id` does not;
- `connection_only`/unavailable actors receive a new generated browser ID on
  every reconnect and the prior browser is reported lost; a repeated proposed
  ID is not stable identity;
- `actors[].canonical` is unique per browser instance;
- `last_heartbeat_rtt_ms` advances and `reconnect_count` is explainable;
- `scheduler.queued_count` and `in_flight_count` return to expected levels;
- `settlement_pending_count` and `settlement_unknown_count` are zero before
  migration or handoff;
- every group has the expected owner, lease state, fence, membership revision,
  and members;
- `events.dropped_count` and all `*_omitted` fields are considered before
  concluding an item is absent.

Check the daemon log identified in
[`mcp-boundary.md`](../runtime/mcp-boundary.md) for handshake, quarantine,
reconnect, compatibility bind, and request errors. Status is bounded and not a
durable audit log.

## Roll back

1. Stop new browser admission at the host/caller layer.
2. Wait for definitive in-flight operations to drain. Do not wait forever on an
   ambiguous mutation.
3. Record every settlement-pending/unknown operation, group, tab key, fence,
   and daemon/actor generation.
4. Treat unresolved dispatched mutations as ambiguous. Do not retry them under
   a new ID or after reconnect.
5. Preserve open user tabs. Do not finalize or close them as generic cleanup.
6. Switch Codex's endpoint selector back to its known direct nativePipe path if
   that adapter had been enabled.
7. Change the daemon mode to `hybrid` first when diagnosis is still needed, or
   `legacy` for full compatibility rollback; restart the targeted daemon.
8. Verify heartbeat/status and one read-only task, then one visually/state
   verified ordinary legacy task.
9. Record the rollback result and unresolved mutation state before declaring
   recovery complete.

Persistent modes atomically checkpoint authority-free recovery hints at
`$XDG_STATE_HOME/sky-cua/browser-control-recovery-v1.json` (or the
`~/.local/state` fallback). On restart, groups are suspended with a fresh
fence; unresolved mutations become `recovery_required`. The journal never
restores active lease authority or operation payloads and never authorizes
replay. Resume only after exact browser/group/principal/member/revision
reconciliation; a connection-only browser identity cannot be reconciled across
reconnect.

## Troubleshooting

### `BrowserControlModeInvalid`

Use exactly `legacy`, `hybrid`, or `strict` (lowercase after trimming). Invalid
or non-UTF-8 values intentionally do not fall back.

### `BrowserRequestContextRequired`

The caller is using `hybrid`/`strict` without the normalized MCP context. Update
the client/installer from the same build; do not invent a shared fallback
identity.

### `BrowserControlUnavailable` or no ready actor

Inspect native-host socket discovery, browser-family selection, host protocol
version, negotiated owner mode, required capabilities, peer identity, and
quarantine reason. A newer socket is not necessarily healthy; do not delete or
pin sockets based only on mtime.

### Codex protocol mismatch or build-flavor rejection

Confirm the raw socket is configured and distinct from service IPC, both peers
use native-endian length framing, and `getInfo` returned `type="iab"`, the
trusted logical `codexSessionId`, and the peer-derived
`codexAppBuildFlavor`. Inspect the Codex peer environment if the flavor is
absent; do not add a sky-cua flavor exemption.

### Reconnect loop or heartbeat loss

Compare actor generation, bridge connection ID, peer PID/start ticks,
reconnect count, heartbeat RTT, and host logs. Connection replacement may be
recoverable, but it is not proof that an in-flight mutation stopped.

### Queue backpressure

Default limits are 128 queued per client and 32 per tab. Find the client and tab
lane accumulating work. Do not widen limits before identifying a stalled actor,
global barrier, or caller flood.

### Fetch continuation deadlock

Raw Codex `Fetch` continuation commands that answer an event raised by an
in-flight same-tab parent must reenter the actor as correlated children. They
must not queue behind that parent. Confirm the continuation method is one of
the allowlisted Fetch methods (or `Runtime.runIfWaitingForDebugger`) and that
the connection/tab still maps to the in-flight parent; do not make arbitrary
same-tab mutations reentrant.

### Settlement pending/unknown or `recovery_required`

Freeze handoff and expiry for the affected group. Seek a matching retained
completion or prove exact target/browser-instance loss. Client EOF, timeout,
debugger detach, actor reconnect, native-host restart, extension restart, or
daemon restart is insufficient. Never replay the mutation automatically.

### Duplicate or ambiguous tab ID

Resolve with browser-instance identity or an owned group. `SKY_CUA_BROWSER` may
narrow discovery during migration, but a bare extension tab ID is not globally
unique and must not be guessed across browsers.

### Large screenshot delays unrelated work

This can be structural head-of-line blocking on the one native-messaging frame
stream. Capture frame size, unrelated-tab latency, queue depth, actor width, and
RSS. Keep the large-frame lane exclusive; do not increase ordinary width as a
blind fix.

## Acceptance evidence

Store generated evidence under ignored
`artifacts/browser-control-plane/<run-id>/`. Include commands, source/build
identity, mode, sanitized config, structured status, lifecycle/operation traces,
same-tab and cross-tab timings, rollback result, and exact skipped gates. Do not
store secrets, auth payloads, or sensitive page content.

The accepted evidence and remaining external VM blockers are recorded in the
feature doc's
[`Verification`](../features/unified-browser-bridge-control-plane.md#verification)
section. Treat optional large-frame performance work as tuning: do not widen
actor concurrency without measured unrelated-tab latency and RSS.
