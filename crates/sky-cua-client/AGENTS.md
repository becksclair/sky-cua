# sky-cua-client Guide

`sky-cua-client` is the MCP stdio client and service launcher. It exposes
the `computer-use` tool surface and translates service responses into MCP
text plus structured content. Useful local probes:

```bash
cargo run -p sky-cua-client -- mcp
cargo run -p sky-cua-client -- health
cargo run -p sky-cua-client -- get-app-state --detail compact --capture-screen if-changed
```

## Layout

- `src/mcp_server.rs` — JSON-RPC framing, session init, message read/write.
- `src/mcp_tools.rs` — tool definitions, schemas, argument parsing, handlers.
- `src/output_shapes.rs` — compact/full snapshot shaping (`AppStateDetail`,
  `compact_snapshot`), reused by both the MCP path and `src/operator_cli.rs`.
- `src/heuristics.rs` — app guidance lookup (`HeuristicsRegistry`); enrich
  snapshots through it, not hardcoded client prose.
- `src/service_launcher.rs` — service startup and Unix-socket client.

## Conventions

- Tool names are stable: `list_apps`, `get_app_state`, `click`,
  `perform_secondary_action`, `scroll`, `drag`, `type_text`, `press_key`,
  `set_value`.
- Return both operator-friendly `content` text and machine-usable
  `structuredContent`. Keep operator CLI output machine-friendly JSON and
  preserve `clear-portal-tokens` output compatibility.
- Keep `get_app_state` summaries explicit about portal lifecycle and
  downgrade diagnostics.
- Never break newline-delimited JSON-RPC support (Codex depends on it), and
  never use login shells in `.mcp.json` — startup stdout noise corrupts MCP
  framing.

## Gotchas

- `get_app_state detail: "compact"` must retain screenshot metadata, element
  indices/bounds, diagnostics, and app identity.
- Tool call success should not imply UI success; summaries should encourage
  fresh state checks where needed.
- If installed plugin startup fails, direct stdio probing of
  `bin/sky-cua-client mcp` is cheaper than guessing.
