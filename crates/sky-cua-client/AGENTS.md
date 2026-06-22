# sky-cua-client Guide

`sky-cua-client` is the MCP stdio client and service launcher. It exposes the
canonical sky-cua MCP tool surface and translates service responses into MCP
text plus structured content. Useful local probes:

```bash
cargo run -p sky-cua-client -- mcp
cargo run -p sky-cua-client -- health
cargo run -p sky-cua-client -- get-app-state --detail compact --capture-screen if-changed
```

## Layout

- `src/mcp_server.rs` — JSON-RPC framing, session init, message read/write.
- `src/mcp_tools.rs` — tool dispatch, argument parsing, and response shaping.
- `src/mcp_tools/definitions.rs` — advertised canonical tool definitions and
  schemas.
- `src/output_shapes.rs` — compact/full snapshot shaping (`AppStateDetail`,
  `compact_snapshot`), reused by both the MCP path and `src/operator_cli.rs`.
- `src/heuristics.rs` — app guidance lookup (`HeuristicsRegistry`); enrich
  snapshots through it, not hardcoded client prose.
- `src/service_launcher.rs` — service startup and Unix-socket client.

## Conventions

- Public MCP tool names are canonical and grouped by surface: `status`,
  `list_resources`, `observe`, `capture_screen`, `capture_desktop`,
  `desktop_*`, `browser_*`, and `phone_*`.
- Return both operator-friendly `content` text and machine-usable
  `structuredContent`. Keep operator CLI output machine-friendly JSON and
  preserve `clear-portal-tokens` output compatibility.
- Keep desktop observation summaries explicit about portal lifecycle and
  downgrade diagnostics.
- Never break newline-delimited JSON-RPC support (Codex depends on it), and
  never use login shells in `.mcp.json` — startup stdout noise corrupts MCP
  framing.

## Gotchas

- `observe(surface="desktop", detail="compact")` must retain screenshot
  metadata, element indices/bounds, diagnostics, and app identity.
- Tool call success should not imply UI success; summaries should encourage
  fresh state checks where needed.
- If installed plugin startup fails, direct stdio probing of
  `bin/sky-cua-client mcp` is cheaper than guessing.
