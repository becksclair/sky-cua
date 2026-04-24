# Python Harness Guide

## Package Identity

`scripts/` contains Python build/install helpers, direct MCP smoke tests, rich `codex app-server` harnesses, and live desktop workflow probes.
Tooling is managed by root `pyproject.toml` with `uv`, Ruff, basedpyright, and pytest.

## Setup & Run

```bash
uv sync --dev
uv run ruff format scripts
uv run ruff check scripts
uv run basedpyright
uv run pytest
python3 scripts/build_plugin.py
python3 scripts/install_plugin.py --bundle-root dist/plugin/sky-cua
```

## Patterns & Conventions

- Shared plugin paths/config helpers live in `_plugin_bundle.py`.
- Shared `codex exec` harness behavior lives in `_codex_exec.py`.
- Shared rich app-server JSON-RPC behavior lives in `_app_server_harness.py`.
- Shared TIDAL constants and validation live in `_tidal_workflow.py`.
- Live operator smokes are named `live_*_smoke.py` and should fail honestly when app state is blocked.
- JSON schemas for final agent messages live in `scripts/schemas/*.json`.
- DO: Use `_app_server_harness.run_rich_app_server_turn` like `live_app_server_smoke.py` for installed-plugin acceptance.
- DO: Validate actual transcript tool calls with `require_computer_use_item`, not only final JSON.
- DO: Put pure helper tests in `test_python_harness_helpers.py` style; keep them free of desktop app requirements.
- DO: Use `Path` and subprocess lists like `build_plugin.py` and `install_plugin.py`.
- DON'T: Let harnesses fake plugin success through shell/process inspection when the test is about computer-use tools.
- DON'T: Read a child process stderr pipe before terminating when that child may still be running.
- DON'T: Add a new live smoke without naming the target app, artifact directory, and proof condition.

## Touch Points / Key Files

- Plugin bundle build: `build_plugin.py`
- Plugin install/config: `install_plugin.py`
- Bundle/config helpers: `_plugin_bundle.py`
- Rich app-server harness: `_app_server_harness.py`
- Codex exec harness: `_codex_exec.py`
- Minimal installed-plugin acceptance smoke: `live_app_server_smoke.py`
- TIDAL workflow harness: `live_app_server_tidal_playlist.py`
- Pure helper tests: `test_python_harness_helpers.py`

## JIT Index Hints

- Find harness entrypoints: `rg -n "def main\\(|if __name__" .`
- Find subprocess calls: `rg -n "subprocess\\.|Popen|run\\(" .`
- Find Codex config writes: `rg -n "config.toml|CODEX_HOME|features.apps|service_tier" .`
- Find app-server protocol handlers: `rg -n "thread/start|turn/start|mcpServer/elicitation|requestUserInput" .`
- Find schemas: `find schemas -type f -name "*.json" -maxdepth 1 -print`

## Common Gotchas

- `pytest` should stay useful without launching desktop apps; keep live smokes as explicit scripts.
- Some live smokes require portal approval, KDE/Wayland, Xvfb, zenity, Krita, Kate, or TIDAL.
- `codex exec` is diagnostic; rich `codex app-server` is the current installed-plugin acceptance lane.

## Pre-PR Checks

```bash
uv run ruff format --check scripts && uv run ruff check scripts && uv run basedpyright && uv run pytest
```
