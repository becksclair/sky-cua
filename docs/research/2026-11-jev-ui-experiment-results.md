# Jev UI feasibility: measured results

## Context

**Conclusion: no production integration is justified by this experiment.** Jev's
request path is substantially faster than the current Luna path, but it did not
provide reliable outcome checks or confident incremental target selection beyond
the fixture rules. Both models accepted a stale success banner in boundary tests.

Follow-up: [lower ranking thresholds and the verified image-size fix](2026-11-jev-ranking-thresholds-and-image-fix.md) add limited incremental ranking coverage and resolve the image transport blocker.

Run date: 2026-11-04. Source base: `2ffbe4431434206ad21dee3efcbaf4609dd2ca21`.
This follows the [research and proposed experiments](2026-11-jev-ui-classification-and-testing.md).
Only local experiment code, evidence and documentation changed; no runtime
integration, deployment, installation, commit or push was performed.

**This was a synthetic feasibility screen, not the proposed recorded-application
benchmark.** The 100 cases are constructed fixture models with separate private
state and visible observations. The application-family holdouts share generation
patterns; they are not independent real applications. The results support stopping
or narrowing these directions, not an estimate of production UI accuracy.

## Investigation

### Design, provenance and gates

The frozen corpus has 64 outcome cases across eight families, 24 ranking cases,
and 12 constructed bounded tool episodes. There are 28 development and 72 held-out
cases. Questions, labels, splits and the selected-option probability threshold
of **0.9** were frozen before inference; no threshold was fitted to results.
Expected labels and private fixture state were excluded from model inputs.

Outcome cases include exact readback, field validation, pending/queued work,
acknowledged clicks without application change, stale banners, contradictory text,
and missing/truncated observations. Ranking includes randomized opaque IDs,
5–20 observed candidates, duplicate labels, disabled controls, absent targets,
explicit multi-target requests, and semantic “dismiss without saving” choices.
The single-target interface must abstain on multi-target requests. Annotation
cases restore successful observation payloads; they are not real smoke replays.

The gate required zero false-success/wrong-target decisions, at least 80% coverage
on decidable cases, and at least 20% useful decision-stage time reduction. A
selected-option probability is a routing heuristic, not a calibrated probability
of correctness. Explicit `insufficient`/`abstain` labels, low-probability
abstentions, and API failures remain distinct in raw evidence.

The initial comparison used 20 family batches per model. Two identical mixed
batches then repeated six boundary inputs. Exploratory ablations presented 12
held-out ranking inputs and those six boundary inputs individually. These are
**130 decisions per model from 100 unique inputs**, not 130 independent examples.
Moving a case from its original family batch to a mixed or single-case request
changes surrounding context; it is not a pure stochastic repeat.

- Jev: `jev-1.13`, direct OpenCode System One endpoint, existing external
  `JevClient`, one reused HTTPS client per phase, no retry or OpenCode/Pi process.
  Full successful responses, probabilities, usage, billing and `last_elapsed_ms`
  are retained.
- Luna: repository `performance_judge` profile, `gpt-5.6-luna`, medium reasoning,
  `codex-cli 0.166.0`, existing `run_codex_exec` path and fast service tier. It
  answered the same bounded questions and visible evidence with a JSON schema.
  **This is not a comparison against the full four-part performance-judge rubric.**
  The actual CLI overhead is included; its additional system context differs
  from the direct Jev route. No tool actions appeared in its retained traces.
- Measured scripts used Python 3.13.11 through the repository's `uv` environment;
  initial transport diagnostics used host Python 3.14.9. No dependency was added.
- Route order alternated by batch. Cold first calls are included. Corpus
  construction took 1.43 ms; local rule and state-serialization microbenchmark
  medians were 0.00088 ms and 0.00476 ms respectively. These exclude UI acquisition.
- The inherited OpenCode credential was stale. The already configured refreshed
  key worked after loading its existing alias correctly; no credential was
  rotated. OpenCode's Luna route returned 403, so the measured Luna calls used the
  repository's actual Codex route. Setup failures are outside measured text calls.

### Outcome checks

Original held-out four-way classifications:

| Path | Correct / 48 | Low-probability abstentions | Decidable coverage |
| --- | ---: | ---: | ---: |
| Fixture-aware rules | 48 | 0 | 36/36 |
| Jev raw choice | 46 | 0 | 35/36 |
| Jev at 0.9 | 36 | 12 | 29/36 |
| Luna | 47 | Not applicable | 35/36 |

The 12 insufficient-evidence cases count as legitimate labels, not API failures.
Rules know all fixture vocabulary and exact readback structure; their perfect
score does not demonstrate a general semantic interpreter. They represent the
kind of explicit predicates already available to deterministic acceptance tests.

The original holdout contained no false-success result. **That favorable result
did not survive boundary testing.** `outcome-04-07` has the same success banner
before and after an acknowledged click, an edited form value, and no evidence of
a new commit. Fixture state records that the operation was not submitted. Its
correct evidence label is `insufficient`.

Both models changed from `insufficient` in the family batch to `saved` in both
mixed-batch repeats. Jev assigned `saved` probability **0.97** in each. Presented
alone, both again returned `saved`; Jev's probability was **0.98**. This is one
unique counterexample observed in three additional contexts/runs, not three
independent failures. It defeats the zero-false-success gate even with the frozen
threshold. It also shows why keeping Luna alone would not solve this evidence
problem: retain deterministic freshness and readback checks.

### Ambiguous target ranking

Original held-out ranking classifications:

| Path | Correct / 18, including correct abstention | Unique targets selected / 12 | Wrong targets |
| --- | ---: | ---: | ---: |
| Exact rules | 12 | 6 | 0 |
| Jev raw choice | 14 | 8 | 0 |
| Jev at 0.9 | 11 | 5 | 0 |
| Luna | 17 | 11 | 0 |

The rules abstain on semantic paraphrases. In the single-case ablation, Jev's raw
choice selected 10/12 targets, versus Luna's 12/12. But the threshold accepted only
4/12 Jev targets: **all four were exact cases already solved by rules**. Across the
six semantic choices, raw Jev selected 4/6 correctly, gated Jev selected **0/6**,
and Luna selected 6/6. No incorrect target was selected; abstention is the limiting
factor. Removing cross-case batching did not recover useful gated coverage.

No browser action was attempted. A future semantic selector would still have to
return observed IDs and pass the existing live resolver's identity/actionability
checks; these measurements do not test action success or reference freshness.

### Bounded judge annotations

On six held-out constructed episodes, raw Jev and rules scored 6/6; gated Jev
returned five correct labels and one abstention; Luna scored 5/6, labeling a final
observation failure `insufficient` rather than `contradicted`. Neither model made
a false supported-claim decision in this small set.

This is limited evidence for an advisory labeler, not replacement of
`scripts/_agent_perf_judge.py`. Its full output includes explanatory triage and
four rubric scores. Its current `condense_transcript` strips successful result
payloads and images, whereas this experiment supplied the relevant observations.
No explanatory equivalence, reviewer-time saving or additional coverage beyond
fixture rules was demonstrated. Full real-smoke replay and reviewer-effort
measurement stopped here; keeping every full-judge call would add Jev overhead.

### Image feasibility

The [current TypeSafe image documentation](https://docs.typesafe.ai/images)
describes Jev 1.13 image input with `TypeSafe-Danger-Zone-Images: true`, PNG/JPEG
assets, and a separate 1.14 requirement for boxes. The existing helper gained only
an optional `experimental_images` flag; default headers are unchanged.

A non-sensitive red-square fixture was encoded as WebP and converted to PNG.
The documented asset payload and header returned **HTTP 400** through OpenCode:
624 ms request time, 747 ms including fixture generation/conversion. The second
blue-square control was therefore not submitted. The helper suppresses provider
error bodies, so the rejection's precise cause is unclassified. This is a failed
route prerequisite, **not an image-accuracy result or proof that Jev lacks image
support**. Text/image comparisons and box experiments did not proceed; no model
upgrade was attempted. The [subsequent diagnosis](2026-11-jev-ranking-thresholds-and-image-fix.md)
identified undersized fixture dimensions; padding to 320 × 280 produced correct
red/blue answers on the same endpoint.

### Latency, cost and actual substitution opportunity

| Phase | Jev p50 / p95 request ms | Luna p50 / p95 request ms | Jev reported charge |
| --- | ---: | ---: | ---: |
| Initial 20 family batches | 315 / 439 | 4,954 / 5,580 | $0.00262483 |
| 12 single-case ranking replays | 275 / 598 | 3,573 / 5,758 | $0.00073348 |
| Six single-case boundary replays | 241 / 467 | 3,706 / 4,735 | $0.00021001 |

P50 is the median; p95 uses nearest rank. With the two mixed-boundary repeats,
there were 40 successful measured calls per model and no measured text API
failures. Jev reported **$0.00388702** total, 92,548 input and 12,079 output tokens.
Luna reported 690,365 input tokens, including 27,008 cached input tokens, and 3,484
output tokens. **Luna provider billing was unavailable; no dollar saving is
claimed.** These totals exclude transport preflights and the rejected image call;
one successful Jev preflight separately reported $0.00001180.

Request speed alone is not the workflow result. Summing measured component
calls for a conservative full-batch escalation on any probability below 0.9 gives
6.7% **more** time for held-out outcomes and only 11.4% less for ranking, below the
20% gate. This is a reconstruction, not an executed agent pipeline.

The single-case semantic ranking comparison is more decisive: all six Jev calls
require fallback. The measured components sum to **24.146 s** for Jev then Luna
versus **22.310 s** for Luna alone: **8.2% more decision time**, before any extra
observation or action. Exact cases offer apparent model-call savings, but rules
already solve them without either call. No incremental gated semantic coverage
was obtained where the model was actually needed.

No end-to-end live agent timing, repeated UI observations/actions, real app
acceptance run, or human-review time was measured. The offline failures and lack
of useful substitution did not justify expanding into controlled browser,
native-desktop or phone fixtures. Consequently there is **no demonstrated total
agent-time or comparative cost improvement**.

## Conclusion

| Candidate | Disposition | Reason / boundary |
| --- | --- | --- |
| Semantic outcome checks | **Defer integration** | Confident stale-banner false success; existing exact predicates remain authoritative. |
| Ambiguous element ranking | **Narrow further research; defer integration** | Only lane with raw semantic decisions beyond rules, but zero accepted single-case semantic targets at the frozen threshold. |
| Judge annotations | **Experimental advisory labels only; defer integration/replacement** | Tiny synthetic signal, no demonstrated additional coverage or avoided full-judge work. |
| Image classification | **Transport fixed; defer semantic adoption** | Follow-up padding fixed the undersized fixture; red/blue controls pass, while UI-image accuracy remains untested. |

This does not establish that every Jev prompt, threshold or UI domain would fail.
It establishes that this frozen configuration does not earn a place in sky-cua's
production workflow. The strongest optional follow-up is a small corpus of real,
individually presented ambiguous candidate decisions with independent intended
IDs, measuring incremental resolution beyond exact rules and actual avoided
agent calls. That is a new experiment, not a reason to tune this holdout until it
passes. No follow-up is required to close the present investigation.

## Implications

The runtime, deterministic acceptance gates, model profiles, full judge and live
reference validation remain unchanged. The original design and unrelated plans
were preserved. The backlog now links this measured disposition.

Reproducible local sources:

- [`scripts/_jev_ui_fixtures.py`](../../scripts/_jev_ui_fixtures.py): constructed
  fixture state, labels, splits and rules.
- [`scripts/jev_ui_experiment.py`](../../scripts/jev_ui_experiment.py): frozen
  payloads, existing Jev client reuse, current Luna route and raw result retention.
- [`scripts/jev_ui_image_probe.py`](../../scripts/jev_ui_image_probe.py): documented
  image-route prerequisite probe.
- [`scripts/test_jev_ui_experiment.py`](../../scripts/test_jev_ui_experiment.py):
  oracle exclusion, frozen-byte integrity and candidate-ID boundary checks.
- External helper: `/home/bex/.agents/research/jev-client/jev.py`, adjacent README
  and focused header test; no other Jev evaluation artifacts were edited.

Evidence is retained locally under [`artifacts/jev-ui/`](../../artifacts/jev-ui/):
`frozen.json`, `frozen.sha256`, `records.json`, `analysis.json`, `summary.json`,
`preprocessing.json`, `provenance.json`, complete `luna/` traces, and the
`single/`, `boundary-single/`, and `images/` subdirectories. Artifacts are ignored
by Git and contain fixture-only input; temporary copied Codex auth was removed.
Raw matrices and per-call probabilities are authoritative for detailed inspection.

The primary frozen manifest SHA-256 is recorded in `frozen.sha256`. To repeat in a
new output directory with the existing credentials and helper available:

```sh
uv run python scripts/jev_ui_experiment.py freeze --out artifacts/jev-ui-new
uv run python scripts/jev_ui_experiment.py run --out artifacts/jev-ui-new
uv run python scripts/jev_ui_experiment.py summarize --out artifacts/jev-ui-new
uv run python scripts/jev_ui_experiment.py freeze-single --out artifacts/jev-ui-new/single
uv run python scripts/jev_ui_experiment.py run --out artifacts/jev-ui-new/single
```

`run` makes paid calls. Freeze/run refuse to overwrite retained manifests/results.
The pre-existing frozen boundary-single payload is retained for exact replay.
Python formatting, Ruff, basedpyright and all **940 pytest tests** passed; the
external helper's 12 mocked tests also passed. No Rust runtime or packaging code
changed, so Rust, packaging and live desktop/VM/device gates were not run.
