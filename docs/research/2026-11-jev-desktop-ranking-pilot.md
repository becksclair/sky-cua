# Jev versus Terra through the native desktop path

## Context

On 2026-11-04, evaluate desktop semantic ranking first, then image-assisted
ranking, using the installed sky-cua desktop path. The comparator is
`gpt-6.1-terra` at medium effort, following the requested change from Luna.
The earlier [browser pilot](2026-11-jev-live-browser-pilot.md) remains a separate
experiment with a different comparator and fixture.

## Investigation

### Protocol and actual execution

The installed runtime passed `scripts/deploy_freshness.py`. Trials ran on the
real KDE/Wayland desktop using an isolated sky-cua service socket and the existing
GTK pointer fixture. A small subclass replaces the visible debug status with
neutral text: the original fixture displays its private state and coordinates,
which would otherwise leak answers into observations and images.

Each lane has ten paired cases: one exact button target, seven semantic targets
(button, checkbox, expander, text-field focus, scrolling, horizontal slider, and
switch), and two abstentions (absent print control and ambiguous slider choice).
Each trial creates a fresh fixture. Route order alternates by case:

- Baseline: unique exact label rule, otherwise Terra.
- Hybrid: same rule, otherwise Jev 1.13; accept a known non-abstain target only
  when its selected probability is at least 0.9, otherwise actually call Terra.

The inputs, instruction, cases, threshold, source hashes, and gate were frozen
before inference. The gate is zero wrong actions, no correctness regression,
and at least 20% reduction in paired total wall time. Provider/transport errors
stop the experiment; this harness is not production error failover.

`observe(surface="desktop", window_id=...)` supplies real AT-SPI candidates.
A fixed role/visibility filter retains ten controls, with names, roles, bounds,
and opaque IDs. Neither model receives private fixture state or scoring rules.
The semantic lane supplies those candidates only. The image-assisted lane adds
the same observation's screenshot, read from the host-private AppShot artifact
and verified against its returned size/hash. Both adapters consume the same PNG
preparation: aspect-preserving shrink if needed, padding to a 280px minimum,
and a 1568px maximum side. Actual evaluated images were 1440×900.

Before action, the harness observes again and matches backend reference, label,
role, and enabled state. It supplies current `appshot_id` and `snapshot_id` fences
to sky-cua's `desktop_pointer`, `desktop_semantic`, `desktop_scroll`, or
`desktop_set_value`. `desktop_pointer` can invoke an AT-SPI action; these are
native target-selection trials, not proof that every action injects mouse events.
Text focus uses the explicit focus operation: calibration established that
clicking that entry semantically invokes Activate rather than focusing it.

Independent GTK state proves the actual effect, including the real `clicked`
flag, expanded/toggled state, scroll adjustment, and slider value 75. Fresh
post-action observations are retained. The harness separately scores target
identity and unintended changes to other controls. Terra transcripts contain
only final agent messages, with no model tool calls.

Timing includes observation, optional image preparation, inference and real
fallback, fresh validation, native action, post-observation, and state readback.
Fixture creation/readiness and initial activation are excluded. The operation
and value are prescribed by the harness; models rank a target, not plan a task.

### Results

| Metric | Semantic baseline | Semantic hybrid | Image baseline | Image hybrid |
| --- | ---: | ---: | ---: | ---: |
| Correct tasks | 10/10 | 10/10 | 10/10 | 10/10 |
| Wrong actions | 0 | 0 | 0 | 0 |
| Terra calls | 9 | 5 | 9 | 5 |
| Targets accepted from Jev | — | 4/7 | — | 4/7 |
| Total measured time | 110.652 s | 83.726 s | 73.689 s | 65.075 s |
| Overall saving | — | 24.3% | — | 11.7% |
| Median semantic-positive trial | 8.951 s | 10.372 s | 8.012 s | 5.332 s |

All forty scored trials completed correctly. Jev's raw choices matched the oracle
on all nine calls in each lane, including abstentions. At the frozen threshold,
both lanes accepted the button, checkbox, expander, and text field; scrolling,
slider, and switch still fell back to Terra. Negative cases also went to Terra.

| Fallback target | Semantic selected probability | With image |
| --- | ---: | ---: |
| Scroll | 0.49 | 0.80 |
| Horizontal slider | 0.82 | 0.84 |
| Switch | 0.43 | 0.49 |

Images changed confidence but added no accepted target at 0.9. Jev's median
request time was 279 ms for semantic input and 636 ms with an image. Provider
reported cost for nine scored calls was $0.00091131 and $0.00221277 respectively;
Terra billing was not available. Terra timings include its existing Codex CLI
harness and context, not a matched direct-API comparison.

The semantic route passes the nominal gate, but its advantage is uneven: it wins
four of seven semantic-positive pairs, and its positive median is worse. Excluding
the first cold semantic pair and the exact case lowers the overall saving to
19.6%. The image-assisted route fails the 20% gate. Its seven positive cases save
16.6%, while its two negative cases cost 8.4% more than baseline.

The lanes ran sequentially with variable native/provider latency. Their absolute
times do not establish that images made the overall workflow faster or slower.
The defensible comparisons are the paired routes within each lane and the
observed absence of additional accepted targets. Correct raw answers on this
small sample do not establish calibrated probabilities or justify lowering the
threshold after observing the results.

### Interruptions and boundaries

Preflight window crops were misaligned and included surrounding desktop pixels.
Those images were not sent to either model. The evaluated fixture uses fullscreen;
its clean capture was visually checked before inference. Cropped-window visual
behavior remains unproven by this experiment.

An initial semantic attempt stopped on Jev DNS failure before a decision. The
scored attempt uses an experiment-local DNS-over-HTTPS resolution for that host,
preserving the original HTTPS hostname, SNI, and certificate validation. System
network settings were unchanged. The first image attempt stopped before Terra
inference because the CLI consumed the prompt as another image argument; an
explicit argument separator fixed that transport issue. Original evidence was
retained, and the image lane restarted in a separate directory. Cases, prompts,
and threshold stayed unchanged; only the adapter source hash changed.

The final image runner wrote all twenty aggregate and individual results, then
exited with SIGTERM (143), without a traceback. Its partially removed temporary
judge directory is consistent with interrupted cleanup; the signal sender is
unproven. All expected result pairs and frozen hashes verified. No matching
runner, fixture, socket-tagged process, or retained auth file remained; the
specific residual temporary directory was removed manually. This is reported
separately from trial correctness, not treated as a clean process exit.
A subsequent provider-free check using the same launcher, temporary judge home,
MCP client, and socket-scoped cleanup exited 0, removed its judge home, and left
no tagged processes (`artifacts/jev-desktop-pilot/cleanup-check/result.json`). The
interruption did not reproduce; its original cause remains unresolved.

This is one small, constructed, static GTK screen with informative accessibility
roles. Images supplement semantic candidates; there is no screenshot-only
ablation, generated-coordinate grounding, drag-and-drop task, autonomous
multi-step planning, phone use, or outcome judging. Successful semantic actions
do not validate every pointer gesture or cursor animation.

### Reproduction and evidence

- `scripts/jev_desktop_fixture.py`: existing pointer fixture with debug oracle hidden.
- `scripts/jev_desktop_pilot.py`: frozen cases, real MCP actions, and scoring.
- `scripts/_jev_desktop_models.py`: Jev/Terra adapters and shared image preparation.
- `artifacts/jev-desktop-pilot/attempt-2/semantic-results.json`: scored semantic run.
- `artifacts/jev-desktop-pilot/attempt-3/image-results.json`: scored image run.
- Each attempt's `frozen.json`/`frozen.sha256`, individual observations,
  model requests/responses, action results, and before/after state retain provenance.
- `artifacts/jev-desktop-pilot/summary.json` and `verification.json`: aggregates
  and completeness checks; `desktop-proof.png` shows the verified slider result.

Use `uv run python scripts/jev_desktop_pilot.py freeze --out <new-directory>`,
then `semantic` and `image` with that same directory and authorized provider
credentials. Existing result directories are deliberately not overwritten.
The DNS workaround used here is retained separately under the artifact directory.

Root validation passed: Ruff formatting and lint, basedpyright, and **1,044 pytest
tests**, including 77 new offline adapter/harness tests. No Rust sources changed,
so Rust tests and unrelated compositor/phone smokes were not run. No commit,
installation, or deployment was performed.

## Conclusion

Keep text/AT-SPI Jev ranking as a promising experimental optimization, with
Terra retained as default/fallback. This run establishes real native-consumer
correctness and limited incremental coverage beyond the tested exact rule.
It does not establish a robust performance gain above 20%: the nominal pass is
small, uneven, and sensitive to the initial pair.

Defer image-assisted ranking for this use case. It added no correctness or
accepted-target coverage and missed the performance gate. This finding does not
rule out images for controls absent from accessibility, which were not tested.

## Implications

No production route, default model profile, runtime code, or installation was
changed. The earlier browser results remain separate, including their Luna
comparator. If native optimization is pursued, a bounded warm repeat of the
frozen semantic experiment is the smallest next test; it should precede production
integration. Preserve deterministic selection, fresh-reference validation,
Terra fallback, and independent state verification.

The subsequent [warm repeat](2026-11-jev-desktop-warm-repeat.md) retained zero wrong
actions and the same four accepted targets, but saved 15.7% overall, below the
20% gate. A smaller benefit survives warming; keep native ranking experimental.
