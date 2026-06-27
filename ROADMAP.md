# sky-cua roadmap

This file is the curated index of active workstreams. For durable behavior,
see [`docs/features/`](docs/features/). For tactical memory, see
[`NOTES.md`](NOTES.md). For active forward-looking design, see
[`plans/`](plans/).

Closed boxes link to the feature doc that describes the shipped behavior.
Open boxes link to the active ExecPlan that owns the work.

## Phase: Linux desktop parity

- [x] AT-SPI rich readback — [`docs/features/atspi-rich-readback.md`](docs/features/atspi-rich-readback.md)
- [x] Detached session-env repair — [`docs/features/session-env-repair.md`](docs/features/session-env-repair.md)
  - [x] Add stripped-env repair to the curated VM runner profile set
        (`session-env` profile, curated member)
  - [x] Non-Codex host smoke (OpenCode/Pi) once VM lane is live
- [x] Session presence (Linux unlock plus lock/suspend inhibition, MCP tools,
      and CLI surface) — [`docs/features/session-presence.md`](docs/features/session-presence.md)
  - [x] Windows power-request inhibition backend (verified live on the
        devbox VM; display inhibition untestable over SSH)
- [x] KWin and X11 workspace metadata — [`docs/features/kwin-x11-workspace-metadata.md`](docs/features/kwin-x11-workspace-metadata.md)
  - [ ] Capture a dedicated `list_resources(surface="desktop", resource="windows")` workspace artifact on real KWin and X11
- [x] KWin window targeting (focused_window + verified activation via KWin scripting) — [`docs/features/kwin-window-targeting.md`](docs/features/kwin-window-targeting.md)
- [x] Linux virtual input (`LinuxVirtualInput` backend) — [`docs/features/linux-virtual-input.md`](docs/features/linux-virtual-input.md)
  - [ ] Privileged helper fast path for Wayland input — [`plans/privileged_linux_uinput_helper.md`](plans/privileged_linux_uinput_helper.md)
- [x] Native agent cursor overlay — [`docs/features/agent-cursor-overlay.md`](docs/features/agent-cursor-overlay.md)
  - [x] WGPU-only Wayland layer-shell visible renderer with evented pointer tracking; legacy SHM, GNOME actor, and X11 shaped-window renderers retired to honest unsupported/Noop reporting
  - [ ] Follow-on X11 WGPU visible overlay host
  - [ ] Follow-on no-no input interception and optional sound feedback
- [x] Compositor cursor hiding (KWin / Hyprland / COSMIC adapters; X11/GNOME values retained only for compatibility) — [`docs/features/compositor-cursor-hiding.md`](docs/features/compositor-cursor-hiding.md)
  - [x] Local KWin cursor shim deploy/update tooling (`install_kwin_effect.py`, `--kwin-effect` deploy flags, rotating generated ids, BuildId convergence — tooling never restarts KWin)
  - [x] Unpatched COSMIC transparent-Xcursor mode (VM-only fallback)
  - [ ] Long-term unpatched COSMIC path or accepted upstream integration
- [ ] Wayland fallback vision anchors — [`plans/wayland_fallback_vision_anchors.md`](plans/wayland_fallback_vision_anchors.md)
  - [ ] Choose a current fallback-only target app to replace the retired TIDAL flow
  - [ ] Live agent-loop or app-server proof on the new target
- [x] CDUL-inspired Linux enhancements (terminal `command_line` fidelity,
      granular input doctor diagnostics, AT-SPI app-root prefiltering,
      GNOME setup-message polish) —
      [`docs/features/linux-targeting-and-diagnostics.md`](docs/features/linux-targeting-and-diagnostics.md)

## Phase: Windows parity

- [x] Windows UIA inspection and semantic actions — [`docs/features/windows-uia-automation.md`](docs/features/windows-uia-automation.md)
  - First-class `focus_element`, `activate_element`, `select_element`,
    `expand_element`, `collapse_element`, `toggle_element` against UIA patterns
- [ ] Windows capture ladder (WGC / DXGI before GDI) — [`plans/windows_capture_ladder.md`](plans/windows_capture_ladder.md)
- [ ] Windows agent cursor overlay and host IPC
  - [x] Refactor overlay host platform modules so Linux-only cursor and
        compositor code is cfg-scoped outside shared imports and contracts
  - [x] Split service overlay transport boundaries by platform: Unix domain
        socket client on Unix, localhost TCP (`serve --tcp`) selected as the
        non-Unix transport for `sky-cua-overlay-host.exe`; Windows-target
        service compilation proves the boundary
  - [ ] Add a Windows visible overlay host using a transparent, click-through,
        topmost layered window that renders the agent cursor without stealing
        focus
  - [ ] Prove Windows overlay behavior with target checks and a Windows VM:
        topmost visibility, click-through behavior, cursor position updates,
        DPI/multi-monitor handling, and cleanup on host exit
  - [ ] Consider global native cursor hiding only as an explicitly risky,
        opt-in backend after visible overlay support is live; avoid making
        brittle process-local `ShowCursor(FALSE)` behavior the default design
- [ ] Broader Windows app-shell live smokes (Edge, Sumwall, others) — [`plans/windows_app_shell_smokes.md`](plans/windows_app_shell_smokes.md)
- [ ] Native Windows/UIA readback (`text`, `numeric_value`, `supports_editable_text`)

## Phase: Host portability

- [x] Codex Desktop compatibility (one active `computer-use` server, Browser Use companion, native-host preflight) — [`docs/features/codex-desktop-compat.md`](docs/features/codex-desktop-compat.md)
- [x] OpenCode/Pi MCP host smoke parity
- [x] Claude Code host support (plugin manifest + marketplace, `--host claude-code` installer, `~/.claude/skills` sync, `~/.claude/settings.json` deny-built-in-computer-use + auto-approve sky-cua) — [`docs/features/claude-code-host.md`](docs/features/claude-code-host.md)
- [x] Canonical MCP tool surface — [`docs/features/mcp-tool-surface.md`](docs/features/mcp-tool-surface.md)
  - [x] Installed MCP probe, Android emulator phone smoke, and OpenCode VM
        host smoke passed on the canonical surface
  - [ ] Broader cross-desktop VM matrix remains a release gate for
        display-specific runtime changes
- [x] Agent-agnostic screenshot delivery (browser MCP image blocks + persisted capture paths, `observe(surface="desktop", screenshot_delivery="inline")`, one CSS-pixel browser coordinate space) — [`docs/features/browser-mcp-tools.md`](docs/features/browser-mcp-tools.md)
  - [ ] Re-run the VM smoke matrix for the CSS-pixel browser contract (only the live KDE host is verified so far)
- [x] Display-targeted desktop screenshots (single-screen capture: main-display default, explicit display/window selectors, and snapshot coordinate remapping) — [`docs/features/display-targeted-screenshots.md`](docs/features/display-targeted-screenshots.md)
  - [x] Desktop observation visual attachment now prefers focused/selected window crops, falls back only to target/primary display scopes, and exposes `inspection_image_path` for visual inspection
- [x] First-class browser MCP tools for `user_chrome` — [`docs/features/browser-mcp-tools.md`](docs/features/browser-mcp-tools.md)
  - [x] Explicit host opt-in gate for OpenCode/Pi, with Codex Desktop kept on the companion Browser Use path
  - [x] Real user-tab listing, session-owned tab creation, existing-tab claiming, and Brave/Chrome/Chromium socket selection
  - [x] Page snapshots, screenshots, cursor movement, click, text entry, key dispatch, scroll, and navigation against session-owned or claimed tabs
  - [x] Live Brave MCP smoke and installed OpenCode MCP probe for the full browser tool set
- [ ] Codex Desktop compat materialization contract (decided 2026-06-11:
      sky-cua owns behavior; the codex-desktop repo owns impersonation and
      materialization of the OpenAI built-in plugin identities —
      `computer-use@openai-bundled`, `browser-use@openai-bundled` — by
      generating plugin cache roots that point at the packaged sky-cua
      implementation)
  - [x] sky-cua side: stable wrapper-friendly plugin assets (`.mcp.json`
        server definitions, `skills/computer-use/SKILL.md`,
        `skills/browser-use/SKILL.md`, assets/docs)
  - [x] sky-cua side: MCP server runs from a copied/symlinked packaged
        location, not only the dev checkout (symlink-safe `bin/` launchers,
        exe-sibling service resolution, exe-ancestor app-instructions root)
  - [x] sky-cua side: documented manifest/template contract for generating a
        compat plugin from the sky-cua payload —
        [`docs/runtime/compat-plugin-contract.md`](docs/runtime/compat-plugin-contract.md)
  - [ ] codex-desktop side: cache-sync materialization of stock plugin roots,
        config.toml enablement, and stock-layout smokes (tracked in the
        codex-desktop repo, not here)
- [x] Remove the status-only `managed` browser target from contracts, tool
      rejection paths, status reporting, and docs (managed/isolated browser
      lifecycle was retired 2026-06-11: driving the user's real logged-in
      browser is the product; an isolated profile defeats that purpose) —
      [`docs/features/browser-mcp-tools.md`](docs/features/browser-mcp-tools.md)
- [x] One-shot installer (`install.py`): system deps, build (repo mode) or
      prebuilt bundle (bundle mode), compat plugin materialized from the
      bundled preflight, MCP registration and skills for all detected agents,
      health checks -
      [`docs/features/one-shot-installer.md`](docs/features/one-shot-installer.md)
- [x] Release package (`scripts/package.py` + `install.py`): self-contained
      tarball for clean-machine install with no checkout, toolchain, or
      marketplace -
      [`docs/features/release-package.md`](docs/features/release-package.md)
- [ ] Deduplicate the Codex compat-enablement sequence (`install_bundle` ->
      `run_browser_preflight` ->
      `update_codex_config(compat_enablement=compat_plugin_targets_payload(...))`),
      currently triplicated across `install_plugin.py`,
      `deploy_plugin.py:fast_deploy`, and `installer.py:run_codex_phase`. This
      is the load-bearing compat-enablement (security) toggle; three copies
      risk a future contract change silently diverging. Extract
      `install_plugin.install_codex_payload(bundle_root, codex_home, *,
      symlink=False, stop_processes=True)` and have the other two call it - as a
      deliberate, separately-reviewed change, not a mechanical extract: the
      sites differ (install_plugin skips the process-stop; installer wraps the
      whole install->preflight->config sequence in `try/except`->`PhaseResult`;
      deploy_plugin appends an installed-MCP refresh), so the helper's
      process-stop and error-scope parameterization must preserve each caller's
      behavior.
- [ ] Wire the retained cross-build staging primitives
      (`build_runtime_packages.py` -> `package_runtime_artifact.py` ->
      `_plugin_bundle.merge_runtime_artifacts`) into `scripts/package.py` so it
      can emit multi-platform / Windows tarballs from per-platform CI artifacts.
      `package.py` is single-platform today (asserts current-host binaries); the
      staging path builds each platform on its native host (no cross-compile),
      stages one artifact per platform, and merges the full
      `REQUIRED_RUNTIME_PLATFORMS` set into a fat bundle. The primitives have no
      production caller since the Heliasar CI matrix was deleted - they are
      retained marketplace-independent infrastructure, not dead code.
- [ ] Detached launch breadth across more desktop/session launchers

## Phase: Android phone control (phone-use)

Active ExecPlan: [`plans/phone-use.md`](plans/phone-use.md). Adds a `phone_*`
tool family and `skills/phone-use` to the existing `sky-cua-client mcp`
process for controlling a real Android phone over USB/wireless ADB, with an
optional companion app (native overlay, accessibility tree, gestures,
notifications) and optional scrcpy acceleration.

- [x] Phase 0 build/host survey — [`docs/research/2026-06-phone-use-android-build-survey.md`](docs/research/2026-06-phone-use-android-build-survey.md)
- [x] Phase 1 contract spine: platform model, service routing, config/env,
      MCP tool family, fakes (source landed, green)
- [x] Phase 2 ADB baseline + wireless control (source landed; live device
      proof pending)
- [x] Phase 3 snapshots, coordinate mapping, cursor planes (source landed)
- [x] Phase 4 Android companion backend host + RPC contract (Rust host and
      `docs/runtime/phone-companion-protocol.md` landed). Live proof achieved on
      the API-36 emulator 2026-06-20: `phone_connection(operation="connect")`
      auto-installs the companion,
      enables its accessibility + notification-listener services, brings up the
      RPC server, and serves `observe(surface="phone")`/accessibility-tree/screenshot through
      `backend=companion`. Three live-only bugs were fixed to get there: the
      expected signing cert is now loaded from the bundled metadata sidecar (it
      was never read, so the signature gate refused every companion); an
      unreadable installed cert proceeds (reported `signature_matches_expected=
      false`) instead of refusing (modern Android hides the cert SHA-256); and the
      RPC token is delivered as an `am start` intent extra instead of a pushed
      file (per-app storage namespaces made the file unreadable by the app, so the
      RPC server never started). Companion bumped to versionCode 2 so installs
      auto-update
- [x] Phase 4b phone-native agent overlay (source landed): the companion draws
      the agent cursor and a persistent "agent in control" edge glow on the
      device via a single full-screen pass-through `TYPE_ACCESSIBILITY_OVERLAY`,
      animated per action (`overlay_active`/`overlay_gesture` RPCs) and hidden
      around model-facing captures. The host-desktop phone-cursor draw
      (`host_cursor_state`/`HostCursorDraw`) was removed. Live proof on the
      API-36 emulator 2026-06-20: the cursor + edge glow render on-device in a
      companion-backed `observe(surface="phone")` capture
- [x] Phase 5 scrcpy acceleration + host-visible overlay (source landed; live
      proof pending). Mid-session crash detection (2s watchdog downgrades the
      capability and hides the host overlay), remap-on-window-resize, and
      explicit-serial window adoption are implemented in source; live-smoke
      proof of these scrcpy paths remains pending.
- [x] Phase 6 packaging, `skills/phone-use`, docs, installed MCP proof
      (skill, docs, bundling, canonical installed `tools/list` proof, and the
      installed ADB-baseline phone smoke landed — [`docs/features/phone-use.md`](docs/features/phone-use.md)).
      Install-bearing bootstrap now
      auto-enables the companion's accessibility + notification-listener services
      over ADB (read-merge-write of `enabled_accessibility_services` + global
      flag; `cmd notification allow_listener` for the listener), verifies via the
      health probe, and surfaces an actionable `PhoneCompanion*ManualSetup`
      diagnostic (and opens the on-device Accessibility screen) when an OEM gates
      the grant. Proven end-to-end on the API-36 emulator (both services bind,
      existing services preserved)
- [ ] Phase 7 adversarial testing
- [ ] Phase 8 full live-smoke and release proof (Redmi/API-36 tablet lane
      blocked until that device is connected). Agent-driven companion-setup
      live-smoke landed — `scripts/live_phone_companion_setup_smoke.py` validates
      the cold-device → reachable-companion workflow (agent or direct driver) by
      adb ground truth plus a pure MCP probe with auto-install off; green on the
      API-36 emulator 2026-06-20. Agent-driven workflow live-smoke landed —
      `scripts/live_phone_workflow_smoke.py` crystallizes the Settings →
      Accessibility navigation and Chrome web-search workflows, each proven by the
      adb resumed-activity ground truth plus a pointer-overlay MCP probe; green on
      the API-36 emulator 2026-06-20 (`--workflow full --agent claude`:
      both workflows reached their target screen, the browser agent surfaced the
      expected answer, and the overlay routed `backend=companion`). The `phone_*`
      tool-driver smoke (`live_phone_use_smoke.py`) full installed run remains
      pending.

## Phase: Performance and runtime tuning

- [x] Model screenshot size and format tuning — [`docs/features/image-size-performance.md`](docs/features/image-size-performance.md)
- [x] Deep performance review backlog closed (criticals through lows fixed or
      explicitly skipped with validation notes) — [`TODO_PERFORMANCE.md`](TODO_PERFORMANCE.md)

## Phase: Diagnostics and operator UX

- [x] Curated VM runner profile set — [`docs/operations/gui-desktop-test-harness.md`](docs/operations/gui-desktop-test-harness.md)
  - [x] Profile descriptors with curated membership, registry-driven
        host-framebuffer-proof dispatch, and a `--list-profiles` registry view
  - [x] Final trimmed pre-merge set decided and wired as `--profile curated`:
        `codex-desktop`, `wayland-pointer`, `session-env`, `text-readback`
        (session-agnostic; first full pass on COSMIC 2026-06-12). The cursor
        pixel matrix is inherently per-compositor and stays in the full
        per-session matrix rather than the one-session trimmed gate.
- [x] Consolidated agent test matrix and tool-use performance judge — [`docs/features/codex-cua-tool-use-gate.md`](docs/features/codex-cua-tool-use-gate.md)
  - [x] opencode/pi collapsed from four dialog-dismiss runs to one read-only
        wiring check each on the free `opencode/deepseek-v4-flash-free` model
  - [x] Single-run `codex-cua` profile exercises the full computer-use +
        browser-use surface (live Chrome + extension + native host) with a
        deterministic coverage/no-error gate; retired the standalone WebP smoke
  - [x] Host-side gpt-5.5 judge scores tool-use 0-100, hard-fails below a
        threshold, and always emits a triage list for follow-up
- [ ] Doctor/setup wording improvements as new launch environments expose blockers

## Code quality / Ultra-review backlog

Residual findings from the latest ultra-review pass. These are behavior-preserving
structural cleanups, performance fixes, and test gaps, not user-facing features;
pick them off between feature work.

- [ ] Split or reduce god files past the ~800–1000 line threshold:
  - `crates/sky-cua-linux/src/backend.rs`
  - `crates/sky-cua-linux/src/capture_plan.rs`
  - `crates/sky-cua-client/src/mcp_tools.rs`
- [ ] Extract KDE clipboard fallback from the generic action executor and
      introduce a backend-abstracted text-input strategy
- [ ] Centralize `input_backend` dispatch and coordinate-mapping helpers
      (currently scattered across `actions/targeting.rs`, `coords.rs`, etc.)
- [ ] Refactor per-backend input sequences in `actions/mod.rs`
      (`type_text`, `set_value_with_fallback_policy`) into backend strategy
      objects with a single transaction/rollback point
- [ ] Refactor `capture_plan::plan_capture` to replace its 9 positional arguments
      with a config struct and make `CaptureInfo` fallback transitions immutable
- [ ] Reduce repeated discovery on action hot paths:
  - `semantic_scroll_vertical_at` snapshots the full AT-SPI tree for every
    candidate app (N+1)
  - `matched_x11_window_for_request` re-runs X11 window discovery on every
    action request
  - `linux_fallback_snapshot` re-discovers X11 windows after desktop observation
    already fetched them
- [ ] Replace the 10 ms busy-poll in `command_output_with_timeout` with
      exponential backoff or `tokio::process::Command` + `tokio::time::timeout`
- [ ] Deduplicate agent stdout JSON parsing in `live_agent_mcp_smoke.py` and
      consolidate failure-status / tool-identity predicates with
      `_agent_mcp_smoke.py`
- [ ] Reduce `scripts/install_mcp_server.py` god file by extracting helpers
      (preserving install semantics and error handling)
- [ ] Split the agent-smoke cases in `scripts/test_live_smoke_helpers.py` into
      focused test files
- [ ] Add targeted unit tests for high-risk branches:
  - targeted-screenshot retry loop in `backend.rs::capture_screen`
  - XTest and `LinuxVirtualInput` scroll-with-target paths
  - `crop_capture` / `prepare_model_capture_from_image`
  - `reject_unactionable_targeted_capture` short-circuits and non-geometry errors
  - `require_screenshot_image` OK / generic-error branches
  - browser bridge `type_text` and `claim_tab`
  - Windows `windows_doctor_report`
- [ ] Finish or drop the intended `displays/providers.rs` / `displays/tests.rs`
      split if display topology code keeps growing

## Backlog / Ideas

- Compositor cursor hiding upstream integrations (Hyprland plugin, COSMIC accepted patch)
- Stronger Windows capture lane: WGC vs DXGI Desktop Duplication tradeoff once one is live
- KWin EIS support for broader pointer/scroll parity; GNOME RemoteDesktop EIS
  pointer and keyboard input is proved in the VM matrix
- Decide whether the X11-targeted keyboard override should also cover XWayland pointer actions
