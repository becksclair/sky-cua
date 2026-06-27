# Model Screenshot Size Performance

## Decision

The default model-facing screenshot cap is `1440x900`, encoded as lossy WebP quality `85`. Captures are only ever downscaled to fit the cap: a capture already within `1440x900` is passed through at its native size rather than upscaled to fill the box.

On a `2560x1440` desktop this produces an aspect-preserved `1440x810` WebP for model inspection while keeping the raw `2560x1440` PNG beside it. Action coordinates remain in the model-facing screenshot coordinate space and are mapped back to the underlying desktop or stream coordinates by the backend.

This preparation is shared by the Linux portal backend and the Windows GDI backend through the `sky-cua-capture` crate, so the bounded model image — cap, no-upscale rule, default format, and the `logical_to_pixel_scale` coordinate derivation — is identical regardless of platform.

This document covers desktop captures only. Browser (CDP) screenshots are prepared by a separate path in `sky-cua-service` (`browser/model_image.rs`); it reads the same `SKY_CUA_MODEL_SCREENSHOT_FORMAT`/quality variables and shares the WebP default, but unlike the desktop path it intentionally upscales to preserve the CSS-pixel pointer coordinate space.

The raw capture contract is unchanged:

- `capture.inspection_image_path` points to the bounded model image agents
  should inspect.
- `capture.pixel_size` describes that bounded model image.
- `capture.original_pixel_size` describes the raw capture.
- `capture.images[]` labels the inspection image with role, scope, and
  recommended use.
- `capture.model_image_format`, `capture.model_image_quality`,
  `capture.model_image_bytes`, and `capture.model_image_encode_ms` describe the
  model-image encoding used for the snapshot.

## Why 1440x900

The historical TIDAL rich app-server workflow was a good stress case because
TIDAL on KDE Wayland exposed only fallback geometric anchors, so the model had
to rely heavily on repeated screenshots. That live workflow has been retired,
but the measured artifacts remain the evidence behind the default cap.

After deleting `Codex Favorites` between runs, the full-flow A/B results were:

| Cap | Artifact | Result | Elapsed | MCP calls | Image views | Total tokens | Avg uncached input | Max uncached input |
| --- | --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| `1920x1080` | `artifacts/codex-e2e/tidal-playlist-app-server/20260424T041253Z/` | completed | `287.24s` | `42` | `21` | `5.17M` | `8.6k` | `60.6k` |
| `1440x900` | `artifacts/codex-e2e/tidal-playlist-app-server/20260424T042117Z/` | completed | `246.97s` | `37` | `19` | `3.72M` | `5.0k` | `26.9k` |

The smaller cap was `40.27s` faster, used `1.44M` fewer total tokens, and still completed the full playlist creation/add/verification workflow. The final proof image for the smaller-cap run was `1440x810`, with the raw `2560x1440` capture preserved.

The plugin backend was not the bottleneck in either run. MCP tool time stayed under 9 seconds in both cases. The win came from reducing model/image-loop cost.

## Format and Size Overrides

For an active smoke sanity check with more visual detail, override the cap
through the plugin MCP environment when running an installed-agent entrypoint:

```bash
SKY_CUA_MODEL_SCREENSHOT_MAX_WIDTH=1920 \
SKY_CUA_MODEL_SCREENSHOT_MAX_HEIGHT=1080 \
python3 scripts/live_agentic_loop_smoke.py
```

The installed plugin receives these variables because `.mcp.json` includes them in `env_vars`. Invalid values and values outside the safe range fall back to the compiled default.

WebP is the default: at equal quality it encodes meaningfully smaller than JPEG,
which trims transport size and upload latency without changing the bounded pixel
dimensions the model sees. Both Claude and GPT vision accept WebP. The WebP path
uses lossy WebP encoding, so the quality setting is meaningful:

```bash
export SKY_CUA_MODEL_SCREENSHOT_WEBP_QUALITY=85
```

JPEG remains available as the most universally decodable format and can be
selected per host, with its own quality knob:

```bash
SKY_CUA_MODEL_SCREENSHOT_FORMAT=jpeg \
SKY_CUA_MODEL_SCREENSHOT_JPEG_QUALITY=85 \
python3 scripts/live_agentic_loop_smoke.py
```

The model still receives a real image input once the screenshot path is
inspected; the file format only changes local encoding, transport size, and
decode/ingest behavior.

The former TIDAL A/B runner has been removed. For future multi-run comparisons,
add a new active workflow-specific runner with isolated state and the same
timing-summary fields below.

## Validation

Relevant checks:

```bash
cargo nextest run -p sky-cua-linux portal::screenshot
python3 scripts/build_plugin.py
```

The Rust screenshot tests cover encoding behavior. `live_agentic_loop_smoke.py`
is installed-MCP tool-use acceptance, not screenshot-format validation; do not
count WebP as live-smoked unless a direct capture smoke asserts returned
metadata such as `model_image_format`, quality, pixel size, and output file.
For workflow-level performance comparisons, use or add a workflow-specific
runner that emits `timing-summary.json` with:

- `elapsed_ms`
- `completed_mcp_tool_calls`
- `item_completed_counts.imageView`
- `last_token_usage.total.totalTokens`
- `avg_uncached_input_tokens`
- `max_uncached_input_tokens`
