# Warm desktop semantic-ranking repeat against Terra

## Context

On 2026-11-04, repeat the [native desktop pilot](2026-11-jev-desktop-ranking-pilot.md)
after warming both routes, to determine whether its 24.3% total saving persists
without the initial cold requests. Comparator: gpt-6.1-terra, medium effort.

## Investigation

The installed sky-cua runtime again passed its source-freshness gate. The frozen
ten cases, instruction, 0.9 threshold, candidate extraction, action validation,
independent GTK scoring, and alternating route order were unchanged. Four unscored
warm-up trials exercised the semantic button case twice through each route, with
reversed order on the second repetition. All four passed. The same desktop service
and DesktopJudges instance were then retained for all twenty measured trials.

This warms the native path, Jev's reusable HTTPS connection, and repeated model
requests. Terra still launches through the existing Codex CLI for each decision;
that consumer overhead remains measured. Upstream model cache state and server
load are not controlled, so this is an operational warm repeat rather than proof
that all remaining latency variation has been removed.

Timing retains the original boundary: observation through decision, actual
fallback, fresh validation, native action, post-observation, and fixture readback.
Warm-ups and fixture setup/initial activation are excluded. No measured pair was
removed afterward. The frozen gate requires zero wrong actions, no correctness
regression, and at least 20% total wall-time reduction across all ten pairs.

| Measurement | Rules then Terra | Rules then Jev then Terra |
| --- | ---: | ---: |
| Correct tasks | 10/10 | 10/10 |
| Wrong actions | 0 | 0 |
| Total measured time | 68.002 s | 57.341 s |
| Median trial time, all cases | 7.335 s | 4.836 s |
| Terra calls | 9 | 5 |
| Positive semantic target total, seven cases | 54.948 s | 42.457 s |
| Positive semantic target median | 7.573 s | 4.409 s |
| Negative-case total, two cases | 8.809 s | 10.742 s |

The warm saving is **15.7% overall**, versus **24.3%** originally. It therefore
misses the fixed 20% gate. Positive semantic targets save **22.7%**, but the two
negative cases cost **21.9% more** through the hybrid route. The three low-confidence
positive targets also cost more with the extra Jev call and Terra fallback.

Jev accepted the same four of seven positive targets: button, checkbox, expander,
and text entry. Scrolling, horizontal slider, and switch still required Terra.
All four accepted-target pairs were faster; the three positive fallback pairs and
both negative pairs were slower. The exact-rule pair was marginally faster, making
five of ten paired wins overall.

Jev's raw decisions matched the target oracle on 8/9 measured calls: it abstained
on the available switch this time. That unnecessary abstention reached Terra and
produced no wrong action. Its request median was 299 ms versus 279 ms originally.
The repeat does not justify lowering the threshold or treating reported
probabilities as calibrated.

Evidence is under `artifacts/jev-desktop-pilot/warm-repeat/`:

- `frozen.json`, `frozen.sha256`, and `repeat-protocol.json`: original inputs and
  the warm-up protocol, written before calls.
- `warmup-results.json`: four excluded warm-ups.
- `semantic-results.json` and `semantic/`: all twenty measured records, provider
  inputs/responses, actual MCP actions, and independent before/after state.
- `summary.json`: original-versus-warm totals and individual pair timings.
- `verification.json`: completeness, oracle, frozen-hash, and cleanup checks.

The experiment-only wrapper is `artifacts/jev-desktop-pilot/warm-repeat.py`;
`warm-dns-run.py` retains the previously required process-local DNS workaround,
with normal HTTPS validation. Runtime and harness source files were unchanged.
All twenty measured records match their individual artifacts and all 24 trials
including warm-ups passed their task checks. The launcher exited 0, removed its
temporary judge home/auth, and completed socket-scoped cleanup. No additional unit
suite was needed for this artifact-only repeat; prior source validation remains
the 1,044-test run documented in the original report.

## Conclusion

A useful but smaller speedup survives warming. The original result was not solely
a cold-start artifact, but a repeatable improvement above 20% overall has not been
established. The benefit comes from the four targets accepted without Terra;
fallback-heavy workloads lose part of that saving.

## Implications

Keep native semantic Jev ranking experimental and retain Terra as the default.
These results support a limited optimization for workloads with enough accepted
targets, rather than a general replacement. The image-assisted result and its
recommendation remain unchanged. No production integration, installation, commit,
or deployment was performed.

The subsequent [saved-response threshold replay](2026-11-jev-desktop-threshold-rescore.md)
accepts one additional correct slider target at either 0.8 or 0.7. Removing its
measured Terra call projects 21.3% overall savings; a live run at that threshold
has not been performed.
