# Linux targeting and diagnostics fidelity

## Status

Shipped. Four CDUL-inspired enhancements to the Linux runtime: terminal
command-line fidelity, granular input doctor diagnostics, AT-SPI app-root
prefiltering, and GNOME setup-message polish. Landed across May 2026
commits (terminal/doctor work in tree by `4f95ed2`, 2026-05-23; app
matcher extracted in `7720bd9`). Last verified: 2026-06-12,
`cargo test -p sky-cua-linux` (206 passed).

## Summary

Agents and operators get more precise terminal-window targeting (full
process command lines, not just command names), actionable input readiness
diagnostics from `doctor`, snapshots that select the intended AT-SPI app
root instead of a similarly named service root, and GNOME window-targeting
setup messages that say exactly which state the extension install reached.

## Contract surface

- `TerminalProcessInfo.command_line` (`crates/sky-cua-platform/src/model.rs`)
  carries the full space-joined `/proc/<pid>/cmdline` contents; it falls back
  to `command_name` when the command line is empty or unreadable.
  `command_name` keeps the short name.
- `DoctorInputReport` exposes per-check `DoctorCheck` fields: `backend`,
  `ydotool` (binary), `ydotoold` (process), `ydotool_socket` (connectable
  socket), `xdotool` (binary), and `uinput` (`/dev/uinput` presence). The
  MCP `doctor` text summary appends failing input checks as
  `Input details: ...`; the structured report always carries all checks.
- `WindowTargetingSetupReport` (`setup_window_targeting`) distinguishes four
  outcomes in `message` and exposes `requires_shell_reload`: files not
  written; files written but enable failed; enabled but the DBus API needs a
  GNOME Shell reload or re-login; exact targeting live now.
- Windowing backend `list_note` strings state that terminal windows may
  include terminal process context when the process tree is readable.
- Operator probes: `sky-cua-client doctor`, `list-windows`,
  `focused-window` (`crates/sky-cua-client/src/operator_cli.rs`); degraded
  states exit non-zero.

## Behavior

- Terminal enrichment reads `/proc/<pid>/cmdline` as NUL-separated
  arguments and joins the non-empty ones; both the root and active terminal
  processes carry the full command line through
  `enrich_terminal_windows_with_processes`.
- The ydotool socket check walks candidate paths in order — the
  `$YDOTOOL_SOCKET` env override, `$XDG_RUNTIME_DIR/.ydotool_socket`, then
  `/tmp/.ydotool_socket` — and reports a connect attempt per candidate, so
  "binary present but socket dead" is distinguishable from "ydotoold not
  running".
- App-root selection (`crates/sky-cua-linux/src/app_match.rs`) is
  score-based: `app_id` match dominates, then desktop-file id, exact or
  partial window title, and normalized name; focused candidates win ties.
  `get_app_state` selects the app root through this scoring before the rich
  AT-SPI tree flattening starts, so two similarly named apps do not produce
  a wrong-app snapshot when window evidence identifies the target. Window
  metadata backfills missing AT-SPI registry metadata
  (`enrich_accessible_apps_from_windows`).
- All-app discovery for `list_apps` is unchanged; prefiltering applies only
  to targeted snapshots.

## Source paths

- `crates/sky-cua-linux/src/windowing/terminal.rs` — command-line read and
  enrichment
- `crates/sky-cua-linux/src/doctor.rs` — input checks and socket probing
- `crates/sky-cua-platform/src/model.rs` — `TerminalProcessInfo`,
  `DoctorInputReport`
- `crates/sky-cua-client/src/mcp_tools.rs` — `doctor_summary`,
  `push_input_diagnostics`
- `crates/sky-cua-linux/src/app_match.rs` — `select_app`,
  `selector_match_score`, window enrichment
- `crates/sky-cua-linux/src/backend.rs` — targeted snapshot selection path
- `crates/sky-cua-linux/src/setup.rs` — `setup_window_targeting_message`
- `crates/sky-cua-linux/src/windowing/registry.rs` — backend `list_note`
  wording

## Verification

Focused unit tests:

```bash
cargo test -p sky-cua-linux windowing::terminal
cargo test -p sky-cua-linux doctor
cargo test -p sky-cua-linux app_match
cargo test -p sky-cua-linux setup
```

Key tests: `process_summary_preserves_full_command_line` and its
command-name fallback twin; ydotool socket connect success/failure against
a bound Unix socket; `select_app_prefers_focused_candidate_when_selector_scores_tie`;
`setup_window_targeting_message_reports_reload_only_after_successful_enable`.

Live proof: these changes have been in tree since 2026-05-23 and were
exercised by the subsequent full VM smoke matrix runs (`--profile all`,
COSMIC, June 2026, recorded in `TODO_IMPROVE_CODEBASE.md` ICA-010/ICA-011
verification notes) and by daily live KDE host use. No dedicated
single-purpose VM profile exists for these seams.

## Known limitations

- No portal screenshot fallback was added: no reproducible `ashpd`
  request-handle failure was found, so the accepted outcome was regression
  coverage on the existing path, per the originating plan.
- `set_value` remains text-first (`EditableText` before numeric `Value`);
  CDUL's numeric-first behavior was rejected as unsafe for text fields
  containing numeric strings.
- GNOME setup-message outcomes are proven by unit tests; the
  reload-required path has not been re-proven on a live GNOME session since
  landing.

## Related

- Research: [`docs/research/2026-05-cdul-comparison.md`](../research/2026-05-cdul-comparison.md)
- ROADMAP entry: [`ROADMAP.md`](../../ROADMAP.md) under "Linux desktop parity"
- Originating ExecPlan retired into this doc; see git history for
  `plans/cdul_linux_enhancements.md`.
