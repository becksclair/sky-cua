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
  policy), `_kwin_effect.py` (KWin effect deploy), `_companion.py` (Android
  phone-companion build/stage lane for `deploy_plugin.py`).
- Build/deploy/distribution entrypoints: `build_plugin.py`,
  `install_plugin.py`, `install_mcp_server.py`, `deploy_plugin.py` (fast local
  dev deploy as `sky-cua@local`), `package.py` (self-contained release
  tarball), `installer.py` / root `install.py` (clean-machine install, repo
  and bundle modes; no marketplace), `install_kwin_effect.py`.
- Live operator smokes are `live_*_smoke.py` and must fail honestly when app
  state is blocked. JSON schemas for final agent messages:
  `scripts/schemas/*.json`.
- Overlay motion evidence: `overlay_motion_animations.py` drives the overlay
  HOST directly on a private socket (no service, no real input; target is
  `sky-cua-overlay-host serve`) and records scripted glide/redirect/swipe/
  tap-settle scenarios to `artifacts/overlay-motion-animations/` (KDE
  ScreenCast portal primary — per-user restore token lives in that gitignored
  dir, never the repo — spectacle-stills fallback; recordings capture the
  live desktop and are never committed). Human-judged video/contact sheets;
  the structured pass/fail glide check is
  `live_agent_cursor_kde_smoke.py --mode layer-shell-motion-glide`. Shared
  pieces: `_kde_screencast.py` (portal recorder), `_contact_sheets.py`
  (montage/pagination, also used by `overlay_pointer_animations.py`).
- Live smokes gate on deploy freshness. `deploy_freshness.py` fingerprints the
  Rust runtime source (`crates/**` + `Cargo.{toml,lock}`) and every build/deploy
  stamps the client binary it produced. The gate is enforced at the shared
  launch choke points — every sky-cua MCP spawn (`_mcp_stdio.McpClient`) and
  agent launch (`_agent_mcp_smoke.run_agent`) — so *any* live smoke that
  exercises the runtime aborts (nonzero, with a redeploy hint) rather than
  testing a binary not built from the current source. Run
  `python3 scripts/deploy_plugin.py` (cua-deploy) first;
  `python3 scripts/deploy_freshness.py` is a standalone preflight; set
  `SKY_CUA_ALLOW_STALE_DEPLOY=1` only to intentionally override.
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
- Phone-use live smokes are device-dependent (adb + an Android device), not VM
  desktop smokes: `live_phone_use_smoke.py` (non-destructive `phone_*` tool
  driver), `live_phone_companion_setup_smoke.py` (cold-device companion setup,
  agent- or direct-driven, proven by adb ground truth plus a pure MCP probe with
  companion auto-install off), and `live_phone_workflow_smoke.py` (agentic
  Settings/Chrome workflows on a ready device, proven by the adb resumed-activity
  ground truth plus a pointer-overlay MCP probe). They SKIP honestly without a
  device/companion APK.
- Never read a child process stderr pipe before terminating a child that may
  still be running.

## Gotchas

- `pytest` must stay useful without launching desktop apps; live smokes are
  explicit scripts. Some require portal approval, KDE/Wayland, Xvfb,
  zenity, Krita, or Kate.
