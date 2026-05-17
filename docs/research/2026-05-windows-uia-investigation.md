# Windows UIA inspection: dependency and capture-lane investigation

## Context

The first Windows app-automation milestone added UIA inspection,
semantic action routing, and a GDI blank-frame diagnostic. Three open
questions came out of that work and were recorded in
`goals/windows-app-automation/blockers.md`:

1. Whether the existing `windows-sys` dependency was enough for UI
   Automation COM traversal, or whether a typed `windows` dependency
   was justified.
2. Whether Windows Graphics Capture (WGC) or DXGI Desktop Duplication
   could be added cleanly in the same milestone, or whether the first
   complete pass should ship blank-frame diagnostics and keep the
   stronger capture backend as a follow-up slice.
3. Whether Sumwall Browser could be launched in a disposable smoke
   profile with accessibility-friendly flags, without touching the
   user's active browsing state.

This research records the investigation and the decisions, so the
follow-up plans (`plans/windows_capture_ladder.md` and
`plans/windows_app_shell_smokes.md`) start from a known baseline rather
than a blank page.

## Investigation

### `windows-sys` vs typed `windows`

UIA traversal in Win32 is a COM-heavy code path: `IUIAutomation`,
`IUIAutomationElement`, pattern interfaces (`IUIAutomationInvokePattern`,
`IUIAutomationValuePattern`, `IUIAutomationSelectionItemPattern`,
`IUIAutomationExpandCollapsePattern`, `IUIAutomationTogglePattern`),
condition factories, and array iteration. A raw `windows-sys`
implementation would require a substantial amount of manual COM glue
and `Vtbl::*` dispatch.

The shipped UIA backend uses the typed `windows` crate through narrowly
scoped feature gates. The dependency footprint is acceptable because:

- The typed crate's COM helpers eliminate hand-rolled `IUnknown::Release`
  bookkeeping and `Vtbl::*` invocation.
- Feature flags scope the binary impact to the UIA-only set:
  `Win32_UI_Accessibility`, `Win32_System_Com`,
  `Win32_System_Com_Marshal`, etc.
- The depth-cap and selection hardening in
  `crates/sky-cua-windows/src/uia.rs` keep traversal predictable and
  testable.

### WGC vs DXGI as the additional capture lane

The original goal scope considered shipping a stronger capture lane in
the same milestone. The live evidence from Edge proved this is its own
investigation:

- GDI / `PrintWindow` returns black for Edge while keyboard input and
  window-title changes still take effect. The window is alive; the
  capture path is wrong.
- WGC is the modern API but is session-and-permission-sensitive.
- DXGI Desktop Duplication captures the whole desktop output and works
  in many headless / RDP cases but is not per-window.

Adding either lane mid-milestone risked bundling two unrelated risk
profiles in one slice. The accepted shape was to ship blank-frame
diagnostics first so the failure mode is at least visible, and split
the capture-lane upgrade into a separate ExecPlan.

### Sumwall live-smoke launch

Sumwall Browser was visible to `list_apps` in the installed-cache MCP
smoke, but its window state was minimized / off-screen and only a root
UIA node was returned. Whether Sumwall has accessibility-friendly
launch flags or a disposable profile mode that surfaces the full app
shell is an open question.

The blocker file's Stop-And-Ask rule is explicit: do not kill,
relaunch, or mutate persistent browser profiles for Edge, Sumwall, or
any user app outside an explicit smoke harness. That rule must carry
into the broader live-smoke ExecPlan.

## Conclusion

1. **Use the typed `windows` crate** with narrowly scoped feature
   flags. `windows-sys` alone would require too much manual COM glue
   for the UIA pattern surface.
2. **Ship the blank-frame diagnostic first**, capture-lane upgrade
   second. Splitting reduces risk and lets each slice be validated
   independently. The capture work continues in
   `plans/windows_capture_ladder.md`.
3. **Sumwall launch question is unresolved** and should be answered
   inside `plans/windows_app_shell_smokes.md` before the smoke is
   implemented. The Stop-And-Ask rule on persistent browser profiles
   carries into that plan.

## Implications

- The shipped feature
  ([`docs/features/windows-uia-automation.md`](../features/windows-uia-automation.md))
  reflects these decisions: UIA via typed `windows`, GDI with blank-
  frame diagnostic, Edge live evidence, Sumwall recorded as a partial
  case.
- Both follow-up ExecPlans inherit the constraints from this
  investigation rather than re-deriving them.
- Future Windows work that touches COM should reuse the typed-`windows`
  pattern. Adding new pattern interfaces should extend the existing
  feature-flag set rather than introducing a parallel `windows-sys`
  path.
