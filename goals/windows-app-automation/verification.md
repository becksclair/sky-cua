# Verification: First-class Windows app automation

## Commands

| Command | Purpose | Expected pass condition | Evidence location |
| --- | --- | --- | --- |
| `cargo fmt --check` | Confirm Rust formatting after backend changes. | Exits 0. | `progress.jsonl` |
| `cargo test -p sky-cua-windows` | Validate focused Windows backend behavior. | Exits 0 on this Windows host. | `progress.jsonl` |
| `cargo test -p sky-cua-platform` | Validate shared model compatibility if platform contracts move. | Exits 0. | `progress.jsonl` |
| `cargo test` | Catch cross-crate regressions after shared or packaging-impacting changes. | Exits 0, or unrelated platform blocker is documented with exact failure. | `progress.jsonl` |
| `uv run ruff format --check scripts` | Confirm Python harness formatting if packaging scripts move. | Exits 0. | `progress.jsonl` |
| `uv run ruff check scripts` | Confirm Python lint for packaging harness changes. | Exits 0. | `progress.jsonl` |
| `uv run basedpyright` | Confirm Python type checks for packaging harness changes. | Exits 0. | `progress.jsonl` |
| `uv run pytest` | Confirm Python harness tests after package/install path changes. | Exits 0. | `progress.jsonl` |
| `python scripts/build_plugin.py` | Build the plugin bundle with the updated Windows backend. | Produces a valid staged plugin bundle. | `progress.jsonl` |
| `python scripts/deploy_release_plugin.py --no-build` | Install the built release bundle into the local Codex plugin cache. | Exits 0 and enables `sky-cua@sky-cua-local`. | `progress.jsonl` |

## Manual Checks

- Use the installed `computer-use` MCP tools to run `list_apps` and confirm real Windows applications are visible.
- Run `get_app_state` against a known UI Automation-capable Windows app and confirm it returns more than a single fallback window element.
- Run `get_app_state` against Sumwall Browser and Microsoft Edge. Record semantic backend, element count, capture backend, screenshot path, and diagnostics.
- Exercise app-shell actions that are not website automation: focus address bar, open a menu, switch or create a tab, and recover focus without changing user data.
- If Edge or another browser-like app still captures black, record the explicit diagnostic and prove that keyboard or semantic state still reflects the target app.
- Rebuild and install the release plugin, then repeat one focused live smoke from the installed plugin instead of the workspace binary.

## Evidence Rules

- Record verification results in `progress.jsonl`.
- Include command, status, timestamp, and artifact path when available.
- Do not rely on passing tests unless they cover the requirement being claimed.
