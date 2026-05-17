# sky-cua roadmap

This file is the curated index of active workstreams. For live session state,
see [`CONTINUITY.md`](CONTINUITY.md). For durable behavior, see
[`docs/features/`](docs/features/). For active forward-looking design, see
[`plans/`](plans/).

Closed boxes link to the feature doc that describes the shipped behavior.
Open boxes link to the active ExecPlan that owns the work.

## Phase: Linux desktop parity

- [x] AT-SPI rich readback — [`docs/features/atspi-rich-readback.md`](docs/features/atspi-rich-readback.md)
- [x] Detached session-env repair — [`docs/features/session-env-repair.md`](docs/features/session-env-repair.md)
  - [ ] Add stripped-env repair to the curated VM runner profile set
  - [ ] Non-Codex host smoke (OpenCode/Pi) once VM lane is live
- [x] KWin and X11 workspace metadata — [`docs/features/kwin-x11-workspace-metadata.md`](docs/features/kwin-x11-workspace-metadata.md)
  - [ ] Capture a dedicated `list_windows` workspace artifact on real KWin and X11
- [x] Linux virtual input (`LinuxVirtualInput` backend) — [`docs/features/linux-virtual-input.md`](docs/features/linux-virtual-input.md)
- [x] Native agent cursor overlay — [`docs/features/agent-cursor-overlay.md`](docs/features/agent-cursor-overlay.md)
- [x] Compositor cursor hiding (KWin / X11 / GNOME / Hyprland / patched COSMIC) — [`docs/features/compositor-cursor-hiding.md`](docs/features/compositor-cursor-hiding.md)
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
- [ ] Broader Windows app-shell live smokes (Edge, Sumwall, others) — [`plans/windows_app_shell_smokes.md`](plans/windows_app_shell_smokes.md)
- [ ] Native Windows/UIA readback (`text`, `numeric_value`, `supports_editable_text`)

## Phase: Host portability

- [x] Codex Desktop compatibility (one active `computer-use` server, Browser Use companion, native-host preflight) — [`docs/features/codex-desktop-compat.md`](docs/features/codex-desktop-compat.md)
- [ ] OpenCode/Pi MCP host smoke parity
- [ ] Detached launch breadth across more desktop/session launchers

## Phase: Performance and runtime tuning

- [x] Model screenshot size and format tuning — [`docs/features/image-size-performance.md`](docs/features/image-size-performance.md)

## Phase: Diagnostics and operator UX

- [ ] Curated VM runner profile set: text-readback smokes, detached session-env,
      and current cursor matrix all in the trimmed pre-merge profile list
- [ ] Doctor/setup wording improvements as new launch environments expose blockers

## Backlog / Ideas

- First-class `browser_*` MCP tools (currently Browser Use is companion-only)
- Compositor cursor hiding upstream integrations (Hyprland plugin, COSMIC accepted patch)
- Stronger Windows capture lane: WGC vs DXGI Desktop Duplication tradeoff once one is live
- KWin/EIS support for broader pointer/scroll parity once RemoteDesktop `Notify*` is stable
- Decide whether the X11-targeted keyboard override should also cover XWayland pointer actions
