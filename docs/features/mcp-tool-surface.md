# Grouped MCP Tool Surface

## Status

Shipped. Last verified: 2026-06-22.

## Summary

`sky-cua-client mcp` now advertises one grouped tool surface. There is no
alternate public launch mode for the old names.
The registry exposes 34 tools by default, or 35 when `SKY_CUA_BROWSER_EVAL` is
enabled, while preserving desktop, browser, and Android phone workflows behind
grouped tools with static approval-safe annotations.

## Contract Surface

The machine authorities are:

- `crates/sky-cua-client/tests/fixtures/mcp_tool_surface_matrix.json` for exact
  public `tools/list` output across image capability and browser eval
  combinations.
- `crates/sky-cua-client/tests/fixtures/tool_contract.json` for
  grouped branch mappings, annotations, schemas, and response policy.
- `crates/sky-cua-client/tests/fixtures/call_cases.json` for minimal
  valid and invalid grouped branch calls.

Grouped response envelopes include `structuredContent.tool`, `branch`, and
`result`. Raw call arguments are validated against the advertised `inputSchema`
before handler dispatch; invalid requests return top-level `isError=true`,
`structuredContent.branch=null`, and `structuredContent.error` before service
dispatch. Branch-exact schemas reject unknown keys and fields from the wrong
surface/operation branch.

Grouped tools:

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
share that registry. Only advertised names are callable; any other name returns
`UnknownTool` before service dispatch. Browser eval availability is also frozen
in the MCP process and service daemon startup snapshot, so request-time
environment changes cannot alter the advertised/callable contract.

Installers persist only live launch policy fields through
`mcp-launch-policy.json`: `SKY_CUA_BROWSER_EVAL` and
`SKY_CUA_MODEL_SUPPORTS_IMAGES`. Removed profile state is ignored and not
emitted into new host configs.

## Source Paths

- `crates/sky-cua-client/src/mcp_tools/definitions.rs`
- `crates/sky-cua-client/src/mcp_tools.rs`
- `crates/sky-cua-client/src/mcp_server.rs`
- `scripts/install_mcp_server.py`
- `scripts/deploy_plugin.py`
- `scripts/probe_mcp_tool_surface.py`
- `skills/computer-use/SKILL.md`
- `skills/browser-use/SKILL.md`
- `skills/phone-use/SKILL.md`

## Verification

- `cargo fmt --check`
- `cargo test`
- `uv run ruff format --check scripts`
- `uv run ruff check scripts`
- `uv run basedpyright`
- `uv run pytest`
- `python3 scripts/build_plugin.py`
- `python3 scripts/deploy_plugin.py --local-install-host opencode`
- `python3 scripts/probe_mcp_tool_surface.py --installed` — exact 34-tool grouped surface
- `SKY_CUA_BROWSER_EVAL=on python3 scripts/probe_mcp_tool_surface.py --installed` — exact 35-tool grouped surface
- `python3 scripts/live_phone_use_smoke.py --profile adb-usb --serial emulator-5554 --installed` — 9 passed / 0 failed on `Pixel_9a`
- `uv run python scripts/run_gui_testing_vm_smoke.py --host 127.0.0.1 --port 22222 --user skycua ... --profile opencode-mcp --desktop-env KDE --wayland-display wayland-0 --sync-opencode-settings` — OpenCode Zenity and kdialog passed with action-tool evidence
- `cargo test -p sky-cua-client`
- `uv run pytest scripts/test_install_flows.py scripts/test_deploy_plugin.py scripts/test_gui_testing_vm.py scripts/test_live_phone_use_smoke.py scripts/test_probe_mcp_tool_surface.py`

The broader cross-desktop VM matrix remains a release gate for display-specific
runtime work, but the grouped installed MCP host path is proven through the
OpenCode VM smoke.

## Related

- `ROADMAP.md`
