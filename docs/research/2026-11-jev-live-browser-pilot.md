# Jev live browser target-selection pilot

## Context

On 2026-11-04, test whether Jev can save time selecting semantic targets beyond
exact deterministic rules while preserving correct browser actions. Earlier
[synthetic results](2026-11-jev-ranking-thresholds-and-image-fix.md) showed limited
single-case coverage, so this experiment measures actual browser interactions.

## Investigation

Twenty authored HTML fixtures cover editing, notification setup, record management,
and file operations: four exact targets, twelve semantic targets, and four cases
requiring abstention. Each page has four buttons with visible group context.
Cases, instructions, models, threshold, and acceptance gate were frozen and hashed
before inference in `artifacts/jev-browser-pilot/frozen.json` and `frozen.sha256`.

Each case ran on fresh pages in Chrome through the browser tool, alternating route
order. Baseline: exact rules then `gpt-5.6-luna`, medium. Hybrid: exact rules then
`jev-1.13`; accept a known non-abstain choice with selected probability >=0.7,
otherwise actually call Luna. Exact rules only match an explicitly named unique,
enabled, visible button; these results do not compare against every possible
deterministic heuristic. Each model receives one live DOM observation per request.

Candidate IDs are opaque and change with each page run. Inputs include visible
labels, roles, bounds, enabled state, and fieldset context. Private expected keys
are excluded. Before clicking, the driver re-observes and checks current ID,
label, group, role, enabled/visible state, and locator uniqueness. Browser
actionability checks remain active. A post-click observation and independent
server-owned action log establish which button was clicked. This proves target
selection, not that an actual document was saved or a download completed.

| Measurement | Rules then Luna | Rules then Jev then Luna |
| --- | ---: | ---: |
| Correct exact targets | 4/4 | 4/4 |
| Correct semantic targets | 12/12 | 12/12 |
| Correct no-target abstentions | 4/4 | 4/4 |
| Wrong clicks | 0 | 0 |
| Luna calls | 16 | 4 |
| Semantic total wall time | 89.853 s | 15.800 s |
| Semantic median wall time | 4.849 s | 0.916 s |

All twelve semantic cases were accepted by Jev and faster through the hybrid route.
Total semantic wall time fell **82.4%**; excluding the first cold pair still gives
**81.0%**. Timing includes navigation, DOM observation, model decision, actual
fallback when needed, live validation, click, post-action observation, and oracle
readback. Across nineteen completely timed pairs, including exact and negative
cases, total time fell from 103.625 s to 32.777 s (**68.4%**).

One baseline negative trial hit the browser tool's 60-second outer limit during a
multi-case call. Its saved Luna response was `abstain`; the original fixture run's
independent state confirmed no action. Correctness was recovered, but its full
wall time was unavailable. Both routes for that case are excluded from paired
timing. The remaining cases ran one pair per tool call without repeated inference.
There are 39 uninterrupted trials and one recovered abstention, not 40 uninterrupted
trials. Resume bookkeeping was added after the interruption; inputs and threshold
were unchanged.

All four negative cases made actual Jev-to-Luna fallback calls. The three fully
timed negative pairs took 11.108 s baseline versus 14.307 s hybrid, so the semantic
speedup does not mean fallback is free. Jev's sixteen calls had a median provider
round trip of 331 ms and reported total cost of **$0.00085505**. Luna billing was
not available; its measured time includes the existing Codex CLI harness and
context overhead, not a matched direct-API comparison.

All accepted target probabilities were >=0.91: replaying these responses at 0.8
or 0.9 would preserve the twelve selections. This dataset therefore does not
establish 0.7 as an optimal threshold or probability calibration. Unlike the
earlier synthetic experiment, criteria include rich descriptions and only four
candidates. Improvement cannot be attributed solely to browser execution.

Separate live probes rejected a stale reference after page navigation and a
disabled target, with no action recorded. Offline tests cover fixture/oracle
separation, rule uniqueness/actionability, threshold boundaries, abstain and
unknown-target fallback, model bypass, and duplicate inference rejection.

Evidence and implementation:

- `scripts/_jev_browser_fixture.py`: fixture pages and independent action log.
- `scripts/jev_browser_pilot.py`: freeze, decision routes, provider evidence.
- `scripts/jev_browser_pilot.mjs`: live observation, validation, actions, timing.
- `artifacts/jev-browser-pilot/browser-records.json`: individual route evidence.
- `artifacts/jev-browser-pilot/summary.json`: aggregate measurements.
- `artifacts/jev-browser-pilot/decisions/`: inputs, raw responses, Luna transcripts.
- `artifacts/jev-browser-pilot/validation-probes.json`: stale/disabled checks.
- `artifacts/jev-browser-pilot/browser-abstention-proof.png`: final no-target page.

Verification: root Python Ruff format/lint, basedpyright, and **967 pytest tests
passed**; `node --check scripts/jev_browser_pilot.mjs` passed. No Rust/runtime
changes were made. The loopback servers were stopped and temporary judge auth
removed after the run. No installation, deployment, or commit was performed.

## Conclusion

The predeclared gate passed: zero wrong clicks, no correctness regression, and
at least 20% less semantic-case wall time. Jev has demonstrated useful incremental
coverage beyond the tested exact rules in this bounded browser setting. This
strengthens the case for an optional semantic target-ranking fallback while
retaining deterministic validation and Luna fallback.

This is a small constructed corpus of single-action pages with unusually clear
group descriptions. It does not prove reliable behavior on dense third-party
pages, native accessibility trees, phone screens, screenshot grounding, or
multi-step task completion. Chrome tool validation here is not the production
Rust resolver or installed sky-cua agent-loop acceptance. Earlier confident
stale-banner false successes remain evidence against using Jev as a completion
judge.

## Implications

Keep the pilot as experimental evidence rather than enabling a production route.
The next useful test is the same frozen selection contract through sky-cua's
actual browser observation/reference/action path on representative complex pages.
For that next test, prefer 0.9 as a conservative starting policy: it retains all
observed acceptances while declining the untested 0.7–0.9 band. This is not evidence
that 0.9 is calibrated or optimal; keep this run's frozen 0.7 results unchanged.
Preserve the existing exact-selector fast path, current-reference validation,
abstention/fallback, and independent outcome verification. Native and phone use
need their own evidence; browser results alone do not justify enabling them.
