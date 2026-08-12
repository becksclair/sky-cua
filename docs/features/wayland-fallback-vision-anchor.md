# Wayland fallback vision anchor

## Status

Shipped. Live-proven through the full agent loop on KDE Plasma Wayland on
2026-07-08; source tests re-verified in the current tree on 2026-08-11.

## Summary

When a native Wayland window has no usable AT-SPI tree, desktop observation
returns one honest window-sized vision anchor alongside the screenshot. This
gives screenshot-guided agents a physical target without inventing semantic
controls the application never exposed.

## Contract surface

- The fallback element has role `window` and carries `vision_anchor`,
  `native_window_fallback`, and physical-target state.
- The element bounds are the real native window bounds.
- No synthetic buttons, navigation regions, or other semantic children are
  exposed.
- The ordinary snapshot and screenshot coordinate contracts remain unchanged.

## Behavior

KWin window evidence selects the native window. If app correlation cannot
produce richer AT-SPI elements, `linux_window_elements` emits a single fallback
anchor. Agents use its screenshot and snapshot-scoped pixel coordinates for
physical interaction.

The deterministic `fallback-anchor` smoke launches an AT-SPI-dark mpv window,
drives an agent through resource discovery and observation, and accepts only
raw tool evidence containing both fallback flags with no richer AT-SPI role.

## Source paths

- `crates/sky-cua-linux/src/backend/elements.rs` — fallback element shape
- `crates/sky-cua-linux/src/backend/tests.rs` — honest-anchor regression
- `scripts/live_fallback_anchor_smoke.py` — mpv fixture and evidence gate
- `scripts/live_agentic_loop_smoke.py` — agent-loop profile routing
- `scripts/test_live_smoke_helpers.py` — fixture and transcript-gate tests

## Verification

- Rust tests prove the fallback exposes the single honest anchor and no
  invented sub-elements.
- Python tests prove fixture selection, launch shape, structured evidence, and
  text transcript recognition.
- On 2026-07-08,
  `python3 scripts/live_agentic_loop_smoke.py --agent opencode --fixture fallback-anchor`
  passed on the installed singleton daemon and returned
  `fallback_proved: true`.

## Known limitations

- The anchor supplies geometry, not accessibility semantics; control discovery
  still depends on screenshot interpretation.
- The proving fixture is KDE/KWin-specific. Other Wayland compositors require
  their own native-window evidence and live acceptance.

## Related

- ROADMAP entry: [`ROADMAP.md`](../../ROADMAP.md) under "Linux desktop parity"
- [`docs/runtime/linux-architecture.md`](../runtime/linux-architecture.md)
- Originating ExecPlan retired into this feature doc; see git history for
  `plans/wayland_fallback_vision_anchors.md`.
