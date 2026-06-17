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

## Surprises & Discoveries

- Observation: the real TIDAL blocker is no longer visibility. The current rich harness can now list and focus `tidal-hifi.desktop`, but the post-focus snapshot only contains one fallback window node and reports `AccessibilityCoverageLimited`.
  Evidence: `artifacts/codex-e2e/tidal-playlist-app-server/20260423T170741Z/last-message.json` and its transcript.

## Decision Log

- Decision: enrich the Wayland fallback with structural region anchors instead of fake widget semantics.
  Rationale: the user explicitly wants screenshot-guided control. Honest anchors plus screenshot truth are safer than pretending geometry alone knows what a button is.
  Date/Author: 2026-04-23 / Codex

## Outcomes & Retrospective

Implemented in source and unit-covered; still pending current live workflow
proof against an active fallback-only target.

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
