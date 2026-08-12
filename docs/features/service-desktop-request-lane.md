# Service desktop request lane

## Status

Shipped. Last live-verified on Plasma on 2026-08-12; full workspace source
validation re-verified in the current tree on 2026-08-12.

## Summary

The long-lived service answers health and independently owned requests without
placing the entire daemon behind one mutex, while desktop-sensitive work stays
ordered through an explicit service-owned lane. Bounded deadlines release the
lane and reset backend session state when a read-only desktop operation hangs.

## Contract surface

- Serialized `ServiceRequest` and `ServiceResponse` shapes are unchanged.
- `Health` bypasses the desktop lane and remains available while desktop work
  is blocked.
- `Health` never waits for desktop backend discovery. It reads the latest
  service-owned input capability snapshot while one background refresher owns
  probing and accessibility recovery.
- Desktop deadline failures are returned as structured
  `DesktopRequestDeadlineExceeded` errors.
- `SKY_CUA_DESKTOP_REQUEST_DEADLINE_MS` overrides the bounded read-only
  desktop request deadline for diagnostics and tests.

## Behavior

`ServiceDaemon::handle` takes `&self`. Desktop-sensitive requests acquire
`desktop_lane`; safe service-owned paths use their narrower state holders.
Read-only backend calls run under a deadline. On expiry, the daemon releases
the lane and initiates a bounded backend session reset so a subsequent health,
doctor, or observe call is not permanently wedged.

The IPC server shares `Arc<ServiceDaemon>` across connections. Connection
teardown performs bookkeeping only; the service-owned idle watchdog exclusively
owns cursor expiry, so teardown never performs overlay I/O or blocks new
connections.

One daemon-owned health capability refresher runs immediately and then every 30
seconds while healthy. It is independent of Health caller cancellation and
applies a 20-second outer deadline around the already bounded portal and AT-SPI
probe/repair path. Degraded refreshes back off through 30 seconds, one minute,
two minutes, four minutes, and five minutes. A total probe failure advertises
conservative input capabilities; semantic-only degradation retains the input
backend that remains independently usable.

## Source paths

- `crates/sky-cua-service/src/daemon/mod.rs` — request classification, lane,
  deadlines, capability snapshot/refresher, and reset policy
- `crates/sky-cua-service/src/daemon/desktop.rs` — desktop request dispatch
- `crates/sky-cua-service/src/daemon/tests.rs` — concurrency and deadline tests
- `crates/sky-cua-service/src/ipc_server.rs` — shared daemon and connection
  tracking

## Verification

- Service tests prove `Health` completes while a fake desktop action blocks and
  never invokes a hanging backend inline.
- Refresher tests prove one caller-independent probe, timeout downgrade,
  semantic-only input preservation, recovery, and bounded degraded backoff.
- `service_runtime_serializes_desktop_lane_requests` proves desktop ordering.
- `desktop_lane_deadline_frees_the_lane` proves a timed-out request does not
  retain the lane.
- Plasma `wayland-pointer` VM smokes passed on 2026-05-19 before and after the
  connection-tracker review fix.
- A live 300 ms deadline probe on 2026-07-08 returned
  `DesktopRequestDeadlineExceeded`, reset the session, and left a subsequent
  doctor call responsive.
- On 2026-08-12, all 1,916 workspace tests passed; the installed Plasma daemon
  served 100 warm Health calls with stable file descriptors.

## Known limitations

- Desktop-mutating operations intentionally remain serialized.
- A deadline reports and recovers service availability; it cannot guarantee
  immediate cancellation inside every third-party desktop API.

## Related

- ROADMAP entry: [`ROADMAP.md`](../../ROADMAP.md) under "Linux desktop parity"
- Originating ExecPlans retired into this feature doc; see git history for
  `plans/service_desktop_request_lane.md` and advisor plan 017.
