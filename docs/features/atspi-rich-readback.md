# AT-SPI rich readback in snapshots

## Status

Shipped on Linux and Windows. Linux AT-SPI extraction is complete; Windows
UIA populates the same public fields from ValuePattern, TextPattern, and
RangeValuePattern. Last live-verified: Linux on 2026-05-17 and Windows on
2026-07-07.

## Summary

`get_app_state` snapshots expose proven text and numeric control readback
so agents can inspect editable fields, avoid stale-text mistakes, and verify
text-entry actions from structured state rather than relying on screenshots
alone.

## Contract surface

Public model in `crates/sky-cua-platform/src/model.rs`:

- `ElementNode.value: Option<String>` — agent-facing summary. Known editable
  text, including `Some("")` for known empty fields, or a numeric value
  summary when only AT-SPI Value data is available.
- `ElementNode.text: Option<TextReadback>` — full text metadata
  (`content`, character count, caret, selections).
- `ElementNode.numeric_value: Option<NumericValueReadback>` — value, range,
  step.
- `ElementNode.supports_editable_text: bool` — defaults to `false`; old
  snapshots without these fields deserialize correctly.

Compact `get_app_state` output preserves `value`, `text`, `numeric_value`,
and `supports_editable_text` so agents can verify text entry without
requesting full snapshots every loop.

## Behavior

- Linux extraction is best-effort: missing Text, EditableText, or Value
  proxies do not turn into snapshot failures. Most controls do not expose
  every interface.
- Text content is capped at 4096 AT-SPI characters; selections capped at 8.
- Password / protected / name-sensitive controls suppress content. Sensitive
  controls preserve structural metadata (character count, caret, selection)
  when available, but `content` and `value` stay absent.
- `readback_summary` preserves sensitive suppression over numeric fallback,
  but non-sensitive text metadata with missing content can still fall back
  to a numeric summary.

## Source paths

- `crates/sky-cua-platform/src/model.rs` — public fields
- `crates/sky-cua-linux/src/atspi/tree.rs` — extraction logic
- `crates/sky-cua-client/src/mcp_server.rs` — compact-snapshot preservation
- `skills/computer-use/SKILL.md` — agent guidance to inspect
  readback before replacing text and reacquire state afterward
- `scripts/_text_readback_smoke.py`,
  `scripts/live_app_server_text_readback_smoke.py`,
  `scripts/live_codex_exec_text_readback_smoke.py` — readback smokes

## Verification

Focused unit tests:

```bash
cargo test -p sky-cua-platform element_node
cargo test -p sky-cua-client compact_element
cargo test -p sky-cua-linux atspi::tree::tests
```

Live VM proof:

- Direct Plasma smoke: `scripts/live_desktop_smoke.py` proved initial
  `stale-smoke`, post-`set_value` `smoke-value`, and post-`type_text`
  `typed-smoke` readback against a `zenity` text entry.
- Codex exec smoke: `/workspace/artifacts/codex-e2e/codex-text-readback-smoke/20260517T041212Z`
- Rich app-server smoke: `/workspace/artifacts/codex-e2e/app-server-text-readback-smoke/20260517T041242Z`

The two agent smokes validate the transcript, not just the final model
message: at least one `get_app_state` tool result must contain
`stale-readback`, and a later `get_app_state` tool result must contain
`verified-readback`.

Windows live proof:

- Notepad editable text populated `text` and `supports_editable_text`.
- Mouse Properties pointer speed populated `numeric_value` from RangeValue
  with current value 50 and range 0–100.

## Known limitations

- The top-level Win32 fallback node has no UIA control pattern and therefore
  has no structured text or numeric readback. Descendant UIA controls expose
  the metadata supported by their patterns.
- Live numeric Value extraction is implemented but live proof currently
  centers on `zenity` text-entry controls. Broader live numeric-control
  proof needs a stable desktop fixture.
- The direct text-readback smoke rides the curated VM runner set as the
  `text-readback` profile (`scripts/live_text_readback_smoke.py`); the
  agent-harness readback smokes remain manual lanes.

## Related

- ROADMAP entry: [`ROADMAP.md`](../../ROADMAP.md) under "Linux desktop parity"
- Originating ExecPlan (retired into this feature doc; see git history for `plans/rich_atspi_readback.md`).
