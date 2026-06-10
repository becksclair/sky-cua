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
  - [ ] Add stripped-env repair to the curated VM runner profile set
  - [x] Non-Codex host smoke (OpenCode/Pi) once VM lane is live
- [x] KWin and X11 workspace metadata — [`docs/features/kwin-x11-workspace-metadata.md`](docs/features/kwin-x11-workspace-metadata.md)
  - [ ] Capture a dedicated `list_windows` workspace artifact on real KWin and X11
- [x] Linux virtual input (`LinuxVirtualInput` backend) — [`docs/features/linux-virtual-input.md`](docs/features/linux-virtual-input.md)
- [x] Native agent cursor overlay — [`docs/features/agent-cursor-overlay.md`](docs/features/agent-cursor-overlay.md)
- [x] Compositor cursor hiding (KWin / X11 / GNOME / Hyprland / patched COSMIC) — [`docs/features/compositor-cursor-hiding.md`](docs/features/compositor-cursor-hiding.md)
  - [x] Local KWin effect deploy/update tooling (`install_kwin_effect.py`, `--kwin-effect` deploy flags, BuildId convergence, session-restart notification — tooling never restarts KWin)
  - [x] Unpatched COSMIC transparent-Xcursor mode (VM-only fallback)
  - [ ] Long-term unpatched COSMIC path or accepted upstream integration
- [ ] Wayland fallback vision anchors — [`plans/wayland_fallback_vision_anchors.md`](plans/wayland_fallback_vision_anchors.md)
  - [ ] Choose a current fallback-only target app to replace the retired TIDAL flow
  - [ ] Live app-server proof on the new target
- [ ] CDUL-inspired Linux enhancements — [`plans/cdul_linux_enhancements.md`](plans/cdul_linux_enhancements.md)
  - [ ] Terminal `command_line` fidelity in `crates/sky-cua-linux/src/windowing/terminal.rs`
  - [ ] Linux input doctor polish in `crates/sky-cua-linux/src/doctor.rs`
  - [ ] App-root prefiltering for AT-SPI snapshots
  - [ ] GNOME setup-message polish

## Phase: Windows parity

- [x] Windows UIA inspection and semantic actions — [`docs/features/windows-uia-automation.md`](docs/features/windows-uia-automation.md)
  - First-class `focus_element`, `activate_element`, `select_element`,
    `expand_element`, `collapse_element`, `toggle_element` against UIA patterns
- [ ] Windows capture ladder (WGC / DXGI before GDI) — [`plans/windows_capture_ladder.md`](plans/windows_capture_ladder.md)
- [ ] Windows agent cursor overlay and host IPC
  - [ ] Refactor overlay host platform modules so Linux-only cursor and
        compositor code is cfg-scoped outside shared imports and contracts
  - [ ] Split service overlay transport boundaries by platform: Unix domain
        socket client on Unix, Windows named-pipe or localhost transport for
        `sky-cua-overlay-host.exe`
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
- [x] Claude Code host support (plugin manifest + marketplace, `--host claude-code` installer, `~/.claude/skills` sync) — [`docs/features/claude-code-host.md`](docs/features/claude-code-host.md)
- [x] Agent-agnostic screenshot delivery (browser MCP image blocks + persisted capture paths, `get_app_state screenshot_delivery: inline`, one CSS-pixel browser coordinate space) — [`docs/features/browser-mcp-tools.md`](docs/features/browser-mcp-tools.md)
  - [ ] Re-run the VM smoke matrix for the CSS-pixel browser contract (only the live KDE host is verified so far)
- [x] First-class browser MCP tools for `user_chrome` — [`docs/features/browser-mcp-tools.md`](docs/features/browser-mcp-tools.md)
  - [x] Explicit host opt-in gate for OpenCode/Pi, with Codex Desktop kept on the companion Browser Use path
  - [x] Real user-tab listing, session-owned tab creation, existing-tab claiming, and Brave/Chrome/Chromium socket selection
  - [x] Page snapshots, screenshots, cursor movement, click, text entry, key dispatch, scroll, and navigation against session-owned or claimed tabs
  - [x] Live Brave MCP smoke and installed OpenCode MCP probe for the full browser tool set
- [ ] Browser MCP managed lifecycle — [`plans/browser_use_mcp.md`](plans/browser_use_mcp.md)
  - [ ] Launch and own an isolated browser/profile instead of using an existing user Chrome-family profile
  - [ ] Run the shipped snapshot/screenshot/action tool sequence in that managed context
  - [ ] Clean up managed browser process/profile state deterministically
  - [ ] Delegate Codex Desktop's companion Browser Use adapter through the shared runtime without exposing duplicate browser tools by default
- [ ] Detached launch breadth across more desktop/session launchers

## Phase: Performance and runtime tuning

- [x] Model screenshot size and format tuning — [`docs/features/image-size-performance.md`](docs/features/image-size-performance.md)

## Phase: Diagnostics and operator UX

- [ ] Curated VM runner profile set: text-readback smokes, detached session-env,
      and current cursor matrix all in the trimmed pre-merge profile list
- [ ] Doctor/setup wording improvements as new launch environments expose blockers

## Backlog / Ideas

- Compositor cursor hiding upstream integrations (Hyprland plugin, COSMIC accepted patch)
- Stronger Windows capture lane: WGC vs DXGI Desktop Duplication tradeoff once one is live
- KWin EIS support for broader pointer/scroll parity; GNOME RemoteDesktop EIS
  pointer and keyboard input is proved in the VM matrix
- Decide whether the X11-targeted keyboard override should also cover XWayland pointer actions
