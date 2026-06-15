# Claude Code host support

## Status

Shipped. Last verified: 2026-06-10 with the live Claude Code terminal MCP
smoke and the Pi MCP smoke after the agent-agnostic screenshot delivery
rework.

## Summary

sky-cua installs into Claude Code two ways: as a Claude Code plugin
(`.claude-plugin/plugin.json` with an inline MCP server entry plus bundled
skills) or through `scripts/install_mcp_server.py --host claude-code`, which
registers the `sky-cua` stdio server at user scope, syncs the
`computer-use`/`browser-use` skills into `~/.claude/skills`, and writes a
`~/.claude/settings.json` permission policy that denies Claude Code's built-in
`computer-use` MCP and auto-approves the sky-cua tools. No Claude-specific
runtime behavior exists; the same agent-agnostic MCP surface serves every
host.

## Contract surface

- `.claude-plugin/plugin.json` declares the plugin and an inline `mcpServers`
  entry launching `${CLAUDE_PLUGIN_ROOT}/bin/sky-cua-client mcp`. The
  `bin/sky-cua-client` wrapper resolves bundled runtimes first and falls back
  to `target/release` for built checkouts.
- `.claude-plugin/marketplace.json` exposes the repository (and the staged
  bundle) as a single-plugin marketplace so
  `claude plugin marketplace add <repo>` followed by
  `claude plugin install sky-cua` works.
- `scripts/install_mcp_server.py --host claude-code` writes
  `claude_code_mcp.json` for inspection, runs
  `claude mcp add-json --scope user sky-cua <config>` when the `claude`
  CLI is on `PATH`, and copies sky-cua skills into `~/.claude/skills` when
  `~/.claude` exists. The server is registered as `sky-cua` because Claude
  Code reserves the name `computer-use` for its native integration; the tool
  names are unchanged.
- `scripts/install_mcp_server.py --host claude-code` also merges a permission
  policy into `~/.claude/settings.json` (user scope, created when absent):
  `permissions.deny` gains `mcp__computer-use` and `mcp__computer-use__*` so
  Claude Code's built-in computer-use MCP cannot be called, and
  `permissions.allow` gains `mcp__sky-cua` and `mcp__sky-cua__*` so the sky-cua
  tools never prompt. The merge preserves existing settings and rules, is
  idempotent (no duplicate rules, no rewrite when already correct), and is
  non-fatal — a missing or malformed `settings.json` is reported and skipped,
  never aborting the install. The Claude config directory defaults to
  `~/.claude` and is overridable end to end with `--claude-config-dir` (on both
  `install_mcp_server.py` and the `installer.py` orchestrator); the
  `health:claude-code-permissions` check reads the same directory the install
  wrote, so it attests the actual file rather than a hardcoded path.
- Claude Code stdio MCP servers inherit the parent process environment, so
  the desktop-session env-var passthrough list required by Codex is
  unnecessary; the generated config pins only `SKY_CUA_REPO_ROOT` and an
  explicit `SKY_CUA_BROWSER` selection when present.
- `--kwin-effect` on the installer also builds, installs, and reloads the
  KWin agent-cursor effect on Plasma hosts, so one command refreshes binaries,
  the Claude registration, and the compositor effect:
  `python3 scripts/install_mcp_server.py --host claude-code --restart-runtime --kwin-effect`.
  Effect updates that cannot hot-reload notify the user to restart the Plasma
  session when convenient; KWin is never restarted by the tooling.
- Screenshot delivery is host-portable rather than Claude-specific:
  `browser_screenshot` attaches an MCP image content block and persists the
  capture to `screenshot_path`; `get_app_state` accepts
  `screenshot_delivery: "inline"` for sessions that cannot read files by
  path.

## Behavior

The plugin lane loads skills from the plugin's `skills/` directory and starts
the MCP server from the plugin root. The installer lane copies platform
binaries to the install target, then registers the server at user scope so
every project sees the `computer-use` tools. Both lanes run the same
`sky-cua-client mcp` entrypoint and the same long-lived service; the runtime
adapts to missing model-capability metadata by assuming image support, which
matches Claude Code's vision-capable models.

## Source paths

- `.claude-plugin/plugin.json`, `.claude-plugin/marketplace.json`
- `scripts/install_mcp_server.py` (`install_claude_code`)
- `scripts/build_plugin.py` (`BUNDLE_SOURCE_PATHS`, `WORKTREE_BUNDLE_FILES`)
- `scripts/_plugin_bundle.py` (`ensure_bundle_structure`,
  `update_plugin_manifest_version`)
- `CLAUDE.md` (imports `AGENTS.md` for Claude Code sessions in this repo)

## Verification

- `uv run pytest scripts/test_plugin_bundle.py` covers bundle
  structure, manifest version bumps, and bundle source selection including
  `.claude-plugin`.
- Live smoke: `--host claude-code` install followed by a headless
  `claude -p` session running `list_apps`/`get_app_state` through the
  registered `sky-cua` server, plus the direct stdio `browser_screenshot`
  probe.

## Known limitations

- Plugin installs from a raw, unbuilt checkout have no runtime binaries; the
  wrapper needs either bundled runtimes (release bundle) or
  `cargo build --release` artifacts.
- The installer only auto-registers when a `claude` executable is on `PATH`;
  otherwise it prints the exact `claude mcp add-json` command to run.
- The repository's own project `.mcp.json` keeps the `computer-use` server
  name for the Codex Desktop contract, so Claude Code sessions in this repo
  skip it with a reserved-name warning and use the user-scope `sky-cua`
  registration (or the plugin) instead.
- Claude Code caps MCP tool output tokens (`MAX_MCP_OUTPUT_TOKENS`); prefer
  compact `get_app_state` snapshots with `element_query`/`element_limit`, and
  `browser_snapshot` element filters on dense pages.

## Related

- [`docs/features/codex-desktop-compat.md`](codex-desktop-compat.md)
- [`docs/features/browser-mcp-tools.md`](browser-mcp-tools.md)
- [`docs/runtime/mcp-boundary.md`](../runtime/mcp-boundary.md)
