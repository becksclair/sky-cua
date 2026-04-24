# Model Screenshot Size Performance

## Decision

The default model-facing screenshot cap is `1440x900`, encoded as JPEG quality `85`.

On a `2560x1440` desktop this produces an aspect-preserved `1440x810` JPEG for model inspection while keeping the raw `2560x1440` PNG beside it. Action coordinates remain in the model-facing screenshot coordinate space and are mapped back to the underlying desktop or stream coordinates by the backend.

The raw capture contract is unchanged:

- `capture.screenshot_path` points to the bounded model image.
- `capture.pixel_size` describes that bounded model image.
- `capture.original_screenshot_path` points to the raw capture.
- `capture.original_pixel_size` describes the raw capture.
- `capture.model_image_format`, `capture.model_image_quality`,
  `capture.model_image_bytes`, and `capture.model_image_encode_ms` describe the
  model-image encoding used for the snapshot.

## Why 1440x900

The TIDAL rich app-server workflow is a good stress case because TIDAL on KDE Wayland exposes only fallback geometric anchors, so the model has to rely heavily on repeated screenshots. The workflow creates or finds `Codex Favorites`, adds exactly five tracks, and verifies the final playlist from a fresh plugin screenshot.

After deleting `Codex Favorites` between runs, the full-flow A/B results were:

| Cap | Artifact | Result | Elapsed | MCP calls | Image views | Total tokens | Avg uncached input | Max uncached input |
| --- | --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| `1920x1080` | `artifacts/codex-e2e/tidal-playlist-app-server/20260424T041253Z/` | completed | `287.24s` | `42` | `21` | `5.17M` | `8.6k` | `60.6k` |
| `1440x900` | `artifacts/codex-e2e/tidal-playlist-app-server/20260424T042117Z/` | completed | `246.97s` | `37` | `19` | `3.72M` | `5.0k` | `26.9k` |

The smaller cap was `40.27s` faster, used `1.44M` fewer total tokens, and still completed the full playlist creation/add/verification workflow. The final proof image for the smaller-cap run was `1440x810`, with the raw `2560x1440` capture preserved.

The plugin backend was not the bottleneck in either run. MCP tool time stayed under 9 seconds in both cases. The win came from reducing model/image-loop cost.

## Format and Size Overrides

For an A/B run or an app that needs more visual detail, override the cap through the plugin MCP environment:

```bash
SKY_CUA_MODEL_SCREENSHOT_MAX_WIDTH=1920 \
SKY_CUA_MODEL_SCREENSHOT_MAX_HEIGHT=1080 \
python3 scripts/live_app_server_tidal_playlist.py
```

The installed plugin receives these variables because `.mcp.json` includes them in `env_vars`. Invalid values and values outside the safe range fall back to the compiled default.

JPEG remains the default because it is the safest cross-host screenshot format
and produced the proven TIDAL win above. WebP is available for profiling:

```bash
SKY_CUA_MODEL_SCREENSHOT_FORMAT=webp \
SKY_CUA_MODEL_SCREENSHOT_WEBP_QUALITY=85 \
python3 scripts/live_app_server_tidal_playlist.py
```

JPEG quality can also be varied:

```bash
SKY_CUA_MODEL_SCREENSHOT_FORMAT=jpeg \
SKY_CUA_MODEL_SCREENSHOT_JPEG_QUALITY=75 \
python3 scripts/live_app_server_tidal_playlist.py
```

The WebP path uses lossy WebP encoding so quality-level A/B runs are meaningful.
The model still receives a real image input once the screenshot path is inspected;
the file format only changes local encoding, transport size, and decode/ingest
behavior.

Use the bundled A/B runner when a multi-run comparison is desired. By default it
runs the cheaper `jpeg-q85` and `webp-q85` pair, and gives each variant an
isolated playlist name so sequential runs do not reuse or mutate the same
playlist state:

```bash
python3 scripts/live_app_server_tidal_image_ab.py
```

Use `--all-variants` for the full `jpeg-q85`, `webp-q80`, `webp-q85`, and
`webp-q95` sweep. The runner writes
`artifacts/codex-e2e/tidal-image-ab-summary.json` with elapsed time, MCP calls,
uncached input-token metrics, final screenshot file size, status, playlist name,
and artifact paths.

## Validation

Relevant checks:

```bash
cargo test -p sky-cua-linux portal::screenshot
python3 scripts/live_app_server_tidal_image_ab.py --variants jpeg-q85 webp-q85
python3 scripts/build_plugin.py
```

For workflow-level performance, compare `timing-summary.json` fields:

- `elapsed_ms`
- `completed_mcp_tool_calls`
- `item_completed_counts.imageView`
- `last_token_usage.total.totalTokens`
- `avg_uncached_input_tokens`
- `max_uncached_input_tokens`
