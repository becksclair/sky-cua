# Grouped MCP Tool Surface

## Status

The canonical grouped surface is shipped. Configurable surface projection was source-verified on 2026-08-07; installed projection proof awaits the next authorized deployment.

## Summary

`sky-cua-client mcp` now advertises one grouped tool surface. There is no
alternate public launch mode for the old names.
With all surfaces enabled, the registry exposes 40 tools by default, or 41 when
`SKY_CUA_BROWSER_EVAL` is enabled. `[surfaces]` in `sky-cua.toml` can independently
project desktop, browser, and phone subsets without introducing alternate tool
profiles; grouped tools retain static approval-safe annotations.

## Contract Surface

The machine authorities are:

- `crates/sky-cua-client/tests/fixtures/mcp_tool_surface_matrix.json` for exact
  public `tools/list` output across image capability and browser eval
  combinations.
- `crates/sky-cua-client/tests/fixtures/mcp_surface_policy_matrix.json` for all
  eight desktop/browser/phone surface combinations and their shared-tool schema
  projection.
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
`phone_app_install`, `phone_accessibility_tree`, `phone_notifications`,
`phone_content`, `phone_clipboard`, `phone_editor`, `phone_camera`, and
`phone_storage`.

## Behavior

The MCP registry is frozen during `initialize`; `tools/list` and `tools/call`
share that registry. Only advertised names are callable; any other name returns
`UnknownTool` before service dispatch. The resolved surface policy is frozen at
the same boundary. Disabled surface-specific names disappear, while `status`,
`list_resources`, `observe`, and `capture_screen` project only enabled branches,
properties, enums, validation, and descriptions. Browser eval availability is
also frozen and requires the browser surface to be enabled.

Installers persist only live launch policy fields through
`mcp-launch-policy.json`: `SKY_CUA_BROWSER_EVAL` and
`SKY_CUA_MODEL_SUPPORTS_IMAGES`. Removed profile state is ignored and not
emitted into new host configs.

## Source Paths

- `crates/sky-cua-client/src/mcp_tools/definitions/`
- `crates/sky-cua-client/src/mcp_tools.rs`
- `crates/sky-cua-client/src/mcp_server.rs`
- `scripts/install_mcp_server.py`
- `scripts/deploy_plugin.py`
- `scripts/probe_mcp_tool_surface.py`
- `skills/computer-use/SKILL.md`
- `skills/browser-use/SKILL.md`
- `skills/phone-use/SKILL.md`

## Verification

Current source verification (2026-08-07):

- `cargo fmt --check && cargo clippy --workspace --all-targets && cargo nextest run` — 1508 Rust tests passed.
- `uv run ruff format --check scripts resources/chrome_preflight.py && uv run ruff check scripts resources/chrome_preflight.py && uv run basedpyright && uv run pytest` — full Python gate passed after the surface/provisioning changes.
- `python3 scripts/build_plugin.py` — complete local plugin bundle staged successfully.
- Real stdio MCP probes against a private service socket passed for all-enabled, browser-only, desktop+phone, and desktop-only projections; the corresponding advertised counts with default browser eval were 41, 12, 34, and 16.
- A machine-config-only probe with `[surfaces] desktop=true, browser=false, phone=true` exposed 34 tools, `observe(surface=desktop|phone)`, phone-only `capture_screen`, and rejected `browser_input` as `UnknownTool` before service dispatch.
- An all-disabled machine-config probe advertised only `doctor`.

Historical installed-host proof from 2026-06-22 used the then-current 34/35-tool surface, before the later phone content/clipboard/editor/camera/storage tools were added. Android emulator and OpenCode VM acceptance from that release remain valid evidence for the canonical host path, but do not count as installed proof of the new configurable projection. The broader cross-desktop VM matrix remains a release gate for display-specific runtime work.

## Related

- `ROADMAP.md`
