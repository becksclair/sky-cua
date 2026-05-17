# Plan: Rich AT-SPI Readback in Snapshots

## Goal

Expose proven text and numeric control readback in `get_app_state` snapshots so agents can inspect editable fields, avoid stale-text mistakes, and verify text-entry actions from structured state rather than screenshots alone.

## Status

Code complete and live-smoked on the Arch `testing-vm` Plasma session.

The first implementation is Linux AT-SPI only. Windows/UIA and other backends keep compiling with empty structured readback metadata until they add native extraction behind the same public model fields.

## Progress Ledger

Complete:

- `crates/sky-cua-platform/src/model.rs` expands `ElementNode` with optional/defaulted `text`, `numeric_value`, and `supports_editable_text`.
- `ElementNode.value` is now the short agent-facing summary: known editable text, including `Some("")` for known empty fields, or a numeric value summary when only AT-SPI Value data is available.
- `crates/sky-cua-linux/src/atspi/tree.rs` extracts best-effort Text, EditableText, and Value metadata without turning missing proxies into snapshot failures.
- Text content is capped at 4096 AT-SPI characters and selections are capped at 8.
- Password/protected/name-sensitive controls suppress content and do not fetch text content.
- Compact output preserves `value`, `text`, `numeric_value`, and `supports_editable_text`.
- MCP tool descriptions, `skills/computer-use-workflows/SKILL.md`, and the Codex exec/app-server prompt wrappers tell agents to inspect readback before replacing text and reacquire state afterward.
- Direct and agent smoke coverage exists for stale initial text, replacement text, and transcript-level `get_app_state` proof.

Partial:

- Linux AT-SPI numeric Value extraction is implemented, but live proof currently centers on `zenity` text-entry controls.
- Desktop-backend probe filtering is now safer for wrong-compositor sessions and conservative when desktop detection is unknown; broader registry/list/focus matrix proof remains tracked in `docs/gui-desktop-test-harness.md`.

Pending:

- Add native Windows/UIA readback extraction when that backend grows beyond fallback metadata.
- Add the text-readback direct and agent smokes to the curated VM runner profile set.
- Add broader live numeric-control proof when a stable desktop fixture is chosen.

## Implementation Notes

- Public model fields are backward-compatible: old `ElementNode` JSON without readback fields deserializes with absent metadata and `supports_editable_text = false`.
- `readback_summary` preserves sensitive suppression over numeric fallback, but non-sensitive text metadata with missing content can still fall back to numeric summary.
- The Linux extractor treats Text/EditableText/Value proxy failures as absent metadata. Normal controls often do not expose every interface.
- Sensitive controls preserve structural metadata such as character count/caret/selection when available, but `content` and `value` stay absent.

## Verification

Focused checks:

```bash
cargo fmt --check
cargo test -p sky-cua-platform element_node
cargo test -p sky-cua-client compact_element
cargo test -p sky-cua-linux atspi::tree::tests
cargo test -p sky-cua-linux windowing::registry::tests
cargo test -p sky-cua-linux portal::remote_desktop::tests::chooses_supported_portal_cursor_mode
uv run ruff check scripts/_text_readback_smoke.py scripts/live_app_server_text_readback_smoke.py scripts/live_codex_exec_text_readback_smoke.py
python3 -m py_compile scripts/_text_readback_smoke.py scripts/live_app_server_text_readback_smoke.py scripts/live_codex_exec_text_readback_smoke.py
```

Broader gates run during implementation:

```bash
cargo test --workspace
cargo clippy --workspace --all-targets
uv run pytest scripts/test_python_harness_helpers.py
python3 -m py_compile scripts/_app_server_harness.py scripts/_codex_exec.py scripts/live_desktop_smoke.py scripts/_text_readback_smoke.py scripts/live_app_server_text_readback_smoke.py scripts/live_codex_exec_text_readback_smoke.py
```

Live VM proof:

- Direct Plasma smoke: `scripts/live_desktop_smoke.py` with `SKY_CUA_SERVICE_SOCKET_PATH=/tmp/sky-cua-vm-text-readback-live-desktop.sock` proved initial `stale-smoke`, post-`set_value` `smoke-value`, and post-`type_text` `typed-smoke` readback.
- Codex exec smoke: `/workspace/artifacts/codex-e2e/codex-text-readback-smoke/20260517T041212Z`.
- Rich app-server smoke: `/workspace/artifacts/codex-e2e/app-server-text-readback-smoke/20260517T041242Z`.

The two agent smokes validate the transcript, not just the final model message: at least one `get_app_state` tool result must contain `stale-readback`, and a later `get_app_state` tool result must contain `verified-readback`.
