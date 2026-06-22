# Canonical MCP tool surface

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

This document follows `plans/AGENTS.md` and the ExecPlan requirements in `~/.agents/PLANS.md`. It is intentionally self-contained: a future contributor should be able to finish this refactor from this file plus the working tree.

## Purpose / Big Picture

After this change, any host that launches `sky-cua-client mcp` sees one canonical Model Context Protocol tool surface for desktop, browser, and Android phone work. Model Context Protocol, or MCP, is the JSON-RPC protocol this project uses to advertise callable tools to hosts such as Codex, OpenCode, Pi, Claude Code, Claude Desktop, and OpenClaw.

The user-visible win is simple: agents no longer receive dozens of duplicate target-specific names that all mean “observe”, “click”, “type”, or “list”. They receive a smaller set of canonical tools whose names encode intent and whose arguments encode the target. A real user can see the result by running an installed server, sending `tools/list`, and observing exactly 34 tools by default, or 35 when browser JavaScript evaluation is explicitly enabled.

There is no compatibility profile, no opt-in profile, and no public fallback surface. Retired direct names such as old browser, phone, and desktop one-off tools must be rejected as `UnknownTool` before service dispatch. Private Rust handler functions may continue to delegate to existing typed request code, but public MCP definitions, bundled skills, installers, probes, docs, and live smoke drivers must speak the canonical names only.

The central design rule is still approval fidelity. MCP tool annotations are static metadata on each tool definition. Hosts use annotations such as `readOnlyHint`, `destructiveHint`, `idempotentHint`, and `openWorldHint` to decide approval behavior. Because annotations cannot vary by a tool's `surface` or `operation` argument, the canonical surface must not merge read-only observation with mutating input, must not merge local desktop or phone input with open-world browser page input, and must keep `browser_eval` separate because it executes JavaScript in real browser pages.

## Progress

- [x] (2026-06-21) Inventoried the public MCP surface and identified desktop, browser, phone, and diagnostics duplication as the main tool-count driver.
- [x] (2026-06-22) Implemented the first grouped surface and proved the basic registry, dispatch, response envelope, and host launch paths.
- [x] (2026-06-22) Pivoted per maintainer direction from a staged profile rollout to one canonical production surface. Removed the public profile registry, runtime/install profile policy, profile-shaped response fields, and duplicate schema advertisement modules. `tools/call` now accepts only names in the frozen session registry.
- [x] (2026-06-22) Removed the old policy-layer registry abstraction. Registry construction now builds the canonical public tool list directly from process config, model image capability, and browser eval enablement.
- [x] (2026-06-22) Removed installer, deploy, VM profile, and probe support for profile selection. Installers persist only remaining launch policy fields: browser eval and model image override.
- [x] (2026-06-22) Updated bundled computer, browser, and phone skills to describe canonical tool names only.
- [x] (2026-06-22) Renamed contract fixtures and docs/plans from migration-era filenames to neutral names: `tool_contract.json`, `call_cases.json`, `docs/features/mcp-tool-surface.md`, and `plans/mcp_tool_surface.md`.
- [x] (2026-06-22) Removed obsolete profile vocabulary from active docs, schema descriptions, tests, and generated fixtures while preserving the legitimate `detail="compact"` observation verbosity value.
- [x] (2026-06-22) Re-ran Rust and Python formatting, lint, typecheck, and unit tests after the cleanup: `cargo fmt --check`, `cargo test`, `uv run ruff format --check scripts`, `uv run ruff check scripts`, `uv run basedpyright`, and `uv run pytest`.
- [x] (2026-06-22) Built and deployed the plugin bundle, then proved the installed MCP server advertises canonical names only. `python3 scripts/probe_mcp_tool_surface.py --installed` reported 34 tools and rejected an old direct name as `UnknownTool`; `SKY_CUA_BROWSER_EVAL=on python3 scripts/probe_mcp_tool_surface.py --installed` reported 35 tools with the same rejection.
- [x] (2026-06-22) Re-ran Android emulator proof against the installed canonical surface. `Pixel_9a` booted as `emulator-5554`, and `python3 scripts/live_phone_use_smoke.py --profile adb-usb --serial emulator-5554 --installed` passed observe, screenshot, tap, current app, app list, and disconnect with 9 passed / 0 failed.
- [x] (2026-06-22) Recovered the VM with a hard destroy/start after soft reboot left the framebuffer inactive, confirmed Plasma/KWin on `wayland-0`, and reran `opencode-mcp`. Zenity and kdialog both passed through OpenCode with `tool_evidence=true` and `action_tool_evidence=true` in `/workspace/artifacts/opencode-zenity-smoke/20260622T190920Z` and `/workspace/artifacts/opencode-kdialog-smoke/20260622T190939Z`.
- [ ] Run one final review pass, update feature docs and `ROADMAP.md`, then retire this ExecPlan after code, docs, and live proof all land.

## Surprises & Discoveries

- Observation: Tool count dropped primarily by deduplicating targets, not by removing behavior.
  Evidence: The canonical registry still supports desktop observation and input, browser tab control and input, phone connection/app/input/notification flows, session presence, diagnostics, and optional browser eval while advertising 34 default names.

- Observation: Profile machinery became more dangerous than useful once the maintainer chose a clean break.
  Evidence: Keeping profile parsing, profile install state, inactive-profile errors, and dual skill instructions preserved old names in the exact places new agents learn behavior: `tools/list`, bundled skills, probes, and host launch configs.

- Observation: Some uses of the word `compact` are not profile residue.
  Evidence: `observe(detail="compact")`, `AppStateDetail::Compact`, and helpers such as `compact_snapshot` describe observation verbosity and response shaping, not an alternate MCP tool surface. These may remain unless a separate API rename is planned.

- Observation: Stale profile keys can exist in old install-state JSON.
  Evidence: The new installer reads only known current fields, so stale `tool_profile` entries are ignored and are not emitted into generated host configs.

## Decision Log

- Decision: Ship exactly one canonical public MCP surface.
  Rationale: Compatibility/profile support keeps duplicate concepts alive and makes agents learn a transition state. The maintainer explicitly does not need that support. The clean public contract is easier to validate and easier for new agents to use.
  Date/Author: 2026-06-22 / Bex + Codex

- Decision: Authorization is registry membership.
  Rationale: A tool is callable if and only if it appears in the session registry frozen during MCP `initialize`. Hidden aliases would bypass the public approval and schema contract.
  Date/Author: 2026-06-22 / Codex

- Decision: Keep `browser_eval` separate and explicitly enabled.
  Rationale: Browser eval executes arbitrary JavaScript in real user browser tabs. Folding it into a broader browser input tool would hide an open-world destructive trust boundary.
  Date/Author: 2026-06-22 / Codex

- Decision: Keep canonical tools split by approval class.
  Rationale: Observation, local navigation, local mutating input, local destructive input, and open-world browser actions carry different host approval semantics. A single universal `action(surface, operation)` tool would either lie in annotations or force every action through the broadest approval class.
  Date/Author: 2026-06-22 / Codex

- Decision: Canonical fixtures are the machine authority.
  Rationale: Human docs drift. Checked-in JSON fixtures can prove exact public `tools/list` order, exact schema text, branch call cases, duplicate-name absence, and browser-eval/image-capability variants.
  Date/Author: 2026-06-22 / Codex

## Outcomes & Retrospective

The refactor is code-complete enough for focused Rust and Python tests to pass on the canonical-only path, but it is not done until the installed artifact and real host smokes are rerun after the final cleanup. The main lesson from the pivot is that profile-era proof was useful during migration but became active risk once the desired end state changed to a clean break.

## Context and Orientation

The MCP server lives in `crates/sky-cua-client`. The main files are:

- `crates/sky-cua-client/src/mcp_tools/definitions.rs`, which builds public tool definitions and test fixtures.
- `crates/sky-cua-client/src/mcp_tools.rs`, which maps public MCP calls to typed service requests and wraps results in canonical response envelopes.
- `crates/sky-cua-client/src/mcp_server.rs`, which freezes the session registry during MCP `initialize` and serves `tools/list` and `tools/call`.
- `crates/sky-cua-client/tests/fixtures/mcp_tool_surface_matrix.json`, which records exact public `tools/list` output for image-capability and eval combinations.
- `crates/sky-cua-client/tests/fixtures/tool_contract.json`, which records canonical branch metadata.
- `crates/sky-cua-client/tests/fixtures/call_cases.json`, which records minimal valid and invalid branch calls.

Install and smoke tooling lives under `scripts/`. The important files are:

- `scripts/install_mcp_server.py`, which writes host configs and persisted launch policy.
- `scripts/deploy_plugin.py`, which builds and installs a local plugin host path.
- `scripts/probe_mcp_tool_surface.py`, which speaks stdio MCP to the server and checks the advertised/callable surface.
- `scripts/live_phone_use_smoke.py` and `scripts/run_gui_testing_vm_smoke.py`, which exercise real Android and VM-host workflows.

Bundled agent instructions live in `skills/computer-use/SKILL.md`, `skills/browser-use/SKILL.md`, and `skills/phone-use/SKILL.md`. Those files must teach only the canonical public tool names.

The canonical default tool names are:

`doctor`, `status`, `list_resources`, `observe`, `capture_screen`, `capture_desktop`, `setup_desktop`, `session_presence`, `activate_window`, `desktop_semantic`, `desktop_toggle`, `desktop_scroll`, `desktop_pointer`, `desktop_keyboard`, `desktop_action`, `desktop_set_value`, `browser_open`, `browser_claim_tab`, `browser_move_mouse`, `browser_navigate`, `browser_input`, `browser_scroll`, `phone_connection`, `phone_pair_wireless`, `phone_setup`, `phone_app_force_stop`, `phone_pointer`, `phone_keyboard`, `phone_notification_action`, `phone_notification_reply`, `phone_app_action`, `phone_app_install`, `phone_accessibility_tree`, and `phone_notifications`.

When `SKY_CUA_BROWSER_EVAL` is explicitly enabled with a truthy value accepted by the service, `browser_eval` is added as the 35th tool.

## Plan of Work

First, finish the canonical-only code cleanup. In `definitions.rs`, public descriptions and test names should say “canonical” when they describe the tool surface, and should avoid profile-era terms unless describing a still-valid concept such as `detail="compact"`. The old schema advertisement modules under `src/mcp_tools/browser/schema.rs` and `src/mcp_tools/phone/schema.rs` remain deleted. No code should expose profile selection, inactive-profile errors, or profile-specific registry builders.

Second, keep installer and probe behavior canonical-only. `scripts/install_mcp_server.py` should ignore stale profile state and should not write profile environment variables into generated host configs. `scripts/deploy_plugin.py` should not accept profile flags. `scripts/probe_mcp_tool_surface.py` should check that the canonical tools exist, that retired names fail as `UnknownTool`, and that eval changes the count only by adding `browser_eval`.

Third, make docs and bundled skills teach the same public contract. The feature doc at `docs/features/mcp-tool-surface.md` may keep its filename until plan retirement, but its title and content should describe the canonical surface. `ROADMAP.md` should point to the current plan and feature doc. Skills should use canonical names and should not instruct agents to branch between old and new names.

Fourth, prove the shipped artifact. Rebuild the plugin, install or deploy it through the host-owned path, and run the MCP probe against the installed server. Then run real Android emulator and VM smokes against the installed canonical tools. Evidence from source tests is not enough for this repo; the installed binary and host wrapper/config paths must be exercised.

## Concrete Steps

Run these commands from `/home/bex/projects/sky-cua`.

After changing Rust definitions or fixtures:

    SKY_CUA_UPDATE_MCP_FIXTURES=1 cargo test -p sky-cua-client fixture_matches
    cargo fmt --check
    cargo test -p sky-cua-client

After changing Python scripts:

    uv run ruff format --check scripts
    uv run ruff check scripts
    uv run basedpyright
    uv run pytest

After changing install/probe behavior or skills:

    python3 scripts/build_plugin.py
    python3 scripts/probe_mcp_tool_surface.py --installed

For Android proof, use the Android emulator QA lane and run the phone smoke against the emulator serial that is active on the machine. The accepted proof must include observe, screenshot, pointer, keyboard or app action, notification/app listing where available, and disconnect or refresh cleanup. If the emulator is unavailable, record the exact blocker and do not mark the plan complete.

For VM proof, use the repo VM smoke skill and run the relevant smoke matrix against the real installed host paths. At minimum, include one desktop pointer/keyboard flow, one OpenCode MCP host flow, and the Pi wrapper materialization readback if Pi auth is usable without unapproved OpenAI-key usage.

## Validation and Acceptance

The root acceptance test is a real installed `tools/list`. It must show 34 canonical tools by default and no retired direct names. With browser eval enabled at process startup, it must show the same tools plus `browser_eval` for a total of 35. Calling a retired direct name must return an MCP tool error whose structured code is `UnknownTool`, and service request logs or test doubles must prove no dispatch occurred.

Rust acceptance requires `cargo test -p sky-cua-client` to pass after fixture regeneration. Python acceptance requires full script lint, typecheck, and pytest to pass. Packaging acceptance requires `python3 scripts/build_plugin.py` to produce a staged bundle whose skills and host config templates contain no profile selection instructions.

Live acceptance requires at least one installed host probe and the real Android/VM smoke evidence described in `Concrete Steps`. If a live gate cannot run because of missing credentials, unavailable emulator, or VM infrastructure failure, record the blocker in `Outcomes & Retrospective`, keep the plan open, and do not call the architecture production-grade.

## Idempotence and Recovery

All commands in this plan are intended to be rerunnable. Fixture regeneration overwrites only checked-in fixture files. Installer and deploy commands should write host-owned configs atomically and should ignore stale profile state. If a host config still contains removed profile variables, rerun the installer/deployer after rebuilding the plugin; if it persists, treat that as a bug in the materialization path and fix the owner that writes the config.

Do not restore removed public names as a recovery path. If an agent or host still calls them, update the skill, probe, smoke, or host prompt that taught the old name.

## Artifacts and Notes

Useful expected evidence snippets:

    cargo test -p sky-cua-client
    test result: ok. 188 passed; 0 failed

    uv run pytest scripts/test_install_flows.py scripts/test_deploy_plugin.py scripts/test_gui_testing_vm.py scripts/test_live_phone_use_smoke.py scripts/test_probe_mcp_tool_surface.py
    188 passed

    python3 scripts/probe_mcp_tool_surface.py --installed
    canonical tools: 34
    retired direct names rejected as UnknownTool

Current Android evidence:

    python3 scripts/live_phone_use_smoke.py --profile adb-usb --serial emulator-5554 --installed
    PASS installed.tools_list canonical_phone_tools=16
    PASS adb-usb.phone_connect serial=emulator-5554
    PASS adb-usb.phone_observe profile=phone-sess-emulator-5554-... actions=13
    PASS adb-usb.phone_screenshot snapshot=phone-emulator-5554-... size=1080x2424
    PASS adb-usb.phone_tap snapshot=phone-emulator-5554-...
    PASS adb-usb.phone_app_current package=com.google.android.apps.nexuslauncher
    PASS adb-usb.phone_app_list count=2
    PASS adb-usb.phone_disconnect serial=emulator-5554
    RESULT full_phone_use_smoke passed=9 skipped=0 failed=0

Earlier VM blocker and recovery evidence:

    virsh --connect qemu:///session screenshot testing-vm artifacts/testing-vm/pre-recovery-screenshot.ppm
    Screenshot text: Display output is not active.

    uv run python scripts/run_gui_testing_vm_smoke.py --host 127.0.0.1 --port 22222 --user skycua ... --profile opencode-mcp --sync-opencode-settings
    opencode zenity smoke FAILED: /workspace/artifacts/opencode-zenity-smoke/20260622T185931Z
    dialog_dismissed=true, tool_evidence=true, action_tool_evidence=false
    Fixture stderr showed: Gtk-WARNING Failed to open display.

    virsh --connect qemu:///session destroy testing-vm
    virsh --connect qemu:///session start testing-vm
    ssh ... 'pgrep -a "kwin_wayland"; ls -l /run/user/1000/wayland-*'
    kwin_wayland --socket wayland-0

Current VM pass evidence:

    uv run python scripts/run_gui_testing_vm_smoke.py --host 127.0.0.1 --port 22222 --user skycua ... --profile opencode-mcp --desktop-env KDE --wayland-display wayland-0 --sync-opencode-settings
    opencode zenity smoke passed: /workspace/artifacts/opencode-zenity-smoke/20260622T190920Z
    action_tool_evidence=true, tool_evidence=true
    opencode kdialog smoke passed: /workspace/artifacts/opencode-kdialog-smoke/20260622T190939Z
    action_tool_evidence=true, tool_evidence=true

## Interfaces and Dependencies

`McpProcessConfig` in `crates/sky-cua-client/src/mcp_tools/definitions.rs` owns process-level MCP config. It should contain only current policy fields: browser eval enablement, optional model image override, and diagnostics for invalid current env values.

`McpToolRegistry` owns the ordered public tools and lookup set used by `tools/list` and `tools/call`. The registry must be built once for a session and must be the single source of callable names.

`canonical_tool_result` and `canonical_invalid_request_result` in `crates/sky-cua-client/src/mcp_tools.rs` define the public response envelope. Successful calls return `structuredContent.tool`, `structuredContent.branch`, and `structuredContent.result`. Invalid branch requests return `structuredContent.tool`, `structuredContent.branch: null`, and `structuredContent.error`.

`scripts/probe_mcp_tool_surface.py` is the fastest installed-surface proof. It should remain small, deterministic, and strict: missing canonical names, extra retired names, wrong counts, or retired names that dispatch are failures.

## Revision Note

2026-06-22: Rewrote this ExecPlan after the maintainer rejected ongoing compatibility/profile support. The plan now describes the canonical-only end state, keeps the prior implementation evidence only as current progress, and removes obsolete profile rollout requirements from the forward path.
