# sky-cua-client Guide

## Package Identity

`sky-cua-client` is the MCP stdio client and service launcher.
It exposes the `computer-use` tool surface and translates service responses into MCP text plus structured content.

## Setup & Run

```bash
cargo test -p sky-cua-client
cargo clippy -p sky-cua-client --all-targets
cargo run -p sky-cua-client -- mcp
```

## Patterns & Conventions

- MCP protocol handling belongs in `src/mcp_server.rs`.
- App guidance lookup belongs in `src/heuristics.rs`.
- Service startup and Unix-socket client behavior belong in `src/service_launcher.rs`.
- Keep tool names stable: `list_apps`, `get_app_state`, `click`, `perform_secondary_action`, `scroll`, `drag`, `type_text`, `press_key`, `set_value`.
- Return both operator-friendly `content` text and machine-usable `structuredContent`.
- DO: Add compact/full output behavior near `AppStateDetail` and `compact_snapshot` in `src/mcp_server.rs`.
- DO: Keep `get_app_state` summaries explicit about portal lifecycle and downgrade diagnostics.
- DO: Use `HeuristicsRegistry` to enrich snapshots with app guidance, not hardcoded client prose.
- DON'T: Break newline-delimited JSON-RPC support; Codex's stdio MCP path depends on it.
- DON'T: Use login shells in `.mcp.json`; startup stdout noise corrupts MCP framing.

## Touch Points / Key Files

- MCP server and tool definitions: `src/mcp_server.rs`
- App-guidance registry: `src/heuristics.rs`
- Service launcher/client: `src/service_launcher.rs`
- CLI entrypoint: `src/main.rs`
- Installed MCP config: `.mcp.json`

## JIT Index Hints

- Find MCP tools: `rg -n "tool_definitions|handle_tool_call|tool_error" src/mcp_server.rs`
- Find `get_app_state` output shaping: `rg -n "AppStateDetail|compact_snapshot|snapshot_summary" src/mcp_server.rs`
- Find action calls: `rg -n "handle_action_call|ActionName" src/mcp_server.rs`
- Find framing support: `rg -n "ContentLength|JsonLine|read_message|write_message" src/mcp_server.rs`
- Find heuristic loading: `rg -n "HeuristicsRegistry|app_guidance|markdown" src`

## Common Gotchas

- `get_app_state detail: "compact"` must retain screenshot metadata, element indices/bounds, diagnostics, and app identity.
- Tool call success should not imply UI success; summaries should encourage fresh state checks where needed.
- If installed plugin startup fails, direct stdio probing of `bin/sky-cua-client mcp` is cheaper than guessing.

## Pre-PR Checks

```bash
cargo test -p sky-cua-client && cargo clippy -p sky-cua-client --all-targets
```
