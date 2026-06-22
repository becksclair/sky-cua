# Compact MCP Tool Surface

## Status

Partial. Last verified: 2026-06-22 on branch `bex/compact-mcp-tool-surface`.

## Summary

`sky-cua-client mcp` now supports an opt-in `compact` tool profile through
`SKY_CUA_MCP_TOOL_PROFILE=compact`. Legacy remains the default. Compact
advertises 34 tools by default, or 35 when `SKY_CUA_BROWSER_EVAL` is enabled,
while preserving desktop, browser, and Android phone workflows behind grouped
tools with static approval-safe annotations.

## Contract Surface

The machine authorities are:

- `crates/sky-cua-client/tests/fixtures/mcp_tool_surface_matrix.json` for exact
  public `tools/list` output across `legacy`/`compact`, image capability, and
  browser eval combinations.
- `crates/sky-cua-client/tests/fixtures/compact_tool_contract.json` for compact
  branch-to-legacy mappings, annotations, schemas, and response policy.
- `crates/sky-cua-client/tests/fixtures/compact_call_cases.json` for minimal
  valid and invalid compact branch calls.

Compact response envelopes set `structuredContent.profile="compact"` and include
`tool`, `branch`, `legacy_tool`, and `result`. Invalid compact branch requests
return a compact error envelope before service dispatch.

Compact tools:

`doctor`, `status`, `list_resources`, `observe`, `capture_screen`,
`capture_desktop`, `setup_desktop`, `session_presence`, `activate_window`,
`desktop_semantic`, `desktop_toggle`, `desktop_scroll`, `desktop_pointer`,
`desktop_keyboard`, `desktop_action`, `desktop_set_value`, `browser_open`,
`browser_claim_tab`, `browser_move_mouse`, `browser_navigate`,
`browser_input`, `browser_scroll`, optional `browser_eval`,
`phone_connection`, `phone_pair_wireless`, `phone_setup`,
`phone_app_force_stop`, `phone_pointer`, `phone_keyboard`,
`phone_notification_action`, `phone_notification_reply`, `phone_app_action`,
`phone_app_install`, `phone_accessibility_tree`, and `phone_notifications`.

## Behavior

The MCP registry is frozen during `initialize`; `tools/list` and `tools/call`
share that registry, and inactive profile names are rejected before service
dispatch. Browser eval availability is also frozen in the MCP process and in the
service daemon startup snapshot, so request-time environment changes cannot
alter the advertised/callable contract.

Installers persist launch policy through `mcp-launch-policy.json`, resolving per
field as CLI, persisted state, recognized environment, then defaults. The
recognized policy environment is `SKY_CUA_MCP_TOOL_PROFILE`,
`SKY_CUA_BROWSER_EVAL`, and `SKY_CUA_MODEL_SUPPORTS_IMAGES`.

## Source Paths

- `crates/sky-cua-client/src/mcp_tools/definitions.rs`
- `crates/sky-cua-client/src/mcp_tools.rs`
- `crates/sky-cua-client/src/mcp_server.rs`
- `crates/sky-cua-service/src/daemon.rs`
- `scripts/install_mcp_server.py`
- `scripts/_openclaw_install.py`
- `scripts/deploy_plugin.py`
- `scripts/deploy_freshness.py`
- `scripts/probe_mcp_tool_surface.py`
- `resources/chrome_preflight.py`

## Verification

- `cargo test -p sky-cua-client`
- `cargo test -p sky-cua-service`
- `uv run pytest scripts/test_install_flows.py`
- `uv run pytest scripts/test_openclaw_install.py`
- `uv run pytest scripts/test_deploy_freshness.py`
- `uv run pytest scripts/test_probe_mcp_tool_surface.py`
- `uv run ruff check ...`
- `uv run basedpyright`

Direct and staged-installed stdio probes verified compact/legacy profile
isolation, compact `tools/list` count, compact success/error response envelopes,
inactive-tool rejection, and degraded desktop/browser/phone status branches.

## Known Limitations

Host-level agent approval smoke, full installed Codex/OpenCode validation,
Android emulator proof, and the full GUI VM smoke matrix are still pending.

## Related

Originating ExecPlan: `plans/compact_mcp_tool_surface.md`.
