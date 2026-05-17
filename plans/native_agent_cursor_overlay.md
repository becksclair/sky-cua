# Native agent cursor overlay for Computer Use

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

This document must be maintained in accordance with `/home/bex/.agents/PLANS.md`. It is self-contained on purpose: a future contributor should be able to start with this file, the current repository, and the commands below, without needing the chat that created the plan.

## Purpose / Big Picture

After this work, the `computer-use` plugin will show an agent cursor for desktop Computer Use actions, similar in spirit to the native OpenAI Computer Use cursor on macOS. The cursor has two separate jobs. The first job is user experience: the human operator should see where the agent is about to click or drag. The second job is model context: the screenshot that Codex sees should contain an honest marker for the agent cursor, even when the operating system refuses to let an ordinary app draw a visible global overlay.

The important result is that cursor support does not depend on one compositor. KDE/KWin should be the first Linux desktop proved deeply because this machine and this repository already have KWin-specific window discovery, KWin fallback anchors, portal capture, and portal input proof. The design must still stay cross-platform. Windows support is deliberately deferred until we have access to a Windows machine, but the shared model, IPC, and capture contract must not make Windows harder later.

The first visible acceptance target is KDE Wayland. A contributor should be able to run a KWin smoke script, move the agent cursor to a requested point without performing a destructive desktop action, capture the desktop, and observe that the model-facing screenshot contains the synthetic cursor marker. After the generic state, IPC, and capture contract are solid, the same KWin smoke should prove a visible overlay path, then a KWin effect path if the generic Wayland overlay is not polished enough.

## Progress

- [x] (2026-05-14 19:09Z) Read `/home/bex/.agents/PLANS.md`, `plans/AGENTS.md`, and the existing plan style in `plans/wayland_fallback_vision_anchors.md`.
- [x] (2026-05-14 19:09Z) Re-read the current service, platform model, Linux capture/input, KWin windowing, Windows capture/input, Chrome cursor overlay, preflight, and smoke-test seams.
- [x] (2026-05-14 19:09Z) Completed the design research pass: ordinary Wayland clients cannot be a universal global overlay; Windows should use a layered topmost click-through window; Wayland should use layer-shell where supported and compositor integrations where required.
- [x] (2026-05-14 19:09Z) Chose this plan's priority order: generic state/IPC/capture contract first, KWin tests and KWin proof second, KWin effect third, Windows implementation deferred until a Windows machine is available.
- [x] (2026-05-14 19:30Z) Implemented the platform-neutral cursor state, capabilities, diagnostics, service requests, and serialization tests in `crates/sky-cua-platform/src/model.rs`.
- [x] (2026-05-14 19:30Z) Implemented service-owned cursor state management with an in-process no-op controller in `crates/sky-cua-service/src/overlay.rs`; existing Computer Use actions continue when the visible overlay is unavailable.
- [x] (2026-05-14 19:30Z) Implemented synthetic cursor compositing for model-facing screenshots and proved it with service unit tests, including edge clipping and WebP output.
- [x] (2026-05-14 19:30Z) Added `crates/sky-cua-overlay-host` with a versioned JSON-lines no-op backend, `probe`, and `serve` commands.
- [x] (2026-05-14 19:30Z) Added the KWin-first synthetic smoke `scripts/live_agent_cursor_kde_smoke.py` and proved it live on KDE Wayland with `synthetic_cursor_found=true`.
- [x] (2026-05-14 19:30Z) Added overlay runtime packaging hooks and environment allowlist entries for `SKY_CUA_AGENT_CURSOR`, `SKY_CUA_OVERLAY_BACKEND`, `SKY_CUA_OVERLAY_HIDE_FOR_CAPTURE`, `SKY_CUA_OVERLAY_HOST_PATH`, and `SKY_CUA_SCREENSHOT_CURSOR`.
- [x] (2026-05-14 19:48Z) Tightened the first slice after review: compact snapshots now include `agent_cursor`, and the tag release workflow builds runtime packages through the same platform binary contract used by runtime packaging.
- [x] (2026-05-14 19:50Z) Connected the service overlay controller to the overlay-host process over private Unix-socket IPC; focused overlay-host and service tests pass.
- [x] (2026-05-14 20:06Z) Implemented and proved the generic Wayland layer-shell visible overlay on KDE/KWin first, while keeping no-op fallback for sessions without `zwlr_layer_shell_v1`.
- [x] (2026-05-14 20:06Z) Added KWin-first smoke coverage for visible-overlay capture and hide-for-capture behavior.
- [x] (2026-05-14 20:13Z) Re-ran the current Linux/KWin validation slice after hardening the live smoke against compositor presentation races.
- [x] (2026-05-14 20:20Z) Tried and rejected the user-level QML `SceneEffect` KWin prototype because it replaced the desktop scene with a fullscreen black surface behind the marker instead of being transparent and click-through. Removed the prototype package and left `kwin-effect-static` as a fail-fast guard until a real transparent, click-through compositor-painting implementation exists.
- [x] (2026-05-14 20:27Z) Added regression coverage for the rejected KWin effect smoke mode and for layer-shell transparent-background drawing; verified KWin no longer has the rejected effect installed or loaded.
- [x] (2026-05-14 20:35Z) Replaced the temporary procedural marker with the same `cursor-chat.png` asset used by the bundled Chrome extension, copied into `crates/sky-cua-overlay-host/assets/` and rendered by both synthetic screenshot compositing and the Wayland layer-shell overlay.
- [x] (2026-05-14 20:55Z) Added an explicit X11 shaped-window backend in `crates/sky-cua-overlay-host`, using `x11rb = "0.13.2"`, the copied Chrome cursor asset, X Shape bounding regions for transparency, an empty input shape for click-through behavior, and the same overlay-host JSON-lines IPC.
- [x] (2026-05-14 20:55Z) Added `x11-debug-visible` smoke plumbing and proved that `SKY_CUA_OVERLAY_BACKEND=x11` instantiates on this KDE Wayland machine's X11/XWayland display. The forced XWayland visible smoke did not appear in portal capture, so the accepted X11 visual proof had to come from a true X11 session rather than host XWayland.
- [x] (2026-05-14 21:05Z) Added a C++ KWin `Effect` prototype that calls `effects->paintScreen()` first and then renders the copied `cursor-chat.png` through an `OffscreenQuickScene`, plus an explicit opt-in `kwin-effect-static` smoke. The prototype builds and installs user-level files, but running KWin did not discover or load the user-level compiled plugin (`loadEffect=false`, `isEffectSupported=false`), so the KWin effect path is not accepted here and the proved KWin backend remains layer-shell.
- [x] (2026-05-14 21:08Z) Added `scripts/live_agent_cursor_x11_overlay_smoke.py`, a nested-X11/Xvfb proof that the X11 shaped-window backend renders the copied Chrome cursor asset into root capture, hides cleanly on request, and stays click-through to a Tk target window underneath.
- [x] (2026-05-14 21:11Z) Recorded Windows native overlay implementation and live proof as explicitly deferred until a Windows machine is available; Linux-only work must not claim Windows completion.
- [x] (2026-05-14 21:13Z) Added `--current-display` to `scripts/live_agent_cursor_x11_overlay_smoke.py` as the dedicated real-X11 acceptance command; on this KDE Wayland session it refuses cleanly with `XDG_SESSION_TYPE=wayland` instead of treating XWayland as proof.
- [x] (2026-05-14 21:20Z) Fixed review-found cursor placement gaps: service-derived cursor states now include native overlay coordinates when capture mapping is available, snapshotless explicit pointer actions use native-only cursor state instead of stale model pixels, unmappable successful pointer actions clear stale cursor state, X11 overlays prefer native coordinates, and deploy cleanup recognizes `sky-cua-overlay-host`.
- [x] (2026-05-14 21:25Z) Made overlay-host `Show` commands authoritative instead of resurrecting a previously hidden cursor when the service has no current cursor state.
- [x] (2026-05-14 21:28Z) Tightened the X11 click-through smoke so it re-shows the cursor with explicit state, captures that re-shown overlay, and only accepts click-through when the overlay was visible at the click point.
- [x] (2026-05-14 21:34Z) Tightened the KWin effect smoke to record KWin's own `listOfEffects` and `loadedEffects` discovery state before install, after install, after reconfigure/load, and after cleanup.
- [x] (2026-05-14 21:37Z) Added and ran an embedded X11 acceptance mode using `Xvfb` plus `Openbox`, proving visible X11 overlay rendering, hide behavior, re-show behavior, and click-through with a real window manager instead of bare Xvfb or host XWayland.
- [x] (2026-05-14 21:37Z) Fixed element-target native cursor derivation so `StreamLogical` element bounds are mapped through capture metadata into the visible-overlay native plane instead of being handed to native overlays as raw stream coordinates.
- [x] (2026-05-14 21:47Z) Revalidated after the rendered-size cursor asset contract and Windows target check: the source PNG remains byte-identical to Chrome's 46x48 asset, native overlays render it at the browser CSS size of 23x24, and `cargo check -p sky-cua-windows --target x86_64-pc-windows-msvc` passes.
- [x] (2026-05-14 21:56Z) Fixed the KWin/Wayland huge-cursor regression by treating the copied Chrome PNG as a 2x source asset and rendering it at Chrome's 23x24 CSS-pixel size in synthetic screenshots, Wayland layer-shell, X11, and KWin prototype resources.
- [x] (2026-05-14 21:56Z) Tightened the native cursor coordinate contract after review: portal/layer-shell native points are now output-local `StreamLogical` coordinates, while X11 model-bounded captures scale native overlay points back to original root pixels through `original_pixel_size`.
- [x] (2026-05-14 22:05Z) Added and ran a KWin Wayland layer-shell click-through proof. The smoke opens the existing GTK pointer fixture, renders the native cursor over its click target, proves the cursor pixels are visible there, then sends a real service `click` through the RemoteDesktop portal and requires the underlying fixture to record `clicked=true`.
- [x] (2026-05-14 22:13Z) Added a system-cursor hiding adapter seam. X11 now hides/restores the OS cursor through an XFixes-backed adapter while the agent cursor is visible, generic Wayland layer-shell reports that system cursor hiding is unsupported, and the KWin C++ effect prototype uses KWin's compositor-side `hideCursor`/`showCursor` helpers with restore on destruction. The embedded X11 smoke now proves `system_cursor_hidden` toggles true on set/show and false on hide.
- [x] (2026-05-14 22:36Z) Promoted system-cursor hiding to an explicit backend contract. `AgentCursorCapabilities` now reports `system_cursor_backend`, the Rust overlay host distinguishes `none`, `wayland_client_unsupported`, and `x11_xfixes`, and the KWin C++ effect owns hide/show through a `KWinSystemCursorAdapter` instead of raw effect-class calls.
- [x] (2026-05-14 22:36Z) Fixed and proved the KWin C++ effect renderer in nested KWin. Scene setup now happens during `prePaintScreen`, matching KWin's own `OffscreenQuickScene` effects, and `uv run python scripts/live_agent_cursor_kde_smoke.py --mode kwin-effect-nested` builds the effect into a temp prefix, starts nested `kwin_wayland` with `QT_PLUGIN_PATH`/`XDG_DATA_DIRS`, loads the effect, captures through KWin `ScreenShot2`, and verifies the copied 23x24 cursor marker at screen center.
- [x] (2026-05-14 22:44Z) Added and proved the KWin effect live cursor-state bridge in nested KWin. The effect exports `/com/skycua/AgentCursor` on KWin's session bus, accepts the shared `AgentCursorState` JSON through `com.skycua.AgentCursor.SetCursorState`, prefers `native_point`, stores `StateJson` for readback, and renders the copied cursor at a non-center requested point.
- [x] (2026-05-14 22:50Z) Connected the KWin effect bridge to the generic overlay-host protocol and auto-detection. The overlay host now auto-selects an already-loaded KWin effect before layer-shell/X11 fallback, while `SKY_CUA_OVERLAY_BACKEND=kwin-effect` remains an explicit override. The nested KWin smoke proves the default auto path through a real `OverlayHostMessage::SetCursor` JSON-line.
- [x] (2026-05-14 22:54Z) Tightened the KWin effect production-discovery proof. The effect metadata now includes the current KWin package fields `KPackageStructure=KWin/Effect` and `KPlugin.Id=sky-cua-agent-cursor`, and the install step now places data under both `kwin/effects` and `kwin-wayland/effects`; the running KWin session still does not list or load the user-level compiled plugin after reconfigure, confirming that production acceptance needs a system package or a KWin restart with plugin paths.
- [x] (2026-05-14 22:59Z) Made KWin effect resources part of the plugin bundle path even while the tree is dirty/untracked. `scripts/build_plugin.py` now copies `resources/kwin` as a worktree bundle directory, and Python regression coverage proves the KWin metadata is bundled.
- [x] (2026-05-15 01:04Z) Added a nested KWin user-install discovery proof. The new `kwin-effect-nested-user-install` smoke installs the effect into a temporary `HOME/.local`, launches a fresh nested `kwin_wayland` without `QT_PLUGIN_PATH`, and records that KWin still does not discover or load the user-level compiled plugin; the existing forced-path nested smoke remains the positive control.
- [x] (2026-05-15 01:37Z) Rejected the KDE Neon container path after live builds repeatedly stalled or failed on Neon apt metadata.
- [x] (2026-05-15 01:52Z) Used an Arch GUI Docker harness as a short-lived package-discovery and nested-compositor proving tool. It helped identify the package set, Chrome/Codex Desktop requirements, host-built runtime boundary, and initial profile scripts.
- [x] (2026-05-15 03:53Z) Retired the Docker harness as an acceptance path because nested container compositors do not prove standalone VM desktop sessions. The accepted direction is now an Arch `testing-vm` with QEMU/libvirt/virt-manager display, real desktop-session autologin, host-built sky-cua binaries synced into `/workspace`, and smoke profiles executed over SSH.
- [x] (2026-05-15 04:10Z) Added `scripts/testing-vm/provision-arch-testing-vm.sh` from the Docker package contract. It installs the Arch desktop matrix, Chrome, optional Codex Desktop package, greetd autologin, SSH, rsync, and the session selector for COSMIC, Plasma, GNOME, Hyprland, and i3.
- [x] (2026-05-15 04:10Z) Added `scripts/run_gui_testing_vm_smoke.py` and moved the reusable profile scripts under `scripts/testing-vm/profiles/`. The runner builds sky-cua runtime artifacts on the host, syncs the checkout and selected `~/.codex` state into the VM, and runs profiles through SSH.
- [x] (2026-05-15 07:38Z) Fixed the Plasma testing-VM session launcher to use the normal user DBus bus instead of `dbus-run-session`, disabled the KDE locker/PowerDevil in the test session, and re-proved KWin DBus discovery from SSH.
- [x] (2026-05-15 07:38Z) Re-proved the KDE Wayland layer-shell cursor on the headed VM framebuffer with a clean bright baseline: artifact `artifacts/kde-framebuffer-cursor-proof/cursor-overlay-clean/after.png` shows only the 23x24 cursor region changed at the requested point.
- [x] (2026-05-15 07:38Z) Re-ran the service smokes on the fixed Plasma VM session. `layer-shell-hide-for-capture` passed at `/workspace/artifacts/codex-e2e/agent-cursor-kde/0515073739578726-hide`, and `layer-shell-click-through` passed at `/workspace/artifacts/codex-e2e/agent-cursor-kde/0515073759757099-click` with `target_clicked=true`.
- [x] (2026-05-15 07:45Z) Ran the real-session KDE `computer-use`/`wayland-pointer` VM smoke. Artifact `/workspace/artifacts/gui-desktop-smoke/wayland-pointer/20260515T074158Z` proves click, drag, and scroll against the fullscreen GTK fixture through the RemoteDesktop portal.
- [x] (2026-05-15 07:53Z) Ran the VM `i3` profile against a real Xorg/i3 session, not XWayland. Artifact `/workspace/artifacts/codex-e2e/agent-cursor-x11-overlay/20260515T075301057704Z` proves visible X11 overlay capture, hide-for-capture, re-show, click-through to the Tk target, and XFixes system cursor hide/show transitions.
- [x] (2026-05-15 07:57Z) Re-ran the production KWin effect discovery smoke after fixing the Plasma VM session bus. Artifact `/workspace/artifacts/codex-e2e/agent-cursor-kde/0515075621741796-kwin` still shows `listed=false`, `effect_supported=false`, and `load_stdout="false"`, so the user-level compiled effect discovery blocker is real and remains outside the accepted production path.
- [x] (2026-05-15 08:09Z) Added a real-session Wayland layer-shell overlay profile and proved it on Hyprland. Artifact `/workspace/artifacts/codex-e2e/agent-cursor-wayland-layer-shell/20260515T080912397166Z` shows `backend=wayland_layer_shell`, `visible_overlay_captured=true`, `hidden_overlay_captured=false`, `click_through=true`, and `capture_output=Virtual-1`.
- [x] (2026-05-15 08:09Z) Fixed a Hyprland-only layer-shell protocol violation: the overlay host now skips unconfigured layer surfaces instead of attaching buffers to them. The failing artifact immediately before the fix reported `layerSurface was not configured, but a buffer was attached`.
- [x] (2026-05-15 08:18Z) Proved the production KWin system-install path manually in the VM. Installing the compiled effect under `/usr`, restarting Plasma, loading it, and driving it through `sky-cua-overlay-host` produced `backend=kwin_effect`, `system_cursor_hidden=true`, and a host-framebuffer cursor diff at `(420,260)` under `artifacts/kde-framebuffer-cursor-proof/kwin-system-install/`. Cleanup removed system files and KWin no longer listed the effect after restart.
- [x] (2026-05-15 08:26Z) Codified the KWin system-install pixel proof fully in the VM runner. `scripts/run_gui_testing_vm_smoke.py --profile kde-kwin-effect-system-install --vm-name testing-vm --libvirt-uri qemu:///session` now captures before/after host framebuffers with `virsh`, waits for the guest smoke's ready file, probes the cursor diff on the host, and writes `host-summary.json`. Artifact `artifacts/kde-framebuffer-cursor-proof/kwin-system-install/20260515T082610924137Z/host-summary.json` has `ok=true`, `host_marker_probe.found=true`, `changed_pixels_near_hotspot=186`, `max_channel_delta_near_hotspot=168`, and remote cleanup leftovers `[]`.
- [x] (2026-05-15 08:30Z) Re-ran the headed KDE `wayland-pointer` VM smoke with `virt-viewer` detached from the agent shell. Artifact `/workspace/artifacts/gui-desktop-smoke/wayland-pointer/20260515T083033Z` proves the fullscreen fixture path again with `clicked=true`, `drag_completed=true`, and `scroll_events=1`.
- [x] (2026-05-15 08:46Z) Proved the GNOME Wayland pointer path in the VM after fixing the session selector, GNOME fullscreen fixture coordinates, and portal scroll wheel conversion. Artifact `/workspace/artifacts/gui-desktop-smoke/wayland-pointer/20260515T084643Z` has `clicked=true`, `drag_completed=true`, and `scroll_events=2`.
- [x] (2026-05-15 08:50Z) Re-ran the KDE `wayland-pointer` VM smoke after the GNOME fixes to prove Plasma was not regressed. Artifact `/workspace/artifacts/gui-desktop-smoke/wayland-pointer/20260515T085020Z` has `clicked=true`, `drag_completed=true`, and `scroll_events=1`.
- [x] (2026-05-15 09:18Z) Proved the COSMIC Wayland pointer path in the VM through `LinuxVirtualInput` after rejecting ydotool as a precise pointer adapter and implementing a direct absolute `/dev/uinput` adapter. Artifact `/workspace/artifacts/gui-desktop-smoke/wayland-pointer/20260515T091758Z` has `clicked=true`, `drag_completed=true`, and `scroll_events=1`.
- [x] (2026-05-15 09:26Z) Extended and re-ran the COSMIC Wayland `wayland-pointer` VM smoke so it now proves text/key injection too. Artifact `/workspace/artifacts/gui-desktop-smoke/wayland-pointer/20260515T092606Z` has `clicked=true`, `drag_completed=true`, `scroll_events=1`, `entry_text="cosmic-text-smoke"`, and `submitted_text="cosmic-text-smoke"`.
- [x] (2026-05-15 09:33Z) Fixed direct uinput scaling for COSMIC by parsing `cosmic-randr` scale and converting desktop logical points into physical absolute-device coordinates. The 125% scale VM proof at `/workspace/artifacts/gui-desktop-smoke/wayland-pointer/20260515T093335Z` has `clicked=true`, `drag_completed=true`, `scroll_events=1`, `entry_text="cosmic-text-smoke"`, and `submitted_text="cosmic-text-smoke"`.
- [x] (2026-05-15 09:37Z) Added and proved the repeatable `wayland-pointer-scaled` VM profile for COSMIC. Artifact `/workspace/artifacts/gui-desktop-smoke/wayland-pointer/20260515T093737Z` proves the same 125% scale path and the profile restored COSMIC to 1280x800 at 100% scale afterward.
- [x] (2026-05-15 09:44Z) Re-ran the packaging/preflight/browser cursor regression slice after the COSMIC input work. `python3 scripts/build_plugin.py` produced `dist/plugin/sky-cua`; the bundle contains executable `bin/sky-cua-overlay-host`, launches `./bin/sky-cua-client`, and preserves `SKY_CUA_AGENT_CURSOR` plus `SKY_CUA_OVERLAY_HOST_PATH` in `.mcp.json`. `uv run pytest scripts/test_python_harness_helpers.py -k 'overlay_host or agent_cursor or bundled_chrome_extension_cursor_overlay_contract or computer_use_env_vars or worktree_bundle_dirs or bundle_entrypoint_paths or runtime_binary_path'` passes 9 selected tests after the VM reset helper was tightened to kill both full `sky-cua-overlay-host` argv matches and Linux's truncated `sky-cua-overlay` comm name.
- [x] (2026-05-15 10:03Z) Fixed two live VM failure classes found while revalidating KDE after COSMIC. The VM runner now refreshes the user portal stack after importing the selected desktop environment so a COSMIC-selected `xdg-desktop-portal` cannot poison Plasma tests. Linux virtual input now probes `cosmic-randr` only for COSMIC desktops and bounds helper commands are timeout-protected, so a KDE service cannot hang behind an orphaned `cosmic-randr list`.
- [x] (2026-05-15 10:03Z) Fixed overlay-host lifecycle cleanup. `sky-cua-service` now handles SIGTERM and exits through normal daemon teardown, and `ProcessOverlayHostClient` waits/reaps after shutdown or kill. A clean KDE cursor sequence left no `sky-cua-service`, `sky-cua-overlay-host`, or `cosmic-randr` processes behind.
- [x] (2026-05-15 10:03Z) Re-ran the clean KDE cursor sequence on the VM after the portal/probe/lifecycle fixes. Accepted artifacts: synthetic `/workspace/artifacts/codex-e2e/agent-cursor-kde/0515100302670580-syn`, visible layer-shell `/workspace/artifacts/codex-e2e/agent-cursor-kde/0515100303845615-vis`, hide-for-capture `/workspace/artifacts/codex-e2e/agent-cursor-kde/0515100305142807-hide`, and click-through `/workspace/artifacts/codex-e2e/agent-cursor-kde/0515100306568235-click`.
- [x] (2026-05-15 10:03Z) Re-ran the KDE portal pointer fixture through the VM runner after the same fixes. Artifact `/workspace/artifacts/gui-desktop-smoke/wayland-pointer/20260515T100113Z` proves click, drag, scroll, `type_text`, and `press_key` on Plasma Wayland.
- [x] (2026-05-15 10:08Z) Fixed the VM session selector's display-manager restart behavior after the KWin system-install profile exposed `greetd` start-limit churn. The selector now resets failed units and uses `enable` plus one `restart`, instead of `enable --now` followed by a second restart.
- [x] (2026-05-15 10:08Z) Hardened the KWin system-install VM runner so a failed guest smoke still writes a host summary instead of crashing while reading a missing remote `summary.json`.
- [x] (2026-05-15 10:08Z) Re-ran the automated production KWin effect proof after the selector and runner hardening. Artifact `artifacts/kde-framebuffer-cursor-proof/kwin-system-install/20260515T100852814643Z/host-summary.json` has `ok=true`, host framebuffer `host_marker_probe.found=true`, `changed_pixels_near_hotspot=186`, KWin discovered and loaded `sky-cua-agent-cursor`, overlay host reported `backend=kwin_effect` with `system_cursor_hidden=true`, cleanup removed all installed `/usr` files, and KWin no longer listed the effect after cleanup restart.
- [x] (2026-05-15 11:18Z) Re-audited the native cursor plan against the current tree and current artifacts, corrected stale KWin proof references in the VM harness docs and sidecars, rechecked guest-side summaries over SSH, and reran the full local validation gate.

## Surprises & Discoveries

- Observation: the older plan in `plans/wayland_fallback_vision_anchors.md` is stale relative to current source. It describes KWin fallback anchors as pending, but the current `crates/sky-cua-linux/src/backend.rs` already contains KWin fallback anchor helpers and tests.
  Evidence: `rg -n "push_kwin_anchor|creates_structural_anchor_regions_for_kwin_fallback_windows" crates/sky-cua-linux/src/backend.rs` finds the implemented helper and tests.

- Observation: the repository already has the right KWin substrate for a focused proof lane. KWin window discovery and activation live in `crates/sky-cua-linux/src/kwin.rs`, and the generic Linux windowing registry already treats KWin as a first-class backend.
  Evidence: `crates/sky-cua-linux/src/windowing/registry.rs` declares `KWIN_BACKEND`, probes KWin, lists KWin windows, and routes activation through `kwin::activate_window`.

- Observation: the current workspace already depends on `wayland-client` and `wayland-protocols`, but not on the wlroots layer-shell protocol crate. Current crates.io lookup on 2026-05-14 reported `wayland-protocols-wlr = "0.3.12"`, `smithay-client-toolkit = "0.20.0"`, `calloop = "0.14.4"`, and `x11rb = "0.13.2"`.
  Evidence: `cargo search wayland-protocols-wlr --limit 5`, `cargo search smithay-client-toolkit --limit 5`, `cargo search calloop --limit 5`, and `cargo search x11rb --limit 5`.

- Observation: the existing browser-use cursor implementation is a good behavioral reference but not an architectural substitute for desktop Computer Use. Browser-use injects a content-script overlay in a tab and waits for `AGENT_CURSOR_ARRIVED`; Computer Use needs desktop capture, desktop input, compositor-specific visible overlays, and a synthetic model-facing cursor.
  Evidence: `scripts/test_python_harness_helpers.py::test_bundled_chrome_extension_cursor_overlay_contract` asserts the existing Chrome cursor overlay contract.

- Observation: Wayland ScreenCast cursor modes are not a visible agent cursor API. They control whether the real pointer is hidden, embedded in frames, or sent as metadata. They can help capture truth, but they do not draw a custom agent cursor on the user's desktop.
  Evidence: the ScreenCast portal defines `Hidden`, `Embedded`, and `Metadata` cursor modes; current code selects metadata in `crates/sky-cua-linux/src/portal/remote_desktop.rs`.

- Observation: the first live KDE proof can be non-destructive. `scripts/live_agent_cursor_kde_smoke.py --mode synthetic` starts a private service socket, captures the desktop, sets the agent cursor state through service IPC, captures again, and verifies localized pixel changes around the requested screenshot-pixel point without clicking any UI.
  Evidence: `uv run python scripts/live_agent_cursor_kde_smoke.py --mode synthetic` wrote `artifacts/codex-e2e/agent-cursor-kde/20260514T193354Z/summary.json` with `synthetic_cursor_found=true`, `changed_pixels_near_hotspot=127`, and `max_channel_delta_near_hotspot=234`.

- Observation: the service-to-host seam now has executable proof before any compositor drawing code exists. `OverlayController` spawns `sky-cua-overlay-host serve --socket <path>`, exchanges versioned JSON-lines messages, and treats missing or failed hosts as cursor diagnostics instead of action failure.
  Evidence: `cargo test -p sky-cua-overlay-host -p sky-cua-service` passes, including `overlay::tests::host_process_round_trips_cursor_state_over_private_socket` and `overlay::tests::host_process_failure_is_diagnostic_not_action_failure`.

- Observation: KWin on this machine advertises `zwlr_layer_shell_v1`, and the layer-shell overlay is visible to portal capture when hide-for-capture is disabled.
  Evidence: `uv run python scripts/live_agent_cursor_kde_smoke.py --mode layer-shell-debug-visible` wrote `artifacts/codex-e2e/agent-cursor-kde/20260514T200626Z/summary.json` with `visible_overlay_captured=true`, `backend=wayland_layer_shell`, `changed_pixels_near_hotspot=109`, and `max_channel_delta_near_hotspot=225`.

- Observation: KWin layer-shell click-through is now live-proved, not just inferred from the empty Wayland input region. The click-through smoke must launch the GTK fixture with the system Python because the uv Python does not have `gi`; the smoke records fixture stdout/stderr in the artifact directory so that launch failures are diagnosable.
  Evidence: `uv run python scripts/live_agent_cursor_kde_smoke.py --mode layer-shell-click-through` wrote `artifacts/codex-e2e/agent-cursor-kde/0514220502698121-click/summary.json` with `backend=wayland_layer_shell`, `visible_overlay_captured=true`, `click_through_proved=true`, `target_clicked=true`, `execute_action.outcome.success=true`, and `observed_marker_probe.changed_pixels_near_hotspot=151`. Earlier failed artifacts `0514220259174856-click` and `0514220412554986-click` showed no cursor because the fixture never reached readiness; the latter recorded `ModuleNotFoundError: No module named 'gi'` in `pointer.stderr.log`.

- Observation: system cursor hiding is backend-specific and must not be conflated with screenshot cursor modes. Generic click-through Wayland layer-shell clients cannot globally hide the compositor pointer because they do not own pointer focus; X11 can hide and restore the desktop cursor through XFixes; KWin's compositor-side effect API exposes `hideCursor()` and `showCursor()`; Windows remains a deferred native adapter because it needs live proof around ShowCursor/SetCursor, layered-window hit testing, DPI, and capture.
  Evidence: `crates/sky-cua-platform/src/model.rs` now includes `system_cursor_backend`, `crates/sky-cua-overlay-host/src/system_cursor.rs` contains the adapter seam, `crates/sky-cua-overlay-host/src/x11.rs` wires the X11 adapter into the visible-overlay lifecycle, `crates/sky-cua-overlay-host/src/layer_shell.rs` reports unsupported system cursor hiding with a Wayland-focus reason, and `resources/kwin/effects/sky-cua-agent-cursor/systemcursoradapter.cpp` restores KWin's cursor on effect destruction. `uv run python scripts/live_agent_cursor_x11_overlay_smoke.py --embedded-session` wrote `artifacts/codex-e2e/agent-cursor-x11-overlay/20260514T221333143236Z/summary.json` with `system_cursor_hide_supported=true`, `system_cursor_hidden_after_set=true`, `system_cursor_hidden_after_hide=false`, and `system_cursor_hidden_after_show=true`.

- Observation: the KWin C++ effect can consume the same shared cursor-state shape as the generic overlay host. Nested KWin DBus proof is enough for the code-level bridge because the smoke drives `SetCursorState` with `visible`, `sequence`, `native_point`, `model_point`, and `updated_at_ms`, then reads the same compact JSON back from `StateJson`.
  Evidence: `uv run python scripts/live_agent_cursor_kde_smoke.py --mode kwin-effect-nested` wrote `artifacts/codex-e2e/agent-cursor-kde/0514224447208158-kwin-nested/summary.json` with `effect_loaded="true"`, `set_state_stdout="true"`, `requested_point={"x":420.0,"y":260.0}`, `state_readback` containing `native_point` and `model_point` at `420,260`, `visible_overlay_captured=true`, and `observed_marker_probe.changed_pixels_near_hotspot=119`.

- Observation: the KWin effect no longer has a parallel state path in the accepted proof. The nested KWin smoke now builds `sky-cua-overlay-host`, leaves `SKY_CUA_OVERLAY_BACKEND` unset so default `auto` detection is exercised, sends a normal overlay-host `set_cursor` message to `serve`, and requires the host reply to report `backend=kwin_effect` before accepting the screenshot probe.
  Evidence: `uv run python scripts/live_agent_cursor_kde_smoke.py --mode kwin-effect-nested` wrote `artifacts/codex-e2e/agent-cursor-kde/0514225029485897-kwin-nested/summary.json` with `overlay_host_set_reply.ok=true`, `overlay_host_set_reply.capabilities.backend="kwin_effect"`, `overlay_host_set_reply.capabilities.system_cursor_backend="kwin_effect"`, `state_readback` at `420,260`, and `observed_marker_probe.changed_pixels_near_hotspot=119`.

- Observation: on this Plasma/KWin install, Wayland effect package data lives under `kwin-wayland/effects`, but adding that user-level data path still does not make a running KWin process discover a new compiled effect plugin. The static smoke now installs QML/assets under both `~/.local/share/kwin/effects/sky-cua-agent-cursor` and `~/.local/share/kwin-wayland/effects/sky-cua-agent-cursor`, while the plugin `.so` installs under `~/.local/lib/qt6/plugins/kwin/effects/plugins`; after KWin reconfigure and explicit `loadEffect`, KWin still reports the effect as unlisted, unsupported, and unloaded.
  Evidence: `uv run python scripts/live_agent_cursor_kde_smoke.py --mode kwin-effect-static --allow-kwin-effect-install` wrote `artifacts/codex-e2e/agent-cursor-kde/0514225541305594-kwin/summary.json` with `kwin_effect_discovery_after_install.listed=false`, `kwin_effect_load.load_stdout="false"`, `kwin_effect_load.effect_supported=false`, `kwin_effect_load.effect_loaded=false`, and cleanup removing the installed user-level files from both data paths plus the user-level plugin path. A follow-up `find ~/.local/lib ~/.local/share -path '*sky-cua-agent-cursor*' -o -path '*kwin-wayland/effects/sky-cua-agent-cursor*'` returned no leftovers after tightening empty-directory pruning.

- Observation: restarting KWin with a normal user-level install is not enough on this machine unless the Qt plugin path is also made visible. A fresh nested `kwin_wayland` with a temporary `HOME`, effect installed into that temp `HOME/.local`, and no forced `QT_PLUGIN_PATH` did not list or load `sky-cua-agent-cursor`; the immediately following forced-path nested control did list, load, auto-detect, and render it.
  Evidence: `uv run python scripts/live_agent_cursor_kde_smoke.py --mode kwin-effect-nested-user-install` wrote `artifacts/codex-e2e/agent-cursor-kde/0514230356463033-kwin-user/summary.json` with `kwin_user_install_discovered=false`, `kwin_user_install_loaded=false`, `kwin_user_install_load_stdout="false"`, and no `sky-cua-agent-cursor` in `nested_kwin.effect_list`. The forced-path control `uv run python scripts/live_agent_cursor_kde_smoke.py --mode kwin-effect-nested` wrote `artifacts/codex-e2e/agent-cursor-kde/0514230404440837-kwin-nested/summary.json` with `effect_loaded="true"`, `overlay_host_set_reply.capabilities.backend="kwin_effect"`, and visible cursor pixels at `(420,260)`.

- Observation: the service capture guard can hide the native layer-shell overlay for capture while still synthesizing the model-facing cursor afterward.
  Evidence: `uv run python scripts/live_agent_cursor_kde_smoke.py --mode layer-shell-hide-for-capture` wrote `artifacts/codex-e2e/agent-cursor-kde/20260514T200632Z/summary.json` with `synthetic_cursor_found=true`, `native_overlay_hidden_for_capture=true`, and `native_overlay_leak_probe.changed_pixels_near_hotspot=0`.

- Observation: KWin portal capture can race a just-committed layer-shell cursor frame. The overlay-host IPC acknowledgement proves the buffer was committed, but the live smoke needs a short compositor settle before the next capture when it is trying to prove that the native overlay itself is visible.
  Evidence: `uv run python scripts/live_agent_cursor_kde_smoke.py --mode layer-shell-debug-visible` wrote `artifacts/codex-e2e/agent-cursor-kde/20260514T201115Z/summary.json` with only `changed_pixels_near_hotspot=22` before the settle; after adding the settle it wrote `artifacts/codex-e2e/agent-cursor-kde/20260514T201245Z/summary.json` with `visible_overlay_captured=true`, `changed_pixels_near_hotspot=109`, and `max_channel_delta_near_hotspot=225`.

- Observation: cross-desktop VM session switches can leave two independent kinds of stale state that break later cursor proofs. First, the user portal service can remain selected for the previous desktop or restart before the new compositor socket is valid. Second, helper probes such as `cosmic-randr list` can hang when called outside their desktop. The VM runner now refreshes the portal stack after importing the target desktop environment, and Linux virtual input scopes `cosmic-randr` to COSMIC desktops with timeout-protected bounds helpers.
  Evidence: a Plasma `get_app_state` smoke hung while `xdg-desktop-portal` tried to use `org.freedesktop.impl.portal.desktop.cosmic`; after portal refresh it reached the pointer action path. A later Plasma pointer run hung behind a child `cosmic-randr list`; after scoping the probe, `/workspace/artifacts/gui-desktop-smoke/wayland-pointer/20260515T100113Z` passed and `pgrep -a "sky-cua-overlay|sky-cua-service|cosmic-randr"` returned no leftovers after the clean KDE cursor sequence.

- Observation: killing `sky-cua-service` with SIGTERM must still clean up a spawned native overlay host. Without a service signal path, Rust destructors did not run and stale layer-shell overlay hosts remained visible across later smokes, causing the click-through proof's baseline capture to already contain an old cursor. The service now handles SIGTERM and the process overlay client waits/reaps the host during teardown.
  Evidence: before the fix, `pgrep -a "sky-cua-overlay|sky-cua-service"` showed multiple `sky-cua-overlay-host serve --socket ...agent-cursor.sock` processes from older artifacts; after the fix, the clean KDE sequence through `/workspace/artifacts/codex-e2e/agent-cursor-kde/0515100306568235-click` passed with `visible_overlay_captured=true`, `click_through_proved=true`, and no leftover service or overlay-host processes.

- Observation: `systemctl enable --now greetd` followed immediately by `systemctl restart greetd` is too aggressive for the VM session selector during KWin effect proof. It can hit `greetd.service: start-limit-hit` and leave KWin DBus unavailable long enough for the system-install smoke to fail after installing the effect. The selector now resets failed state, enables the service without starting it, and performs exactly one restart.
  Evidence: the failed KWin system-install artifact `artifacts/kde-framebuffer-cursor-proof/kwin-system-install/20260515T100608032917Z/remote.stderr.log` timed out waiting for KWin DBus, and the guest journal showed `greetd.service: Start request repeated too quickly`. After the selector change, `artifacts/kde-framebuffer-cursor-proof/kwin-system-install/20260515T100852814643Z/host-summary.json` passed with `restart_after_install.returncode=0`, `restart_after_cleanup.returncode=0`, and `system_leftovers_after_cleanup=[]`.

- Observation: KWin QML `SceneEffect` is the wrong primitive for a cursor overlay. It can install and load as a user-level KWin effect, but it replaces the default scene instead of painting transparently above it; the live result was a fullscreen black screen with the cursor marker at the center.
  Evidence: `uv run python scripts/live_agent_cursor_kde_smoke.py --mode kwin-effect-static --allow-kwin-effect-install` wrote `artifacts/codex-e2e/agent-cursor-kde/20260514T201800Z/summary.json` with cleanup successful but no acceptable marker proof, and the operator observed the fullscreen black scene replacement. KDE's `SceneEffect` API describes it as a way to replace the default scene; KWin's C++ `Effect::paintScreen` docs describe painting after `effects->paintScreen()` as the compositor-side overlay mechanism.

- Observation: the desktop cursor renderer must use the browser-use image asset, not an approximate marker. The copied native source asset is byte-identical to `resources/chrome-extension/codex/1.1.4_0/images/cursor-chat.png` at 46x48 pixels, while native visible overlays render it at the browser CSS size of 23x24 with the matching 10x11 hotspot. The smoke probe checks that rendered footprint instead of a small center-only circle.
  Evidence: `scripts/test_python_harness_helpers.py::test_bundled_chrome_extension_cursor_overlay_contract` compares the source bytes and asserts the source/rendered dimensions, `crates/sky-cua-overlay-host/src/layer_shell.rs` and `crates/sky-cua-overlay-host/src/x11.rs` resize the copied asset into the native overlay buffer, and `crates/sky-cua-service/src/overlay.rs` composites the same asset into model-facing screenshots.

- Observation: the browser extension stores `cursor-chat.png` as a 46x48 source bitmap but renders it at 23x24 CSS pixels. Native overlays that draw the raw PNG 1:1 look roughly twice as large on KWin Wayland. The native overlay contract now keeps both dimensions explicit: source dimensions prove the asset copy is exact, rendered dimensions prove the desktop cursor matches the browser overlay footprint.
  Evidence: the bundled content script uses 23x24 cursor dimensions for the image wrapper; `crates/sky-cua-overlay-host/src/lib.rs` exposes 46x48 source constants and 23x24 rendered constants; `artifacts/codex-e2e/agent-cursor-kde/0514215147287912-vis/summary.json` reports a 23x24 `checked_box` of `[710,394,733,418]` for the visible layer-shell overlay; `artifacts/codex-e2e/agent-cursor-x11-overlay/20260514T214716796386Z/summary.json` reports a 23x24 `checked_box` of `[320,229,343,253]`.

- Observation: timestamp-only smoke artifact directories are too collision-prone, and long artifact names can exceed Unix socket path limits.
  Evidence: parallel smoke runs collided in the same timestamp directory, then the first longer microsecond/mode artifact path failed service startup with `Error: path must be shorter than SUN_LEN`. The smoke now uses short mode slugs and `svc.sock`, producing paths such as `artifacts/codex-e2e/agent-cursor-kde/0514203517942912-syn/`.

- Observation: X11/XWayland is not a substitute for a native Wayland overlay. The X11 backend can connect on this KDE Wayland session, report `backend=x11_shaped_window`, and round-trip cursor state through the overlay-host protocol, but the forced `x11-debug-visible` smoke did not produce portal-captured cursor pixels over native Wayland surfaces.
  Evidence: `SKY_CUA_OVERLAY_BACKEND=x11 target/debug/sky-cua-overlay-host probe` reports `X Shape visible overlay active on X11/XWayland display; native Wayland surfaces may cover it`; `uv run python scripts/live_agent_cursor_kde_smoke.py --mode x11-debug-visible` wrote `artifacts/codex-e2e/agent-cursor-kde/0514204724762827-x11/summary.json` with `backend=x11_shaped_window` but `visible_overlay_captured=false` and `changed_pixels_near_hotspot=0`.

- Observation: the X11 backend now has an isolated live proof that does not depend on the host Wayland compositor. The embedded-session smoke starts a private Xvfb server, launches Openbox as the X11 window manager, opens a Tk target window, drives `sky-cua-overlay-host serve` with `SKY_CUA_OVERLAY_BACKEND=x11`, captures the X11 root before/visible/hidden/re-shown states, and clicks through the visible shaped overlay into the target window. The same smoke still has `--current-display` for an externally supplied X11 desktop and refuses the host Wayland/XWayland session before doing work.
  Evidence: `uv run python scripts/live_agent_cursor_x11_overlay_smoke.py --embedded-session` wrote `artifacts/codex-e2e/agent-cursor-x11-overlay/20260514T213733162901Z/summary.json` with `mode=embedded-x11-session`, `window_manager.name=Openbox`, `backend=x11_shaped_window`, `visible_overlay_captured=true`, `hidden_overlay_captured=false`, `reshown_overlay_captured=true`, `overlay_visible_for_click=true`, `click_through_proved=true`, `visible_marker_probe.changed_pixels_near_hotspot=970`, `reshown_marker_probe.changed_pixels_near_hotspot=970`, and target click coordinates `x_root=330`, `y_root=240`. `uv run python scripts/live_agent_cursor_x11_overlay_smoke.py --current-display` on this host session still exits with `--current-display is only accepted in a real X11 session; XDG_SESSION_TYPE=wayland`.

- Observation: model screenshot coordinates are not always valid native overlay coordinates. Service-derived states now preserve both planes when the capture metadata can map them: `model_point` remains stream pixels for synthetic screenshots, while `native_point` uses `CaptureInfo.logical_rect` and `pixel_size` for visible overlays. When a successful pointer action cannot be mapped, the service clears the previous cursor state instead of leaving stale synthetic cursor pixels.
  Evidence: `cargo test -p sky-cua-service overlay::tests::` covers bounded-capture native mapping, `StreamLogical` element mapping through capture scale into the native overlay plane, snapshotless native-only cursor state, and stale-cursor clearing; `cargo test -p sky-cua-overlay-host cursor_point` covers native coordinate preference for both layer-shell and X11 visible overlays.

- Observation: the visible-overlay native plane is backend-specific, not a single desktop-global coordinate space. KWin layer-shell surfaces use output-local placement, so portal captures should hand the host `StreamLogical` coordinates within the captured stream/output. X11 shaped windows use root-window pixels, so model-bounded X11 captures must scale from model pixels back to `original_pixel_size` before publishing native overlay coordinates.
  Evidence: `cargo test -p sky-cua-service overlay::tests::` includes `derives_x11_native_cursor_from_original_capture_pixels` and portal logical-rect tests that now expect output-local `StreamLogical` native points; `scripts/test_python_harness_helpers.py::test_kde_smoke_native_point_for_portal_capture_is_output_local` covers the live-smoke fixture side; `artifacts/codex-e2e/agent-cursor-kde/0514215147287912-vis/summary.json` reports `native_point.coordinate_space="stream_logical"` and a successful layer-shell visible proof.

- Observation: the overlay host is a first-class runtime process and must be stopped before deploys replace plugin/cache trees. Runtime cleanup now recognizes `sky-cua-overlay-host` alongside client, service, chrome host, and COSMIC helper processes.
  Evidence: `uv run pytest scripts/test_python_harness_helpers.py -k 'stop_unix_runtime_processes or x11_overlay'` terminates a fake cached `sky-cua-overlay-host` process and still ignores unrelated system `sky-cua-client` processes.

- Observation: `Show` must carry the service's current cursor state and must not cause the host to resurrect stale hidden state after the service intentionally cleared an unmappable cursor. Smokes that talk directly to the host must send explicit state when re-showing, otherwise they can accidentally test click-through after the cursor has been cleared.
  Evidence: `cargo test -p sky-cua-overlay-host` covers `Show` with `state=None` clearing the no-op backend state; `uv run pytest scripts/test_python_harness_helpers.py -k x11_overlay` covers the X11 smoke's stateful show-message helper and visible-state assertion; the refreshed live X11 smoke `artifacts/codex-e2e/agent-cursor-x11-overlay/20260514T212808867996Z/summary.json` shows `reshown_overlay_captured=true`, `overlay_visible_for_click=true`, and `click_through_proved=true`; the refreshed KDE hide-for-capture smoke `artifacts/codex-e2e/agent-cursor-kde/0514212500885392-hide/summary.json` shows `native_overlay_hidden_for_capture=true`, `native_overlay_leak_probe.changed_pixels_near_hotspot=0`, and `synthetic_cursor_found=true`.

- Observation: the safer C++ KWin effect shape is buildable but not currently loadable as a user-level effect in the running KWin session. The effect source under `resources/kwin/effects/sky-cua-agent-cursor/` uses KWin's compositor-painting path rather than `SceneEffect`, embeds valid Qt plugin metadata, installs under `~/.local/lib/qt6/plugins/kwin/effects/plugins/`, and installs QML/assets under `~/.local/share/kwin/effects/sky-cua-agent-cursor/`, but KWin returns `loadEffect=false` and `isEffectSupported=false`. The smoke now records that KWin's DBus `listOfEffects` never includes `sky-cua-agent-cursor` before install, after user-level install, after reconfigure, after load, or after cleanup on this session. This confirms the current blocker is discovery/load path, not marker rendering.
  Evidence: `python3 scripts/live_agent_cursor_kde_smoke.py --mode kwin-effect-static --allow-kwin-effect-install` wrote `artifacts/codex-e2e/agent-cursor-kde/0514213336512348-kwin/summary.json` with `kwin_effect_discovery_after_install.listed=false`, `kwin_effect_load.discovery_after_reconfigure.listed=false`, `kwin_effect_load.load_stdout="false"`, `effect_loaded=false`, `effect_supported=false`, `kwin_effect_static_marker_found=false`, and cleanup removed the installed user-level files. The same run built `kwin-effect-build/lib/kwin/effects/plugins/sky-cua-agent-cursor.so`. Local package evidence from `pacman -Ql kwin` shows KWin 6.6.4 ships built-in effect metadata under `/usr/share/kwin-wayland/builtin-effects/` and scripted effect packages under `/usr/share/kwin-wayland/effects/`, but no system compiled effect plugins under `/usr/lib/qt6/plugins/kwin/effects/plugins/`.

- Observation: KWin discovers, loads, and renders the compiled effect when its Qt plugin and XDG data paths include the effect prefix at compositor startup. This proves the C++ effect code, metadata, resource layout, transparent compositor painting, and KWin system-cursor adapter shape independently of the running-session user-level discovery blocker.
  Evidence: `uv run python scripts/live_agent_cursor_kde_smoke.py --mode kwin-effect-nested` wrote `artifacts/codex-e2e/agent-cursor-kde/0514223551637070-kwin-nested/summary.json` with `ok=true`, `nested_kwin.effect_list` containing `sky-cua-agent-cursor`, `nested_kwin.load_stdout="true"`, `nested_kwin.effect_loaded="true"`, `visible_overlay_captured=true`, and `observed_marker_probe.changed_pixels_near_hotspot=119`. The captured PNG is `artifacts/codex-e2e/agent-cursor-kde/0514223551637070-kwin-nested/nested-kwin-effect-capture.png`.

- Observation: `OffscreenQuickScene` setup must not happen from inside `paintScreen` for this effect. The original prototype crashed nested KWin after load with a backtrace ending in `KWin::EglContext::streamingVbo() const` / `KWin::ItemRendererOpenGL::endFrame()`. KWin's own `showfps`, `showcompositing`, and `outputlocator` effects create or load their `OffscreenQuickScene` during `prePaintScreen` or explicit activation and only render it during `paintScreen`.
  Evidence: moving `ensureScene()` to `SkyCuaAgentCursorEffect::prePaintScreen` made the nested KWin proof pass. Local source inspection in `/tmp/kwin-v6.6.4/src/plugins/showfps/showfpseffect.cpp`, `/tmp/kwin-v6.6.4/src/plugins/showcompositing/showcompositing.cpp`, and `/tmp/kwin-v6.6.4/src/plugins/outputlocator/outputlocator.cpp` shows the same setup-before-render pattern.

## Decision Log

- Decision: implement two cursor planes: a user-visible overlay plane and a model-facing synthetic screenshot plane.
  Rationale: Wayland does not provide one universal app-level way to draw above every other client. A synthetic screenshot cursor gives consistent model behavior even when the visible overlay is unavailable, hidden during capture, or compositor-specific.
  Date/Author: 2026-05-14 / Codex

- Decision: keep the overlay controller in `sky-cua-service`, not in the MCP client and not inside the `DesktopBackend` trait.
  Rationale: `sky-cua-service` already owns backend lifetime, snapshot storage, action routing, and IPC. It can supervise a helper process and keep cursor state without making every platform backend own process management.
  Date/Author: 2026-05-14 / Codex

- Decision: allow platform backends to report cursor intent through `ActionOutcome`, but keep the authoritative latest cursor state in the service.
  Rationale: Linux and Windows backends know how they map screenshot coordinates to native input coordinates. The service knows the previous snapshot and can publish the state to the overlay host. An optional field on `ActionOutcome` avoids duplicating coordinate logic in the service while keeping action success independent from overlay success.
  Date/Author: 2026-05-14 / Codex

- Decision: KWin tests and proof come before a KWin effect.
  Rationale: a KWin effect is a compositor-specific integration. It should consume a proven generic cursor-state and capture contract, not define the contract. This prevents a beautiful KDE-only solution from boxing in GNOME, COSMIC, X11, Windows, or macOS.
  Date/Author: 2026-05-14 / Codex

- Decision: Windows implementation is deferred until a Windows machine is available, but the plan records the Windows contract now.
  Rationale: the correct Windows path needs live proof of layered-window focus behavior, DPI/multi-monitor coordinates, capture exclusion, and RDP/UIPI degradation. Implementing it blind on Linux would create exactly the kind of paper feature this project avoids.
  Date/Author: 2026-05-14 / Codex

- Decision: visible overlay failure must never fail a Computer Use action.
  Rationale: the cursor overlay is observability and operator experience. If the overlay host crashes, an action that would otherwise click, type, or capture correctly should still proceed and include diagnostics.
  Date/Author: 2026-05-14 / Codex

- Decision: system cursor hiding lives behind backend adapters and is reported separately from visible overlay and screenshot cursor capabilities.
  Rationale: KWin Wayland, generic Wayland layer-shell, X11, GNOME/COSMIC extensions, Windows layered windows, and macOS panels do not share one cursor API. A single `hideCursor()` call in rendering code would either lie on Wayland or make Windows and compositor-specific paths harder later. The shared contract now records `system_cursor_backend`, `system_cursor_hide_supported`, and `system_cursor_hidden`; each backend adapter owns the OS/compositor-specific hide and restore behavior.
  Date/Author: 2026-05-14 / Codex

- Decision: the first overlay-host crate is deliberately a versioned no-op backend.
  Rationale: it lets packaging, protocol shape, and process invocation settle before compositor-specific Wayland or KWin drawing code is introduced. The service still owns the authoritative cursor state and synthetic screenshot marker in this slice.
  Date/Author: 2026-05-14 / Codex

- Decision: implement the generic Wayland visible backend with `smithay-client-toolkit` layer-shell support before any KWin effect work.
  Rationale: the docs/API pass showed `smithay-client-toolkit` exposes the layer-shell role, overlay layer, configure handling, shared-memory buffers, and empty input regions with less raw protocol plumbing than hand-written `wayland-client` dispatch. Proving this on KWin first keeps KDE as the evidence target while preserving a compositor-probed path for other layer-shell desktops.
  Date/Author: 2026-05-14 / Codex

- Decision: do not install or ship a QML `SceneEffect` KWin cursor prototype.
  Rationale: the cursor overlay must be transparent and click-through. A QML `SceneEffect` is a fullscreen scene replacement, and the live proof showed it can blank the desktop behind the marker. A KWin effect path must instead be a true compositor-painting effect, likely a C++ `Effect` that calls `effects->paintScreen()` first and paints the marker afterward, or it should be skipped in favor of the already-proved layer-shell path.
  Date/Author: 2026-05-14 / Codex

- Decision: keep one copied native cursor asset in the overlay-host crate and render it from both native overlay and synthetic screenshot paths.
  Rationale: the browser extension already defines the operator-facing cursor art. The native Computer Use path should match that asset exactly while avoiding a runtime dependency on bundled Chrome extension files.
  Date/Author: 2026-05-14 / Codex

- Decision: do not auto-select the X11 backend on Wayland sessions.
  Rationale: X11/XWayland windows can be covered by native Wayland surfaces and cannot be treated as a reliable global Wayland overlay. Auto mode should prefer layer-shell on Wayland and use X11 automatically only for real X11 sessions or sessions without `WAYLAND_DISPLAY`; explicit `SKY_CUA_OVERLAY_BACKEND=x11` remains available for testing and true X11 desktops.
  Date/Author: 2026-05-14 / Codex

- Decision: make layer-shell native cursor points output-local `StreamLogical`, not desktop-global `DesktopLogical`.
  Rationale: the current layer-shell backend creates a compositor-chosen overlay layer and positions the cursor by margins on that output. Passing desktop-global coordinates would offset the cursor on non-zero-origin monitor layouts. Keeping portal/layer-shell points output-local matches the ScreenCast stream/input contract and leaves a clear future path for one-surface-per-output binding.
  Date/Author: 2026-05-14 / Codex

- Decision: keep the KWin C++ effect as an explicit prototype and nested proof, not a default production backend on this machine.
  Rationale: the C++ effect is the right transparent/click-through compositor-painting primitive, and nested KWin proves the compiled effect can be discovered, loaded, render the cursor, and hide the system cursor through a KWin adapter when KWin starts with the right plugin/data paths. The running user session still does not discover the user-level compiled plugin from `~/.local/lib/qt6/plugins/kwin/effects/plugins`. Accepting it as a default backend would require system packaging or launching/restarting KWin with a modified plugin path, both outside the current safe user-level contract. The already-proved Wayland layer-shell backend remains the default KWin visible overlay path.
  Date/Author: 2026-05-14 / Codex

- Decision: default overlay-host behavior should auto-detect the active backend, including a loaded KWin effect, with environment variables kept as overrides.
  Rationale: ordinary plugin startup should not require the caller to know the compositor integration in advance. A loaded KWin effect is a stronger Wayland backend than layer-shell because it can also hide the system cursor, so auto mode should pick it first when it is already present. The unresolved production boundary is effect installation/discovery, not state routing; until that path exists, auto detection simply falls back to the already-proved layer-shell or X11 backends.
  Date/Author: 2026-05-14 / Codex

## Outcomes & Retrospective

- 2026-05-14: Generic cursor state, service IPC, action-derived cursor placement, and synthetic screenshot compositing are implemented. The model-facing cursor is now independent of a native overlay backend.
- 2026-05-14: KDE Wayland synthetic smoke passed live. The proof used a private service socket and pixel inspection of the generated `.agent-cursor.jpg` screenshot, so this is stronger than a source-only or file-exists check.
- 2026-05-14: The release runtime build path now asks the same platform binary contract as `scripts/package_runtime_artifact.py`, so adding `sky-cua-overlay-host` to runtime packaging cannot silently drift from the tag workflow.
- 2026-05-14: Service-to-host process IPC is implemented and covered by focused Rust tests. Overlay-host failures degrade into cursor diagnostics and do not fail Computer Use actions.
- 2026-05-14: KWin Wayland layer-shell visible overlay passed live. Debug-visible mode proved the native overlay is rendered, and hide-for-capture mode proved the service hides that native overlay while still adding the synthetic model-facing cursor.
- 2026-05-14: Current validation passed with Rust workspace tests, Python checks, plugin build, runtime package build, and live KDE smoke modes for synthetic, layer-shell debug-visible, and layer-shell hide-for-capture.
- 2026-05-14: The QML KWin effect package path was rejected after live proof because it was not transparent or click-through. The smoke mode now refuses to install a KWin effect until a safer compositor-painting implementation exists.
- 2026-05-14: After the QML rejection, validation passed again with `cargo fmt --all --check`, `cargo test --workspace`, `uv run ruff format --check scripts`, `uv run ruff check scripts`, `uv run basedpyright`, `uv run pytest`, and a direct fail-fast run of the disabled `kwin-effect-static` mode.
- 2026-05-14: The native cursor now uses the Chrome extension's `cursor-chat.png` image. Sequential KDE smoke passed with the real asset: `0514203517942912-syn` found the synthetic cursor (`changed_pixels_near_hotspot=367`), `0514203521283220-vis` captured the visible layer-shell overlay (`changed_pixels_near_hotspot=42`), and `0514203524777327-hide` proved hide-for-capture (`native_overlay_leak_probe.changed_pixels_near_hotspot=0`, synthetic cursor found with `changed_pixels_near_hotspot=243`).
- 2026-05-14: Final validation for the real cursor asset slice passed with `cargo fmt --all --check`, `cargo test --workspace`, `cargo clippy --workspace --all-targets`, `uv run ruff format --check scripts`, `uv run ruff check scripts`, `uv run basedpyright`, `uv run pytest`, `python3 scripts/build_plugin.py`, and `python3 scripts/build_runtime_packages.py --platform linux-x64`. `sha256sum` confirms the native asset matches the bundled Chrome extension asset.
- 2026-05-14: The X11 shaped-window backend is implemented and covered by focused Rust/Python tests. Explicit probe on the current KDE Wayland session succeeds against X11/XWayland, but the visible XWayland smoke did not appear in portal capture, so X11 acceptance required a non-XWayland session.
- 2026-05-14: The X11 shaped-window backend gained a nested-X11 live proof under Xvfb. The proof captures visible cursor pixels, verifies hide removes those pixels, and proves click-through by delivering a click through the visible overlay to a Tk target. This strengthened the backend proof before the later Openbox-backed embedded-session acceptance mode.
- 2026-05-14: The dedicated X11 overlay smoke has a `--current-display` mode for externally supplied X11 desktops. On this KDE Wayland host it refuses cleanly instead of accepting XWayland as evidence.
- 2026-05-14: Fresh validation after the X11 backend passed with `cargo fmt --all --check`, `cargo test --workspace`, `cargo clippy --workspace --all-targets`, `uv run ruff format --check scripts`, `uv run ruff check scripts`, `uv run basedpyright`, `uv run pytest`, `python3 scripts/build_plugin.py`, `python3 scripts/build_runtime_packages.py --platform linux-x64`, and `git diff --check`. Fresh KDE live smoke artifacts are `0514204942102267-syn`, `0514204947573747-vis`, and `0514204953401984-hide`.
- 2026-05-14: Fresh validation after the X11 smoke hardening passed with `cargo fmt --all --check`, `cargo test --workspace`, `cargo clippy --workspace --all-targets`, `uv run ruff format --check scripts`, `uv run ruff check scripts`, `uv run basedpyright`, `uv run pytest`, `uv run python scripts/live_agent_cursor_x11_overlay_smoke.py`, `uv run python scripts/live_agent_cursor_x11_overlay_smoke.py --current-display` returning the expected Wayland refusal, `python3 scripts/build_plugin.py`, `python3 scripts/build_runtime_packages.py --platform linux-x64`, `git diff --check`, and a SHA-256 check confirming the native cursor asset still matches the Chrome extension cursor.
- 2026-05-14: Fresh KDE Wayland live smoke artifacts against the current tree are `0514211541846981-syn` (`synthetic_cursor_found=true`, `changed_pixels_near_hotspot=367`), `0514211548411697-vis` (`visible_overlay_captured=true`, `backend=wayland_layer_shell`, `changed_pixels_near_hotspot=359`), and `0514211556449915-hide` (`native_overlay_hidden_for_capture=true`, leak probe `changed_pixels_near_hotspot=0`, synthetic cursor found with `changed_pixels_near_hotspot=482`).
- 2026-05-14: Review-fix validation passed with `cargo test -p sky-cua-service overlay::tests::`, `cargo test -p sky-cua-overlay-host cursor_point`, `cargo test --workspace`, `cargo clippy --workspace --all-targets`, `uv run ruff format --check scripts`, `uv run ruff check scripts`, `uv run basedpyright`, `uv run pytest`, `uv run python scripts/live_agent_cursor_x11_overlay_smoke.py` (`20260514T212047209215Z`), `uv run python scripts/live_agent_cursor_x11_overlay_smoke.py --current-display` returning the expected Wayland refusal on the host session, the same `--current-display` path against an externally started private Xvfb display (`20260514T212215692667Z`), `python3 scripts/build_plugin.py`, `python3 scripts/build_runtime_packages.py --platform linux-x64`, `git diff --check`, and a SHA-256 cursor asset match.
- 2026-05-14: Final refresh after making overlay-host `Show` authoritative passed with `cargo fmt --all --check`, `cargo test -p sky-cua-overlay-host`, `cargo test -p sky-cua-service overlay::tests::`, `cargo test --workspace`, `cargo clippy --workspace --all-targets`, `uv run ruff format --check scripts`, `uv run ruff check scripts`, `uv run basedpyright`, `uv run pytest`, `uv run python scripts/live_agent_cursor_x11_overlay_smoke.py` (`20260514T212456479041Z`), `uv run python scripts/live_agent_cursor_kde_smoke.py --mode layer-shell-hide-for-capture` (`0514212500885392-hide`), `uv run python scripts/live_agent_cursor_x11_overlay_smoke.py --current-display` returning the expected Wayland refusal, `python3 scripts/build_plugin.py`, `python3 scripts/build_runtime_packages.py --platform linux-x64`, `git diff --check`, and a SHA-256 cursor asset match.
- 2026-05-14: X11 smoke proof was tightened after the authoritative `Show` change. `uv run python scripts/live_agent_cursor_x11_overlay_smoke.py` now requires `reshown_overlay_captured=true` and `overlay_visible_for_click=true` before accepting `click_through_proved=true`; artifact `20260514T212808867996Z` has all three. Validation passed with `uv run pytest scripts/test_python_harness_helpers.py -k x11_overlay`, `uv run ruff format --check scripts`, `uv run ruff check scripts`, `uv run basedpyright`, `uv run pytest`, `cargo fmt --all --check`, `cargo test --workspace`, `cargo clippy --workspace --all-targets`, and `git diff --check`.
- 2026-05-14: KWin effect smoke proof was tightened to record KWin's actual effect discovery state. Artifact `0514213336512348-kwin` shows the user-level compiled effect builds and installs, but KWin's own `listOfEffects` reports `listed=false` before install, after install, after reconfigure, after load, and after cleanup. Focused Python validation passed with `uv run pytest scripts/test_python_harness_helpers.py -k 'kwin_effect or x11_overlay'`.
- 2026-05-14: The X11 acceptance proof now runs in an embedded X11 desktop session using Xvfb plus Openbox. Artifact `20260514T213733162901Z` proves visible overlay capture, hide removes the marker, re-show restores it before the click, and click-through reaches the Tk target while `wmctrl -m` reports `Openbox` as the window manager. The service coordinate edge-case test `derives_element_native_cursor_through_stream_logical_capture_scale` now covers `StreamLogical` element bounds mapped into native overlay coordinates.
- 2026-05-14: Final local validation after the embedded X11 acceptance pass completed with `cargo fmt --all --check`, `cargo test --workspace`, `cargo clippy --workspace --all-targets`, `uv run ruff format --check scripts`, `uv run ruff check scripts`, `uv run basedpyright`, `uv run pytest`, `python3 scripts/build_plugin.py`, `python3 scripts/build_runtime_packages.py --platform linux-x64`, and `git diff --check`. The plugin bundle includes `bin/sky-cua-overlay-host` and `bin/runtimes/linux-x64/sky-cua-overlay-host`; SHA-256 still proves the native cursor asset is an exact copy of the bundled Chrome extension `cursor-chat.png`.
- 2026-05-14: Final refresh caught and fixed one Windows-only compile gap hidden by the Linux default target: Windows physical fallback matches now reject `perform_action` as UIA-only instead of leaving the enum non-exhaustive. Validation passed with `cargo test -p sky-cua-platform -p sky-cua-overlay-host -p sky-cua-service -p sky-cua-client -p sky-cua-linux`, `cargo check -p sky-cua-windows --target x86_64-pc-windows-msvc`, `uv run ruff format --check scripts`, `uv run ruff check scripts`, `uv run basedpyright`, `uv run pytest`, `python3 scripts/build_plugin.py`, `python3 scripts/build_runtime_packages.py --platform linux-x64`, `python3 scripts/package_runtime_artifact.py --platform linux-x64 --output-root artifacts/runtime`, and `uv run python scripts/live_agent_cursor_x11_overlay_smoke.py --embedded-session` (`20260514T214651087311Z`).
- 2026-05-14: Final refresh after the cursor rendered-size and native-coordinate fixes passed with `cargo fmt --all --check`, `cargo test --workspace`, `cargo clippy --workspace --all-targets`, `cargo check -p sky-cua-windows --target x86_64-pc-windows-msvc`, `uv run ruff format --check scripts`, `uv run ruff check scripts`, `uv run basedpyright`, `uv run pytest`, `python3 scripts/build_runtime_packages.py --platform linux-x64`, `python3 scripts/build_plugin.py`, `python3 scripts/package_runtime_artifact.py --platform linux-x64 --output-root artifacts/runtime`, `git diff --check`, and a SHA-256 cursor asset match. Fresh live artifacts are KWin visible `0514215147287912-vis`, KWin hide-for-capture `0514215158159799-hide`, and embedded X11 `20260514T214716796386Z`; each records a 23x24 rendered cursor footprint.
- 2026-05-14: KWin layer-shell click-through is now live-proved with artifact `0514220502698121-click`: the cursor was visible over the GTK fixture target and the service-delivered portal click reached the target underneath with `target_clicked=true`.
- 2026-05-14: System cursor hiding now has backend adapters and wire-level capability fields. The embedded X11 smoke artifact `20260514T221333143236Z` proves the XFixes adapter reports `system_cursor_hide_supported=true`, hides the OS cursor while the agent cursor is visible, restores it on hide, and hides it again on show. Generic layer-shell reports unsupported system cursor hiding instead of pretending it can control the compositor pointer; the KWin C++ effect prototype uses KWin's compositor API but remains blocked on user-level plugin discovery.
- 2026-05-14: Final validation after the system-cursor adapter slice passed with `cargo fmt --all --check`, `cargo test --workspace`, `cargo clippy --workspace --all-targets`, `cargo check -p sky-cua-windows --target x86_64-pc-windows-msvc`, `uv run ruff format --check scripts/live_agent_cursor_x11_overlay_smoke.py scripts/test_python_harness_helpers.py`, `uv run ruff check scripts/live_agent_cursor_x11_overlay_smoke.py scripts/test_python_harness_helpers.py`, `uv run basedpyright`, `uv run pytest`, `python3 scripts/build_plugin.py`, `python3 scripts/build_runtime_packages.py --platform linux-x64`, `python3 scripts/package_runtime_artifact.py --platform linux-x64 --output-root artifacts/runtime`, `git diff --check`, and a SHA-256 cursor asset match. Fresh live artifacts are embedded X11 `20260514T221333143236Z` and KWin layer-shell click-through `0514221547628720-click`.
- 2026-05-14: The system-cursor capability contract now names the adapter backend with `system_cursor_backend`, so callers can distinguish `none`, `wayland_client_unsupported`, and `x11_xfixes` instead of inferring behavior from prose. Focused validation passed with `cargo fmt --all --check`, `cargo test -p sky-cua-platform -p sky-cua-overlay-host -p sky-cua-service overlay::tests -- --nocapture`, `cargo test -p sky-cua-overlay-host -p sky-cua-platform -- --nocapture`, `cargo clippy -p sky-cua-platform -p sky-cua-overlay-host -p sky-cua-service --all-targets`, `uv run ruff format --check scripts/live_agent_cursor_kde_smoke.py scripts/test_python_harness_helpers.py`, `uv run ruff check scripts/live_agent_cursor_kde_smoke.py scripts/test_python_harness_helpers.py`, and `uv run basedpyright`.
- 2026-05-14: Nested KWin effect proof now passes with artifact `0514223551637070-kwin-nested`: nested KWin listed and loaded `sky-cua-agent-cursor`, KWin `ScreenShot2` captured the compositor output, and the smoke found the 23x24 cursor marker at the expected center box with `changed_pixels_near_hotspot=119`. The C++ effect setup now follows KWin's own pattern by loading `OffscreenQuickScene` in `prePaintScreen` and only rendering it in `paintScreen`.
- 2026-05-14: Final validation for the adapter-backend and nested KWin effect slice passed with `cargo fmt --all --check`, `cargo test --workspace`, `cargo clippy --workspace --all-targets`, `cargo check -p sky-cua-windows --target x86_64-pc-windows-msvc`, `uv run ruff format --check scripts`, `uv run ruff check scripts`, `uv run basedpyright`, `uv run pytest`, `python3 scripts/build_plugin.py`, `python3 scripts/build_runtime_packages.py --platform linux-x64`, `python3 scripts/package_runtime_artifact.py --platform linux-x64 --output-root artifacts/runtime`, `git diff --check`, `uv run python scripts/live_agent_cursor_kde_smoke.py --mode kwin-effect-nested` (`0514223551637070-kwin-nested`), `uv run python scripts/live_agent_cursor_x11_overlay_smoke.py --embedded-session` (`20260514T223846716418Z`), `uv run python scripts/live_agent_cursor_kde_smoke.py --mode layer-shell-click-through` (`0514223956016449-click`), and a SHA-256 cursor asset match.
- 2026-05-14: The KWin effect live cursor bridge now passes in nested KWin with artifact `0514224447208158-kwin-nested`: the smoke loaded `sky-cua-agent-cursor`, called `SetCursorState` over DBus with the shared `AgentCursorState` shape, read the state back through `StateJson`, and captured the 23x24 cursor at requested non-center point `(420,260)`.
- 2026-05-14: The KWin effect live cursor bridge is now driven through the generic native host protocol, not direct test-only DBus. Artifact `0514225029485897-kwin-nested` proves default `sky-cua-overlay-host serve` auto-detected the loaded KWin effect, returned `backend=kwin_effect`, `system_cursor_backend=kwin_effect`, `system_cursor_hidden=true`, and rendered the requested non-center cursor in nested KWin.
- 2026-05-14: Validation after the auto-detected KWin effect host backend passed with `cargo fmt --all --check`, `cargo test --workspace`, `cargo clippy --workspace --all-targets`, `cargo check -p sky-cua-windows --target x86_64-pc-windows-msvc`, `uv run ruff format --check scripts`, `uv run ruff check scripts`, `uv run basedpyright`, `uv run pytest`, `python3 scripts/build_plugin.py`, `python3 scripts/build_runtime_packages.py --platform linux-x64`, `python3 scripts/package_runtime_artifact.py --platform linux-x64 --output-root artifacts/runtime`, `git diff --check`, `target/debug/sky-cua-overlay-host probe`, `uv run python scripts/live_agent_cursor_kde_smoke.py --mode kwin-effect-nested` (`0514225029485897-kwin-nested`), and a SHA-256 cursor asset match. The host-session probe auto-selected `wayland_layer_shell` because the production KWin effect is not loaded there, while the nested proof auto-selected `kwin_effect` after loading the effect.
- 2026-05-14: The running-session KWin effect discovery blocker remains after matching local KWin Wayland data paths. Artifact `0514225541305594-kwin` proves that the user-level install writes the compiled plugin plus both `kwin/effects` and `kwin-wayland/effects` data resources, but the already-running KWin process still does not list, support, or load the new compiled effect after reconfigure. Cleanup now prunes the new `kwin-wayland` empty directories as well. This keeps layer-shell as the production default until a real package/system install or KWin restart-with-plugin-path flow is available.
- 2026-05-14: Validation after the KWin package-shape and cleanup fix passed with `uv run python scripts/live_agent_cursor_kde_smoke.py --mode kwin-effect-nested` (`0514225508042900-kwin-nested`), `uv run python scripts/live_agent_cursor_kde_smoke.py --mode kwin-effect-static --allow-kwin-effect-install` (`0514225541305594-kwin`, expected failure proving running-session discovery remains blocked), `uv run ruff format --check scripts/live_agent_cursor_kde_smoke.py`, `uv run ruff check scripts/live_agent_cursor_kde_smoke.py`, `uv run basedpyright`, `git diff --check`, and an explicit leftover-file check under `~/.local`.
- 2026-05-14: KWin effect source/resources are now protected by the bundle path before commit tracking. Validation passed with `uv run pytest scripts/test_python_harness_helpers.py -k 'worktree_bundle_dirs or kwin_effect'`, `python3 scripts/build_plugin.py`, direct checks that `dist/plugin/sky-cua/resources/kwin/effects/sky-cua-agent-cursor/metadata.json` contains `KPackageStructure=KWin/Effect` and `KPlugin.Id=sky-cua-agent-cursor`, direct checks that bundled `CMakeLists.txt` and `qml/main.qml` exist, `uv run ruff format --check scripts/build_plugin.py scripts/test_python_harness_helpers.py`, `uv run ruff check scripts/build_plugin.py scripts/test_python_harness_helpers.py`, and `git diff --check`.
- 2026-05-15: Nested KWin now has both sides of the restart/relogin question covered. The user-install proof `0514230356463033-kwin-user` shows a fresh nested `kwin_wayland` does not discover the compiled effect from temp `HOME/.local` without `QT_PLUGIN_PATH`; the forced-path control `0514230404440837-kwin-nested` shows the same build works when KWin starts with the plugin/data paths. Validation passed with `uv run ruff format --check scripts/live_agent_cursor_kde_smoke.py`, `uv run ruff check scripts/live_agent_cursor_kde_smoke.py`, `uv run basedpyright`, `git diff --check`, and a stale nested-process check.
- 2026-05-15: The KDE Neon Docker path was abandoned after live builds exposed slow and unreliable Neon apt metadata/mirror behavior. The later Arch Docker harness is also retired as an acceptance path because it proves nested container compositors, not standalone guest sessions. Keep only the useful output from that work: the package list, Chrome/Codex Desktop requirements, host-built runtime boundary, and smoke-profile shape.
- 2026-05-15: Linux GUI acceptance now targets an Arch `testing-vm` under QEMU/libvirt/virt-manager. The VM should boot a real COSMIC, Plasma, GNOME, Hyprland, or i3 session on its own display. Host builds are still the rule: `scripts/run_gui_testing_vm_smoke.py` builds `sky-cua-client`, `sky-cua-service`, and `sky-cua-overlay-host` on the host, syncs them under `/workspace`, copies selected `~/.codex` state, and runs profiles over SSH.
- 2026-05-15: The VM provisioner, `scripts/testing-vm/provision-arch-testing-vm.sh`, is the new package source of truth. It installs the desktop matrix, matching terminal apps for COSMIC/KDE/GNOME/Hyprland/i3 tests, Chrome for Browser Use tests, OpenCode for future non-Codex harness tests, cursor overlay build dependencies, optional Codex Desktop from the local CodexDesktop-Rebuild Arch package, greetd autologin, SSH, and rsync. OpenCode config/auth are copied separately with `scripts/testing-vm/sync-opencode-to-vm.sh` so the VM does not inherit the host OpenCode DB/log/snapshot history.
- 2026-05-15: Legacy nested visual-debug profiles are still useful for tight compositor debugging, but they are not session-matrix acceptance. COSMIC acceptance means booting the VM into Wayland COSMIC and running the Computer Use smoke suite there; the same rule applies to Plasma, GNOME, Hyprland, and i3.
- 2026-05-15: The first Arch `testing-vm` was live-provisioned and booted into a real COSMIC Wayland guest session. `cosmic-session`, `cosmic-comp`, and `/run/user/1000/wayland-1` were active. The old embedded-X11 VM smoke was retired; `computer-use` now maps to the visible real-session pointer smoke.
- 2026-05-15: The same COSMIC VM now has backend-specific helper proof. `scripts/run_gui_testing_vm_smoke.py --profile cosmic-helper --wayland-display wayland-1` launched `weston-flower`, and `sky-cua-cosmic-helper` proved `probe`, `list-windows`, `activate-window`, and `focused-window` against the real COSMIC session. Artifact: `/workspace/artifacts/gui-desktop-smoke/cosmic-helper/20260515T031400Z/`.
- 2026-05-15: Plasma VM acceptance needs the graphical session on the normal user DBus bus. Launching Plasma under `dbus-run-session` made KWin and PowerDevil invisible from SSH/user services, broke KWin effect discovery, and made portal/session state misleading. The provisioner now imports the desktop env into `/run/user/<uid>/bus` and execs the session directly.
- 2026-05-15: KDE visual framebuffer proofs must keep the VM awake and compare screenshots in RGB. A blanked/locked guest and RGBA comparisons against `virsh screenshot` PNGs both produced false negatives during cursor proof. After disabling the locker/PowerDevil and comparing RGB, the layer-shell cursor proof reduced to the expected 23x24 changed region.
- 2026-05-15: KWin layer-shell transparency is valid with the full-output transparent surface when the session is awake. The accepted clean proof is `artifacts/kde-framebuffer-cursor-proof/cursor-overlay-clean/after.png`; the service-level accepted artifacts are `/workspace/artifacts/codex-e2e/agent-cursor-kde/0515073739578726-hide` and `/workspace/artifacts/codex-e2e/agent-cursor-kde/0515073759757099-click`.
- 2026-05-15: The KDE VM full pointer smoke artifact `/workspace/artifacts/gui-desktop-smoke/wayland-pointer/20260515T074158Z` has `clicked=true`, `drag_completed=true`, `scroll_events=1`, and successful MCP `click`, `drag`, and `scroll` results. The local runner transport dropped before cleanup, so stale `sky-cua-service`/overlay processes were killed manually after confirming the artifact.
- 2026-05-15: The i3/X11 VM profile needed to derive the real Xorg display and authority file from the running `Xorg` command line. After `startx` selected `:1`, the user systemd environment still had stale Plasma `DISPLAY=:0`/`WAYLAND_DISPLAY=wayland-0`; `scripts/testing-vm/profiles/i3.sh` now reconstructs a temporary Xauthority for the real display and unsets Wayland before running the X11 overlay smoke.
- 2026-05-15: X11 acceptance is now proved through the preferred VM runner, not just the old embedded-X11 harness. Artifact `/workspace/artifacts/codex-e2e/agent-cursor-x11-overlay/20260515T075301057704Z` reports `backend=x11_shaped_window`, `visible_overlay_captured=true`, `hidden_overlay_captured=false`, `reshown_overlay_captured=true`, `click_through_proved=true`, `system_cursor_hidden_after_set=true`, `system_cursor_hidden_after_hide=false`, and `system_cursor_hidden_after_show=true`.
- 2026-05-15: The KWin production effect blocker persists even after the Plasma VM session runs on the normal user DBus bus. Artifact `/workspace/artifacts/codex-e2e/agent-cursor-kde/0515075621741796-kwin` proves KWin DBus is reachable and the effect builds/installs/cleans up, but KWin's own discovery remains `listed=false` before install, after install, after reconfigure, after load, and after cleanup. Keep layer-shell as the production KWin default until the effect is packaged in a KWin-discoverable system path or another supported plugin deployment path is proved.
- 2026-05-15: The system package path is viable and automated in the VM. Installing to `/usr/lib/qt6/plugins/kwin/effects/plugins` plus `/usr/share/kwin{,-wayland}/effects`, then restarting Plasma, makes KWin list and load `sky-cua-agent-cursor`. The KWin effect bridge reports `system_cursor_hidden=true` through the overlay host, and the VM runner now performs host-side libvirt framebuffer capture/probing. In-guest real-session `ScreenShot2` remains authorization-blocked, so the runner deliberately uses host framebuffer capture for the production KWin pixel proof.
- 2026-05-15: Real session switching in the VM needs explicit stale-process cleanup. A raw Plasma-to-Hyprland greetd restart left old `kwin_wayland` alive on `wayland-0` while Hyprland was active on `wayland-1`; use `scripts/testing-vm/select-session.sh` before session-matrix runs.
- 2026-05-15: Hyprland is stricter than KWin about layer-shell configure ordering and `grim` output selection. The overlay host must skip unconfigured layer surfaces, and the real-session smoke must capture the focused nonzero Hyprland output (`Virtual-1` in the current VM) instead of relying on a default `grim` capture when a zero-sized output exists.
- 2026-05-15: COSMIC portal setup is now understood and documented. The VM provisioner exports `XDG_CURRENT_DESKTOP`/session variables for each real session, and the runner can pass `--desktop-env` for SSH-launched smokes. With `XDG_CURRENT_DESKTOP=COSMIC`, `xdg-desktop-portal` selects `org.freedesktop.impl.portal.desktop.cosmic` and exposes ScreenCast/Screenshot. Current Arch and upstream `xdg-desktop-portal-cosmic` do not advertise or implement RemoteDesktop, so COSMIC physical input uses `LinuxVirtualInput` rather than a forced GNOME/KDE portal.
- 2026-05-15: COSMIC VM smoke status after the Linux virtual input pass: `cosmic-helper` passed at `/workspace/artifacts/gui-desktop-smoke/cosmic-helper/20260515T034206Z/`, `codex-desktop` passed at `/workspace/artifacts/gui-desktop-smoke/codex-desktop/20260515T034206Z/`, and `computer-use`/`wayland-pointer` now passes at 1x (`/workspace/artifacts/gui-desktop-smoke/wayland-pointer/20260515T092606Z/`) and through the repeatable 125% scale profile (`/workspace/artifacts/gui-desktop-smoke/wayland-pointer/20260515T093737Z/`) with `clicked=true`, `drag_completed=true`, `scroll_events=1`, `entry_text="cosmic-text-smoke"`, and `submitted_text="cosmic-text-smoke"`. Pointer movement/click/drag/scroll uses direct absolute `/dev/uinput`; text/key uses the ydotool sub-adapter. The earlier no-backend blocker remains documented at `/workspace/artifacts/gui-desktop-smoke/wayland-pointer/20260515T034151Z/`, and the intermediate pointer-only proof remains `/workspace/artifacts/gui-desktop-smoke/wayland-pointer/20260515T091758Z/`. Live ydotool calibration showed ydotool is not acceptable for coordinate-precise COSMIC pointer work.
- 2026-05-15: Completion-audit validation after the final documentation reconciliation passed with `cargo fmt --all --check`, `cargo test --workspace --no-fail-fast`, `cargo clippy --workspace --all-targets`, `cargo check -p sky-cua-windows --target x86_64-pc-windows-msvc`, `uv run ruff format --check scripts`, `uv run ruff check scripts`, `uv run basedpyright`, `uv run pytest`, `python3 scripts/build_plugin.py`, `python3 scripts/build_runtime_packages.py --platform linux-x64`, `python3 scripts/package_runtime_artifact.py --platform linux-x64 --output-root artifacts/runtime`, `git diff --check`, a bundle `.mcp.json` overlay-env sanity check, and a SHA-256 cursor asset match. The VM-side audit rechecked the accepted KDE layer-shell synthetic/visible/hide/click-through summaries, the Hyprland layer-shell summary, the i3/X11 summary, KDE/GNOME/COSMIC pointer `final-state.json` files, and the KWin system-install `host-summary.json`.
- Deferred: Windows backend/live proof remains intentionally deferred until a Windows machine is available. The KWin compositor-painting effect and live state bridge are proved in nested KWin with explicit plugin/data paths, and the disposable VM system-install profile now proves the production KWin effect path through `/usr` installation plus Plasma restart. User-level compiled-effect discovery in an already-running KWin session remains unaccepted; ordinary plugin startup must not auto-install the effect.

## Context and Orientation

`sky-cua` is a Rust workspace plus Python harnesses. The plugin is invoked by Codex through an MCP server in `crates/sky-cua-client`; that client launches or connects to `sky-cua-service`; the service calls a platform backend such as `sky-cua-linux` or `sky-cua-windows`. A daemon is a long-running helper process. IPC means inter-process communication: in this repository, the service currently speaks line-delimited JSON over a Unix socket on Unix and TCP loopback on Windows.

The platform-neutral service model lives in `crates/sky-cua-platform/src/model.rs`. Important existing types are `CoordinateSpace`, `RectF`, `PixelSize`, `CaptureInfo`, `ActionRequest`, `ActionOutcome`, `AppStateSnapshot`, `ServiceRequest`, and `ServiceResponse`. `CoordinateSpace::StreamPixels` means the bounded screenshot pixels that the model sees and uses for explicit coordinates. `CoordinateSpace::StreamLogical` means logical coordinates within a portal ScreenCast stream. `CoordinateSpace::DesktopLogical` means desktop or virtual-screen coordinates used by native input systems.

The service dispatcher lives in `crates/sky-cua-service/src/daemon.rs`. `ServiceDaemon::handle` currently handles `GetAppState` by calling `backend.get_app_state`, stores the snapshot, and handles `ExecuteAction` by enriching the action request with the latest snapshot before calling `route_action`. The socket/TCP serving loop lives in `crates/sky-cua-service/src/ipc_server.rs`.

The Linux backend orchestration lives in `crates/sky-cua-linux/src/backend.rs`. It captures app state, uses portal RemoteDesktop and ScreenCast under Wayland when available, maps screenshot-pixel coordinates back to portal or X11 coordinates, and executes actions. `apply_model_capture` is where Linux PipeWire or Screenshot portal captures become model-facing screenshots. `point_from_screenshot_pixels` maps model screenshot coordinates to native input coordinates.

The Linux portal session manager lives in `crates/sky-cua-linux/src/portal/remote_desktop.rs`. It starts a combined RemoteDesktop and ScreenCast portal session, selects keyboard and pointer devices, requests monitor capture, and currently uses `CursorMode::Metadata`. Portal absolute pointer motion is stream-logical: `NotifyPointerMotionAbsolute` receives x/y in the logical coordinate space of a PipeWire stream.

KWin-specific window discovery and activation live in `crates/sky-cua-linux/src/kwin.rs`. The windowing registry in `crates/sky-cua-linux/src/windowing/registry.rs` lists the supported Linux window-listing backends, including GNOME extension, GNOME Introspect, COSMIC helper, KWin, Hyprland, i3, and X11. KWin is already a first-class backend for listing windows and, when `qdbus6` or `qdbus` is available, exact activation.

The Windows backend lives in `crates/sky-cua-windows/src/backend.rs`. It already captures with GDI, records screenshot pixel metadata in `CaptureInfo`, maps stream pixels back to desktop coordinates in `stream_to_desktop_point`, and injects input with `SendInput`, `SetCursorPos`, or window messages depending on environment. Windows overlay implementation is not part of the first Linux/KWin implementation pass, but these existing coordinate helpers are the contract that the later Windows overlay must share.

The browser-use cursor proof lives in the bundled Chrome extension under `resources/chrome-extension/codex/1.1.4_0/`. It is not a desktop overlay, but it demonstrates the useful behavior pattern: maintain cursor state, render an overlay, animate movement, and optionally wait until the cursor arrives. Python tests assert that contract in `scripts/test_python_harness_helpers.py`, and the live browser smoke tests visual diffs in `scripts/live_chrome_host_client_smoke.py`.

The packaging/preflight seam lives in `resources/chrome_preflight.py` and `scripts/build_plugin.py`. `DEFAULT_COMPUTER_USE_ENV_VARS` is the allowlist that passes desktop-session and sky-cua runtime variables through to the `computer-use` plugin. New overlay environment variables must be added there and validated by Python tests.

Definitions used in this plan:

- An agent cursor is a visual marker representing the agent's intended or latest pointer location. It is not necessarily the operating system's real hardware pointer.
- A visible overlay is a native window, layer, or compositor effect that the user can see on the desktop.
- A synthetic screenshot cursor is a marker composited into the screenshot that Codex receives. It may be visible to the model even if no native overlay is visible to the user.
- Layer-shell is a Wayland protocol used by desktop components such as panels, notifications, and overlays. It can create surfaces above normal windows on compositors that support it.
- KWin is KDE Plasma's compositor and window manager. A KWin effect is compositor-side code that can paint visuals as part of KWin itself.
- Capture exclusion means the visible overlay is hidden from screen captures, either by an operating system API or by hiding the overlay before capture and restoring it afterward.

## Plan of Work

Milestone 1 creates the platform-neutral cursor contract. Add cursor model structs and enums to `crates/sky-cua-platform/src/model.rs`. Keep them serializable with `serde(rename_all = "snake_case")` for public enums and `#[serde(default, skip_serializing_if = "Option::is_none")]` for optional fields. Extend `ActionOutcome` with an optional cursor field so a backend can say, "this successful action placed the agent cursor here." Extend `AppStateSnapshot` with optional cursor state so the client and tests can inspect what the service believes. Add `ServiceRequest` and `ServiceResponse` variants for overlay status and for explicitly setting cursor state in tests and debugging. These service requests should not be added to the MCP tool list initially; they are internal service/CLI/runtime controls, not model-facing actions.

Milestone 2 gives the service ownership of cursor state and overlay process supervision. Add `crates/sky-cua-service/src/overlay.rs` and wire it from `crates/sky-cua-service/src/main.rs` and `crates/sky-cua-service/src/daemon.rs`. The controller should store the latest `AgentCursorState`, return `AgentCursorCapabilities`, and supervise an overlay host later. At this stage it can use a no-op backend that logs state changes and always reports that visible overlay is unavailable while synthetic screenshot cursor is available if the capture compositor is enabled. Existing actions must behave exactly as before if the overlay controller returns an error.

Milestone 3 adds synthetic cursor compositing to screenshots. Implement this in the service after `backend.get_app_state` returns, not inside Linux-only code, so Windows and future macOS can use the same model-facing behavior. The service should read `snapshot.capture.screenshot_path`, draw a small cursor image or marker at the current cursor's screenshot-pixel coordinate, write a sibling model image with a deterministic suffix such as `.agent-cursor.jpg` or `.agent-cursor.webp`, and update `CaptureInfo.screenshot_path`, `model_image_bytes`, and `model_image_encode_ms`. If the image format is unknown or decoding fails, leave the original capture untouched and add a diagnostic instead of failing `get_app_state`. Use a simple built-in cursor marker first; reusable cursor artwork can come later.

Milestone 4 derives cursor state from actions. There are two paths. For model-facing placement, the service can derive a screenshot-pixel point from the enriched `ActionRequest`: explicit `x` and `y` arguments are already screenshot pixels; element-targeted actions can use the center of `resolved_element.bounds` when the bounds are in `StreamPixels`; drags should update the cursor to the final target point. For native overlay placement, the platform backend should optionally return a more native cursor point through `ActionOutcome.agent_cursor`, because the backend already knows how it mapped the model point to portal, X11, or Windows coordinates. If backend-provided native placement is absent, the overlay controller can still publish the model-facing point and keep the visible overlay degraded.

Milestone 5 creates the overlay-host IPC and a Linux host skeleton. Add a new workspace crate `crates/sky-cua-overlay-host`. The first executable should accept commands such as `probe`, `serve`, and `set-cursor` for manual debugging. The service should communicate with the host over a private JSON-lines socket under the same runtime directory as the service socket, for example `$XDG_RUNTIME_DIR/sky-cua/agent-cursor.sock`. On Windows later, this can be TCP loopback or a named pipe, but do not implement Windows yet. The host protocol should have messages named `hello`, `capabilities`, `set_cursor`, `hide`, `show`, `ping`, and `shutdown`. All messages should be versioned so later host binaries can reject incompatible service commands clearly.

Milestone 6 adds KWin-first proof before visible overlay. Add a Python smoke script, tentatively `scripts/live_agent_cursor_kde_smoke.py`, that runs only when the current environment is KDE Wayland or when an explicit `--allow-non-kde` flag is passed. The smoke should start or connect to the real service, call a debug CLI command to set the agent cursor in screenshot-pixel coordinates, call `get_app_state`, and assert that the resulting model-facing screenshot contains a localized cursor marker near the requested point. This proof should not click on arbitrary desktop UI. Prefer a stable local target such as `kdialog` if present, or fall back to the currently focused window with a clear warning. Add helper assertions modeled on `scripts/live_chrome_host_client_smoke.py` cursor diff checks.

Milestone 7 implements the generic Wayland layer-shell visible overlay with KWin as the first proving target. Add the dependencies needed for layer-shell support only after confirming compatible versions with `cargo info` at implementation time. As of 2026-05-14, the likely additions are `wayland-protocols-wlr = "0.3.12"` with the `client` feature and possibly `smithay-client-toolkit = "0.20.0"` plus `calloop = "0.14.4"` if raw `wayland-client` event handling becomes too noisy. The overlay host should probe the Wayland registry for `zwlr_layer_shell_v1`. If absent, report `visible_overlay=false` and a diagnostic reason. If present, create one transparent layer-shell surface per output or a compositor-chosen output, use the overlay layer, draw only the cursor marker, and set an empty input region so pointer and touch events fall through. In KWin proof mode, add a debug option that keeps the overlay visible during capture so a smoke can prove it rendered; in normal mode, the service should hide the overlay before capture and rely on synthetic screenshot compositing.

Layer-shell also needs an explicit system-cursor adapter answer. Because a click-through Wayland layer-shell surface has an empty input region and does not own pointer focus, the generic layer-shell backend must report `system_cursor_hide_supported=false`. Do not call Wayland pointer-focus APIs from this backend and claim global cursor hiding. Compositor-specific backends such as a KWin effect, GNOME Shell extension, COSMIC helper protocol, or a future desktop-portal extension can add their own adapters later.

Milestone 8 implements capture hide/show policy. The overlay controller needs a capture guard that hides the visible overlay just before `GetAppState` capture and restores it after capture. The guard should be best-effort and must not break capture if the overlay host is absent. Add `SKY_CUA_OVERLAY_HIDE_FOR_CAPTURE=auto|always|never` and `SKY_CUA_SCREENSHOT_CURSOR=auto|always|never`. The default should be `auto`: hide visible overlays when they might contaminate capture, then synthesize the model-facing cursor after capture. The debug smoke can set `SKY_CUA_OVERLAY_HIDE_FOR_CAPTURE=never` to prove that the native overlay is physically rendered.

Milestone 9 adds the KWin effect prototype after the generic contract and KWin layer-shell proof are solid. The effect must be transparent and click-through. A QML `SceneEffect` package was tried and rejected because it replaced the desktop scene. The next KWin effect attempt should be a true compositor-painting path, most likely a C++ `Effect` compiled against the exact KWin version that first calls `effects->paintScreen()` and then paints only the cursor marker afterward, or an `OffscreenQuickView` rendered from such a C++ effect. The KWin effect is also the KWin Wayland system-cursor adapter: while the agent cursor is visible it should call KWin's compositor-side `hideCursor()`, and it must call `showCursor()` when the cursor is hidden, disabled, or the effect is destroyed. The first acceptable KWin effect milestone is a static proof: install or load the effect explicitly, paint a small marker without obscuring the desktop or taking input, hide the system cursor while the marker is active, prove the desktop is still usable, then disable and remove it. The second milestone connects the effect to the generic cursor-state IPC. If a compiled effect is too ABI-fragile or requires system installation, record that in `Surprises & Discoveries` and keep KWin on the already-proved layer-shell backend instead. KWin effect installation must be explicit and reversible; do not auto-install it during ordinary plugin startup.

Milestone 10 adds X11 visible overlay support as a later Linux backend. Use `x11rb = "0.13.2"` or a repository-approved X11 helper after checking current dependency compatibility. The X11 backend should create an override-redirect transparent window above normal windows, use X Shape or input-shape behavior so it does not receive clicks, hide the system cursor with XFixes while the agent cursor is visible, restore the system cursor on hide/show/shutdown/drop, and reuse the same cursor-state IPC. This is not the first visible proof because the user specifically wants KWin proof prioritized, but it is important for Linux cross-platform coverage.

The current implementation covers the X11 backend itself through an externally
supplied `--current-display` smoke and an `x11-debug-visible` service smoke. On
a Wayland session, auto mode must not fall back to X11/XWayland and claim a
universal overlay. Embedded X11 VM smokes are retired; X11 acceptance on the VM
requires a real X11 guest session.

The preferred VM X11 acceptance command is:

    python3 scripts/run_gui_testing_vm_smoke.py --host testing-vm --profile i3

This requires the VM to be booted into i3/X11. It refuses Wayland sessions
rather than starting a private X server.

On a real externally supplied X11 desktop, run:

    uv run python scripts/live_agent_cursor_x11_overlay_smoke.py --current-display

The expected summary should show `mode=current-x11-display`, `backend=x11_shaped_window`, `visible_overlay_captured=true`, `hidden_overlay_captured=false`, `reshown_overlay_captured=true`, `overlay_visible_for_click=true`, and `click_through_proved=true`. This is the external-desktop acceptance command because it creates a harmless Tk target, draws the native X11 overlay above it, verifies root-capture visibility, sends `hide`, verifies the overlay disappears from capture, sends `show` with explicit state, verifies the overlay is visible again, and clicks through the visible overlay into the target.

The service-integrated X11 visible smoke remains useful as a second pass on a real X11 desktop:

    python3 scripts/live_agent_cursor_kde_smoke.py --mode x11-debug-visible --allow-non-kde

Its expected summary should show `backend=x11_shaped_window`, `visible_overlay_captured=true`, and a localized cursor diff near the requested point. If the smoke is run under Wayland/XWayland and reports `changed_pixels_near_hotspot=0`, record that as an XWayland stacking/capture limitation rather than an accepted X11 proof.

Milestone 11 records the deferred Windows implementation. Do not try to implement or claim Windows live proof on Linux. When a Windows machine is available, add the Windows backend to `crates/sky-cua-overlay-host` behind `cfg(target_os = "windows")`. It should create a borderless topmost layered Win32 window using `WS_EX_LAYERED`, `WS_EX_TOPMOST`, `WS_EX_NOACTIVATE`, and `WS_EX_TOOLWINDOW`; make it click-through with `WM_NCHITTEST` returning `HTTRANSPARENT`; use a Per-Monitor V2 DPI-aware manifest; share virtual-screen coordinate helpers with `crates/sky-cua-windows/src/backend.rs`; hide and restore the system cursor through a Windows-specific system-cursor adapter rather than inline calls in the rendering loop; and set `SetWindowDisplayAffinity(WDA_EXCLUDEFROMCAPTURE)` when available, with hide-before-capture fallback. The Windows proof must run on an actual Windows desktop and verify focus, click-through, multi-monitor or negative-origin behavior when possible, system cursor hide/show, capture exclusion or hide/show, RDP degradation, and UIPI/secure-desktop diagnostics.

Milestone 12 updates packaging and preflight. Add the overlay host binary to `scripts/build_plugin.py` so plugin bundles include it under `bin/`. Add overlay-related environment variables to `resources/chrome_preflight.py::DEFAULT_COMPUTER_USE_ENV_VARS` and update the associated Python tests. Keep default behavior conservative: `SKY_CUA_AGENT_CURSOR=auto`, `SKY_CUA_OVERLAY_BACKEND=auto`, `SKY_CUA_SCREENSHOT_CURSOR=auto`, `SKY_CUA_OVERLAY_HIDE_FOR_CAPTURE=auto`, and `SKY_CUA_OVERLAY_HOST_PATH` defaulting to the bundled binary if present.

## Concrete Steps

Run all commands from `/home/bex/projects/sky-cua` unless a step says otherwise.

First create and test the platform model changes. Edit `crates/sky-cua-platform/src/model.rs`. Add the cursor and overlay types near `CoordinateSpace`, `CaptureInfo`, and `ActionOutcome`, because they are part of the same serialized model contract. Add serialization tests in the existing `#[cfg(test)]` section of that file. Run:

    cargo fmt --all
    cargo test -p sky-cua-platform

The expected result is a passing platform test suite. New tests should prove that enum values serialize as snake_case, optional cursor fields are skipped when absent, and old responses still deserialize when the cursor fields are missing.

Next add the service overlay controller. Edit `crates/sky-cua-service/src/main.rs` to include a new `overlay` module. Add `crates/sky-cua-service/src/overlay.rs`. Edit `crates/sky-cua-service/src/daemon.rs` so `ServiceDaemon` owns an `OverlayController`. The controller initially stores state in memory and reports no visible overlay. Add service tests that call `SetAgentCursor`, `AgentCursorStatus`, and `GetAppState` through `ServiceDaemon::handle` where practical. Run:

    cargo fmt --all
    cargo test -p sky-cua-service

Then add synthetic screenshot compositing in the service. Add a small image helper either inside `crates/sky-cua-service/src/overlay.rs` or a sibling `crates/sky-cua-service/src/cursor_compositor.rs`. Add `image.workspace = true` and, if WebP re-encoding is needed in the service, `webp.workspace = true` to `crates/sky-cua-service/Cargo.toml`. The helper should accept a `CaptureInfo`, a screenshot-pixel point, and an output suffix, then return an updated `CaptureInfo` plus diagnostics. Unit tests should generate tiny temporary images, composite a marker, and assert localized pixel changes. Run:

    cargo fmt --all
    cargo test -p sky-cua-service

Then teach actions to update cursor state. Start with service-derived model-space cursor placement so the feature works before native overlay support exists. Use existing enriched `ActionRequest` fields from `ServiceDaemon::enrich_action_request`: `resolved_element`, `resolved_target_element`, `resolved_capture`, and `arguments`. Add service tests for explicit click coordinates, element-centered click coordinates, drag final target coordinates, and actions that should not move the cursor. After that, let Linux backends return optional native placement in `ActionOutcome` only where it is easy and low-risk. Run:

    cargo fmt --all
    cargo test -p sky-cua-platform
    cargo test -p sky-cua-service
    cargo test -p sky-cua-linux

Then create `crates/sky-cua-overlay-host`. Add the crate to the workspace `Cargo.toml`. Start with `probe` and `serve` commands and a no-op backend. The no-op backend should be useful in tests: it accepts JSON-lines cursor messages, records the latest state, and answers `capabilities`. Add crate-local tests for protocol parsing and version negotiation. Run:

    cargo fmt --all
    cargo test -p sky-cua-overlay-host
    cargo test -p sky-cua-service

Then add the KWin synthetic smoke. Create `scripts/live_agent_cursor_kde_smoke.py` and focused helpers in `scripts/` if needed. Read `scripts/AGENTS.md` before editing Python. The smoke should be explicit about environment requirements: KDE Wayland, portal capture available, and a running desktop session. It should produce artifacts under `artifacts/codex-e2e/agent-cursor-kde/<timestamp>/`, including before and after screenshots and a small JSON summary. Run the Python checks:

    uv run ruff format scripts
    uv run ruff check scripts
    uv run basedpyright
    uv run pytest scripts/test_python_harness_helpers.py

When on KDE Wayland, run:

    python3 scripts/live_agent_cursor_kde_smoke.py --mode synthetic

The expected summary should say that the synthetic marker was found near the requested screenshot-pixel coordinate and that no visible-overlay backend was required.

Then add the Wayland layer-shell backend to `crates/sky-cua-overlay-host`. Probe before drawing. If `zwlr_layer_shell_v1` is unavailable, return a capability response that says exactly that. If it is available, draw a transparent surface with only the cursor marker and an empty input region. Add unit tests for probe parsing where possible, and add a live smoke mode:

    python3 scripts/live_agent_cursor_kde_smoke.py --mode layer-shell-debug-visible
    python3 scripts/live_agent_cursor_kde_smoke.py --mode layer-shell-hide-for-capture

The debug-visible mode should deliberately allow the native overlay to appear in a capture and assert a localized diff. The hide-for-capture mode should assert that the normal model screenshot contains the synthetic marker and does not rely on the native overlay being captured.

Only after those pass, begin the KWin effect prototype. Add a separate plan update before this step if the layer-shell proof reveals a blocker. The effect must be transparent and click-through. Do not use the QML `SceneEffect` package path; that path has been live-rejected because it can replace the desktop scene. The current C++ prototype is a compositor-painting effect that creates its `OffscreenQuickScene` before rendering, calls `effects->paintScreen()` first, and renders the same copied `cursor-chat.png` afterward. The code-level proof runs nested KWin with explicit plugin/data paths:

    python3 scripts/live_agent_cursor_kde_smoke.py --mode kwin-effect-nested

The preferred VM KWin gate is:

    python3 scripts/run_gui_testing_vm_smoke.py --host testing-vm --profile kde-kwin-effect

This should run against a VM booted into a real Plasma Wayland session. Legacy
nested KWin debug profiles may still help isolate effect build/load/IPC issues,
but session acceptance comes from the VM display and preserved artifacts.

For visual cursor-pixel proof, observe the VM display through virt-manager or
virt-viewer and preserve the smoke artifact under
`artifacts/codex-e2e/agent-cursor-kde/<timestamp>/`.

The running-session user-level install smoke remains explicit because it writes user-level KWin files and asks the real KWin session to reconfigure:

    python3 scripts/live_agent_cursor_kde_smoke.py --mode kwin-effect-static --allow-kwin-effect-install

On this machine the nested proof passes, but the running-session command builds and installs the user-level C++ effect without KWin discovering the compiled plugin from `~/.local/lib/qt6/plugins/kwin/effects/plugins`; the expected current result for `kwin-effect-static` is still a nonzero smoke with `kwin_effect_load.load_stdout="false"` and cleanup removing the installed user-level files. Treat that as a KWin compiled-plugin production load-path blocker, not as a reason to revive the rejected QML `SceneEffect`.

If the effect cannot be proved without destabilizing KWin, stop and record the failure. Do not proceed to live IPC.

When packaging changes are made, update `resources/chrome_preflight.py`, `scripts/build_plugin.py`, and tests. Run:

    uv run ruff format scripts
    uv run ruff check scripts
    uv run basedpyright
    uv run pytest
    python3 scripts/build_plugin.py

For Linux cursor, backend, plugin, or MCP changes, use the Arch testing VM as
the live desktop harness before claiming the Linux slice complete:

    python3 scripts/run_gui_testing_vm_smoke.py --host testing-vm --profile kde-kwin-effect
    python3 scripts/run_gui_testing_vm_smoke.py --host testing-vm --profile computer-use
    python3 scripts/run_gui_testing_vm_smoke.py --host testing-vm --profile codex-desktop
    python3 scripts/run_gui_testing_vm_smoke.py --host testing-vm --profile cosmic-helper
    python3 scripts/run_gui_testing_vm_smoke.py --host testing-vm --profile wayland-layer-shell-overlay
    python3 scripts/run_gui_testing_vm_smoke.py --host testing-vm --profile i3
    python3 scripts/run_gui_testing_vm_smoke.py --host testing-vm --profile all

When the change affects a compositor-specific path, boot the VM into that
desktop session first, then run the relevant app/plugin smoke. COSMIC proof
means `SKY_CUA_TESTING_VM_SESSION=cosmic` plus the Computer Use smoke suite in
that real guest session, not nested `cosmic-comp`.

The retained `kde-plasma`, `gnome`, `cosmic`, and `hyprland` profiles are
legacy nested visual-debug lanes. They may be useful while debugging compositor
startup, but they are not acceptance proof for the session matrix.

The `all` profile is an execution-suite sanity check, not a desktop-matrix
proof. It runs fast non-session-specific profiles and should not be used to
claim COSMIC, GNOME, Plasma, Hyprland, or i3 acceptance by itself.

Future Codex Desktop app-level MCP live proofs should run inside this same
testing VM rather than introducing a separate Linux desktop harness.

When switching the visible VM session, prefer:

    cd /workspace && sudo scripts/testing-vm/select-session.sh hyprland

Use the matching session name (`plasma`, `cosmic`, `gnome`, `hyprland`, or
`i3`). This helper prevents stale compositor sockets from contaminating the next
proof.

At each stopping point, update this plan's `Progress`, `Surprises & Discoveries`, and `Decision Log` sections.

## Validation and Acceptance

The generic contract is accepted when `cargo test -p sky-cua-platform` proves all new cursor and overlay types serialize and deserialize correctly, and when old snapshots without cursor fields still deserialize. This matters because plugin clients and existing artifacts may not know about the new fields.

The service cursor state is accepted when `cargo test -p sky-cua-service` proves that `SetAgentCursor` updates state, `AgentCursorStatus` returns state and capabilities, overlay-host failure produces diagnostics instead of action failure, and `ExecuteAction` updates cursor state for at least explicit click, element click, and drag target cases.

The synthetic screenshot cursor is accepted when service tests create a tiny image, composite the marker, and assert that pixels changed only near the expected cursor location. It is also accepted in live KDE proof when `python3 scripts/live_agent_cursor_kde_smoke.py --mode synthetic` writes an artifact summary with `synthetic_cursor_found=true`, a requested coordinate, an observed marker bounding box, and a screenshot path.

The KWin layer-shell visible overlay is accepted only after the generic state and synthetic capture milestones pass. Its live proof should show four things: the host detected layer-shell support on the KWin session, a debug capture with hide disabled contains the native overlay marker near the requested point, a normal capture with hide enabled still contains the synthetic cursor marker after compositing, and a click-through smoke delivers a real pointer action to the target underneath while the marker is visible. The layer-shell backend must report `system_cursor_hide_supported=false` unless a compositor-specific adapter is active. If KWin does not expose layer-shell on the test desktop, that is not failure of the generic contract; record the probe result and proceed to the KWin effect prototype.

The KWin effect prototype has two acceptance levels. The code-level compositor proof is accepted when the VM `kde-kwin-effect` profile starts the KWin proof path with explicit plugin/data paths, KWin lists and loads the effect, the overlay-host reply reports `backend=kwin_effect`, and KWin exits cleanly. Pixel proof is accepted from the VM display or a reliable compositor capture showing the cursor rendered over the real Plasma session. Production acceptance is stricter: an explicit opt-in smoke must install or package the effect for the running KWin session, render a static marker, hide the system cursor while the marker is active, disable and uninstall the effect, restore the system cursor, and leave KWin usable. The live cursor bridge is accepted only when the effect consumes the same `AgentCursorState` as the generic overlay host. If the effect requires a different state contract, reject that design and adapt the effect to the generic contract instead.

The X11 backend is accepted when the VM `i3` profile or an externally supplied real X11-session smoke proves visible overlay rendering, click-through behavior while the overlay is visible, clean screenshot behavior, and XFixes system cursor hide/show state transitions. XWayland on a Wayland desktop may instantiate the backend, but that does not count as a global visible-overlay proof when native Wayland surfaces can cover or hide the X11 window.

The Windows backend remains deferred. It is accepted only on a Windows machine when a Windows live smoke proves topmost rendering, no focus theft, click-through, DPI-aware coordinate alignment, capture exclusion or hide/show fallback, and honest degradation for RDP, UIPI, and secure desktop. Do not mark Windows complete from Linux-only compilation.

The whole feature is accepted when default `computer-use` behavior remains compatible, cursor overlay failures degrade to diagnostics, model-facing screenshots can show the agent cursor on KDE Wayland, and packaging includes the overlay host and environment variables without breaking the existing browser-use cursor proof.

## Idempotence and Recovery

All new commands should be safe to run repeatedly. Service-side cursor state is ephemeral and should reset when the service exits. Overlay-host sockets should live under the existing per-user runtime directory and should be removed or replaced on startup the same way the service socket is handled.

The KWin effect smoke must install only user-level resources and must uninstall them during cleanup. If KWin refuses to unload an effect until reconfigure or logout, the smoke should print the exact cleanup command and leave `Progress` updated with the manual cleanup needed. Do not run the KWin effect smoke without an explicit opt-in flag.

If the overlay host crashes, the service should record a diagnostic and keep serving Computer Use requests. Restarting the service should be enough to recover the overlay host. If a live smoke leaves a helper process running, kill only the `sky-cua-overlay-host` process launched for that artifact directory.

If image compositing fails, preserve the original screenshot and add a diagnostic. Never replace a valid capture with a broken cursor-decorated file.

If layer-shell probing fails, keep synthetic screenshot cursor enabled and report `visible_overlay=false` with the probe reason. Do not fall back to an ordinary Wayland toplevel and pretend it is a reliable overlay.

## Artifacts and Notes

The important pre-existing browser cursor proof is:

    scripts/test_python_harness_helpers.py::test_bundled_chrome_extension_cursor_overlay_contract
    scripts/live_chrome_host_client_smoke.py::run_cursor_proof

The important current Computer Use seams are:

    crates/sky-cua-platform/src/model.rs
    crates/sky-cua-service/src/daemon.rs
    crates/sky-cua-service/src/ipc_server.rs
    crates/sky-cua-linux/src/backend.rs
    crates/sky-cua-linux/src/portal/remote_desktop.rs
    crates/sky-cua-linux/src/kwin.rs
    crates/sky-cua-linux/src/windowing/registry.rs
    crates/sky-cua-windows/src/backend.rs
    resources/chrome_preflight.py
    scripts/build_plugin.py

The preferred Linux GUI harness seams are:

    docs/operations/gui-desktop-test-harness.md
    scripts/testing-vm/provision-arch-testing-vm.sh
    scripts/testing-vm/sync-opencode-to-vm.sh
    scripts/testing-vm/profiles/run-profile.sh
    scripts/testing-vm/profiles/kde-kwin-effect.sh
    scripts/testing-vm/profiles/i3.sh
    scripts/run_gui_testing_vm_smoke.py

Short source-grounded facts from the research pass, embedded here so the plan is self-contained:

- Windows can draw a native overlay with a top-level layered, topmost, non-activating window. Click-through should be implemented with hit-test transparency as well as transparent extended styles. Capture exclusion can use a Windows display-affinity API where available, but still needs a hide-before-capture fallback.
- Wayland ordinary clients cannot reliably know global surface positions or draw above every other client. A normal desktop toplevel is not a valid universal overlay design.
- Wayland layer-shell is the generic Wayland path for desktop layers on compositors that support it. The overlay must probe support instead of assuming it.
- Generic Wayland layer-shell is not a global system-cursor hiding API. It can make the agent overlay click-through with an empty input region, but compositor pointer hiding needs a compositor-specific adapter.
- X11 can hide the system cursor through XFixes while the shaped overlay is visible. That behavior is now live-proved in the embedded X11 smoke.
- GNOME visible overlay likely requires a GNOME Shell extension. That is not in the first implementation slice.
- KWin visible overlay can start with layer-shell if the compositor supports it. KWin effect/plugin work comes later and must consume the same generic cursor-state contract; the effect is also the right KWin-specific place to hide the system cursor with KWin's compositor API.
- Portal ScreenCast cursor metadata is about the real OS cursor in a capture stream. It is useful capture metadata, not a custom visible agent cursor.

## Interfaces and Dependencies

Add these names in `crates/sky-cua-platform/src/model.rs`, adjusting field details only if implementation reveals a better local convention:

    pub enum AgentCursorBackendKind {
        None,
        ScreenshotSynthetic,
        WaylandLayerShell,
        KwinEffect,
        X11ShapedWindow,
        WindowsLayeredWindow,
        MacosPanel,
    }

    pub enum AgentCursorPlane {
        UserVisible,
        ScreenshotSynthetic,
    }

    pub struct AgentCursorPoint {
        pub x: f64,
        pub y: f64,
        pub coordinate_space: CoordinateSpace,
        pub mapping_id: Option<String>,
    }

    pub struct AgentCursorState {
        pub visible: bool,
        pub sequence: u64,
        pub model_point: Option<AgentCursorPoint>,
        pub native_point: Option<AgentCursorPoint>,
        pub snapshot_id: Option<String>,
        pub source_action: Option<ActionName>,
        pub updated_at_ms: u64,
    }

    pub struct AgentCursorCapabilities {
        pub backend: AgentCursorBackendKind,
        pub visible_overlay: bool,
        pub screenshot_synthetic_cursor: bool,
        pub click_through: bool,
        pub capture_exclusion: bool,
        pub system_cursor_hide_supported: bool,
        pub system_cursor_backend: AgentCursorSystemCursorBackend,
        pub system_cursor_hidden: bool,
        pub needs_user_install: bool,
        pub reason: Option<String>,
    }

Extend `ActionOutcome` with:

    pub agent_cursor: Option<AgentCursorState>

Extend `AppStateSnapshot` with:

    pub agent_cursor: Option<AgentCursorState>

Add service variants with names close to:

    ServiceRequest::AgentCursorStatus
    ServiceRequest::SetAgentCursor { state: AgentCursorState }
    ServiceRequest::HideAgentCursor { reason: Option<String> }
    ServiceRequest::ShowAgentCursor

And matching `ServiceResponse` variants carrying capabilities, state, and diagnostics. These are service IPC variants, not MCP tools at first.

The overlay host protocol should be JSON-lines and versioned. A minimal message envelope is:

    pub struct OverlayHostMessage {
        pub version: u32,
        pub kind: String,
        pub state: Option<AgentCursorState>,
        pub reason: Option<String>,
    }

    pub struct OverlayHostReply {
        pub version: u32,
        pub ok: bool,
        pub capabilities: Option<AgentCursorCapabilities>,
        pub state: Option<AgentCursorState>,
        pub diagnostics: Vec<DiagnosticEntry>,
    }

Use the repository's current workspace-managed dependency style. Do not add crate-local versions when a dependency belongs in root `Cargo.toml`. For Wayland layer-shell, verify current compatible versions at implementation time. The 2026-05-14 lookup found `wayland-protocols-wlr = "0.3.12"` and `smithay-client-toolkit = "0.20.0"`. Prefer the smallest dependency set that lets the overlay host create a layer-shell surface, draw a marker, and set an empty input region. For X11 later, the current lookup found `x11rb = "0.13.2"`.

## Revision Notes

- 2026-05-14 / Codex: Initial ExecPlan created from repo inspection, documentation research, and oracle second-model design pass. The plan prioritizes generic cursor state, service IPC, synthetic capture, and KWin proof before KWin effect work; Windows implementation is explicitly deferred until live Windows proof is possible.
- 2026-05-14 / Codex: Updated after the service-to-overlay-host IPC, KWin layer-shell overlay, capture hide/show guard, live smoke proofs, and current validation slice.
- 2026-05-14 / Codex: Updated after adding the explicit X11 shaped-window backend, recording the XWayland live-smoke limitation, and rerunning the full validation plus KDE live-smoke slice.
- 2026-05-14 / Codex: Updated after adding the transparent C++ KWin effect prototype, proving it builds and cleans up, and recording that running KWin does not load the user-level compiled plugin path on this machine.
- 2026-05-14 / Codex: Updated after fixing stale `Show` resurrection in overlay-host backends and rerunning focused, workspace, packaging, X11 nested, and KDE hide-for-capture validation.
- 2026-05-14 / Codex: Updated after tightening the X11 smoke so click-through is only accepted when a re-shown overlay was captured at the click point.
- 2026-05-14 / Codex: Updated after tightening the KWin effect smoke to prove the current blocker through KWin's own effect-discovery DBus properties.
- 2026-05-14 / Codex: Updated after adding the embedded X11 acceptance smoke and fixing `StreamLogical` element native cursor placement.
- 2026-05-14 / Codex: Updated after rerunning full Rust, Python, plugin-build, runtime-package, diff, and cursor-asset validation for the embedded X11 acceptance slice.
- 2026-05-14 / Codex: Updated after fixing the KWin/Wayland cursor size regression, making the layer-shell coordinate plane explicitly output-local, adding X11 original-pixel native overlay mapping, and rerunning the full validation and live smoke slice.
- 2026-05-14 / Codex: Updated after adding backend-specific system cursor hiding adapters, wiring X11 to XFixes hide/show, marking generic Wayland layer-shell unsupported for global cursor hiding, adding KWin effect hide/show restoration, and proving X11 system cursor state transitions in the embedded X11 smoke.
- 2026-05-14 / Codex: Updated after naming the system-cursor adapter backend in the shared capability contract, refactoring KWin effect hide/show into `KWinSystemCursorAdapter`, moving KWin `OffscreenQuickScene` setup out of `paintScreen`, and proving the compiled effect in nested KWin with `ScreenShot2` capture.
- 2026-05-14 / Codex: Updated after adding the KWin effect DBus state bridge, proving it consumes the shared `AgentCursorState` JSON in nested KWin, and recording the remaining production discovery boundary.
- 2026-05-14 / Codex: Updated after adding the auto-detected `kwin-effect` overlay-host backend and proving the nested KWin effect through the generic JSON-lines native host protocol.
- 2026-05-15 / Codex: Updated after retiring the Docker GUI harness as the acceptance path and retargeting Linux desktop proof to an Arch `testing-vm` with real guest sessions, VM provisioning, SSH sync/run, and host-built runtime artifacts.
- 2026-05-15 / Codex: Updated after proving the COSMIC direct-uinput backend at 1x and 125% scale, adding the repeatable scaled VM profile, and rerunning packaging/preflight/browser cursor regressions.
- 2026-05-15 / Codex: Updated after fixing stale portal selection, COSMIC probe hangs on Plasma, and SIGTERM overlay-host leaks, then rerunning clean KDE pointer and cursor-overlay VM proofs plus the full local Rust/Python validation suite.
- 2026-05-15 / Codex: Updated after fixing display-manager restart churn in the VM session selector, hardening failed KWin system-install evidence capture, and rerunning the production KWin effect proof successfully.
