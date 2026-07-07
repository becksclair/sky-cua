# Native Wayland fallback vision anchors

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

This document must be maintained in accordance with `~/.codex/PLANS.md`.

## Purpose / Big Picture

After this change, native Wayland apps that expose only a KWin fallback window instead of a real AT-SPI tree will still give Codex something useful to work with: a small, honest region tree that marks likely navigation, search, action, and content areas. The model will use those regions as structural anchors, then use the screenshot as the final source of truth for targeting. The original proving target was TIDAL on KDE Wayland, but the TIDAL live workflow is now retired; future proof should use a current fallback-only target with isolated, resettable state.

## Progress

- [x] (2026-04-23 19:11Z) Re-read current repo state, the KWin fallback implementation, the rich TIDAL harness evidence, and PLANS.md.
- [x] (2026-05-15 06:36Z) Current source contains the richer native-Wayland fallback region tree in `crates/sky-cua-linux/src/backend.rs`, with `vision_anchor` roles such as `wayland_header_band`.
- [x] (2026-05-15 06:36Z) Retired the TIDAL-specific live workflow command path. TIDAL artifacts remain historical proof; future workflow proof needs a current fallback-only target with isolated, resettable state.
- [x] (2026-05-15 06:36Z) Re-ran the broad local gates for the current diff: `cargo fmt --check`, `cargo test`, Ruff, basedpyright, pytest, `python3 scripts/build_plugin.py`, and `git diff --check`.
- [x] (2026-05-15 06:36Z) Updated `CONTINUITY.md`, `NOTES.md`, and command docs to point away from deleted TIDAL and nested-X11 harness scripts.
- [x] (2026-07-08) **Vision-anchor fallback proven live on a KDE Plasma 6
  Wayland host** (runtime at `bb5a3da`, deployed via `deploy_plugin.py`). A
  fallback-only window — an untitled `kwin:{ec4ad73c-…}` KWin window with no
  app correlation and no AT-SPI tree — resolved through
  `observe(surface="desktop", app_id="kwin:{…}")` returns exactly ONE element:
  `role: "window"`, `backend_ref: null` (no AT-SPI backing), real KWin bounds
  (1707×1067 desktop-logical), state_flags
  `["native_window_fallback", "physical_target", "vision_anchor", "container",
  "content_like"]` — the exact single-honest-anchor shape
  `linux_window_elements` emits (`crates/sky-cua-linux/src/backend.rs`, unit
  test `emits_only_the_honest_window_anchor_for_kwin_fallback_windows`). The
  portal capture backend was live in the same snapshot
  (`capture.backend = portal_pipe_wire`), giving the screenshot pixel path the
  fallback relies on. Contrast confirmed: `com.hiresti.player.desktop` (a
  native Wayland player) returned a rich 256-element AT-SPI tree — NOT
  fallback-only — so the anchor path is correctly gated on AT-SPI absence.
- [~] (2026-07-08) Setup lesson (root-caused, see Decision Log): the proof only
  works against the **installed singleton daemon** on the default socket. An
  ad-hoc client that sets `SKY_CUA_SERVICE_SOCKET_PATH` spawns a second daemon
  whose portal/AT-SPI/KWin-callback session is not established, yielding empty
  snapshots and `activate_window` KWin-callback timeouts. Do NOT override the
  socket. Also: `observe` desktop-branch selectors take `{surface, app_id}` (or
  `name`/`window_title`) with NO `include_accessibility` — mixing that field is
  a schema `InvalidRequest`.
- [x] (2026-07-08) Repeatable-fixture target found and landed: mpv, not
  Obsidian. The `fallback-anchor` agentic-loop fixture
  (`scripts/live_fallback_anchor_smoke.py`) landed with unit coverage.
  Obsidian was tried first and found NOT fallback-only: live probing showed
  it registers a shallow AT-SPI tree (`backend_ref:
  ":1.x:/org/a11y/atspi/accessible/root"`, role `application` → `frame`,
  `AccessibilityCoverageLimited: False`), so the vision-anchor fallback never
  fires for it, on top of launch friction (XWayland auth failure without
  `XAUTHORITY`; `--ozone-platform=wayland` did not start). mpv, launched via
  `mpv --idle --force-window --no-config --title <T>`, is the verified
  fallback-only target instead: a native Wayland window with ZERO AT-SPI,
  needing no vault/config and no first-run affordance, resetting by simply
  closing. The mechanism proof above used an ambient `kwin:{uuid}` window;
  the mpv target is verified for the fixture.
- [~] (2026-07-08) Fixture launch path live-validated; two real bugs found and
  fixed by live runs. (1) mpv rejects space-separated `--title value`; the
  argv now uses the joined `--title=<value>` form. (2) the fixture inherits
  the session env, so mpv's ancillary XWayland connection needs `XAUTHORITY`
  set (present on any real desktop session; a real host or the VM harness has
  it). With both fixed, mpv launches under the harness and the agent CLI
  receives the correct observe-and-report prompt. The full end-to-end
  agent-loop PASS is BLOCKED on agent-CLI billing, not on any sky-cua code:
  pi's only configured model (`opencode/deepseek-v4-flash-free`) times out
  repeatedly, and opencode's zen provider returns `401 CreditsError`
  ("Insufficient balance"). Completing the agent-loop pass needs a
  reliably-credentialed model (top up opencode credits or add a pi provider
  API key), then rerun:
  `python3 scripts/live_agentic_loop_smoke.py --agent <pi|opencode>
  --fixture fallback-anchor --model <working-model>` with `XAUTHORITY` set.

## Surprises & Discoveries

- Observation: the real TIDAL blocker is no longer visibility. The current rich harness can now list and focus `tidal-hifi.desktop`, but the post-focus snapshot only contains one fallback window node and reports `AccessibilityCoverageLimited`.
  Evidence: `artifacts/codex-e2e/tidal-playlist-app-server/20260423T170741Z/last-message.json` and its transcript.
- Observation (2026-07-08, superseded by the live proof above): an early hand
  probe using an ad-hoc `sky-cua-client mcp` on a scratch socket failed
  (`activate_window` KWin-callback timeout, empty `observe`). Root-caused to
  the `SKY_CUA_SERVICE_SOCKET_PATH` override spawning a second un-established
  daemon; against the installed singleton daemon the proof succeeds (see
  Decision Log). Recorded so the false trail isn't re-walked.
- (2026-07-08) Landed a fallback-only-app fixture (`fallback-anchor`) for `scripts/live_agentic_loop_smoke.py`, driven through `scripts/live_fallback_anchor_smoke.py`. mpv, launched idle via `mpv --idle --force-window --no-config --title <T>`, is the verified fallback-only proving target (native Wayland window, ZERO AT-SPI, resettable by closing). Obsidian was tried first and rejected: it registers a shallow AT-SPI tree (`backend_ref` populated, role `application` → `frame`, `AccessibilityCoverageLimited: False`), so the vision-anchor fallback path never fires for it — it would have proven the wrong code path — and it also had launch friction on this host (XWayland auth without `XAUTHORITY`; `--ozone-platform=wayland` did not start). The deterministic pass gate does not trust the agent's self-report: it scans the agent CLI's raw (unredacted) stdout transcript for an `observe` tool result whose `elements` carry a `vision_anchor` state flag with no richer AT-SPI role alongside it, matching `linux_window_elements`'s single-anchor shape in `crates/sky-cua-linux/src/backend.rs`. The harness normally redacts tool-result payloads out of persisted logs (`_agent_mcp_smoke.redact_pi_json_stdout`), so this fixture opts into the existing `SKY_CUA_SMOKE_KEEP_RAW_AGENT_LOG` escape hatch for the duration of its own run instead of weakening redaction generally. Implementation + unit tests only; the full agent-loop live run against mpv has not passed yet (the underlying fallback mechanism was proven separately via an ambient `kwin:{uuid}` window, see above).

## Decision Log

- Decision: enrich the Wayland fallback with structural region anchors instead of fake widget semantics.
  Rationale: the user explicitly wants screenshot-guided control. Honest anchors plus screenshot truth are safer than pretending geometry alone knows what a button is.
  Date/Author: 2026-04-23 / Codex
- Decision (superseded 2026-07-08): the rich multi-region tree (header band,
  nav rail, sidebar, action strip) this plan's Plan of Work describes was
  replaced in source by a SINGLE honest `window` vision-anchor with no
  synthetic children. Rationale in code (`backend.rs`
  `emits_only_the_honest_window_anchor_for_kwin_fallback_windows`): "inventing
  sub-elements a Wayland app never exposed misleads the agent into semantic
  targeting that cannot work; the honest signal is the screenshot +
  snapshot_id pixel path." The live proof (Progress, 2026-07-08) validates
  this simpler shape, not the plan's original region tree. Sections below that
  describe `wayland_header_band`-style child regions are stale.
- Decision (2026-07-08): live proofs of this lane MUST run against the
  installed singleton daemon (default socket), never an ad-hoc
  `SKY_CUA_SERVICE_SOCKET_PATH` spawn. Root cause (code analysis): the socket
  override spawns a second concurrent daemon whose portal session, AT-SPI
  connection, and in-process KWin-script callback channel are all
  unestablished; every "empty snapshot / activation timeout" symptom traces to
  that per-daemon state, not per-client state. The KWin callback uses the
  connection's unique bus name (no well-known-name collision), so the fix is
  purely "reuse the approved daemon."

## Outcomes & Retrospective

Implemented in source and unit-covered. The vision-anchor fallback mechanism is
**live-proven** on a KDE Plasma 6 Wayland host (2026-07-08, see Progress): a
KWin-only window with no AT-SPI tree yields the single honest `vision_anchor`
window element with portal capture live. The `fallback-anchor` agentic-loop
fixture landed as tested infrastructure, targeting mpv (Obsidian was tried
first and found not fallback-only, plus had host-specific launch friction); a
full mpv-targeted agent-loop pass remains the one bounded gap. The plan's
original multi-region-anchor design was superseded by the single-honest-anchor
shape (see Decision Log).

## Context and Orientation

The KWin fallback lives in `crates/sky-cua-linux/src/backend.rs` inside `kwin_fallback_snapshot`. Current source returns a root window plus `vision_anchor` fallback regions such as `wayland_header_band`, giving screenshot-guided models structural anchors without pretending geometry alone knows real widgets.

The element model is defined in `crates/sky-cua-platform/src/model.rs` as `ElementNode`. The only fields available for richer fallback guidance are `role`, `name`, `description`, `state_flags`, `bounds`, and the tree relation via `parent_index`. That means the fallback must encode its extra guidance through names, descriptions, and state flags.

App guidance is loaded from `resources/app-instructions/index.json` and resolved by the client from the focused app in `crates/sky-cua-client/src/heuristics.rs`. The former rich TIDAL harness has been removed, so this plan is historical evidence rather than an active command recipe.

## Plan of Work

First, replace the one-node KWin fallback with a helper that emits a root window plus a few heuristic child regions derived from the window bounds. These regions must be clearly labeled as candidates or anchors, not as definite controls. Each region should describe how it might help a screenshot-guided model: header/search band, navigation rail, main content region, list-like region, and action-like strip. The state flags should make those affordances grep-friendly, for example `vision_anchor`, `navigation_like`, `search_like`, `list_like`, `text_like`, `action_like`, `content_like`, `container`, and `leaf`.

Second, add app-specific Markdown guidance only for active target apps that still need special handling. The retired TIDAL proof should not drive new command or prompt surface unless TIDAL becomes an active target again.

Finally, prove the remaining workflow gap with the narrowest meaningful validators. The unit tests already confirm the fallback tree shape. Any future workflow proof should use the active agent-loop or app-server smoke infrastructure with a current app target, not the retired TIDAL runner.

## Concrete Steps

Run from `/home/bex/projects/sky-cua`.

1. Choose a current fallback-only target app with isolated, resettable state.
2. Add or update app guidance only for that target app, then register it in `resources/app-instructions/index.json` if source evidence shows the target needs app-specific guidance.
3. Run:

    cargo fmt --all
    cargo test -p sky-cua-linux
    python3 scripts/live_agentic_loop_smoke.py

## Validation and Acceptance

Acceptance for the code change is behavioral, not cosmetic:

- `cargo test -p sky-cua-linux` passes.
- A current agent-loop or rich app-server transcript shows `list_apps` finding the target app and `get_app_state` for the focused fallback-only window returning the `vision_anchor` fallback regions.
- If the full playlist flow still cannot complete, the failure message must be more informative than “no app visible”; it should clearly reflect the next real blocker.

## Idempotence and Recovery

These edits are additive and safe to rerun. If the live harness leaves behind a stuck `codex app-server` or `sky-cua-service` process, kill the specific per-artifact process tree and rerun the harness from a fresh artifact directory.

## Artifacts and Notes

The key proving artifact from before this change is:

    artifacts/codex-e2e/tidal-playlist-app-server/20260423T170741Z/last-message.json

It proves the visibility seam is already fixed and that the next work should focus on richer fallback guidance.

## Interfaces and Dependencies

The main interface to enrich is `sky_cua_platform::model::ElementNode`. Do not change the struct unless the current fields prove insufficient. Prefer implementing the richer fallback entirely inside `crates/sky-cua-linux/src/backend.rs` so the public model stays stable while the semantics of the existing fields become more useful.
