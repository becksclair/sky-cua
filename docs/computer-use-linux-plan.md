# Initial Linux Backend Execution Plan

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

This repository follows `/home/bex/.codex/PLANS.md`; this document must be maintained in accordance with that file.

## Purpose / Big Picture

This change created the first working Linux backend slice for Codex in `/home/bex/projects/sky-cua`. After this work, Codex could load a local plugin package, start a split client/service backend, probe the live Linux desktop, list currently exposed accessible applications, and fetch a structured app-state snapshot with capability diagnostics and heuristic guidance.

The goal is not to pretend Linux parity is magically finished. The goal is to stand up the hard architectural seams honestly: plugin packaging, MCP transport, service lifecycle, Linux environment probing, AT-SPI discovery, and a snapshot model that action routing can build on later.

## Progress

- [x] (2026-04-22 08:00Z) Chose concrete implementation stack: Rust workspace, split client/service, zbus plus ashpd for portals, atspi for accessibility.
- [x] (2026-04-22 08:00Z) Created repo scaffolding: workspace manifest, plugin metadata, executable wrapper scripts, heuristics resources, and this execution plan.
- [x] (2026-04-22 08:00Z) Implemented the shared platform model, Linux environment probe, AT-SPI app discovery, focused-app selection, and flattened accessibility snapshot generation.
- [x] (2026-04-22 08:00Z) Implemented the service daemon, Unix-socket IPC, MCP stdio client, and heuristics loading.
- [x] (2026-04-22 08:00Z) Ran `cargo build`, `cargo test`, direct socket smoke requests, and an end-to-end MCP initialize/tools/list/get_app_state smoke through `./bin/sky-cua-client`.
- [x] (2026-04-22 06:05Z) Added a portal-backed still-capture fallback so `get_app_state` now returns a real screenshot reference and pixel dimensions on the live KDE session.
- [x] (2026-04-22 12:40Z) Added snapshot-scoped action enrichment in the service, semantic AT-SPI action execution, cached RemoteDesktop session ownership, and Wayland input routing for click, secondary action, scroll, drag, type text, press key, and set value.
- [x] (2026-04-22 12:40Z) Added live operator-run smoke coverage with `scripts/live_desktop_smoke.py`, using `zenity` for semantic/text actions and `gtk_pointer_smoke_fixture.py` for physical pointer actions.
- [x] (2026-04-22 14:35Z) Switched the primary Wayland snapshot image path from Screenshot-portal fallback to live PipeWire frame capture via the active ScreenCast session, keeping Screenshot only as a fallback path.
- [x] (2026-04-22 14:35Z) Hardened repeated desktop requests by caching the AT-SPI connection, reusing the PipeWire remote fd across captures, and tightening selector-miss behavior to fail honestly instead of silently falling back.
- [x] (2026-04-22 14:35Z) Added an X11/XWayland discovery lane via `xprop`, merged X11-only windows into `list_apps`, and taught `get_app_state` to return an honest focused-window fallback snapshot when the active window has no matching AT-SPI tree.
- [x] (2026-04-22 14:35Z) Tightened the live smoke harness so the XWayland `xmessage` probe now verifies both `list_apps` visibility and focused-window fallback through `get_app_state`.
- [x] (2026-04-22 15:45Z) Upgraded the X11/XWayland fallback snapshot to expose a synthetic root element with real window bounds from `xwininfo`, so X11-only windows are targetable by bounds even without an AT-SPI tree.
- [x] (2026-04-22 15:45Z) Replaced the helper-based PipeWire capture path with an in-process GStreamer pipeline (`pipewiresrc -> videoconvert -> pngenc -> appsink`) and re-proved the live KDE smoke against that path.
- [x] (2026-04-22 17:40Z) Hardened pure-X11 fallback selection so explicit-coordinate actions probe the live environment without requiring a snapshot, and added real X11 capture/input implementations plus a dedicated nested-`Xvfb` smoke harness.
- [x] (2026-04-22 18:25Z) Hardened the Wayland portal session lane with startup timeouts, stale-session reset/retry for PipeWire capture, and a wider client-side IPC timeout; re-proved both the live KDE Wayland smoke and the nested pure-X11 smoke.
- [x] (2026-04-22 18:55Z) Added explicit operator-facing `PortalApprovalPending` diagnostics for missed Wayland approval prompts and tightened X11/XWayland correlation with score-based matching over PID, desktop id, executable, class, instance, title, and focus hints.
- [x] (2026-04-22 19:20Z) Tightened the X11/XWayland matcher again so exact title only helps rank already plausible matches, then re-proved both the live KDE Wayland smoke and the nested pure-X11 smoke with the corrected focused-window fallback.
- [x] (2026-04-22 19:45Z) Improved the MCP-facing portal-wait text so `PortalApprovalPending` tells the operator to approve the KDE dialog and retry, then added two-window X11/XWayland selector probes to both live smoke harnesses and re-proved the exact-title selector path.
- [x] (2026-04-22 20:10Z) Enriched X11/XWayland fallback snapshots with child-region recovery from `xwininfo -id -tree`, then re-proved both live smokes with `xmessage` fallback snapshots growing from a single root element to a 5-element pseudo-tree on Asgard.
- [x] (2026-04-22 20:25Z) Extended both live smoke harnesses to click a recovered descendant `x11_region` on `xmessage` and verify that the dialog exits, proving descendant-element targeting through the richer fallback tree.
- [x] (2026-04-22 20:50Z) Added conservative structural role/state hints for recovered X11/XWayland child regions (`x11_container`, `x11_leaf_region`, `x11_action_region`) and tightened both live smokes to require an action-like fallback descendant on `xmessage`.
- [x] (2026-04-22 21:05Z) Surfaced successful portal lifecycle transitions as first-class diagnostics and MCP text summaries (`PortalSessionStarted`, `PortalSessionRebuilt`), then re-proved Rust tests plus both live smokes and a fresh-service `get_app_state` call on the live KDE session.
- [x] (2026-04-22 21:30Z) Split the primary capture lane from the actual image-producing lane with `capture.image_backend`, added `CaptureBackendDowngraded` diagnostics for PipeWire-to-Screenshot fallback, and re-proved Rust tests plus both live smokes with backend/image-backend assertions.
- [x] (2026-04-22 21:55Z) Added an isolated-socket operator smoke for forced PipeWire image failure (`scripts/live_portal_downgrade_smoke.py`), proved `CaptureBackendDowngraded` live through MCP, and kept the normal-path regression check honest by recording its portal-approval block explicitly.
- [x] (2026-04-22 22:20Z) Added the first machine-readable app-action policy seam (`set_value_fallback`) and used it to enable a heuristics-backed physical `set_value` fallback for Kate-style editor replacement flows when semantic AT-SPI editing is unavailable.
- [x] (2026-04-23 09:55Z) Added and live-proved a native-Wayland graphical workflow smoke at `scripts/live_krita_smoke.py`, using a hybrid accessibility-tree plus screenshot-guided control model to create a document in Krita, draw a visible mark, save a `.kra`, and verify `mergedimage.png` inside the saved archive is not blank.

- [x] (2026-04-23 01:40Z) Implemented persisted RemoteDesktop restore-token storage and rotation, surfaced restore-aware lifecycle diagnostics (`PortalSessionRestored`, `PortalSessionRestoreMiss`, `PortalSessionTokenRotated`), and added a client/service reset path for persisted portal tokens.
- [x] (2026-04-23 02:30Z) Added a bundled plugin skill (`skills/computer-use-workflows`) with cross-platform hybrid tree + vision + physical-action guidance, and wired `.codex-plugin/plugin.json` to ship plugin-local skills.
- [x] (2026-04-23 02:50Z) Added distributable plugin build/install tooling (`scripts/build_plugin.py`, `scripts/install_plugin.py`), validated the built bundle shape under `dist/plugin/sky-cua`, and proved installation/discovery in both a temporary Codex home and the real `~/.codex` config as `sky-cua@debug`.
- [x] (2026-04-23 03:00Z) Added opt-in `codex exec` operator harnesses plus JSON-schema artifacts for an installed-plugin smoke and a Tidal playlist workflow, with transcript validation that rejects runs that never issue `computer-use` MCP calls.

## Surprises & Discoveries

- Observation: the remote shell environment reports `XDG_SESSION_TYPE=tty` even though the real desktop stack is a live KDE 6 Wayland session.
  Evidence: `kwin_wayland`, `xdg-desktop-portal-kde`, `pipewire`, and `at-spi2-registryd` are running under the user session on Asgard.
- Observation: the `atspi` ecosystem is far better than a grim FFI death march.
  Evidence: `atspi` and `atspi-connection` provide a real Rust path to the accessibility bus and registry root.
- Observation: a naive “first active root wins” focus heuristic is rubbish on KDE because session services like `ksmserver` are accessible too.
  Evidence: the first live `get_app_state` snapshot initially selected `ksmserver`; adding a preference score for human-facing windows shifted focus selection to the actual Ghostty app root with window title `openclaw`.
- Observation: the daemon and client architecture works cleanly with a newline-delimited Unix-socket protocol for internal IPC and Content-Length framing for MCP stdio.
  Evidence: direct socket calls returned `health`, `list_apps`, and `get_app_state`, while the MCP smoke returned `initialize`, `tools/list`, and `tools/call` responses end to end.
- Observation: KDE's Screenshot portal can be used from this remote-project session and returns a local file URI without derailing the service architecture.
  Evidence: `get_app_state` now copied a portal screenshot into `/run/user/1000/sky-cua/captures/69fa0959-9e24-4c3f-aee9-8e284b403463.png`, and the copied file existed with a non-zero size.
- Observation: `kdialog` is a lousy semantic smoke fixture here even when it is visibly present on screen.
  Evidence: launching a live `kdialog --inputbox` produced a visible dialog, but `list_apps` and targeted `get_app_state` did not surface it from the AT-SPI side.
- Observation: a custom fullscreen GTK pointer fixture is good enough for physical Wayland input proof even when it does not appear in `list_apps`.
  Evidence: the fixture stayed absent from the AT-SPI app list, while explicit-coordinate `click`, `perform_secondary_action`, `drag`, and `scroll` all changed the fixture state file through MCP-routed RemoteDesktop input.
- Observation: KDE Wayland scroll proof on this machine was happier with smooth axis injection than discrete wheel injection.
  Evidence: the live fixture only acknowledged scroll after the backend started using `NotifyPointerAxis` for `delta_y`-based scroll requests.
- Observation: repeated fresh AT-SPI connections were enough to wedge otherwise healthy desktop requests on this KDE session.
  Evidence: back-to-back `list_apps` and `get_app_state` calls stopped hanging only after the Linux backend started reusing a cached `AccessibilityConnection` instead of reconnecting each time.
- Observation: opening a fresh PipeWire remote for each snapshot was a nice way to make frame capture sulk.
  Evidence: repeated `get_app_state` snapshots stabilized only after the RemoteDesktop session cached the `open_pipe_wire_remote()` result and duplicated that fd per capture.
- Observation: XWayland semantic visibility is still patchy even when the X side is definitely alive.
  Evidence: a live `xmessage` probe under XWayland on KDE 6 Wayland now appears in `list_apps` and can become the focused app through X11 window metadata, but it only exposes a synthetic root element rather than a useful AT-SPI child tree.
- Observation: a synthetic X11 fallback root element is useful, but it is still only a physical targeting aid.
  Evidence: after adding `xwininfo`-derived bounds, the live `xmessage` probe now returns one element with targetable geometry, but there is still no semantic child tree behind it.
- Observation: the in-process PipeWire path is less flaky than the old helper seam once the GObject property types are set correctly.
  Evidence: the live KDE smoke re-passed with `portal_pipe_wire` capture after moving capture into a service-local GStreamer pipeline and correcting the `path` and `keepalive-time` property types expected by `pipewiresrc`.
- Observation: pure X11 without a window manager does not owe us `_NET_CLIENT_LIST`.
  Evidence: the first nested-`Xvfb` smoke failed to surface `xmessage` in `list_apps`; adding an `xwininfo -root -tree` fallback made the same probe pass with a synthetic root element and real X11 screenshot data.
- Observation: explicit-coordinate actions cannot infer an input backend from vibes.
  Evidence: the first pure-X11 pointer smoke failed with “no physical input backend is available” until the service started probing the environment for action requests that do not carry a `snapshot_id`.
- Observation: the client-side IPC timeout is part of the real UX on Wayland, not an implementation footnote.
  Evidence: unattended KDE Wayland `get_app_state` requests timed out at the old fifteen-second socket read limit while waiting on a portal-backed round trip, so the client timeout had to be widened to sixty seconds.
- Observation: a fresh service and a stale service are not morally equivalent once portal state is involved.
  Evidence: the previously failing live KDE Wayland smoke passed immediately against a freshly started foreground service; after adding portal-session reset/retry and startup timeouts, the full Wayland smoke re-passed through the normal MCP client path as well.
- Observation: “approval pending” is a distinct user-facing state, not just a slower denial.
  Evidence: after tightening portal startup timeout and surfacing the specific code, unattended Wayland `type_text` now fails with `PortalApprovalPending` and a clear approval-prompt message instead of collapsing into a generic internal MCP error.
- Observation: first-match X11 correlation was too naive for multi-window or pid-poor cases.
  Evidence: the new score-based matcher can now prefer the more specific focused/title-aligned window and can still correlate class/instance-driven windows even when titles are unhelpful.
- Observation: exact title alone is still too weak for X11/XWayland correlation on KDE.
  Evidence: the first score-based matcher let service roots such as `ksmserver` and `kaccess` inherit the live `xmessage` title and block focused-window fallback; requiring a second identity signal fixed the regression and made both live smokes pass again.
- Observation: proving selector quality through `list_apps` alone is too optimistic for pure X11.
  Evidence: two simultaneous `xmessage` windows were both visible through `xwininfo -root -tree`, but waiting for both through `list_apps` was flaky; using the raw X11 tree for existence plus MCP `get_app_state` for selection gave a stable end-to-end selector proof.
- Observation: raw X11 child-window geometry is good enough to make fallback snapshots less blunt.
  Evidence: `xwininfo -id -tree` for the live `xmessage` probe exposed a simple descendant hierarchy, and the fallback snapshot now reports 5 elements in both the pure-X11 and XWayland smokes instead of only a single root element.
- Observation: the richer fallback tree is operationally useful, not just better JSON.
  Evidence: both live smokes now dismiss `xmessage` by clicking the lowest recovered descendant region through the normal `click` tool with `snapshot_id + element_index`.
- Observation: conservative structural hints are enough to make recovered X11 descendants easier to reason about without lying about semantics.
  Evidence: the fallback tree now distinguishes `x11_container`, `x11_leaf_region`, and `x11_action_region`, and both live smokes require at least one `x11_action_region` before accepting the `xmessage` fallback snapshot as good enough.
- Observation: lifecycle polish matters on successful Wayland runs too, not just on failure.
  Evidence: a fresh MCP `get_app_state` call on the live KDE session now returns summary text ending with `Started a new combined RemoteDesktop and ScreenCast portal session.` alongside a matching `PortalSessionStarted` diagnostic.
- Observation: the primary capture lane and the actual image-producing lane need to be modeled separately.
  Evidence: after adding `capture.image_backend`, the live KDE smoke now reports `backend=portal_pipe_wire` plus `image_backend=portal_pipe_wire`, while the pure-X11 smoke reports both as `x11`; the downgrade path is now covered in Rust tests instead of being smuggled through an ambiguous `backend` field.
- Observation: proving the downgrade branch live is much easier with an isolated service socket than by trampling the default long-lived service.
  Evidence: `scripts/live_portal_downgrade_smoke.py` now uses `SKY_CUA_SERVICE_SOCKET_PATH` and `SKY_CUA_FORCE_PIPEWIRE_CAPTURE_FAILURE` to force a fresh-service downgrade path without disturbing the normal socket under `$XDG_RUNTIME_DIR/sky-cua/service.sock`.
- Observation: heuristics can influence backend routing without coupling the client-side guidance loader to backend policy parsing.
  Evidence: the client ignores unknown fields in `resources/app-instructions/index.json`, while the Linux backend now reads `set_value_fallback` directly from the same file to enable a policy-gated fallback path.
- Observation: the strongest native-Wayland workflow proof on this machine is currently hybrid rather than purely semantic.
  Evidence: Krita surfaced as `krita.desktop` with a rich AT-SPI tree and a non-`x11:` app id, but semantic `click` on `New Image` wedged while physical click on the same bounds plus screenshot-guided dialog/canvas targeting completed the workflow end to end and produced a saved `.kra` with a non-white merged preview image.

- Observation: prompt wording alone is not enough to prove a `codex exec` workflow used the installed plugin.
  Evidence: the first installed-plugin smoke run found the zenity dialog via shell/process inspection and killed it without issuing any `computer-use` MCP calls, so the harness now validates the JSONL transcript for `computer-use` tool calls before accepting success.
- Observation: installed-plugin ChatGPT-auth E2E had two separate runtime seams, not one.
  Evidence: with `features.apps = true`, detached ChatGPT-auth turns exposed the host harness tool world instead of `mcp__computer_use__...`; with `features.apps = false`, the real plugin namespace appeared again, but `codex exec` still cancelled generic MCP approval prompts until the run used `--dangerously-bypass-approvals-and-sandbox`.

## Decision Log

- Decision: implement the first vertical slice before real input injection or PipeWire frame decoding.
  Rationale: plugin lifecycle, MCP transport, environment truth, and AT-SPI snapshotting are the seams most likely to shape everything else. They can be validated without pretending frame capture is already done.
  Date/Author: 2026-04-22 / Sarah
- Decision: keep heuristics in `resources/app-instructions/*.md`, keyed by `resources/app-instructions/index.json`.
  Rationale: this mirrors the bundled macOS plugin shape better than forcing heuristics into Codex plugin `.app.json` metadata.
  Date/Author: 2026-04-22 / Sarah
- Decision: use `zenity` plus a purpose-built GTK fullscreen pointer fixture for live desktop smoke on Asgard KDE 6.
  Rationale: `zenity` exposes stable semantic AT-SPI affordances for text and button actions, while `kdialog` and the fullscreen fixture are unreliable as semantic targets but perfectly adequate for physical-coordinate smoke.
  Date/Author: 2026-04-22 / Sarah
- Decision: use a narrowly scoped `gst-launch-1.0` helper for the first live PipeWire frame decode path.
  Rationale: it proves the real ScreenCast-to-image seam immediately without dragging the repo into a bigger in-process GStreamer integration before the architecture and smoke harness are stable.
  Date/Author: 2026-04-22 / Sarah
- Decision: use a narrow `xprop`-based X11/XWayland discovery lane before attempting any heavier Xlib or compositor-specific integration.
  Rationale: it proves visible-window association and X11-only app surfacing with very little blast radius, while staying honest about the remaining semantic limitations.
  Date/Author: 2026-04-22 / Sarah
- Decision: replace the helper-based PipeWire frame capture with an in-process GStreamer pipeline before doing any wider capture refactors.
  Rationale: it removes an obvious source of flakiness and shell/process overhead while preserving the same high-level ScreenCast architecture and validation surface.
  Date/Author: 2026-04-22 / Sarah
- Decision: validate the pure-X11 lane under nested `Xvfb` instead of trying to fake confidence from the Wayland host session alone.
  Rationale: it forces the X11 capture/input path to stand on its own feet, including discovery behavior when no EWMH window manager is present.
  Date/Author: 2026-04-22 / Sarah
- Decision: let the service own portal-session recovery rather than pushing all recovery burden onto the client.
  Rationale: stale cached RemoteDesktop/PipeWire state is a backend concern, and the service has the state needed to drop and rebuild that session cleanly.
  Date/Author: 2026-04-22 / Sarah
- Decision: surface missed Wayland approval prompts as `PortalApprovalPending` rather than pretending they are denials.
  Rationale: operators need to know the difference between “you said no”, “the backend broke”, and “the compositor is still waiting for your click”.
  Date/Author: 2026-04-22 / Sarah
- Decision: treat window title as a correlation hint, not an identity source, when reconciling X11/XWayland windows with AT-SPI application roots.
  Rationale: KDE service apps can expose or inherit dialog titles they do not own; forcing a second identity signal avoids false positives while keeping title useful for ranking legitimate candidates.
  Date/Author: 2026-04-22 / Sarah
- Decision: score selector matches instead of taking the first candidate that fits.
  Rationale: multi-window apps and repeated X11/XWayland helper windows need exact-title and focused-window preferences, otherwise `desktop_file_id` selectors collapse into whichever candidate happened to be enumerated first.
  Date/Author: 2026-04-22 / Sarah
- Decision: use X11 child windows as pseudo-elements, but keep their roles explicitly blunt.
  Rationale: descendant bounds from `xwininfo -tree` materially improve physical targeting, but geometry alone is not enough evidence to fabricate real button/text semantics.
  Date/Author: 2026-04-22 / Sarah
- Decision: prove richer fallback trees with real descendant-element actions before adding any more inference.
  Rationale: the strongest validation for this seam is that a recovered child region can be targeted end to end through the existing action router, not that the snapshot merely looks more detailed.
  Date/Author: 2026-04-22 / Sarah
- Decision: keep the new X11/XWayland fallback hints structural rather than semantic.
  Rationale: `x11_container`, `x11_leaf_region`, and `x11_action_region` communicate useful physical-targeting shape without pretending geometry alone proves a real widget role.
  Date/Author: 2026-04-22 / Sarah
- Decision: surface successful portal lifecycle transitions in both diagnostics and MCP text.
  Rationale: operators need to know when the service started or rebuilt a portal session, not just when the compositor is still waiting on approval.
  Date/Author: 2026-04-22 / Sarah
- Decision: model capture downgrade explicitly instead of pretending one `backend` field can explain both the intended lane and the actual image source.
  Rationale: a PipeWire snapshot that falls back to Screenshot needs both pieces of truth: what the environment selected and what actually produced the pixels.
  Date/Author: 2026-04-22 / Sarah
- Decision: prove the downgrade path with a dedicated operator smoke instead of trying to piggyback that proof onto the ordinary KDE smoke.
  Rationale: forcing PipeWire image failure is a debug action, and isolating it behind a dedicated smoke plus a fresh service socket avoids contaminating the normal-path validation story.
  Date/Author: 2026-04-22 / Sarah
- Decision: make the first heuristics-driven action slice a very narrow `set_value` fallback rather than a broad generic typing policy.
  Rationale: `set_value` was the clearest product gap, and a tightly scoped app-blessed replacement flow is much safer than pretending arbitrary physical typing is acceptable everywhere.
  Date/Author: 2026-04-22 / Sarah
- Decision: treat “real graphical workflow” proof on Wayland as a hybrid tree-plus-vision problem, not an all-semantic problem.
  Rationale: Krita proved the useful split cleanly: the tree can anchor app/window discovery and stable surfaces, while screenshot-guided physical actions are more reliable for dialog and canvas transitions that wedge or lie semantically.
  Date/Author: 2026-04-23 / Sarah

- Decision: make the `codex exec` harnesses validate transcript-level `computer-use` MCP calls rather than trusting the model's final message.
  Rationale: a final JSON blob can still describe a workflow the model completed with shell hacks or process inspection; the transcript is the only honest proof that the installed plugin was actually used.
  Date/Author: 2026-04-23 / Sarah
- Decision: keep installed-plugin E2E on ChatGPT-auth, but run it through a dedicated test `CODEX_HOME` with `features.apps = false` and `codex exec --dangerously-bypass-approvals-and-sandbox`.
  Rationale: that is the first combination that both exposes the local `computer-use` namespace under ChatGPT-auth and stops exec-mode generic MCP approval cancellation, producing a real installed-plugin desktop smoke on this machine.
  Date/Author: 2026-04-23 / Sarah
- Decision: treat `codex exec` as a diagnostic harness and `codex app-server` as the installed-plugin acceptance harness.
  Rationale: `exec` can be made to work with `features.apps = false` plus the bypass flag, but it still side-steps the rich-client approval path plugins actually expect. The minimal JSON-RPC client in `scripts/_app_server_harness.py` now proves the plugin through the richer app-server contract directly.
  Date/Author: 2026-04-23 / Sarah

## Outcomes & Retrospective

The current milestone now proves much more than the initial architectural slice. This repository contains a real Codex plugin package, a Rust client/service split, a live Linux environment probe, AT-SPI-backed app enumeration, focused-app selection, flattened accessibility snapshots, heuristics resolution, cached portal session ownership, semantic AT-SPI action routing, physical Wayland input routing through the RemoteDesktop portal, conservative X11/XWayland fallback-region hints, operator-facing portal lifecycle diagnostics, an explicit split between the primary capture lane and the actual image-producing lane, and now a minimal rich-client `codex app-server` harness that proves installed-plugin acceptance with real approval round-trips. The live KDE smoke now exercises the full planned tool surface end to end through the MCP client, and `scripts/live_app_server_smoke.py` proves the packaged plugin through the richer Codex runtime contract rather than only through `codex exec`.

What remains is narrower and much more specific: hardening the ugly corners of semantic richness, improving AT-SPI bounds quality on native Wayland apps that currently report misleading physical coordinates, broadening policy-driven workflows beyond Kate, making the normal-path KDE smoke less dependent on prompt timing during repeated manual runs, broader XWayland/X11 parity now that both the X11 fallback lane and the in-process PipeWire capture seam are live, and turning screenshot-first interaction on sparse fallback-only consumer apps into a repeatable regression target. The first rich-client TIDAL workflow harness is now in place and has a live passing proof: `artifacts/codex-e2e/tidal-playlist-app-server/20260423T190545Z/last-message.json` completed with `gpt-5.5`, medium reasoning, the `Codex Favorites` playlist, five songs, and a fresh verification screenshot. The core transport and action architecture are no longer speculative.

One honest remaining annoyance is the operator-facing nature of the Wayland portal flow. The pure-X11 smoke now passes unattended under `Xvfb`, and the Wayland lane now reports `PortalApprovalPending` explicitly when the compositor is still waiting for approval, but any first-run portal approval still depends on a human clicking the compositor dialog.

## Context and Orientation

The repository started effectively empty, with only `/home/bex/projects/sky-cua/README.md`. Codex plugin discovery already expects a local plugin root to contain `.codex-plugin/plugin.json` and `.mcp.json`. The bundled macOS Computer Use plugin uses a client binary in MCP mode that ensures a longer-lived service is running, so this Linux slice follows that same shape.

In this repository, a “client” means the binary launched by Codex over standard input and output. A “service” means the longer-lived background process that keeps desktop state, probes the environment, and answers requests over a Unix socket. A “snapshot” means a structured representation of the current desktop app state: environment facts, focused app metadata, flattened AT-SPI elements, and diagnostics.

## Plan of Work

Create four crates under `/home/bex/projects/sky-cua/crates`: `sky-cua-platform` for shared types and traits, `sky-cua-linux` for Linux environment probing and AT-SPI discovery, `sky-cua-service` for the daemon and Unix-socket IPC, and `sky-cua-client` for the MCP server and service launcher.

In the platform crate, define the snapshot model, capability model, diagnostics, and request/response types that cross the client/service boundary. In the Linux crate, implement environment probing for session kind, compositor hints, portal versions, and AT-SPI availability, then implement AT-SPI app enumeration and focused-app snapshot building. In the service crate, implement a line-delimited JSON IPC server, snapshot caching, and stubbed action routing. In the client crate, implement MCP framing over stdio, advertise the planned Computer Use tools, and route `list_apps` and `get_app_state` to the service.

## Concrete Steps

From `/home/bex/projects/sky-cua`, run:

    cargo build
    cargo test

Then smoke the service:

    ./bin/sky-cua-service daemon

In a second shell, smoke the client wrapper:

    ./bin/sky-cua-client mcp

The MCP smoke will wait for framed JSON-RPC input. The goal of that smoke is simply to prove the binary starts cleanly and can ensure the service process is available.

## Validation and Acceptance

Acceptance for this milestone means:

1. `cargo build` succeeds for the whole workspace.
2. `cargo test` succeeds for the workspace.
3. Starting the service creates a Unix socket under `$XDG_RUNTIME_DIR/sky-cua/service.sock` or its safe fallback.
4. A `list_apps` request returns structured Linux environment diagnostics and at least one accessible application on the live KDE session.
5. A `get_app_state` request returns a snapshot with a `snapshot_id`, environment info, capability matrix, focused-app data, a flattened element list, and heuristic guidance when an app matches the registry.
6. On the live KDE Wayland session, `get_app_state` should also include a PipeWire-backed screenshot reference under `/run/user/1000/sky-cua/captures/`, with Screenshot-portal capture only as fallback when PipeWire fails.

## Idempotence and Recovery

All file creation in this milestone is additive. If the service socket already exists from a dead process, the service removes and recreates it. If the service process is already running, the client should reconnect instead of trying to create a duplicate. Re-running the wrapper scripts is safe.

## Artifacts and Notes

The most important artifacts in this milestone are the plugin package files, the wrapper scripts, and the structured snapshot JSON returned by the service.

    REQ health
    {"type":"health","ok":true,"service_socket":"/run/user/1000/sky-cua/service.sock"}

    REQ list_apps
    ... "session_kind":"wayland","compositor":"kde-kwin-wayland","capture_backend":"portal_pipe_wire","input_backend":"portal_remote_desktop","semantic_backend":"atspi" ...

    REQ get_app_state
    ... "focused_app":{"desktop_file_id":"ghostty.desktop","window_title":"openclaw"} ...
    ... "capture":{"screenshot_path":"/run/user/1000/sky-cua/captures/...png","pixel_size":{"width":2560,"height":1440}} ...

    MCP smoke summary
    {
      "response_count": 3,
      "tool_names": [
        "list_apps",
        "get_app_state",
        "click",
        "perform_secondary_action",
        "scroll",
        "drag",
        "type_text",
        "press_key",
        "set_value"
      ]
    }

## Interfaces and Dependencies

The repository uses:

- `tokio` for async I/O in the service and Linux backend
- `serde` and `serde_json` for IPC payloads and MCP payloads
- `zbus` for D-Bus property inspection
- `ashpd` for future portal session control and typed portal wrappers
- `atspi` for accessibility-bus access and AT-SPI object traversal

The platform crate must expose:

    pub trait DesktopBackend {
        async fn probe_environment(&self) -> Result<EnvironmentInfo, BackendError>;
        async fn list_apps(&self) -> Result<Vec<AppInfo>, BackendError>;
        async fn get_app_state(&self) -> Result<AppStateSnapshot, BackendError>;
        async fn execute_action(&self, request: ActionRequest) -> Result<ActionOutcome, BackendError>;
    }

The service crate must expose a daemon entrypoint that can answer `Health`, `ListApps`, `GetAppState`, and `ExecuteAction` requests over a Unix socket. The client crate must expose an MCP server over stdio that handles `initialize`, `notifications/initialized`, `tools/list`, and `tools/call`.

Revision note: created the initial living ExecPlan alongside repo scaffolding so implementation progress can be tracked in-tree from the start.

Revision note: updated the plan after implementation and validation to record the live daemon/MCP proof, the focus-selection correction, and the remaining PipeWire/input milestones.

Revision note: updated the plan again after landing the primary PipeWire image path, AT-SPI/PipeWire request-stability fixes, and the XWayland visibility probe in the live smoke harness.

Revision note: updated the plan again after landing the xprop-backed X11/XWayland discovery lane and proving focused-window fallback for an X11-only `xmessage` probe on the live KDE desktop.

Revision note: updated the plan again after upgrading the X11 fallback snapshot with synthetic root bounds and replacing the helper-based PipeWire capture seam with an in-process GStreamer pipeline.

Revision note: updated the plan again after landing real X11 capture/input routing, the nested-`Xvfb` smoke harness, the `xwininfo -root -tree` fallback for WM-less X11 discovery, and the wider client-side socket timeout for portal-driven waits.

Revision note: updated the plan again after hardening portal-session startup/recovery in the service and re-proving both the Wayland and pure-X11 smoke harnesses.

Revision note: updated the plan again after landing explicit `PortalApprovalPending` diagnostics and the score-based X11/XWayland correlation pass.

Revision note: updated the plan again after tightening the X11/XWayland matcher so title-only matches no longer steal focused-window correlation from the real X11 fallback target.

Revision note: updated the plan again after adding explicit operator guidance to portal-pending MCP text and proving exact-title selector behavior with paired `xmessage` windows in both live smoke harnesses.

Revision note: updated the plan again after enriching X11/XWayland fallback snapshots with child-region recovery from `xwininfo -id -tree` and re-proving that richer fallback tree in both live smoke harnesses.

Revision note: updated the plan again after proving descendant-element targeting on the richer X11/XWayland fallback tree by dismissing `xmessage` through a recovered child region in both live smokes.

Revision note: updated the plan again after adding conservative structural role hints to recovered X11/XWayland fallback regions and surfacing successful portal lifecycle transitions in both diagnostics and MCP text summaries.

Revision note: updated the plan again after splitting the primary capture lane from the actual image-producing lane with `capture.image_backend` and adding explicit downgrade diagnostics for PipeWire-to-Screenshot fallback.

Revision note: updated the plan again after adding the dedicated forced-downgrade operator smoke and the isolated service-socket seam it uses for fresh-service portal validation.

Revision note: updated the plan again after introducing the first machine-readable app-action policy and using it to enable a heuristics-backed `set_value` fallback for Kate-style editor replacement flows.

Revision note: updated the plan again after live-proving the Kate editor flow on Asgard by forcing Kate onto XWayland, then preferring X11/XTest over portal keyboard injection for matched X11 windows because the portal path reported success without editing the document.

Revision note: updated the plan again after codifying the Kate/XWayland proof as `scripts/live_kate_smoke.py`, so the heuristics-backed editor replacement path has a reusable operator smoke instead of living only in ad-hoc shell snippets.

Revision note: updated the plan again after live-proving a native-Wayland Krita workflow through a hybrid accessibility-tree plus screenshot-guided smoke that creates a document, draws on the canvas, saves a `.kra`, and validates the merged preview image inside the saved archive.
