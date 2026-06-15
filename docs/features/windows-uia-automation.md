# Windows UIA inspection and semantic actions

## Status

Shipped. Last verified: 2026-05-08 via release-plugin install plus
direct-installed-cache MCP smoke against Microsoft Edge and Sumwall
Browser. Capture-ladder upgrades, broader app-shell live smokes, and
native UIA readback remain open ExecPlans (see "Related").

## Summary

The Windows backend inspects real app-shell controls through Windows UI
Automation (UIA) and prefers semantic UIA pattern invocation over
SendInput where available. Browser-like apps such as Edge are driven
through native chrome controls (address bar, tab strip, menu buttons)
rather than as opaque rectangles. GDI capture remains the primary
screenshot lane with explicit blank-frame diagnostics; richer Windows
capture paths are tracked separately.

## Contract surface

Public model in `crates/sky-cua-platform/src/model.rs`:

- `SemanticBackendKind::Uia` — Windows backend reports `uia` when UIA
  inspection succeeded for the selected window.
- `CaptureBackendKind::WindowsGdi` — primary Windows capture lane.
- `InputBackendKind::SendInput` / `WindowsMessages` — physical input
  fallbacks; preserved for non-UIA paths.

MCP tool surface (unchanged): `list_apps`, `list_windows`,
`get_app_state`, plus the canonical semantic action set
`focus_element`, `activate_element`, `select_element`, `expand_element`,
`collapse_element`, `toggle_element`, and the physical-action set
`click`, `perform_secondary_action`, `scroll`, `drag`, `type_text`,
`press_key`, `set_value`. Tool schemas are stable across the Linux and
Windows backends.

## Behavior

`WindowsDesktopBackend::get_app_state` selects a top-level window
through Win32 enumeration, then attempts UIA collection for that HWND:

- If UIA succeeds, the backend flattens the UIA subtree into
  `ElementNode` values with stable roles, names, values, state flags,
  bounds, backend references, and semantic actions. `semantic_backend`
  reports `uia`.
- If UIA fails or returns no useful children, the top-level Win32
  fallback element is returned and a precise diagnostic is added.
  Top-level fallback elements expose screenshot-local bounds.

Action routing:

- Element-targeted actions first try the element's semantic backend
  reference. If the reference points to a UIA element with the needed
  pattern (e.g. `InvokePattern`, `ValuePattern`, `SelectionItemPattern`,
  `ExpandCollapsePattern`, `TogglePattern`), the backend invokes that
  pattern.
- If the pattern is unavailable, the backend resolves element bounds
  through the existing coordinate path and uses SendInput or window
  messages.
- Tool results report which lane was used.

Capture:

- GDI / `PrintWindow` is the primary lane.
- Capture coordinates use the same observe-act contract as the rest of
  the plugin: model-visible action coordinates are screenshot / stream
  pixels, and the backend translates them through `capture.logical_rect`
  to desktop pixels only at the native input boundary.
- Blank-frame detection emits a structured diagnostic instead of
  silently accepting a black screenshot as normal state. This was
  prompted by Edge accepting keyboard input and changing window titles
  while GDI returned a black image.

## Source paths

- `crates/sky-cua-windows/src/backend.rs` — Win32 discovery, GDI
  capture, UIA inspection wiring, semantic action routing, SendInput
  fallback
- `crates/sky-cua-windows/src/uia.rs` — UIA traversal and pattern
  invocation
- `crates/sky-cua-platform/src/model.rs` — `SemanticBackendKind::Uia`,
  capture / input backend kinds
- `crates/sky-cua-client/src/mcp_server.rs` — canonical semantic action
  tool definitions (cross-platform)
- `scripts/build_plugin.py` — Windows release bundle
- `install.py` / `scripts/deploy_plugin.py` - installs the bundle into the
  local Codex cache and (on Windows, which has no compat root) enables
  `sky-cua@local` directly

## Verification

Focused tests on a Windows host:

```powershell
cargo +nightly --config 'profile.dev.codegen-backend="llvm"' --config 'profile.test.codegen-backend="llvm"' test -p sky-cua-windows
cargo +nightly --config 'profile.dev.codegen-backend="llvm"' --config 'profile.test.codegen-backend="llvm"' test -p sky-cua-platform
cargo +nightly --config 'profile.dev.codegen-backend="llvm"' --config 'profile.test.codegen-backend="llvm"' test -p sky-cua-client
cargo +nightly --config 'profile.dev.codegen-backend="llvm"' --config 'profile.test.codegen-backend="llvm"' test
cargo +nightly fmt --check
```

The `codegen-backend=llvm` overrides are required because the host
nightly defaults to cranelift on this Windows machine but the backend
crate is not cranelift-clean.

Python harness:

```powershell
uv run ruff format --check scripts
uv run ruff check scripts
uv run basedpyright
uv run pytest
```

Bundle install plus direct installed-cache MCP smoke:

```powershell
python scripts\build_plugin.py
python install.py --mode bundle --agents codex --skip-build
```

Latest accepted live smoke evidence (per
`goals/windows-app-automation/progress.jsonl` 2026-05-08):

- Direct installed-cache MCP smoke listed `computer-use` tools, found
  Edge and Sumwall windows, exposed Edge UIA chrome controls, set the
  Edge address bar via `ValuePattern`, switched tabs and restored via
  UIA, and activated the Edge Settings menu button via the widened UIA
  click path.
- Sumwall Browser was observable but reported as minimized / off-screen
  with only a root UIA node and a blank GDI capture diagnostic.
- The semantic-primitive smoke proved
  `focus_element`/`activate_element`/`select_element` against Edge
  through Windows UI Automation.

## Known limitations

- **Sumwall app-shell coverage is sparse.** When Sumwall is minimized or
  off-screen, only a root UIA node is returned and GDI capture is blank.
  The blank-frame diagnostic is honest, but a richer fallback (and a
  smoke harness that can launch Sumwall in a disposable profile with
  accessibility-friendly flags) is tracked in
  [`plans/windows_app_shell_smokes.md`](../../plans/windows_app_shell_smokes.md).
- **Edge GDI capture can be black.** Browser-like GPU windows are not
  reliably captured by GDI / `PrintWindow`. The blank-frame diagnostic
  fires; the capture-ladder upgrade to Windows Graphics Capture or DXGI
  Desktop Duplication is tracked in
  [`plans/windows_capture_ladder.md`](../../plans/windows_capture_ladder.md).
- **No native UIA readback yet.** `ElementNode.text`,
  `numeric_value`, and `supports_editable_text` compile but extract
  empty values on Windows. Native extraction is a roadmap item under
  "Windows parity"; the Linux AT-SPI extraction shape is the model.
- **Windows native overlay is deferred.** See
  [`docs/features/agent-cursor-overlay.md`](agent-cursor-overlay.md).

## Related

- Open ExecPlan: [`plans/windows_capture_ladder.md`](../../plans/windows_capture_ladder.md)
- Open ExecPlan: [`plans/windows_app_shell_smokes.md`](../../plans/windows_app_shell_smokes.md)
- Research: [`docs/research/2026-05-windows-uia-investigation.md`](../research/2026-05-windows-uia-investigation.md)
- ROADMAP entry: [`ROADMAP.md`](../../ROADMAP.md) under "Windows parity"
- Originating goal package retired: `goals/windows-app-automation/` (replaced by this doc and the two ExecPlans above)
