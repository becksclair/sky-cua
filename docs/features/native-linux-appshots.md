# Native Linux AppShots

## Status

Shipped producer contract. Last verified: 2026-07-25 focused Rust and
`@heliasar/sky-cua` contract tests. Codex Desktop owns the consumer bridge,
composer UI, and packaged-runtime acceptance.

## Summary

Sky CUA can produce a one-shot AppShot for an explicit Linux window or the
window that is frontmost when the request begins. A successful result combines
a verified target-window image, application identity, capture provenance, and
best-effort AT-SPI text without exposing screenshot bytes on the service wire.

## Contract surface

- Service health advertises `appshot_capture.v1`.
- `ServiceRequest::AppshotCapture` serializes as `type: "appshot_capture"` and
  accepts `request_id`, exactly one of `target` or `frontmost: true`, and
  `flags.include_ax_text`.
- `ServiceResponse::AppshotCapture` returns `request_id`, application/window
  identity, image path/MIME/byte size/dimensions, `ax_status`, nullable
  `ax_text`, `capture_scope`, selected and actual image backends, display
  metadata, and diagnostics.
- The Node facade exposes `sky.appshot_capture(input)`. This is a host API, not
  an MCP capture tool.
- Artifacts live in the per-user runtime temporary directory at
  `$XDG_RUNTIME_DIR/sky-cua/appshots`, falling back to the UID-scoped system
  temporary directory. The service keeps the directory private and removes
  files older than 24 hours opportunistically.

Request IDs are also artifact keys. They must contain 1–128 ASCII letters,
digits, `.`, `_`, or `-`, and may not start with `.`.

## Behavior

The service resolves the requested window before beginning native capture.
For `frontmost: true`, it snapshots the focused window ID immediately and uses
that exact ID for the capture; an explicit `target` always wins and must resolve
to exactly one window.

The existing Linux targeted-screenshot path then activates and focus-verifies
the resolved window, captures through the active ScreenCast/PipeWire lane with
Screenshot portal fallback, and crops to proven window bounds. AppShot accepts
only `capture_scope: "window"`; a display, primary-display, or unknown frame is
a terminal failure rather than a successful downgraded AppShot.

When requested, AT-SPI state is read for the already-resolved compositor
window without another screenshot or a reconstructed generic app selector.
Correlation first requires one AT-SPI application with the exact compositor
PID, then an exact normalized top-level title. Duplicate titles require one
active or focused frame whose screen bounds have at least 0.80 IoU with the
compositor bounds and beat every sibling by at least 0.50. Ambiguous or
insufficient evidence fails closed and attaches no accessibility tree. Only
the selected top-level subtree is flattened.

`ax_status` distinguishes `available`, `empty`, and `unavailable`; AT-SPI
failures and rejected correlations remain in structured diagnostics. Portal
approval pending, denial, unsupported compositor, missing source geometry,
and the shared desktop-request deadline preserve their existing stable
service error codes.

The whole operation runs in the serialized desktop lane under the existing
server-side desktop deadline. The client may disconnect without leaving the
lane permanently blocked; timeout recovery drops cached AT-SPI and portal
session state consistently with other desktop reads.

## Source paths

- `crates/sky-cua-platform/src/model/service.rs`
- `crates/sky-cua-platform/src/paths.rs`
- `crates/sky-cua-service/src/appshot.rs`
- `crates/sky-cua-service/src/daemon/desktop.rs`
- `crates/sky-cua-linux/src/backend/desktop_backend.rs`
- `packages/sky-cua-js/src/targets/linux.ts`

## Verification

- Platform serialization and capability contract tests.
- Service request-ID, AX text, artifact persistence, and daemon dispatch tests.
- Linux correlation fixtures cover Kate dash/whitespace normalization,
  multi-window Dolphin, Ghostty, duplicate-title ChatGPT/Electron, PID
  mismatch, no-winner ambiguity, and sibling-subtree isolation.
- `cargo nextest run -p sky-cua-platform -p sky-cua-service`
- 2026-07-25 isolated live AppShot against ChatGPT/Electron: exact window
  capture, `portal_pipe_wire`, available AX text, and duplicate-title
  active/bounds selection at 0.915 IoU.
- `bun run typecheck` and `bun test test` in `packages/sky-cua-js`

## Known limitations

- AT-SPI text is a bounded, flattened text projection, not a serialized
  accessibility tree.
- Windows without a compositor PID or usable title, or duplicate-title windows
  without a unique active high-IoU frame, deliberately report accessibility as
  unavailable instead of risking a sibling window's tree.
- Artifact cleanup is opportunistic on subsequent AppShot requests; there is no
  dedicated timer.
- Codex Desktop packaging and consumer IPC validation are intentionally outside
  this producer feature.

## Related

- [`display-targeted-screenshots.md`](display-targeted-screenshots.md)
- [`../runtime/mcp-boundary.md`](../runtime/mcp-boundary.md)
