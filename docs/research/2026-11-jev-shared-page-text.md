# Shared page text in browser ranking payloads

## Context

On 2026-11-05, a review of the optional [browser ranking](../features/browser-ranking.md)
route found that every candidate's `context` embedded the full observed page
text (up to 4,000 characters), and each candidate is serialized twice (in
`state.candidates` and in the choice `criteria`). Payload size therefore grew
as roughly 2 × candidates × page text. The [real TodoMVC browser workflow](2026-11-jev-real-browser-workflow.md)
that justified the 0.8 Jev threshold used this same layout, so changing it
required re-measurement rather than a review-only fix.

## Change

Page text moves once into `state.page_text`. Candidate `context` keeps only
candidate-specific facts: `href: <destination>` for links, otherwise empty.
Instructions, criteria, IDs, threshold and fallback are unchanged, so the
layout is the only varied input. The change is identical in the Rust route
(`mcp_tools/browser_ranking.rs`) and the research harness
(`_jev_browser_workflow.py`, `_jev_desktop_models.py`).

## Investigation

### Payload size

Offline, using the same payload builder with 4,000 characters of page text:

| Candidates | Per-candidate layout | Shared layout | Reduction |
| ---: | ---: | ---: | ---: |
| 9 | 76 KB | 8 KB | 89.4% |
| 50 | 421 KB | 24 KB | 94.3% |
| 300 | 2.53 MB | 123 KB | 95.1% |

TodoMVC's page text is short (about 150 characters), so its saving is smaller:
mean request size fell from 7.1 KB to 4.3 KB (−39%), and Jev input tokens from
3,043 to 2,376 (−22%).

### Controlled A/B replay

`scripts/jev_page_text_replay.py` replayed the 18 measured, oracle-verified
observations from the original pilot (three stages × six workflows) through
Jev in both layouts, three repeats each, alternating which layout ran first.
The frozen gate required zero wrong accepted selections in the shared layout
and accepted-correct coverage within one of the legacy layout.

| 54 Jev calls per layout | Per-candidate | Shared |
| --- | ---: | ---: |
| Accepted at ≥0.8 | 36 | 36 |
| Accepted and correct | 36 | 36 |
| Wrong accepted | 0 | 0 |
| Raw choice correct | 36 | 45 |
| Mean input tokens | 3,043 | 2,376 |
| Median Jev latency | 241 ms | 239 ms |

The add field (p 0.99–1.0) and Active filter (p 0.96–0.98) stayed confidently
correct. The unnamed individual checkbox remains Jev's known weakness, and the
shared layout made it *less* confident rather than more: the per-candidate
layout chose the wrong unnamed input every time at p 0.49–0.73, while the
shared layout split between the correct and wrong input at p 0.37–0.48. Both
stay below 0.8 and fall back to Terra; the shared layout widens the margin
below the threshold. The gate passed. The 0.8 threshold remains appropriate
on this evidence.

### Paired workflow re-run

`scripts/jev_real_browser.py` re-ran the unchanged protocol (one warm-up per
route, three alternating measured pairs, Terra `gpt-6.1-terra` medium) with
the shared layout. A replicate was declared after the first run, before its
outcome was known; both are reported.

| Measurement | Original | Shared, run 1 | Shared, replicate |
| --- | ---: | ---: | ---: |
| Correct workflows | 6/6 | 6/6 | 6/6 |
| Wrong accepted targets | 0 | 0 | 0 |
| Terra requests (baseline → hybrid) | 9 → 3 | 9 → 3 | 9 → 3 |
| Total saving | 26.7% | 14.2% | **25.2%** |
| Pair savings | 22.9 / 26.8 / 30.0% | 36.3 / −6.3 / 13.9% | 23.0 / 27.3 / 25.3% |
| Non-model time per workflow | 13.1–13.3 s | 4.9–19.4 s | 13.0–13.2 s |

Routing was identical in every run: Jev accepted the add field and Active
filter, and rejected every checkbox proposal. Model-time savings were stable
(−60% original, −56% run 1). Run 1 missed the 20% wall-time gate because of
non-model variance: in two hybrid workflows the fresh observation after a
fast Jev answer waited about 6.2 s inside sky-cua's observation path before
capture, and one baseline Terra call took 10.9 s. The payload does not affect
observation timing. The replicate's non-model time matched the original and
it passed the gate.

Follow-up investigation identified the mechanism, though not this run's exact
trigger. `Page.captureScreenshot` waits for a compositor frame; when a window
produces none, the capture ran to the bridge's ~6 s CDP timeout, and
wedged-session recovery then re-attached and replayed it in ~0.2 s. The daemon
log records each instance as a recovered wedged debugger session on
`Page.captureScreenshot`. It is reproducible on KWin 6.8 when Chromium
minimizes its own window, and occurs only transiently right after a
compositor-initiated minimize. Windows fully covered by other windows kept
rendering in direct tests, so the earlier explanation that pilot windows
opening behind others caused it is not supported. It is independent of the
ranking payload.

### Real Rust route

`scripts/live_browser_ranking_smoke.py` ran freshly built debug client and
service binaries through the real extension and native host. It passed with
the same sources as the earlier accepted run: two deterministic selections,
Jev at p 1.0 and 0.99, Terra selecting the checkbox after Jev abstained, and
the absent-target request unresolved after both models abstained. The final
DOM task proof matched.

All live runs used `SKY_CUA_ALLOW_STALE_DEPLOY=1` for their own process only.
The installed runtime predates the working tree, but the paired pilot's
varied input is Python-side, and the Rust smoke used fresh binaries.

## Conclusion

Sending page text once preserves gated accuracy on the evidence that
calibrated the route: identical acceptance and zero wrong accepted selections
in a controlled A/B, identical routing in two paired workflows, and a passing
Rust smoke. It removes 89–95% of the payload on pages with long text and many
candidates, where the old layout could reach megabytes. The 0.8 threshold
stays. Wall-time savings are within the original range in the replicate; the
one miss traces to stalled screenshot captures, not ranking.

## Implications

- Ship the shared layout in the Rust route and keep the harness aligned.
- The checkbox weakness is unchanged in kind; the confidence gate and Terra
  fallback remain essential.
- Wall-time pilots should check the daemon log for recovered
  `Page.captureScreenshot` sessions, which can dominate a run. A 1 s
  screenshot cap shortens each to about 1.2–2.0 s; it does not remove it.
- This is still a small application with short page text. It does not show
  ranking quality on long, link-heavy pages, only that their payloads are
  now bounded.

Evidence under `artifacts/jev-page-text/`: `replay/` (frozen protocol, records,
summary), `pilot/` and `pilot-replicate/` (paired workflows), `rust-smoke/`.
