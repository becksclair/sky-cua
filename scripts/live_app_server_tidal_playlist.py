#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json

from _tidal_workflow import (
    TIDAL_WORKFLOW_MODEL,
    TidalAppServerWorkflowFailure,
    run_tidal_app_server_workflow,
)


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Run the Tidal playlist workflow against the installed sky-cua plugin through codex app-server."
    )
    parser.add_argument(
        "--model",
        default=TIDAL_WORKFLOW_MODEL,
        help=f"Codex model to use (default: {TIDAL_WORKFLOW_MODEL}).",
    )
    parser.add_argument(
        "--image-format",
        choices=["jpeg", "webp"],
        help="Override SKY_CUA_MODEL_SCREENSHOT_FORMAT for this run.",
    )
    parser.add_argument(
        "--jpeg-quality",
        type=int,
        help="Override SKY_CUA_MODEL_SCREENSHOT_JPEG_QUALITY for this run.",
    )
    parser.add_argument(
        "--webp-quality",
        type=int,
        help="Override SKY_CUA_MODEL_SCREENSHOT_WEBP_QUALITY for this run.",
    )
    args = parser.parse_args()

    try:
        result, message = run_tidal_app_server_workflow(
            model=args.model,
            image_format=args.image_format,
            jpeg_quality=args.jpeg_quality,
            webp_quality=args.webp_quality,
        )
    except TidalAppServerWorkflowFailure as exc:
        raise SystemExit(str(exc)) from exc
    artifact_dir = result.artifact_dir
    print(f"tidal app-server workflow passed: {artifact_dir}")
    if result.timing_summary_path.exists():
        timing_summary = json.loads(result.timing_summary_path.read_text())
        print(
            "timing summary: "
            f"elapsed_ms={timing_summary.get('elapsed_ms')} "
            f"completed_mcp_tool_calls={timing_summary.get('completed_mcp_tool_calls')} "
            f"mcp_tool_duration_total_ms={timing_summary.get('mcp_tool_duration_total_ms')} "
            f"path={result.timing_summary_path}"
        )
    print(json.dumps(message, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
