#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json

from _app_server_harness import (
    require_computer_use_item,
    run_rich_app_server_turn,
    with_plugin_mention,
)
from _codex_exec import make_artifact_dir
from _tidal_workflow import (
    TIDAL_APP_SERVER_TIMEOUT_SECONDS,
    TIDAL_RESULT_SCHEMA,
    TIDAL_WORKFLOW_MODEL,
    TIDAL_WORKFLOW_REASONING_EFFORT,
    tidal_playlist_prompt,
    validate_tidal_result,
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
    args = parser.parse_args()

    artifact_dir = make_artifact_dir("tidal-playlist-app-server")
    prompt = with_plugin_mention(tidal_playlist_prompt(app_server=True))
    try:
        result = run_rich_app_server_turn(
            prompt=prompt,
            artifact_dir=artifact_dir,
            output_schema=TIDAL_RESULT_SCHEMA,
            model=args.model,
            reasoning_effort=TIDAL_WORKFLOW_REASONING_EFFORT,
            max_turn_seconds=TIDAL_APP_SERVER_TIMEOUT_SECONDS,
        )
    except TimeoutError as exc:
        raise SystemExit(str(exc)) from exc
    require_computer_use_item(result.transcript_path)
    message = json.loads(result.last_message_path.read_text())
    validate_tidal_result(message, artifact_dir=artifact_dir, require_screenshot=True)
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
