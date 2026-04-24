# Model Screenshot Size Performance

## Decision

The default model-facing screenshot cap is `1440x900`.

On a `2560x1440` desktop this produces an aspect-preserved `1440x810` JPEG for model inspection while keeping the raw `2560x1440` PNG beside it. Action coordinates remain in the model-facing screenshot coordinate space and are mapped back to the underlying desktop or stream coordinates by the backend.

The raw capture contract is unchanged:

- `capture.screenshot_path` points to the bounded JPEG sent to the model.
- `capture.pixel_size` describes that bounded JPEG.
- `capture.original_screenshot_path` points to the raw capture.
- `capture.original_pixel_size` describes the raw capture.

## Why 1440x900

The TIDAL rich app-server workflow is a good stress case because TIDAL on KDE Wayland exposes only fallback geometric anchors, so the model has to rely heavily on repeated screenshots. The workflow creates or finds `Codex Favorites`, adds exactly five tracks, and verifies the final playlist from a fresh plugin screenshot.

After deleting `Codex Favorites` between runs, the full-flow A/B results were:

| Cap | Artifact | Result | Elapsed | MCP calls | Image views | Total tokens | Avg uncached input | Max uncached input |
| --- | --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| `1920x1080` | `artifacts/codex-e2e/tidal-playlist-app-server/20260424T041253Z/` | completed | `287.24s` | `42` | `21` | `5.17M` | `8.6k` | `60.6k` |
| `1440x900` | `artifacts/codex-e2e/tidal-playlist-app-server/20260424T042117Z/` | completed | `246.97s` | `37` | `19` | `3.72M` | `5.0k` | `26.9k` |

The smaller cap was `40.27s` faster, used `1.44M` fewer total tokens, and still completed the full playlist creation/add/verification workflow. The final proof image for the smaller-cap run was `1440x810`, with the raw `2560x1440` capture preserved.

The plugin backend was not the bottleneck in either run. MCP tool time stayed under 9 seconds in both cases. The win came from reducing model/image-loop cost.

## Override

For an A/B run or an app that needs more visual detail, override the cap through the plugin MCP environment:

```bash
SKY_CUA_MODEL_SCREENSHOT_MAX_WIDTH=1920 \
SKY_CUA_MODEL_SCREENSHOT_MAX_HEIGHT=1080 \
python3 scripts/live_app_server_tidal_playlist.py
```

The installed plugin receives these variables because `.mcp.json` includes them in `env_vars`. Invalid values and values outside the safe range fall back to the compiled default.

## Validation

Relevant checks:

```bash
cargo test -p sky-cua-linux portal::screenshot
python3 scripts/build_plugin.py
```

For workflow-level performance, compare `timing-summary.json` fields:

- `elapsed_ms`
- `completed_mcp_tool_calls`
- `item_completed_counts.imageView`
- `last_token_usage.total.totalTokens`
- `avg_uncached_input_tokens`
- `max_uncached_input_tokens`
