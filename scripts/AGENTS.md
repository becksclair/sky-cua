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
- Build/deploy/distribution entrypoints: `build_plugin.py`,
  `install_plugin.py`, `install_mcp_server.py`, `deploy_plugin.py` (fast local
  dev deploy as `sky-cua@local`), `package.py` (self-contained release
  tarball), `installer.py` / root `install.py` (clean-machine install, repo
  and bundle modes; no marketplace), `install_kwin_effect.py`.
- Live operator smokes are `live_*_smoke.py` and must fail honestly when app
  state is blocked. JSON schemas for final agent messages:
  `scripts/schemas/*.json`.
- Pure helper tests stay free of desktop app requirements. Each subsystem
  gets a focused `test_<subsystem>.py` module (for example
  `test_plugin_bundle.py`, `test_gui_testing_vm.py`,
  `test_install_flows.py`); shared bundle-tree fixtures live in
  `_test_support.py`. Do not reintroduce a catch-all test module.

## Conventions

- Installed-plugin agent-loop acceptance goes through
  `live_agentic_loop_smoke.py`, which drives the installed MCP server through
  an external agent CLI. `codex exec` and `codex app-server` harnesses are
  diagnostic probes only.
- Pi agent-loop acceptance enforces MCP use with the JSON/stdout tool-evidence
  checks in `live_agent_mcp_smoke.py`, not only the final JSON. Other agent
  lanes must add equivalent enforcement before being treated as acceptance.
  For app-server diagnostic transcripts, use `require_computer_use_item`.
  Harnesses must not fake plugin success through shell/process inspection when
  the test is about computer-use tools.
- New live smokes must name the target app, artifact directory, and proof
  condition.
- Never read a child process stderr pipe before terminating a child that may
  still be running.

## Gotchas

- `pytest` must stay useful without launching desktop apps; live smokes are
  explicit scripts. Some require portal approval, KDE/Wayland, Xvfb,
  zenity, Krita, or Kate.
