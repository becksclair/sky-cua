# Blockers: First-class Windows app automation

## Open Questions

- Whether the existing `windows-sys` dependency is maintainable enough for UI Automation COM traversal, or whether a typed `windows` dependency is justified.
- Whether Windows Graphics Capture or DXGI Desktop Duplication can be added cleanly in this milestone, or whether the first complete pass should ship blank-frame diagnostics and keep the stronger capture backend as a follow-up slice.
- Whether Sumwall Browser can be launched in a disposable smoke profile with accessibility-friendly flags, without touching the user's active browsing state.

## Stop And Ask

- Before killing, relaunching, or mutating persistent browser profiles for Edge, Sumwall, or any user app.

## Dangerous Or High-Risk Actions

- Process-wide browser termination outside an explicit smoke harness.
- Registry, group policy, startup, service, or persistent profile changes.
- Storing screenshots that expose private browser state, credentials, tokens, or messages.
- Destructive git operations such as reset, checkout-over-user-work, rebase, amend, or force-push.
- Treating CDP/WebView2 inspection as permission to automate page DOMs; this goal is app-shell automation only.

## Known Blockers

- No active blocker yet. The next implementation action is to read the Windows backend instructions and implement the first UI Automation inspection slice.
