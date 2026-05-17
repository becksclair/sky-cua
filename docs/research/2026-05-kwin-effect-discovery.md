# KWin compiled-effect discovery: user-level install vs system install

## Context

The agent cursor overlay's preferred KDE Plasma backend is a compositor-side
KWin effect that paints a transparent, click-through cursor marker through
`OffscreenQuickScene` after `effects->paintScreen()`. The effect builds and
installs cleanly to user-level paths, but a running KWin process did not
discover or load it, so the production proof path needed to be settled
before we could claim KWin compiled-effect support.

This research records the user-level vs system-install comparison and the
accepted production lane.

## Investigation

The C++ KWin effect lives in
`resources/kwin/effects/sky-cua-agent-cursor/`. It calls
`effects->paintScreen()` first and then renders the bundled
`cursor-chat.png` through an `OffscreenQuickScene`. A separate
`KWinSystemCursorAdapter` hides and restores the real KWin cursor through
the effect's hooks.

The user-level smoke is
`scripts/live_agent_cursor_kde_smoke.py --mode kwin-effect-static --allow-kwin-effect-install`.
On Asgard (Plasma 6 Wayland) it installed:

- QML and assets under both `~/.local/share/kwin/effects/sky-cua-agent-cursor/`
  and `~/.local/share/kwin-wayland/effects/sky-cua-agent-cursor/`
- the compiled `.so` under `~/.local/lib/qt6/plugins/kwin/effects/plugins/`

After install plus KWin reconfigure plus explicit
`org.kde.KWin.Effects.loadEffect`, KWin still reported the effect as
`listed=false`, `effect_supported=false`, `effect_loaded=false`. Removing
and rerunning produced the same blocker. Artifact:
`artifacts/codex-e2e/agent-cursor-kde/0514225541305594-kwin/summary.json`.

A nested-KWin smoke with explicit `QT_PLUGIN_PATH` and explicit data paths
proved the effect itself works:
`scripts/live_agent_cursor_kde_smoke.py --mode kwin-effect-nested` brought
the effect to `effect_loaded=true` and rendered the marker at `(420,260)`
in nested capture. Artifact:
`artifacts/codex-e2e/agent-cursor-kde/0514230404440837-kwin-nested/summary.json`.

A user-install nested control without the forced `QT_PLUGIN_PATH`
reproduced the running-session behavior:
`kwin-effect-nested-user-install` came back with
`kwin_user_install_discovered=false`,
`kwin_user_install_loaded=false`, and no `sky-cua-agent-cursor` in the
nested KWin's `effect_list`. Artifact:
`artifacts/codex-e2e/agent-cursor-kde/0514230356463033-kwin-user/summary.json`.

The system-install path was tried as the alternative production lane.
`scripts/run_gui_testing_vm_smoke.py --profile kde-kwin-effect-system-install`
installs the effect under the VM's `/usr` paths, restarts Plasma, captures
host framebuffers via `virsh screenshot`, and verifies the cursor at
`(420,260)`. That run has KWin's own `listOfEffects` and `loadedEffects`
DBus properties listing `sky-cua-agent-cursor`, `host_marker_probe.found`
true with 186 changed pixels and a max channel delta of 168, remote
`backend=kwin_effect`, remote `system_cursor_hidden=true`, and clean
uninstall after the run. Artifact:
`artifacts/kde-framebuffer-cursor-proof/kwin-system-install/20260515T100852814643Z/host-summary.json`.

## Conclusion

On a running Plasma session, KWin discovers compiled effect plugins from
system paths but not from user-level `~/.local/lib/qt6/plugins/kwin/effects/plugins/`,
even after KWin reconfigure and explicit `loadEffect`. The blocker is at
KWin's plugin discovery layer, not at the build, install, data-path, or
DBus layer.

The accepted production lane is system install under `/usr` plus a Plasma
restart. The VM runner profile `kde-kwin-effect-system-install` automates
that path and is the acceptance command for KWin compiled-effect work.

User-level installs remain useful for development iteration only when paired
with a nested KWin process that has an explicit `QT_PLUGIN_PATH`. They are
not a production-equivalent proof on a running desktop session.

## Implications

- The KWin effect path is shipped, but only via the system-install profile.
  The KWin layer-shell backend remains the live fallback when system install
  is not present.
- `live_agent_cursor_kde_smoke.py --mode kwin-effect-static` on a running
  Plasma host is expected to fail with the KWin compiled-plugin discovery
  blocker. Treat that as evidence of the discovery boundary, not as a
  regression in the effect itself.
- Future work to expand KWin acceptance should focus on either an upstream
  KWin discovery fix or on the system-install lane as the canonical
  deployment path.
- This finding is referenced from
  `docs/features/agent-cursor-overlay.md` under "Known limitations".
