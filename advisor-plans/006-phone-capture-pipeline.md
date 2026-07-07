# Plan 006: Fix the phone capture pipeline (decode once, downscaled model delivery, narrower phone lock)

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan
> in `advisor-plans/README.md` — unless a reviewer dispatched you and told you
> they maintain the index.
>
> **Drift check (run first)**: `git diff --stat ed3aef3..HEAD -- crates/sky-cua-service/src/phone/manager/capture.rs crates/sky-cua-service/src/daemon.rs crates/sky-cua-service/src/browser/model_image.rs crates/sky-cua-capture/src/lib.rs`
> On any in-scope drift, re-verify the excerpts below; on mismatch, STOP.

## Status

- **Priority**: P2
- **Effort**: M-L (three sub-changes; the downscale one carries the design risk)
- **Risk**: MED — coordinate mapping must stay in device pixels while the
  delivered image shrinks
- **Depends on**: 002 (adb parser + geometry tests are the regression net);
  005 recommended first (establishes the spawn_blocking pattern)
- **Category**: perf
- **Planned at**: commit `ed3aef3`, 2026-07-07

## Why this matters

The phone lane is the highest-frequency capture loop and the least efficient:

1. **Triple image pass**: every capture fully decodes the PNG once as an
   integrity gate (`decode_png_dimensions`), then — when the synthetic cursor
   is composited — `composite_cursor` decodes the *same bytes again*, and
   re-encodes to PNG.
2. **Full-res delivery**: the final image ships to the model as
   full-device-resolution base64 PNG (1080×2400 ≈ 1–3MB, +33% base64) while
   the desktop lane delivers downscaled WebP/JPEG measured in tens of KB.
   Per-call latency and token cost are dominated by this.
3. **Wide lock**: the daemon holds the global phone mutex across the entire
   `phone.handle(request)` — including the adb screencap subprocess and all
   image work — so any concurrent phone status/notification poll blocks for
   the full capture.

**Important nuance the fix must preserve**: the full decode in
`decode_png_dimensions` is deliberate — its doc comment says a truncated
screencap over a flaky wireless link must fail closed rather than become a
degenerate snapshot. Keep one full decode as the integrity gate; eliminate
the *second* decode and the full-res delivery.

## Current state

`crates/sky-cua-service/src/phone/manager/capture.rs`:

- :75-81 — both companion-fallback and plain-adb branches:
  ```rust
  let png = self.adb_screencap(ctx).await?;
  let (w, h) = decode_png_dimensions(&png)?;
  ```
- :418-431 — the gate (keep its semantics):
  ```rust
  /// ... A truncated or non-PNG payload (e.g. a partial `screencap` over a
  /// flaky wireless link) must not become a degenerate 0x0 "successful"
  /// screenshot; it is a structured capture failure ...
  fn decode_png_dimensions(png: &[u8]) -> Result<(u32, u32), DiagnosticEntry> {
      match image::load_from_memory_with_format(png, ImageFormat::Png) {
          Ok(image) if image.width() > 0 && image.height() > 0 => Ok((image.width(), image.height())),
          _ => Err(DiagnosticEntry { code: "PhoneScreencapDecodeFailed".to_string(), ... })
  ```
- :115-127 — cursor compositing re-decodes:
  ```rust
  if self.selection.screenshot_cursor && !contains_native_overlay
      && let Some(point) = ... { composite_cursor(&mut png, point); }
  ```
  and :431-446:
  ```rust
  fn composite_cursor(png: &mut Vec<u8>, point: PhonePoint) {
      let Ok(image) = image::load_from_memory_with_format(png, ImageFormat::Png) else { return; };
      let mut rgba = image.to_rgba8();
      if cursor::compose_synthetic_cursor(&mut rgba, point).is_err() { return; }
      ... re-encode to PNG, *png = out.into_inner();
  }
  ```
- :105-113 — the snapshot mapping is minted as `identity_mapping(...,
  device_size, ...)`: coordinate actions resolve against **device pixels**.
- :140-147 — delivery:
  ```rust
  let inline_image = include_image.then(|| PhoneImage {
      mime_type: "image/png".to_string(),
      data_base64: BASE64.encode(&png),
      width: Some(width), height: Some(height),
  });
  ```

Exemplars for downscale+encode:
- Desktop: `crates/sky-cua-capture/src/lib.rs:169-255`
  (`prepare_model_capture*` — resize to model bounds, WebP/JPEG encode,
  honors `SKY_CUA_MODEL_SCREENSHOT_FORMAT`/quality/max-dims env knobs).
- Browser: `crates/sky-cua-service/src/browser/model_image.rs:57-171`
  (`prepare_browser_capture` — same idea over base64 payloads; note
  `model_image.rs:18` re-declares the format env key locally).

Lock span — `crates/sky-cua-service/src/daemon.rs:243-256`:

```rust
let response = {
    let mut phone = self.phone.lock().await;
    if let Some(default) = scrcpy_size_default { phone.set_scrcpy_host_size_default(default); }
    if adoption_candidate.is_some() { phone.set_scrcpy_adoption_candidate(adoption_candidate); }
    let response = phone.handle(request).await;   // <-- adb + image work under the lock
    phone.set_scrcpy_adoption_candidate(None);
    response
};
```

Note the surrounding code already hoists slow probes *outside* the lock
(`scrcpy_adoption_candidate_for` at :241 runs before locking) — that's the
established pattern.

## Commands you will need

| Purpose | Command | Expected on success |
|---|---|---|
| Service tests | `cargo nextest run -p sky-cua-service` | all pass (incl. `phone/manager/tests.rs`, 4,400 lines of coverage) |
| Client tests | `cargo nextest run -p sky-cua-client` | all pass (phone tool fixtures) |
| Whole workspace | `cargo nextest run` | all pass |
| Format | `cargo fmt --check` | exit 0 |

## Scope

**In scope**:
- `crates/sky-cua-service/src/phone/manager/capture.rs`
- `crates/sky-cua-service/src/phone/manager/tests.rs` (new/updated tests)
- `crates/sky-cua-service/src/daemon.rs` (phone lock narrowing ONLY —
  Step 3; skip if it violates a STOP condition)
- `crates/sky-cua-platform/src/model/phone.rs` — only if `PhoneImage` needs
  an additive optional field (e.g. `original_width/height`); no breaking
  changes

**Out of scope** (do NOT touch):
- The snapshot mapping contract: coordinate actions MUST keep resolving in
  device pixels. Do not change `identity_mapping` or snapshot registration.
- The companion (Kotlin) capture path and its native overlay logic.
- Desktop/browser capture code (exemplars only).
- scrcpy adoption logic in `daemon.rs`.

## Git workflow

- Branch: `bex/advisor-006-phone-capture-pipeline`
- Commits: `perf(phone): decode captures once and reuse the raster`,
  `perf(phone): deliver downscaled model images`,
  `perf(phone): capture outside the manager lock` (if step 3 lands).
- Do NOT push or open a PR unless the operator instructed it.

## Steps

### Step 1: Decode once

Refactor the capture assembly so the integrity gate returns the decoded
image instead of discarding it:

- Change `decode_png_dimensions(png) -> Result<(u32,u32), DiagnosticEntry>`
  into `decode_capture(png) -> Result<image::DynamicImage, DiagnosticEntry>`
  (same validation: decode must succeed, dims nonzero, same
  `PhoneScreencapDecodeFailed` diagnostic on failure). Derive `(w, h)` from
  the returned image at the call sites.
- Change `composite_cursor` to operate on the decoded `DynamicImage`
  (`fn composite_cursor(image: &mut image::RgbaImage, point: PhonePoint)`),
  removing its internal decode. The final PNG/model encode happens once,
  after optional compositing (step 2 merges this with the downscale).
- Preserve the "compositing failure is non-fatal" behavior (today a failed
  decode/compose just leaves the PNG untouched).

**Verify**: `cargo nextest run -p sky-cua-service -E 'test(capture)'` → pass;
`grep -c load_from_memory crates/sky-cua-service/src/phone/manager/capture.rs`
→ exactly 1.

### Step 2: Downscaled model delivery, device-pixel mapping preserved

- Reuse the capture crate: call the downscale/encode helpers from
  `sky-cua-capture` (`prepare_model_capture_from_image` or the underlying
  resize+encode functions — read `crates/sky-cua-capture/src/lib.rs:169-255`
  and pick the narrowest reusable function; `sky-cua-service` already
  depends on `sky-cua-capture` for the browser lane — verify with
  `grep sky-cua-capture crates/sky-cua-service/Cargo.toml`, and add the dep
  only if missing).
- Deliver the downscaled image in `PhoneImage` with `mime_type` matching the
  chosen encoding and `width/height` = the **delivered** image dims. If
  consumers need the device dims too, add optional additive fields rather
  than repurposing existing ones — check every consumer first
  (`grep -rn "PhoneImage" crates/`).
- **Mapping invariant**: snapshot registration continues to use
  `device_size` exactly as today. The model receives a smaller image; the
  client-side coordinate scaling for phone actions must be checked: find how
  phone coordinate actions map model coordinates → device coordinates
  (grep `identity_mapping` and the snapshot record consumers in
  `phone/`). If actions assume the delivered image dims == device dims
  (identity), you must record the scale factor in the snapshot record the
  same way the desktop lane records `CoordinateSpace`/pixel size — study
  `record_from_mapping` (`phone/manager/snapshot`) and extend the record
  additively. If this turns out to require changing the wire shape of
  snapshot records consumed by the client, STOP and report the exact shape
  needed.
- Respect the same env knobs as the desktop lane
  (`SKY_CUA_MODEL_SCREENSHOT_FORMAT`, max width/height, quality) via the
  capture crate's existing resolution logic — do not re-parse env vars
  locally.
- Keep a bypass: if the resolved model max-dims are ≥ device dims, skip the
  resize (encode-only), so small phones don't pay a no-op resize.

**Verify**: `cargo nextest run -p sky-cua-service -p sky-cua-client` → all
pass. `phone/manager/tests.rs` has capture tests asserting on delivered
images — update expectations deliberately, and add: (a) delivered image dims
≤ configured max; (b) a coordinate action on a downscaled snapshot still
resolves to the correct device pixel (this is THE regression test for the
mapping invariant).

### Step 3: Narrow the phone lock

In `daemon.rs:243-256`: today the lock wraps `phone.handle(request)` for
every request type. Full lock-splitting is out of budget; do the targeted
version:

- Inside `PhoneManager::handle`'s screenshot/observe path (in
  `phone/manager/`), move the adb screencap + decode + composite + encode
  onto `tokio::task::spawn_blocking` per plan 005's pattern (the adb call is
  async subprocess I/O — keep it async, but the image work goes to
  spawn_blocking). This shortens the lock's *CPU-bound* span even though the
  lock itself still wraps the call.
- Do NOT attempt to release/re-acquire the manager lock around the adb
  round trip in this plan — the manager's session/snapshot state relies on
  the single-writer guarantee. Record it as the deferred follow-up.

**Verify**: `cargo nextest run -p sky-cua-service` → all pass.

## Test plan

- Step 1: existing capture tests keep passing with exactly one decode
  (grep gate above); add one test that a corrupt/truncated PNG still yields
  `PhoneScreencapDecodeFailed` (the integrity gate survives the refactor).
- Step 2: the two new tests described inline (dims cap, coordinate
  round-trip on a downscaled snapshot). Pattern:
  `phone/manager/tests.rs` existing capture cases.
- Live gates not run (state in report): real-device
  `scripts/live_phone_use_smoke.py`, emulator observe round-trip.

## Done criteria

- [ ] `cargo fmt --check && cargo nextest run` exits 0
- [ ] Exactly one `load_from_memory` in `phone/manager/capture.rs`
- [ ] Delivered phone model image honors `SKY_CUA_MODEL_SCREENSHOT_*` knobs (test proves dims cap)
- [ ] Coordinate-action round-trip test on a downscaled snapshot passes
- [ ] Truncated-PNG integrity test passes
- [ ] No changes to snapshot mapping registration semantics (`identity_mapping` call unchanged aside from plumbing)
- [ ] `advisor-plans/README.md` status row updated

## STOP conditions

- Phone coordinate actions turn out to consume the *delivered image* dims
  for mapping (not device dims) in a way that additive snapshot-record
  fields can't fix — the wire contract needs a design decision.
- `sky-cua-capture`'s helpers can't be reused without moving/duplicating the
  env-knob resolution (plan 007 territory) — report the coupling instead of
  copy-pasting the resolver.
- Any `phone/manager/tests.rs` failure whose expected values you'd have to
  change in a way that weakens an assertion (vs. updating a deliberate
  format change).

## Maintenance notes

- Deferred: true lock-splitting in `PhoneManager` (capture outside the
  manager lock with snapshot re-registration), and WebP-vs-JPEG default
  choice for phone (inherits the desktop default; the operator may want a
  phone-specific override someday).
- Reviewer scrutiny: the coordinate round-trip test is the load-bearing
  one — verify it actually downscales (delivered dims != device dims) or
  it proves nothing.
- Plan 007 (env-key dedup) touches `model_image.rs:18`'s duplicated env
  const; if 007 landed first, use the shared const it introduced.
