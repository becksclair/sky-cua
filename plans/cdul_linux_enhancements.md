# Implement selected computer-use-linux enhancements

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

This document must be maintained in accordance with `/home/bex/.agents/PLANS.md`.

## Purpose / Big Picture

After this change, the Linux Computer Use runtime in `sky-cua` will be easier to diagnose and more precise when targeting terminal-backed workflows and desktop apps. A user will be able to run `sky-cua-client doctor` and see more actionable input readiness details, ask for terminal windows by command line with better fidelity, and rely on clearer setup messages when GNOME window targeting is installed but not yet live. The implementation deliberately does not port the compact `computer-use-linux` architecture wholesale, because the current `sky-cua` codebase already has a stronger split between client, service, platform model, Linux backend, Chrome host, and packaging scripts.

The work is grounded in two repositories available on this machine. The target repository is `/home/bex/projects/sky-cua`. The reference implementation is `/home/bex/projects/codex-desktop-linux/computer-use-linux`, abbreviated below as CDUL. CDUL is useful here as a source of small implementation ideas and test cases, not as the architecture to copy.

## Progress

- [x] (2026-05-17 08:00Z) Compared CDUL against the current `sky-cua` source with three read-only `codex-worker` lanes: windowing/session management, capture/input/AT-SPI, and packaging/host/diagnostics.
- [x] (2026-05-17 08:00Z) Confirmed the main adoption strategy: do not port CDUL wholesale; implement small fidelity and operator-experience enhancements where `sky-cua` still has a gap.
- [x] (2026-05-17 08:00Z) Authored this ExecPlan under `plans/`.
- [ ] Implement terminal command-line fidelity in `crates/sky-cua-linux/src/windowing/terminal.rs`.
- [ ] Improve Linux input diagnostics and readiness text in `crates/sky-cua-linux/src/doctor.rs` and the platform model if needed.
- [ ] Add portal screenshot/request-handle regression coverage or a lower-level fallback only if the current `ashpd` path proves insufficient under tests.
- [ ] Add app-root prefiltering for targeted AT-SPI snapshots without weakening the existing rich tree output.
- [ ] Polish GNOME setup messages and window backend operator notes.
- [ ] Add or document stable operator probe commands and run focused validation.

## Surprises & Discoveries

- Observation: `sky-cua` already has most of CDUL's larger ideas, often in a stronger form.
  Evidence: `crates/sky-cua-linux/src/windowing/registry.rs` aggregates multiple environment-appropriate backends and includes X11 fallback, while CDUL's `src/windowing/registry.rs` stops at the first usable backend. `crates/sky-cua-linux/src/portal/remote_desktop.rs` also owns a long-lived portal session manager with lifecycle diagnostics and persisted restore-token behavior.

- Observation: the strongest concrete gap is terminal command-line fidelity.
  Evidence: `crates/sky-cua-platform/src/model.rs` defines `TerminalProcessInfo.command_line`, but `crates/sky-cua-linux/src/windowing/terminal.rs` currently sets `command_line` to `command_name` in `process_summary`. CDUL carries a real `command_line` field through `src/terminal.rs`.

- Observation: `sky-cua` already has some granular input checks, so the doctor work is likely model/reporting polish rather than a brand-new probe subsystem.
  Evidence: `crates/sky-cua-linux/src/doctor.rs` already contains `ydotool_socket_check`, `ydotool_socket_candidates`, `binary_check`, `process_check`, and `path_check`, but the public readiness summary still mainly reports high-level `input_backend` and `can_send_input` state.

## Decision Log

- Decision: Implement selected enhancements inside the existing `sky-cua` architecture rather than copying CDUL's single-crate MCP server.
  Rationale: `sky-cua` already has a cross-platform platform model, Linux service backend, MCP client, helper binaries, plugin packaging, and live-smoke infrastructure. Porting CDUL wholesale would discard useful maturity.
  Date/Author: 2026-05-17 / Codex

- Decision: Treat terminal command-line fidelity and input doctor details as the first implementation slice.
  Rationale: They are localized, testable, and immediately improve user-visible behavior without changing public tool semantics.
  Date/Author: 2026-05-17 / Codex

- Decision: Keep `set_value` text-first unless live evidence shows role-specific numeric controls need a different policy.
  Rationale: CDUL tries numeric `Value` first when the payload parses as a number. `sky-cua` currently tries `EditableText` first in `crates/sky-cua-linux/src/atspi/actions.rs`, which is safer for text fields containing numeric strings. Any change should be role- or metadata-gated, not global.
  Date/Author: 2026-05-17 / Codex

## Outcomes & Retrospective

Pending implementation. At completion, record which enhancements landed, which were deferred, the exact validation commands run, and whether any CDUL idea was rejected after deeper source review.

## Context and Orientation

`sky-cua` is a Rust workspace plus Python harnesses. The public data model lives in `crates/sky-cua-platform/src/model.rs`. The Linux backend lives in `crates/sky-cua-linux/src`. The MCP-facing client that exposes tools such as `doctor`, `list_windows`, `get_app_state`, `click`, and `set_value` lives in `crates/sky-cua-client/src/mcp_server.rs`.

A terminal window is a graphical terminal application such as Ghostty, Konsole, GNOME Terminal, Kitty, Alacritty, or xterm. `sky-cua` enriches window metadata with terminal process context so a caller can target a terminal by `tty`, `terminal_pid`, `terminal_command`, or `terminal_cwd`. The enrichment code is `crates/sky-cua-linux/src/windowing/terminal.rs`. The public model fields are `TerminalProcessInfo` and `TerminalWindowInfo` in `crates/sky-cua-platform/src/model.rs`. At the time this plan was written, `TerminalProcessInfo.command_line` exists, but the Linux enrichment fills it with the short command name rather than the full `/proc/<pid>/cmdline` contents.

The doctor report is the structured readiness report returned by `sky-cua-client doctor` and the MCP `doctor` tool. It is built by `crates/sky-cua-linux/src/doctor.rs` and summarized by `crates/sky-cua-client/src/mcp_server.rs`. It currently reports platform, portal, accessibility, windowing, input, session-env, and browser integration state. It already checks some input details, including ydotool socket candidates, but the highest-level readiness text is still relatively coarse.

AT-SPI is the Linux accessibility bus used to inspect app UI trees. `sky-cua` discovers AT-SPI application roots in `crates/sky-cua-linux/src/apps/discovery.rs` and flattens a selected app tree in `crates/sky-cua-linux/src/atspi/tree.rs`. CDUL has a simpler app-root selection path that can prefilter by target PID and app-name before flattening. The goal here is not to flatten less richly; it is to reduce wrong-app snapshots before the rich `sky-cua` tree extraction starts.

The RemoteDesktop and Screenshot portals are desktop services exposed over D-Bus. The RemoteDesktop portal injects pointer and keyboard events and can also provide ScreenCast streams. The Screenshot portal returns a still image. `sky-cua` uses `ashpd`, a Rust wrapper for XDG portals, in `crates/sky-cua-linux/src/portal/screenshot.rs` and has a more mature RemoteDesktop manager in `crates/sky-cua-linux/src/portal/remote_desktop.rs`. CDUL manually handles D-Bus portal request paths and stream metadata. That lower-level code is a useful reference for regression tests and fallback behavior if a portal implementation rewrites request handles in a way `ashpd` does not handle for us.

The GNOME window-targeting setup path writes a GNOME Shell extension under the user's local extension directory and enables it. In `sky-cua`, the setup code is `crates/sky-cua-linux/src/setup.rs`, and the bundled extension source is under `resources/gnome-shell-extension/codex-window-control@openai.com/`. CDUL has more explicit messages for the case where files are written and enabling is requested, but GNOME Shell has not loaded the DBus API yet.

## Plan of Work

Begin with terminal command-line fidelity. Extend `ProcessInfo` in `crates/sky-cua-linux/src/windowing/terminal.rs` with a `command_line: String` field. When reading `/proc/<pid>`, parse `/proc/<pid>/cmdline` as NUL-separated arguments and join non-empty arguments with spaces. If `cmdline` is empty or unreadable, fall back to the current `command_name`. Update `process_summary` so `TerminalProcessInfo.command_line` receives the full command line. Add unit tests beside the existing terminal enrichment tests to prove both root and active process command lines are preserved. If direct `/proc` fixture injection is awkward, keep the unit tests around `ProcessInfo` values and `enrich_terminal_windows_with_processes`; do not add filesystem-dependent tests for `/proc`.

Next, make input diagnostics more actionable. Inspect `DoctorInputReport` in `crates/sky-cua-platform/src/model.rs` and `input_report` in `crates/sky-cua-linux/src/doctor.rs`. If the model already exposes separate fields for ydotool, ydotoold, ydotool socket, and uinput, ensure they are all populated and reflected in `checks` or summaries. If the model only exposes a single high-level input detail, add backward-compatible optional fields with `#[serde(default, skip_serializing_if = "Option::is_none")]` where needed. The report should distinguish at least these states: input backend selected, `ydotool` binary present, `ydotoold` process running, connectable ydotool socket found, and `/dev/uinput` accessible. Update `doctor_summary` in `crates/sky-cua-client/src/mcp_server.rs` if the text summary currently hides those details.

Then add portal regression coverage without replacing the current portal manager. Search `crates/sky-cua-linux/src/portal/remote_desktop.rs` and `crates/sky-cua-linux/src/portal/screenshot.rs` for existing tests. Add focused unit tests around any pure helper functions that map portal streams to coordinates, parse portal capabilities, choose cursor modes, or classify portal failures. If screenshot request-handle behavior cannot be tested without a live portal, document that as a live-smoke follow-up instead of writing a brittle mock. Only implement a lower-level D-Bus screenshot fallback if there is a reproducible failure showing `ashpd::desktop::screenshot::Screenshot::request()` cannot handle the portal response. The default path should remain the current `ashpd` path and the existing model-image preparation logic in `prepare_model_capture`.

Next, add targeted AT-SPI app-root prefiltering. In `crates/sky-cua-linux/src/apps/discovery.rs`, keep the current all-app discovery behavior for `list_apps`. Add a helper that ranks or filters discovered apps by known target evidence: process ID from a selected window, executable, desktop file id, app name, and window title. Wire this helper into the `get_app_state` path in `crates/sky-cua-linux/src/backend.rs` only when a window target or focused-window context provides such evidence. The observable behavior should be that targeted snapshots prefer the AT-SPI root matching the requested or focused window instead of accidentally flattening a service root or unrelated app. Keep the existing rich tree shaping in `crates/sky-cua-linux/src/atspi/tree.rs`, including text readback, numeric readback, action aliases, sensitive text suppression, and compact output.

Polish operator wording after the behavioral changes. Update `crates/sky-cua-linux/src/setup.rs` so `setup_window_targeting_message` distinguishes these outcomes: files could not be written; files were written but enabling failed; enabling succeeded but the DBus API is not available until GNOME Shell reloads or the user logs out and back in; and exact targeting is live now. Update `crates/sky-cua-linux/src/windowing/registry.rs` descriptors so `list_note` mentions that terminal windows may include terminal process context when the process tree is readable. Do not change backend ordering or aggregation: `sky-cua` should keep aggregating environment-appropriate backends and should not regress to CDUL's first-usable-backend behavior.

Finally, review operator probe commands. `sky-cua-client` already exposes operator commands through `crates/sky-cua-client/src/operator_cli.rs`. Confirm that `doctor`, `list-windows` or `list_windows`, `focused-window` or `focused_window`, and setup commands are stable and documented. If a useful probe exists only as an MCP tool and not as an operator command, add the smallest CLI wrapper through the existing operator CLI parser. The goal is not a new binary; it is a stable set of post-install probes using the existing client binary.

## Concrete Steps

Run all commands from `/home/bex/projects/sky-cua`.

1. Inspect the current source before editing:

    git status --short
    rg -n "command_line|TerminalProcess|ProcessInfo|read_process_table|process_summary" crates/sky-cua-linux/src/windowing/terminal.rs crates/sky-cua-platform/src/model.rs
    rg -n "DoctorInputReport|input_report|ydotool|uinput|doctor_summary" crates/sky-cua-platform/src/model.rs crates/sky-cua-linux/src/doctor.rs crates/sky-cua-client/src/mcp_server.rs

2. Implement terminal command-line fidelity in `crates/sky-cua-linux/src/windowing/terminal.rs` and add focused tests in the same file.

3. Improve input doctor details in `crates/sky-cua-linux/src/doctor.rs`, `crates/sky-cua-platform/src/model.rs`, and `crates/sky-cua-client/src/mcp_server.rs` only as needed by the current model shape.

4. Add portal helper tests or documented live-smoke notes after reading existing tests:

    rg -n "#\\[test\\]|tokio::test|portal|screenshot|stream|cursor_mode" crates/sky-cua-linux/src/portal

5. Implement AT-SPI targeted app-root prefiltering in `crates/sky-cua-linux/src/apps/discovery.rs` and `crates/sky-cua-linux/src/backend.rs`, with tests that construct multiple discovered app candidates and prove the focused or requested window evidence wins.

6. Polish GNOME setup and backend descriptor text in `crates/sky-cua-linux/src/setup.rs` and `crates/sky-cua-linux/src/windowing/registry.rs`.

7. Confirm or add operator probes in `crates/sky-cua-client/src/operator_cli.rs`, then update docs only if the commands are new or the current docs omit them.

8. Run formatting and focused validation:

    cargo fmt --all
    cargo test -p sky-cua-platform terminal
    cargo test -p sky-cua-linux windowing::terminal
    cargo test -p sky-cua-linux doctor
    cargo test -p sky-cua-linux apps::discovery
    cargo test -p sky-cua-client doctor

9. If any public model fields change, run broader Rust validation:

    cargo test -p sky-cua-platform
    cargo test -p sky-cua-linux
    cargo test -p sky-cua-client

10. If CLI or Python packaging docs change, run the relevant Python checks:

    uv run ruff format --check scripts
    uv run ruff check scripts
    uv run basedpyright
    uv run pytest

## Validation and Acceptance

The terminal enhancement is accepted when a unit test proves that a terminal process with command name `codex` and command line `codex --dangerously-bypass-approvals-and-sandbox` appears in `TerminalProcessInfo.command_line` exactly as the full command line, while `command_name` remains `codex`. A second test should prove the fallback path still uses `command_name` when the full command line is absent.

The doctor enhancement is accepted when `sky-cua-client doctor` or the MCP `doctor` output exposes actionable input details. A successful report should let a human distinguish these cases without reading source: no input backend selected, ydotool missing, ydotoold not running, ydotool socket not connectable, and `/dev/uinput` missing or inaccessible. The exact output shape may be JSON or summarized text, but it must preserve structured fields for machine callers.

Portal work is accepted only if it improves proof. If no reproducible `ashpd` screenshot failure exists, the accepted outcome is regression coverage or a documented live-smoke note, not a speculative fallback. If a fallback is implemented, it must be exercised by a focused test or live smoke and must not replace the existing `prepare_model_capture` model-image pipeline.

The AT-SPI prefilter is accepted when tests show that two discovered apps with similar names do not cause a wrong-app snapshot if the target window PID or title identifies the intended app. Existing rich AT-SPI readback tests must keep passing.

The GNOME and descriptor wording work is accepted when setup reports and `list_windows` summaries are more specific without changing behavior. No backend should disappear from the registry, and mixed Wayland/X11 aggregation must still work.

For a local proof after implementation, run:

    cargo fmt --check
    cargo test -p sky-cua-linux windowing::terminal
    cargo test -p sky-cua-linux doctor
    cargo test -p sky-cua-linux apps::discovery
    cargo test -p sky-cua-client doctor

If the implementation touches shared platform model fields, additionally run:

    cargo test -p sky-cua-platform
    cargo test -p sky-cua-client

## Idempotence and Recovery

All planned edits are additive or localized. Re-running `cargo fmt --all` and the focused test commands is safe. If a test creates temporary files or sockets for doctor probes, use temporary directories or environment overrides and clean them up automatically in the test. Do not require a real desktop session for unit tests; keep live portal or VM proof as a separate optional validation step.

If a model-field change causes downstream serialization failures, revert only the new model field or add `#[serde(default, skip_serializing_if = "Option::is_none")]` so older JSON remains compatible. If an AT-SPI prefilter causes the wrong app to be selected in tests, disable only the new targeted path and keep the existing all-app discovery behavior intact while debugging.

## Artifacts and Notes

The CDUL source references that motivated this plan are:

    /home/bex/projects/codex-desktop-linux/computer-use-linux/src/terminal.rs
    /home/bex/projects/codex-desktop-linux/computer-use-linux/src/diagnostics.rs
    /home/bex/projects/codex-desktop-linux/computer-use-linux/src/remote_desktop.rs
    /home/bex/projects/codex-desktop-linux/computer-use-linux/src/screenshot.rs
    /home/bex/projects/codex-desktop-linux/computer-use-linux/src/atspi_tree.rs
    /home/bex/projects/codex-desktop-linux/computer-use-linux/src/windowing/registry.rs
    /home/bex/projects/codex-desktop-linux/computer-use-linux/src/gnome_extension.rs

The key `sky-cua` source references for implementation are:

    crates/sky-cua-linux/src/windowing/terminal.rs
    crates/sky-cua-platform/src/model.rs
    crates/sky-cua-linux/src/doctor.rs
    crates/sky-cua-client/src/mcp_server.rs
    crates/sky-cua-linux/src/portal/screenshot.rs
    crates/sky-cua-linux/src/portal/remote_desktop.rs
    crates/sky-cua-linux/src/apps/discovery.rs
    crates/sky-cua-linux/src/atspi/tree.rs
    crates/sky-cua-linux/src/backend.rs
    crates/sky-cua-linux/src/setup.rs
    crates/sky-cua-linux/src/windowing/registry.rs
    crates/sky-cua-client/src/operator_cli.rs

Do not store raw worker transcripts or large JSON artifacts in this plan. If future implementation produces live proof artifacts, record only the artifact path and the specific observation it proves.

## Interfaces and Dependencies

Preserve the existing public types unless a backward-compatible extension is necessary. `TerminalProcessInfo` already has the required fields, so terminal command-line fidelity should not require a public model change.

If `DoctorInputReport` needs new detail fields, define them in `crates/sky-cua-platform/src/model.rs` as optional fields and populate them from `crates/sky-cua-linux/src/doctor.rs`. Prefer reusing `DoctorCheck` for each input sub-check because it already carries `name`, `ok`, and `detail` and is used elsewhere in the doctor model.

Do not introduce new third-party dependencies for these enhancements. The required information comes from `/proc`, existing command probes, existing portal wrappers, existing AT-SPI types, and the current platform model.

Revision note: Created 2026-05-17 by Codex to turn the CDUL comparison proposal into a self-contained implementation plan. The plan records that `sky-cua` should adopt small fidelity and diagnostics improvements, not CDUL's architecture wholesale.
