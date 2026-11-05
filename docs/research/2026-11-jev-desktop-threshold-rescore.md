# Desktop ranking replay at 0.7 and 0.8

## Context

On 2026-11-04, replay saved desktop Jev responses at 0.7 and 0.8 after the
[warm semantic repeat](2026-11-jev-desktop-warm-repeat.md) saved 15.7% at 0.9.
Determine additional coverage, wrong targets, and potential avoided Terra time.

## Investigation

No model or desktop calls were made. Frozen manifests were hash-verified and all
twenty records per lane loaded. The replay retains the original rule: a known
non-abstain choice must meet the selected-probability threshold; abstentions
continue to Terra regardless of their probability. Target correctness was
recomputed against the private fixture oracle and checked against saved scoring.

For each newly accepted choice, its ID also equals the target actually selected
by Terra and executed successfully in that trial. The timing estimate subtracts
only that trial's measured Terra adapter elapsed time from the observed hybrid
total. It retains Jev, observation, validation, native action, and readback time.
This is a counterfactual estimate with unchanged downstream work, not a new live
timing measurement or proof of provider latency stability.

| Saved run | Threshold | Accepted Jev targets | Wrong accepted targets | Terra calls | Hybrid time | Saving versus baseline |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Warm semantic | 0.9, observed | 4/7 | 0 | 5 | 57.341 s | 15.7% |
| Warm semantic | 0.8, projected | 5/7 | 0 | 4 | 53.551 s | 21.3% |
| Warm semantic | 0.7, projected | 5/7 | 0 | 4 | 53.551 s | 21.3% |
| Original semantic | 0.9, observed | 4/7 | 0 | 5 | 83.726 s | 24.3% |
| Original semantic | 0.8 or 0.7, projected | 5/7 | 0 | 4 | 77.426 s | 30.0% |
| Image-assisted | 0.9, observed | 4/7 | 0 | 5 | 65.075 s | 11.7% |
| Image-assisted | 0.8 or 0.7, projected | 6/7 | 0 | 3 | 56.134 s | 23.8% |

Accepted-target denominators exclude the exact-rule case and two negatives.
Baselines remain 68.002 s warm, 110.652 s original, and 73.689 s image-assisted.

The additional semantic target is the horizontal slider: selected probability
0.86 warm and 0.82 original. Avoiding the warm slider's Terra call removes
**3.790 s**, improving total savings by **5.57 percentage points**, or reducing
the observed hybrid time by 6.6%. The warm scroll remains below both thresholds
at 0.57; the switch is an abstention. Both negative cases continue to Terra.

Image assistance additionally admits scroll at exactly 0.80 and slider at 0.84.
Those two removed calls total 8.941 s. The switch remains below threshold at 0.49.
There are no newly accepted wrong targets in any of the three saved datasets.

Evidence:

- `artifacts/jev-desktop-pilot/rescore-thresholds.py`: reproducible offline replay.
- `artifacts/jev-desktop-pilot/threshold-rescore.json`: probabilities, target
  checks, remaining fallbacks, and individual removed timings.
- Original evidence remains under `attempt-2`, `attempt-3`, and `warm-repeat`.

## Conclusion

Both 0.7 and 0.8 provide more coverage without wrong targets in the saved results.
They are identical here: **0.7 adds no benefit over 0.8**. At 0.8, the warm-run
projection clears the 20% gate narrowly, at 21.3%; that gate has not been passed
by a live warm run at the new threshold.

## Implications

0.8 is the better candidate for a subsequent live validation because it retains
all gains observed at 0.7. This is a post-hoc threshold selection on a small
constructed corpus, not evidence of calibrated probabilities or general
zero-error reliability. No runtime threshold, frozen protocol, or prior result
was changed. The image replay also warrants re-examining its threshold before
concluding that images can never add useful ranking coverage, while leaving its
original live result intact.
