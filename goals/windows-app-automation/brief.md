# First-class Windows app automation

## Outcome

Add first-class Windows app-shell inspection, capture, and semantic action support for browser-like desktop apps in sky-cua.

## Context

- `sky-cua` is a Rust workspace plus Python harnesses that exposes desktop computer-use tools through a Codex plugin.
- The Windows backend lives in `crates/sky-cua-windows/src/backend.rs`. It currently discovers top-level Win32 windows, captures screenshots with GDI/`PrintWindow`, and sends physical input with `SendInput` or an RDP-safe window-message fallback.
- The shared contract in `crates/sky-cua-platform/src/model.rs` already separates capture, input, and semantic backends. Windows reports `CaptureBackendKind::WindowsGdi`, `InputBackendKind::SendInput` or `WindowsMessages`, and `SemanticBackendKind::None`.
- Live release-plugin testing showed Sumwall Browser can be driven by screenshot coordinates, while Microsoft Edge accepts keyboard input but returns black GDI screenshots and no semantic child tree.
- The user wants first-class automation of the actual Windows app shell, not website or DOM automation. The target surface includes windows, tabs, address bars, menus, dialogs, browser chrome, settings pages, and native controls.

## Constraints

- Keep the existing MCP tool contract stable unless a schema change is explicitly justified and tested.
- Preserve current Win32 window discovery and SendInput behavior as fallbacks.
- Do not claim semantic support unless the backend returns real UI Automation or equivalent provider data.
- Do not treat browser CDP as the primary answer. CDP is allowed only as an optional app-shell side channel when a target exposes it.
- Keep screenshot coordinate mapping honest: returned coordinates must match the model-facing image.
- Keep Windows failures diagnosable with structured diagnostics instead of silent fallback.
- Avoid destructive operations against the user's live browser profiles, app data, or Codex configuration.

## Non-Goals

- Building general website automation, DOM locators, or page-content scraping.
- Replacing Playwright, CDP browser automation, or external web test tooling.
- Requiring users to relaunch Edge or Sumwall with special flags for the normal path.
- Committing live screenshots, tokens, browser profile data, or private app state.
- Making Appium or WinAppDriver a runtime dependency for `sky-cua`.

## Ask Before

- Ask before changing the public MCP tool schema in a way that can break existing callers.
- Ask before killing, relaunching, or changing startup flags for user-owned Edge/Sumwall processes outside an explicit smoke harness.
- Ask before writing persistent browser profile, registry, or system accessibility settings.

## Done Means

- Windows `get_app_state` returns richer first-class app-shell state when UI Automation is available, reports honest fallback diagnostics when it is not, and uses a more reliable capture ladder for browser-like app windows.
- Semantic actions prefer real UI Automation patterns where available and fall back to the existing physical input path when needed.
- Edge and Sumwall live smokes demonstrate the improved behavior or clearly record the remaining provider/capture limitation.
- Rust tests, Python harness tests, and at least one live Windows app smoke pass with evidence recorded in `progress.jsonl`.
