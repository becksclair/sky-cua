# Jev for UI classification and semantic testing

## Context

Investigation date: 2026-11-04.

Research question: can Jev reduce agent work or improve test coverage in
sky-cua by making bounded decisions about observed UI elements, application
state, or tool-use evidence?

The strongest first experiment is semantic outcome checking against recorded
fixture states. Ambiguous element ranking and test-judge assistance are also
plausible. Basic widget-role detection, coordinate validation, and exact test
assertions already have better deterministic inputs. This design-stage
investigation did not integrate Jev or run sky-cua-specific model experiments. The subsequent [measured results](2026-11-jev-ui-experiment-results.md)
evaluate these proposals and recommend no production integration.

## Investigation

### Existing evidence and model boundary

Jev returns closed-set choices, yes/no probabilities (`noul`), and rubric
scores. It does not produce an explanatory review or operate tools. Multiple
questions can share one observed state in a request.

A separate local synthetic text benchmark on 2026-11-04 measured these median
end-to-end times through OpenCode's paid `jev-1.13` endpoint:

| Client path | Median | Samples |
| --- | ---: | ---: |
| Python, new process per request | 585 ms | 16 requests |
| Python, reused process and connection | 255 ms | 16 requests |
| Python, 16 questions per batch | 285 ms per batch | 5 requests |
| Pi classifier SDK, reused process | 268 ms | 16 requests |

The retained benchmark is at
`/home/bex/.agents/research/jev-client/results.json` on the research host.
It contains 224 scored decisions from only 16 unique synthetic cases. These
numbers demonstrate transport feasibility, not UI accuracy, image latency,
or savings in a complete sky-cua workflow. A classifier that leaves the
agent's subsequent work unchanged adds a request rather than saving one.

The earlier skill-description probes also showed why coverage matters:
0.3/0.7 yes/no bands produced 17 correct decisions and three abstentions
across 20 pairs. Those bands are candidates to evaluate, not calibrated UI
thresholds. Confidence and rubric scores must not be treated as independent
proof that a judgment is correct.

Jev 1.13 accepts images behind the experimental
`TypeSafe-Danger-Zone-Images: true` header. Bounding-box output requires Jev
1.14. TypeSafe explicitly describes image and box support as not ready for
production. Its documented image formats are PNG/JPEG; sky-cua's normal
WebP delivery would need a separate supported representation for this path.
Image support through the available OpenCode route has not been verified.
See [TypeSafe images](https://docs.typesafe.ai/images),
[Box](https://docs.typesafe.ai/primitives/box),
[confidence](https://docs.typesafe.ai/confidence), and
[OpenCode model endpoints](https://opencode.ai/v2/docs/console/models/#jev).

### Where sky-cua already has authoritative information

- [`ElementNode`](../../crates/sky-cua-platform/src/model.rs) exposes role,
  name, description, value, state flags, supported semantic actions and bounds.
  A classifier should not overwrite observed capabilities or manufacture
  widget semantics absent from the backend.
- The [browser snapshot producer](../../crates/sky-cua-service/src/browser/snapshot.rs)
  emits DOM-derived tags, roles, labels, values, disabled state, bounds and
  opaque references. The [element resolver](../../crates/sky-cua-service/src/browser/resolve.rs)
  checks live identity and actionability. Semantic selection cannot replace
  those checks.
- [Phone snapshots](../../crates/sky-cua-service/src/phone/snapshot.rs)
  validate session, device, age, orientation and resolution. These are exact
  conditions, not model judgments.
- [AppShots](../features/native-linux-appshots.md) provide verified window
  images and best-effort AT-SPI text with explicit availability and provenance.
  Missing text is missing evidence, not evidence that an element is absent.
- The [web-workbench acceptance harness](../../runtime/cua-node/production/web-workbench-acceptance.ts)
  already checks exact readback and uploaded contents. The
  [CUA coverage gate](../../scripts/_cua_coverage.py) checks tool use and fixture
  ground truth. Retain these as deterministic assertions.

### Candidate uses

| Priority | Candidate | Example question | Potential benefit |
| --- | --- | --- | --- |
| First | Semantic outcome checks | Does the observed state show a completed save, validation rejection, pending operation, or insufficient evidence? | Reusable checks across interfaces with different wording |
| Second | Ambiguous element ranking | Which observed control best matches “dismiss without saving”? | Avoid a reasoning round trip when exact selectors leave several candidates |
| Third | Test-judge assistance | Does this bounded recovery episode support the claimed outcome? | Cheap annotations or triage of larger test collections |
| Exploratory | Image or crop classification | Is this dialog an error, confirmation, or permission request? | Help when accessibility text is unavailable |

For element ranking, candidate IDs, labels, relevant parent context and the
user's intent are suitable inputs. Return an observed ID or abstain. Keep
exact role/name selectors first and resolve the selected reference through
the existing runtime immediately before acting. Jev supplies an interpretation;
the backend still supplies identity, geometry and supported actions.

The [performance judge](../../scripts/_agent_perf_judge.py) is an existing
model call worth comparing against. It currently grades tool selection,
recovery, efficiency and completion, and produces explanatory triage. Its
`condense_transcript()` removes successful result payloads and images. A new
semantic checker would need selected observed state restored to its input;
a successful tool status alone does not prove the intended UI result.

Narrow Jev annotations could assist that judge, but cannot replace its prose
or establish that its 0–100 score is equivalent. Counts of repeated calls,
missing operations and known error codes remain code-derived features.

## Conclusion

Test Jev first as an offline semantic checker with independent fixture truth.
Then evaluate whether it can resolve ambiguous element candidates cheaply
enough to reduce total agent work. The full performance judge and visual
grounding need separate evidence before any replacement is justified.

Do not insert an unconditional model call into every observation or action.
The useful outcome is fewer expensive decisions or broader reliable semantic
coverage, not merely a fast response from an additional model.

## Implications

The following experiments are proposed follow-up work. They do not change
runtime contracts, model profiles, release gates or default routing.

### Experiment 1: semantic outcome checks

Start with approximately 60–100 recorded fixture cases spanning saved,
rejected, pending and insufficient-evidence outcomes. Use loopback browser
fixtures first, then add native desktop and phone examples if the first
comparison is promising.

Each case should include the intended action, relevant before/after observed
UI state and fixture-owned ground truth. Include difficult negatives:
a click acknowledged without an application change, an old success banner,
field-level validation, contradictory text, and truncated or unavailable
accessibility data. Do not expose the hidden fixture outcome or label in
the model input.

Compare existing deterministic checks, Jev, and the current Luna-based
approach on the same visible evidence. Include rules plus Jev if the intended
deployment is a fallback after rules fail. Split by application/template
rather than random paraphrases, keeping unseen wording and layouts held out.
Freeze questions and thresholds before evaluating the holdout.

Report the confusion matrix, false-success rate, coverage, abstention,
transport failures, p50/p95 latency and cost. Never score missing or
unavailable evidence as an automatic success. Repeat boundary cases to
measure stability without counting repeated inputs as new independent cases.

### Experiment 2: ambiguous element ranking

Build snapshots with multiple plausible controls, duplicate labels, nested
contexts, disabled elements, no matching element and legitimate multi-match
requests. Use fixture-owned intended IDs as labels. Compare exact selectors,
the current agent, and a selector-to-Jev fallback using identical candidates.

Randomize candidate order, test larger lists, and keep a no-match/abstention
path. Evaluate unique-target and multi-target requests separately. A winning
choice alone does not show that it is safe to select that element.

Measure correct-target rate, wrong-target rate, abstention, candidate coverage,
extra observations and total task time. In later live fixture tests, retain
reference freshness and actionability checks and measure actual task success.
Proceed only if reduced agent work survives the classifier and fallback costs
without an unacceptable increase in wrong-target actions.

### Experiment 3: test-judge annotations

Replay bounded episodes from the CUA smoke suite with fixture results and
reviewed labels. Test small questions about outcome evidence and recovery
behavior, rather than asking Jev to recreate the entire four-part rubric.
Include selected successful tool outputs where they are necessary to judge
the outcome; record any truncation.

Compare Luna-only judging with Jev annotations plus escalation. Measure
missed substantive failures, false alarms, escalation rate, reviewer effort,
total latency and cost. Preserve the deterministic coverage verdict and the
full judge during this comparison. An annotation stage that leaves every
downstream judge call unchanged needs a demonstrated quality benefit to
justify its extra cost.

### Experiment 4: image feasibility

First verify the available endpoint, version, header and PNG/JPEG transport
using non-sensitive fixture screenshots. Then compare text-only,
image-only and combined evidence on labeled dialogs or candidate crops.
Include themes, scale changes, icon-only controls, small text and missing
targets. Measure image conversion/upload time as well as inference time.

Keep Jev 1.13 classification separate from a possible 1.14 box experiment.
For boxes, assess target correctness and click success as well as overlap;
a plausible rectangle is not sufficient proof of an actionable control.

### Shared evidence and adoption decision

Use the existing Python harness conventions and
[`scripts/model_profiles.yaml`](../../scripts/model_profiles.yaml) for the
baseline. Use the already implemented Python helper at
`/home/bex/.agents/research/jev-client/jev.py` for Jev calls in these experiments;
usage and benchmark details are in the adjacent `README.md`. Import and reuse
`JevClient` across cases to retain its HTTPS connection, and record
`last_elapsed_ms` alongside the complete response, including probabilities,
token usage and provider-reported cost. The helper calls OpenCode's System One
endpoint directly using `OPENCODE_API_KEY`, with `OPENCODE_ZEN_API_KEY` as a
fallback, and defaults to `jev-1.13`.

For single-request smoke checks, run
`python3 /home/bex/.agents/research/jev-client/jev.py request.json --timing`.
The current helper has fixed request headers; the image experiment will need
a small extension for the required experimental images header. Reuse this
client rather than launching OpenCode or Pi per case or adding a provider
dependency to the core Rust runtime.

Retain exact model IDs, prompt/question versions, input provenance, split
membership, raw answers, labels, timing and token usage. Record provider-billed
cost separately from token-based estimates. Treat API errors separately from
semantic abstention. Total workflow measurements must include classifier,
preprocessing, escalation and any repeated observations or actions.

Before each experiment, set an acceptable error/coverage target and a useful
time or cost improvement against its measured baseline. Decide from holdouts
whether to adopt, narrow the scope or stop. Promising offline results justify
an optional experiment on controlled live fixtures; they do not establish
production UI reliability. Any adopted feature needs its own implementation
plan, feature documentation and normal sky-cua verification.
