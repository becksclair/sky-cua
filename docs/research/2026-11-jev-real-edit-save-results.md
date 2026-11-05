# Jev at 0.8 versus Terra in a real KWrite workflow

## Context

On 2026-11-04, continue the [blocked real-workflow pilot](2026-11-jev-real-workflow-pilot.md)
to test actual Terra-call avoidance and complete workflow latency. Use the installed
sky-cua native path, Jev 1.13 at 0.8, and gpt-6.1-terra medium.

## Investigation

### Fixes and workflow scope

Inspection of the full retry errors corrected the earlier diagnosis: dry2/dry3
reported **stale**, not repeated `wrong_target`. The probe activated KWrite after
obtaining a fresh observation. Activation captures a destination AppShot, making
the earlier fences stale. Moving activation before observation fixed that ordering;
the File menu action then passed. No runtime identity check was weakened.

The original single `wrong_target` remains a separate, unobserved focus readback
mismatch/error. Its exact cause was not established. The overlay-disabled retry
also contained the stale-ordering bug, so it did not rule out overlay involvement
in that original failure.

Save As exposed another issue. An owned AT-SPI diagnostic found two different
same-title Qt objects: one active real dialog at local `(0,0,864,540)` and one
inactive small object at `(260,185,100,30)`. KWin placed the actual dialog at
`(415,246,876,574)`. Duplicate-title correlation requires screen-space IoU >=0.8;
the real dialog's local-origin geometry produced 0.158. This explains the rejected
Save As correlation. Evidence: `artifacts/jev-real-workflow/atspi-dialog-diagnostic/`.
The dialog-specific fix remains a follow-up; this pilot did not change that policy.

The completed pilot therefore uses the repository's existing KWrite edit-and-save
consumer pattern (`scripts/live_kwrite_smoke.py`), rather than Save As:

1. Launch an owned KWrite instance on a disposable existing document, with private
   configuration/data/state homes.
2. Observe real native candidates and rank the main editor for a caller-known
   replacement intent. Baseline calls Terra; hybrid asks Jev first and uses Terra
   when Jev abstains or fails the 0.8 acceptance threshold.
3. Reactivate before the fresh observation, resolve the same native reference,
   and write replacement text through `desktop_set_value`.
4. Save with the common deterministic, fresh-fenced `desktop_keyboard` Ctrl+S
   action. Independently verify changed exact file bytes and content hash.

Both models receive the same projected native labels, roles, context, bounds,
enabled/visible information. The shared adapter currently omits native state flags
and description fields from model input; editability is retained for private
target validation. This is a representation limitation, not a proven explanation
of Jev's abstentions. No images are supplied.

The private oracle counts and rejects wrong target selections before executing
them; any such selection fails the experiment. It is not sent to either model.
The script fixes the intent before inference; no Terra planning turn is made
before the routing choice. This demonstrates an actual caller boundary where
a selection request could be avoided, rather than adding Jev after Terra planning.

### Frozen protocol and results

The protocol was written and hashed before calls: 0.8 threshold, one complete
unscored warm-up per route, three alternating measured pairs, zero wrong targets,
exact file proofs, and at least 20% total workflow saving. Setup and cleanup are
excluded; timing runs from first observation through persisted file proof.

| Measurement | Terra | Jev then Terra |
| --- | ---: | ---: |
| Measured workflows completed correctly | 3/3 | 3/3 |
| Wrong target selections | 0 | 0 |
| Actual Terra calls | 3 | 3 |
| Actual Jev calls | 0 | 3 |
| Total workflow time | 46.695 s | 50.917 s |
| Terra calls avoided | — | **0** |
| Time saving | — | **−9.0%** |

Pair times (baseline / hybrid): 16.273 / 18.748 s, 15.303 / 16.204 s, and
15.119 / 15.965 s. The hybrid route was slower in all three pairs. Jev abstained
on all three editor selections, with selected-abstention probabilities 0.61,
0.55, and 0.56. It also abstained during its warm-up. Every actual selected editor
was chosen by Terra. Lowering the non-abstain threshold would not accept these
responses because abstentions always fall through.

This is a small operational comparison, not a latency distribution estimate.
Host/provider load was uncontrolled; final-pair execution overlapped Python
verification checks. The 9.0% difference is descriptive, and cannot all be
attributed causally to Jev's request time. The decisive call-count result is
independent of that timing noise: Jev removed no Terra requests.

### Evidence and verification

- `scripts/jev_real_workflow.py`: caller-level routing and real native workflow.
- `scripts/test_jev_real_workflow.py`: 19 offline tests for projection, target
  validation, 0.8 routing, file proof, failure evidence, and paired summaries.
- `artifacts/jev-real-workflow/edit-save-preflight/dry/`: successful deterministic
  workflow and exact-byte proof before model calls.
- `artifacts/jev-real-workflow/edit-save-pilot/frozen.json` and `frozen.sha256`:
  frozen protocol and oracle (excluded from model requests).
- The same directory's `results.json`, `summary.json`, warm-up directories,
  and individual `pair-*` directories retain all observations, requests,
  responses, executed actions, and file proofs.

The installed runtime source-freshness gate passed. Root Python Ruff formatting,
lint, basedpyright, and **1,063 pytest tests passed**. No final Rust source changes
remain. A temporary diagnostic trace was reverted and its release build stopped
after the harness ordering bug was identified; no diagnostic runtime was installed.
The pilot exits 1 because the performance gate fails, while every workflow record
has status `ok`. Temporary judge credentials and owned GUI/service processes were
cleaned up. No commit, installation, or deployment was performed.

## Conclusion

**This real workflow does not justify adding Jev to the selection path.** The
application workflow succeeded, but the hybrid route avoided zero Terra calls
and failed the performance gate. The favorable fixture replay did not transfer
to this observed native editor representation.

## Implications

Retain the existing deterministic/Terra path for this workflow. The editor is
also distinguishable by its native editable role, so a deterministic selector
should be considered before paying either model for this fixed operation.
This experiment does not establish savings in a generic autonomous Terra loop
or resolve Save As dialog support. Further Jev work would need to show accepted,
correct decisions on genuinely ambiguous real controls with complete native
metadata, rather than infer production utility from the previous fixtures.
