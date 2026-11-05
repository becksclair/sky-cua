# Real-workflow Jev pilot: native preflight blocked

Follow-up: [completed edit/save comparison and corrected diagnosis](2026-11-jev-real-edit-save-results.md).
The dry2/dry3 errors were later confirmed to be `stale`, caused by activating
after observation; they do not establish repeated `wrong_target`. Save As remains
limited by Qt local-origin geometry correlation. A real edit/Ctrl+S variant ran
successfully, but Jev avoided no Terra calls.

## Context

On 2026-11-04, attempt a real application workflow at Jev threshold 0.8 against
gpt-6.1-terra medium. The question is whether caller-level routing can avoid
actual Terra requests and improve complete workflow time, following the
[saved-response replay](2026-11-jev-desktop-threshold-rescore.md).

## Investigation

Built `scripts/jev_real_workflow.py`, an experimental KWrite Save As prototype.
The intended workflow opens the File menu, selects Save As from semantic intent,
sets a disposable destination through the dialog, and commits the save. It is
designed to verify both exact output bytes and preservation of the original.
The caller knows each next intent before deciding whether to call a model.
Baseline uses exact rules then Terra; hybrid uses exact rules then Jev at 0.8
then Terra fallback. The planned run has an excluded warm-up per route and three
alternating measured pairs. That measured run was not started.

The installed sky-cua runtime passed the source-freshness gate. Preflight used
real desktop observe/action tools, owned application processes, scratch files,
and disposable application configuration. No model received preflight snapshots.
The initial KWrite inventory included hidden recent-file items from shared data;
subsequent probes isolate config, data, and state homes. Candidate projection
filters out hidden items. No screenshots were sent to providers.

### KWrite on Wayland

AT-SPI exposed usable document controls and menus. `observe` returned fresh
AppShot and snapshot IDs for the owned KWrite window, with `WindowFocusVerified`.
The first File menu action nevertheless returned `AppShotRequired` with reason
`wrong_target` before execution.

Follow-up probes remained rejected after explicitly reactivating the window
immediately before action. Their full errors were subsequently confirmed to be
`stale`: activation created a newer snapshot after observation. A matching saved
focused-window readback therefore did not make the older fences valid. The
overlay-disabled probe had the same ordering bug and cannot rule out overlay
involvement in the original, separate `wrong_target` failure.

Source inspection narrows the rejection but does not establish its root cause:
`validate_action_appshot` in `crates/sky-cua-service/src/daemon/desktop.rs` checks
snapshot freshness, then compares capture window IDs with a new backend
focused-window lookup. `wrong_target` can represent a different ID, no focused
window, or a lookup error. It does not compare application IDs. Capture focus
verification and action validation use separate readbacks; these probes do not
record the exact backend lookup result at rejection.

### Control and alternate route

The previously successful GTK pointer control ran once through the same installed
runtime and passed its actual click oracle. This rules out a blanket failure of
all current native action calls; it does not isolate the KWrite cause.

A separately recorded KWrite XWayland probe failed earlier, during activation:
the requested X11 ID and the focus-verified KWin ID disagreed. No action fence or
identity check was bypassed.

LibreOffice Writer was checked as an alternative real application with a private
profile. Its initial observation exposed only a native-window fallback, without
the semantic controls needed for this experiment. A further launch with explicit
accessibility support did not yield the uniquely identified document window
within the bounded startup wait. No model-driven Writer trial ran.

### Evidence and validation

Evidence is under `artifacts/jev-real-workflow/`:

- `pilot/dry/`: first KWrite rejection, observations, action preparation, error.
- `dry2/dry/file_menu/`: fresh observation, explicit activation and matching
  `focused-before-action.json`, followed by the rejected action in `error.txt`.
- `dry3/dry/`: overlay-disabled repetition.
- `dry-xwayland/dry/`: separate XWayland activation rejection.
- `gtk-control/dry/result.json`: passing actual GTK click control.
- `preflight.json` and `preflight-lo.py`: alternative Writer probe.

The prototype is incomplete past the blocked first action: its file-dialog
selectors and model-driven workflow have not been validated. Dry-run failures
now return a nonzero process exit. Owned test applications and isolated MCP
services were stopped. No runtime source, installation, deployment, or commit
was changed.

Root Python validation passed: Ruff format/lint, basedpyright, and 1,044 tests.
These checks do not establish successful execution of the new workflow prototype.
`verification.json` records the blocked status and absence of measured/model calls.

## Conclusion

**The real-workflow comparison is blocked before model inference.** There are
zero measured workflows and zero Jev or Terra requests from this attempt. No
claim about real-workflow correctness, avoided Terra calls, or speedup can be
made. The earlier fixture results and timing projections remain unchanged.

## Implications

The immediate prerequisite is to resolve the KWrite focus/window-identity
discrepancy while retaining rejection for genuinely wrong windows. The useful
next diagnostic is the exact focused-window result during action validation,
compared with the capture IDs. Then complete the deterministic Save As preflight
and run the unchanged 0.8 paired model comparison. Further threshold tuning
cannot address this platform blocker.
