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
  - [ ] Capture a dedicated `list_windows` workspace artifact on real KWin and X11
- [x] KWin window targeting (focused_window + verified activation via KWin scripting) — [`docs/features/kwin-window-targeting.md`](docs/features/kwin-window-targeting.md)
- [x] Linux virtual input (`LinuxVirtualInput` backend) — [`docs/features/linux-virtual-input.md`](docs/features/linux-virtual-input.md)
- [x] Native agent cursor overlay — [`docs/features/agent-cursor-overlay.md`](docs/features/agent-cursor-overlay.md)
- [x] Compositor cursor hiding (KWin / X11 / GNOME / Hyprland / patched COSMIC) — [`docs/features/compositor-cursor-hiding.md`](docs/features/compositor-cursor-hiding.md)
  - [x] Local KWin effect deploy/update tooling (`install_kwin_effect.py`, `--kwin-effect` deploy flags, BuildId convergence, session-restart notification — tooling never restarts KWin)
  - [x] Unpatched COSMIC transparent-Xcursor mode (VM-only fallback)
  - [ ] Long-term unpatched COSMIC path or accepted upstream integration
- [ ] Wayland fallback vision anchors — [`plans/wayland_fallback_vision_anchors.md`](plans/wayland_fallback_vision_anchors.md)
  - [ ] Choose a current fallback-only target app to replace the retired TIDAL flow
  - [ ] Live app-server proof on the new target
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
- [x] Agent-agnostic screenshot delivery (browser MCP image blocks + persisted capture paths, `get_app_state screenshot_delivery: inline`, one CSS-pixel browser coordinate space) — [`docs/features/browser-mcp-tools.md`](docs/features/browser-mcp-tools.md)
  - [ ] Re-run the VM smoke matrix for the CSS-pixel browser contract (only the live KDE host is verified so far)
- [x] Display-targeted desktop screenshots (primary-display default, explicit display/window/all-displays capture, and snapshot coordinate remapping) — [`docs/features/display-targeted-screenshots.md`](docs/features/display-targeted-screenshots.md)
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
- [x] One-shot installer (`install.py`): system deps, build, Heliasar
      marketplace + compat plugin, MCP registration and skills for all
      detected agents, health checks —
      [`docs/features/one-shot-installer.md`](docs/features/one-shot-installer.md)
- [ ] Detached launch breadth across more desktop/session launchers

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
- [ ] Doctor/setup wording improvements as new launch environments expose blockers

## Backlog / Ideas

- Compositor cursor hiding upstream integrations (Hyprland plugin, COSMIC accepted patch)
- Stronger Windows capture lane: WGC vs DXGI Desktop Duplication tradeoff once one is live
- KWin EIS support for broader pointer/scroll parity; GNOME RemoteDesktop EIS
  pointer and keyboard input is proved in the VM matrix
- Decide whether the X11-targeted keyboard override should also cover XWayland pointer actions
