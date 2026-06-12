# CDUL vs sky-cua: adoption strategy

## Context

`computer-use-linux` (CDUL) at `/home/bex/projects/codex-desktop-linux/computer-use-linux`
is a reference Linux Computer Use implementation that predates `sky-cua`'s
current architecture. Several CDUL ideas were considered for adoption to
improve `sky-cua`'s Linux runtime. This research records the comparison and
the chosen adoption strategy: do not port CDUL wholesale; selectively adopt
small fidelity and operator-experience enhancements where `sky-cua` still
has a gap.

The resulting enhancements shipped; the behavior is documented in
[`docs/features/linux-targeting-and-diagnostics.md`](../features/linux-targeting-and-diagnostics.md).

## Investigation

A three-lane read-only `codex-worker` comparison covered:

1. Windowing and session management
2. Capture, input, and AT-SPI
3. Packaging, host, and diagnostics

### Where sky-cua already has a stronger form

- **Window registry**: `crates/sky-cua-linux/src/windowing/registry.rs`
  aggregates multiple environment-appropriate backends and includes X11
  fallback. CDUL's `src/windowing/registry.rs` stops at the first usable
  backend.
- **Portal session manager**:
  `crates/sky-cua-linux/src/portal/remote_desktop.rs` owns a long-lived
  portal session manager with lifecycle diagnostics and persisted
  restore-token behavior. CDUL's portal handling is shorter-lived.
- **Architecture**: `sky-cua` has a cross-platform platform model, Linux
  service backend, MCP client, helper binaries, plugin packaging, and live-
  smoke infrastructure. CDUL is a single-crate MCP server. Porting CDUL
  wholesale would discard useful maturity.
- **Input doctor**: `crates/sky-cua-linux/src/doctor.rs` already contains
  granular `ydotool_socket_check`, `ydotool_socket_candidates`,
  `binary_check`, `process_check`, and `path_check`. The work needed is
  reporting / wording polish, not a new probe subsystem.
- **AT-SPI tree extraction**: `crates/sky-cua-linux/src/atspi/tree.rs`
  produces a richer flattened tree than CDUL's. Adopting CDUL's app-root
  prefilter idea is fine, but sky-cua should not flatten less richly.

### Where CDUL is the better reference

- **Terminal `command_line` fidelity**:
  `crates/sky-cua-platform/src/model.rs` defines
  `TerminalProcessInfo.command_line`, but
  `crates/sky-cua-linux/src/windowing/terminal.rs` currently sets
  `command_line` to the short `command_name` value. CDUL carries a real
  `command_line` field through `src/terminal.rs`, populated from
  `/proc/<pid>/cmdline` parsed as NUL-separated arguments.
- **GNOME setup messages**: CDUL has more explicit messages for the case
  where extension files are written and enabling is requested but GNOME
  Shell has not yet loaded the DBus API. The sky-cua GNOME setup path
  (`crates/sky-cua-linux/src/setup.rs`) already writes and enables the
  bundled extension; the gap is wording, not functionality.

### Decisions where sky-cua's choice is safer

- **`set_value` strategy**: CDUL tries numeric `Value` first when the
  payload parses as a number. `sky-cua` tries `EditableText` first in
  `crates/sky-cua-linux/src/atspi/actions.rs`, which is safer for text
  fields containing numeric strings. Any change should be role- or
  metadata-gated, not global.
- **Acceptance lane**: local unit tests are not sufficient acceptance for
  portal, input, windowing, or AT-SPI behavior that depends on real
  compositors. Both projects need live VM proof; for sky-cua that is the
  Arch `testing-vm` runner described in
  [`docs/operations/gui-desktop-test-harness.md`](../operations/gui-desktop-test-harness.md).

## Conclusion

Implement selected enhancements inside the existing `sky-cua` architecture
rather than copying CDUL's single-crate MCP server. Specifically:

1. Adopt CDUL's `/proc/<pid>/cmdline` parsing for terminal command-line
   fidelity.
2. Polish input doctor reporting so the existing granular checks surface
   in the readiness summary.
3. Add app-root prefiltering for AT-SPI snapshots without weakening the
   existing rich tree output.
4. Polish GNOME setup messages.

Reject:

- Porting CDUL's window registry "first match wins" pattern; the
  multi-backend aggregation is more useful.
- Switching `set_value` to numeric-first; text-first is safer.
- Porting CDUL's portal handling; the long-lived portal session manager
  with restore-token persistence is stronger.

## Implications

- The shipped behavior is documented in
  [`docs/features/linux-targeting-and-diagnostics.md`](../features/linux-targeting-and-diagnostics.md);
  the originating ExecPlan (`plans/cdul_linux_enhancements.md`) is retired
  to git history.
- Each implementation slice should be paired with a real VM proof per
  the [`vm-tests`](../../.agents/skills/vm-tests/SKILL.md) skill rather
  than relying on local unit tests alone.
- This research closes the comparison question. Future cross-project
  questions that aren't covered here should be answered by reading
  current `sky-cua` and CDUL source rather than reusing this summary.
