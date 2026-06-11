# Python Harness Guide

`scripts/` contains Python build/install helpers, direct MCP smoke tests,
rich `codex app-server` harnesses, and live desktop workflow probes. Tooling
is managed by root `pyproject.toml` with `uv`, Ruff, basedpyright, and
pytest.

## Layout

- Shared helpers: `_plugin_bundle.py` (plugin paths/config),
  `_smoke_config.py` (live-smoke model/reasoning settings), `_codex_exec.py`
  (codex exec harness), `_codex_app_server.py` (shared codex app-server
  stdio JSON-RPC client), `_app_server_harness.py` (rich app-server turn
  policy), `_kwin_effect.py` (KWin effect deploy).
- Build/deploy entrypoints: `build_plugin.py`, `install_plugin.py`,
  `install_mcp_server.py`, `deploy_debug_plugin.py`,
  `deploy_release_plugin.py`, `install_kwin_effect.py`.
- Live operator smokes are `live_*_smoke.py` and must fail honestly when app
  state is blocked. JSON schemas for final agent messages:
  `scripts/schemas/*.json`.
- Pure helper tests stay free of desktop app requirements. Each subsystem
  gets a focused `test_<subsystem>.py` module (for example
  `test_plugin_bundle.py`, `test_gui_testing_vm.py`,
  `test_install_flows.py`); shared bundle-tree fixtures live in
  `_test_support.py`. Do not reintroduce a catch-all test module.

## Conventions

- Installed-plugin acceptance goes through
  `_app_server_harness.run_rich_app_server_turn` (see
  `live_app_server_smoke.py`); `codex exec` is a diagnostic probe only.
- Validate actual transcript tool calls with `require_computer_use_item`,
  not only the final JSON — harnesses must not fake plugin success through
  shell/process inspection when the test is about computer-use tools.
- New live smokes must name the target app, artifact directory, and proof
  condition.
- Never read a child process stderr pipe before terminating a child that may
  still be running.

## Gotchas

- `pytest` must stay useful without launching desktop apps; live smokes are
  explicit scripts. Some require portal approval, KDE/Wayland, Xvfb,
  zenity, Krita, or Kate.
