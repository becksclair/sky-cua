# Jev follow-up: ranking thresholds and image rejection

## Context

Date: 2026-11-04. Follow-up to the [initial feasibility experiment](2026-11-jev-ui-experiment-results.md).
Bex requested re-scoring saved element-ranking responses at 0.7 and 0.8 and
investigation of the image HTTP 400. No new ranking inference was performed.
The initial frozen experiment and its 0.9 results remain intact.

**Conclusion:** lower thresholds recover some correct ranking coverage beyond
rules, but the single-case benefit remains modest. The image rejection was a
fixture-size error, now fixed and verified through the same Jev 1.13 endpoint.
The earlier image transport blocker is resolved; production adoption remains
unsupported by these results.

## Investigation

### Post-hoc ranking sensitivity

Thresholds apply to the probability assigned to the selected choice, matching
the original gate, not Jev's separate `confidence` field. Rules remain first:
retain exact matches and use Jev only for unresolved candidates. The held-out
unique-target set has six exact targets and six semantic targets. Rules solve
all six exact targets; the family batches additionally contain six legitimate
no-target/multi-target abstentions.

| Presentation | Threshold | Additional semantic targets / 6 | Rules + Jev targets / 12 | Wrong targets observed |
| --- | ---: | ---: | ---: | ---: |
| Family batches | 0.7 | 3 | 9 | 0 |
| Family batches | 0.8 | 3 | 9 | 0 |
| Single-case replay | 0.7 | 1 | 7 | 0 |
| Single-case replay | 0.8 | 0 | 6 | 0 |

The family-batch gains are `ranking-04-02`, `ranking-05-02`, and `ranking-06-02`.
The sole single-case gain at 0.7 is `ranking-06-02`. Across every saved ranking
response—including development cases and repeated boundary cases—neither
threshold admitted an incorrect target. Repetitions are not independent safety
examples. At 0.7 the repeated mixed-batch `ranking-04-02` answers are accepted
(probabilities 0.76 and 0.71); at 0.8 they are not. Its standalone boundary replay
still abstains at both thresholds.

For the six single-case semantic opportunities, summing the measured Jev calls
and necessary Luna fallbacks gives:

| Route | Reconstructed decision time |
| --- | ---: |
| Luna alone | 22.310 s |
| Jev 0.7 then Luna fallback | 20.282 s, 9.1% less |
| Jev 0.8 then Luna fallback | 24.146 s, 8.2% more |

Thus 0.7 has a small observed incremental benefit in the presentation closest to
an individual agent decision. Neither result meets the initial 20% time-saving
gate, and accepted semantic coverage at 0.7 is only 1/6. Batch results alone would
overstate that benefit. This is post-hoc sensitivity analysis on the same
synthetic examples, not newly validated calibration or live workflow timing.
Zero observed wrong targets does not establish a reliable wrong-target rate.

Re-scoring code and per-case probabilities are retained in
[`artifacts/jev-ui/rescore-ranking.py`](../../artifacts/jev-ui/rescore-ranking.py)
and [`ranking-rescore.json`](../../artifacts/jev-ui/ranking-rescore.json).

### Image HTTP 400: exact cause and verified fix

Replaying the original payload and inspecting the provider's sanitized error body
revealed:

```json
{"detail":{"error_type":"api_usage_error","message":"Invalid image dimensions."}}
```

The original PNG was **320 × 240 = 76,800 pixels**. Current
[official limits](https://docs.typesafe.ai/images#limits) require width and height
between **280 and 1568 pixels**, and total area between 78,400 and 2,458,624 pixels.
The fixture violated both minimum height and minimum area. The API rejects an
out-of-range image rather than resizing it. This was not a Jev-version problem
or an unsupported image route.

The fix pads 20 white pixels above and below the original image, producing
**320 × 280**, with the original pixels preserved. Model `jev-1.13`, endpoint,
experimental header, asset format, questions and reference remain unchanged.

| Fixture | HTTP | Red answer | Blue answer | Diagnostic request time |
| --- | ---: | ---: | ---: | ---: |
| Padded red | 200 | 1 | 0 | 444 ms |
| Padded blue | 200 | 0 | 1 | 540 ms |

Both responses name `jev-1.13`. Each reports 541 input tokens, 36 output tokens,
and $0.00002272, for $0.00004544 total. These two fresh-connection diagnostic
samples are not a latency benchmark. The opposite answers establish that the
model distinguished the control images; they do not establish UI-dialog,
small-text, icon or screenshot accuracy.

The original helper deliberately hid error bodies. This investigation inspected
the diagnostic body without changing that helper's default behavior. A separate
local DNS failure initially prevented replay; request-local DNS-over-HTTPS
resolution restored transport while preserving the original HTTPS hostname,
SNI and certificate verification. No host DNS setting changed. That temporary
DNS problem did not cause the original HTTP 400.

Evidence is in [`artifacts/jev-ui/image-diagnosis/`](../../artifacts/jev-ui/image-diagnosis/):
`original-response.json`, `vendor-images.md`, `padded-results.json`, exact request
and response files, PNGs and `verification.json`. Verification checks preservation
of original pixels and that only image bytes changed in successful requests.

## Conclusion

- **Ranking:** 0.7 is worth considering in a further bounded experiment; it adds
  one correct semantic decision out of six individually presented cases without
  observed wrong targets. The evidence is too small and context-sensitive to
  recommend production adoption. 0.8 offers no single-case incremental coverage.
- **Images:** the transport blocker is resolved. No provider switch, model
  upgrade or credential rotation is required. Pad or scale undersized inputs,
  constrain both dimensions to the documented range, and convert WebP to PNG/JPEG
  before sending. Actual UI-image classification is now feasible to investigate.

## Implications

[`scripts/jev_ui_image_probe.py`](../../scripts/jev_ui_image_probe.py) now uses the
verified padding and supports `--out` so a fresh run preserves original evidence:

```sh
uv run python scripts/jev_ui_image_probe.py --out artifacts/jev-ui/images-valid
```

This command makes paid API calls and requires the existing configured key.
A focused mocked regression test decodes both emitted PNGs, checks the size
bounds, and verifies that their original image regions are unchanged. No
production runtime or routing configuration changed. The original outcome
false-success finding is unaffected by these ranking thresholds or image fix.

Validation: repository Python formatting, Ruff and basedpyright passed; **941
pytest tests passed** (including the image-dimensions regression). Live diagnostic
red/blue requests both succeeded. No Rust or production runtime changed.
