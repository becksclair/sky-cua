# Native Wayland fallback vision anchors

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

This document must be maintained in accordance with `~/.codex/PLANS.md`.

## Purpose / Big Picture

After this change, native Wayland apps that expose only a KWin fallback window instead of a real AT-SPI tree will still give Codex something useful to work with: a small, honest region tree that marks likely navigation, search, action, and content areas. The model will use those regions as structural anchors, then use the screenshot as the final source of truth for targeting. The immediate proving target is TIDAL on KDE Wayland: the app is already visible and focusable, and after this change the fallback snapshot should contain more than one element and guidance that tells the model how to use those anchors without pretending they are real semantics.

## Progress

- [x] (2026-04-23 19:11Z) Re-read current repo state, the KWin fallback implementation, the rich TIDAL harness evidence, and PLANS.md.
- [ ] Implement a richer native-Wayland fallback region tree in `crates/sky-cua-linux/src/backend.rs`.
- [ ] Add TIDAL-oriented app guidance in `resources/app-instructions/` and register it in `resources/app-instructions/index.json`.
- [ ] Re-run the narrow validations: Linux unit tests and the rich TIDAL app-server harness.
- [ ] Update `CONTINUITY.md` and `NOTES.md` with the new blocker or proof.

## Surprises & Discoveries

- Observation: the real TIDAL blocker is no longer visibility. The current rich harness can now list and focus `tidal-hifi.desktop`, but the post-focus snapshot only contains one fallback window node and reports `AccessibilityCoverageLimited`.
  Evidence: `artifacts/codex-e2e/tidal-playlist-app-server/20260423T170741Z/last-message.json` and its transcript.

## Decision Log

- Decision: enrich the Wayland fallback with structural region anchors instead of fake widget semantics.
  Rationale: the user explicitly wants screenshot-guided control. Honest anchors plus screenshot truth are safer than pretending geometry alone knows what a button is.
  Date/Author: 2026-04-23 / Codex

## Outcomes & Retrospective

Pending implementation.

## Context and Orientation

The KWin fallback lives in `crates/sky-cua-linux/src/backend.rs` inside `kwin_fallback_snapshot`. Right now that function returns a single `ElementNode` with role `window`. That is enough to prove a Wayland window exists and can be targeted physically, but it is not enough to guide a model through a custom UI like TIDAL.

The element model is defined in `crates/sky-cua-platform/src/model.rs` as `ElementNode`. The only fields available for richer fallback guidance are `role`, `name`, `description`, `state_flags`, `bounds`, and the tree relation via `parent_index`. That means the fallback must encode its extra guidance through names, descriptions, and state flags.

App guidance is loaded from `resources/app-instructions/index.json` and resolved by the client from the focused app in `crates/sky-cua-client/src/heuristics.rs`. The rich TIDAL harness is `scripts/live_app_server_tidal_playlist.py`.

## Plan of Work

First, replace the one-node KWin fallback with a helper that emits a root window plus a few heuristic child regions derived from the window bounds. These regions must be clearly labeled as candidates or anchors, not as definite controls. Each region should describe how it might help a screenshot-guided model: header/search band, navigation rail, main content region, list-like region, and action-like strip. The state flags should make those affordances grep-friendly, for example `vision_anchor`, `navigation_like`, `search_like`, `list_like`, `text_like`, `action_like`, `content_like`, `container`, and `leaf`.

Second, add a TIDAL app-instruction Markdown file that tells the model exactly how to use these fallback anchors: narrow to the likely region using the tree, confirm the target on the screenshot, then click/type physically and re-check state. Register that guidance in `resources/app-instructions/index.json` under `tidal-hifi.desktop` with sensible aliases.

Finally, prove the change with the narrowest meaningful validators. The unit tests should confirm the fallback tree shape. The rich TIDAL harness should progress from “one fallback window node” to a snapshot with multiple fallback anchor nodes, even if the full playlist workflow is still blocked by UI complexity.

## Concrete Steps

Run from `/home/bex/projects/sky-cua`.

1. Edit `crates/sky-cua-linux/src/backend.rs` to factor KWin fallback elements into a helper that returns multiple nodes.
2. Add or update unit tests in the `#[cfg(test)]` section of the same file.
3. Add `resources/app-instructions/TIDAL.md` and register it in `resources/app-instructions/index.json`.
4. Run:

    cargo fmt --all
    cargo test -p sky-cua-linux
    python3 scripts/live_app_server_tidal_playlist.py

## Validation and Acceptance

Acceptance for the code change is behavioral, not cosmetic:

- `cargo test -p sky-cua-linux` passes.
- The rich TIDAL harness transcript shows `list_apps` finding `tidal-hifi.desktop` and `get_app_state` for the focused TIDAL window returning more than one fallback element instead of a single `window` node.
- If the full playlist flow still cannot complete, the failure message must be more informative than “no app visible”; it should clearly reflect the next real blocker.

## Idempotence and Recovery

These edits are additive and safe to rerun. If the live harness leaves behind a stuck `codex app-server` or `sky-cua-service` process, kill the specific per-artifact process tree and rerun the harness from a fresh artifact directory.

## Artifacts and Notes

The key proving artifact from before this change is:

    artifacts/codex-e2e/tidal-playlist-app-server/20260423T170741Z/last-message.json

It proves the visibility seam is already fixed and that the next work should focus on richer fallback guidance.

## Interfaces and Dependencies

The main interface to enrich is `sky_cua_platform::model::ElementNode`. Do not change the struct unless the current fields prove insufficient. Prefer implementing the richer fallback entirely inside `crates/sky-cua-linux/src/backend.rs` so the public model stays stable while the semantics of the existing fields become more useful.
