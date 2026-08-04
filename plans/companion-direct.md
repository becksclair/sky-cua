# Autonomous Companion Direct

This ExecPlan is maintained while implementation is active. Follow
`plans/AGENTS.md` when updating or retiring it.

## Purpose

Add `phone-control.v2`: an Android-initiated, authenticated WebSocket link over
Tailscale that lets phone-use operate without ADB. The direct Companion exposes
AppShots, UI actions, content transfer, clipboard/editor operations, CameraX/
Camera2 capture, and broad Android-permitted storage while preserving the
existing ADB-forwarded v1 backend as optional compatibility.

## Progress

- [x] (2026-08-02) Created isolated branch/worktree and froze the original
  checkout's dirty-path manifest in the implementation conversation.
- [ ] Complete deterministic and physical feasibility gates.
- [x] (2026-08-02) Froze shared identity, capability, content, enrollment,
  framing, and MCP contracts with cross-language fixtures; the Phase 1 broad
  and focused reviews are closed.
- [ ] Implement direct listener, Android outbound link, enrollment, auth,
  reconnect, epoch fencing, and conformance peers.
- [ ] Implement ContentBroker, clipboard/editor, storage, and camera waves.
- [ ] Integrate routing, packaging, installed-host portability, and regression
  coverage.
- [ ] Pass slice reviews, whole-feature ultra-review, standalone autoreview,
  and final physical acceptance.

## Surprises & Discoveries

- Current Companion v1 is intentionally loopback-only and reached through an
  ADB forward. Its token setup, device discovery, and host session identity are
  therefore coupled to ADB serials.
- Current phone screenshots are inline base64; direct binary content needs a
  separate descriptor and chunk protocol rather than extending that payload.
- Enrollment cannot call a device active when Saga merely emitted the secret:
  Android may fail before its durable credential commit. The direct protocol
  therefore needs an explicit pending state and proof-of-storage commit point.

## Decision Log

- Decision: the phone initiates one persistent WebSocket to an explicitly
  configured Saga tailnet address. Wildcard/public binds are invalid.
- Decision: enrollment produces a stable random `device_id` and per-device
  secret; reconnect uses mutual HMAC-SHA256 challenge proofs and new links
  atomically supersede old epochs.
- Decision: durable enrollment is `Pending` until Android first commits the
  credential and sends an authenticated idempotent acknowledgement. A valid
  pending mutual-HMAC reconnect is the lost-ack promotion path; pending devices
  cannot dispatch control, and consumed bootstrap secrets are never reissued.
- Decision: JSON control frames and bounded binary chunks share one socket;
  control has priority and camera preview is latest-frame-wins.
- Decision: no runtime ADB, Device Owner, Shizuku, root, shell broker, pre-unlock
  control, or AppServer integration is part of v1.

## Outcomes & Retrospective

Pending.

## Context

Shared contracts live in `crates/sky-cua-platform`. Host runtime ownership is in
`crates/sky-cua-service` and `crates/sky-cua-client`. Android code is under
`android/phone-companion`; `docs/runtime/phone-companion-protocol.md` remains
authoritative for the legacy v1 RPC and will link to the v2 protocol.

## Plan of Work

1. Add explicit direct-link config and shared v2 protocol types/limits.
2. Implement QR/manual enrollment, Keystore-wrapped credential storage, mutual
   challenge authentication, per-device actors, epoch fencing, and reconnect.
3. Add verified finite transfers and private temporary artifact lifecycles.
4. Implement feature families only after the content contract freezes.
5. Route operations by per-operation provider availability rather than backend
   booleans, keeping ADB serial and direct `device_id` separate.
6. Stage/install the Companion and prove all MCP hosts use the same contract.

## Validation

- Rust/Kotlin protocol fixture parity, malformed/replayed proof tests, loopback
  integration, and disconnect/supersession cases.
- Physical outbound link across backgrounding, screen-off, network changes, and
  reboot then first unlock, with the service ADB path invalid. On the Galaxy S26
  Ultra, whole-package process death requires Android or the user to recreate the
  package and then use the visible Retry action; the Companion reports
  `process_death_autorestart=false` rather than promising an OEM restart.
- Digest/length verification, interrupted-write cleanup, bounded queues,
  latency/backpressure measurements, and artifact expiry.
- Feature-specific emulator and Galaxy S26 Ultra acceptance.
- Full deterministic suite, compatibility regressions, deep ultra-review, then
  standalone autoreview as specified by the implementation request.

## Idempotence and Recovery

Non-idempotent actions are never automatically replayed. Disconnect aborts
incomplete writes. Idempotent finite reads may restart from byte zero. Every
command, result, event, and chunk is rejected when its link epoch is stale.

## Artifacts

Ignored evidence belongs under `artifacts/phone-companion-direct/`, including
transcripts, hashes, decoded media metadata, screenshots, videos, latency data,
and review reports. Captured personal content is never committed.

## Interfaces and Dependencies

- `phone-control.v2` WebSocket protocol and frozen cross-language fixtures.
- Android Keystore AES-GCM wrapping; HMAC-SHA256 challenge proofs.
- `ContentRef`, capability routes, grouped phone MCP families, and universal
  AppShot envelopes.
- Tailscale is routing only; application enrollment/authentication remains in
  sky-cua.
