# Codex Goal Prompt: First-class Windows app automation

After every critical document in this folder is approved with Plannotator, paste or set this goal:

```text
/goal Implement first-class Windows app-shell automation for sky-cua using `goals/windows-app-automation/` as the durable source of truth. The outcome is that the Windows backend can inspect real app-shell controls with UI Automation where available, prefer semantic UIA actions before SendInput fallbacks, improve or honestly diagnose browser-like app capture, and prove the result through Windows live smokes against Edge/Sumwall or another comparable app.

Read `goals/windows-app-automation/brief.md`, `plan.md`, `verification.md`, and `blockers.md` before editing. Keep `goals/windows-app-automation/progress.jsonl` append-only and record each implementation milestone, command, result, diagnostic, and artifact path.

Stay focused on actual Windows desktop app automation: windows, tabs, menus, address bars, dialogs, settings, title bars, app chrome, and native controls. Do not build website automation. Preserve the existing MCP contract and SendInput fallback unless a tested schema change is explicitly justified. Ask before killing or relaunching user browser processes.

Acceptance requires observable evidence: UIA-capable apps return more than one real element with `semantic_backend = uia`; semantic click/value actions report their lane; browser-like black captures are diagnosed rather than silently accepted; Edge/Sumwall live smokes record current behavior; and the relevant Rust/Python/package checks pass or have documented external blockers.
```
