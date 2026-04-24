#!/usr/bin/env python3
from __future__ import annotations

import argparse

from _codex_exec import (
    DESKTOP_E2E_EXEC_ARGS,
    make_artifact_dir,
    prepare_chatgpt_plugin_test_home,
    read_last_message,
    require_computer_use_tool_call,
    run_codex_exec,
    with_plugin_mention,
)
from _tidal_workflow import (
    TIDAL_RESULT_SCHEMA,
    TIDAL_WORKFLOW_MODEL,
    TIDAL_WORKFLOW_REASONING_EFFORT,
    tidal_playlist_prompt,
    validate_tidal_result,
)


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Run the Tidal playlist codex-exec workflow against the installed sky-cua plugin."
    )
    parser.add_argument(
        "--symlink",
        action="store_true",
        help="Symlink the built bundle into the dedicated test Codex home.",
    )
    parser.add_argument(
        "--model",
        default=TIDAL_WORKFLOW_MODEL,
        help=f"Codex model to use (default: {TIDAL_WORKFLOW_MODEL}).",
    )
    args = parser.parse_args()

    artifact_dir = make_artifact_dir("tidal-playlist")
    codex_home = prepare_chatgpt_plugin_test_home(
        artifact_dir=artifact_dir,
        symlink=args.symlink,
    )

    prompt = with_plugin_mention(tidal_playlist_prompt(app_server=False))
    result = run_codex_exec(
        prompt=prompt,
        artifact_dir=artifact_dir,
        output_schema=TIDAL_RESULT_SCHEMA,
        model=args.model,
        reasoning_effort=TIDAL_WORKFLOW_REASONING_EFFORT,
        extra_env={"CODEX_HOME": str(codex_home)},
        extra_args=DESKTOP_E2E_EXEC_ARGS,
    )
    if result.exit_code != 0:
        raise SystemExit(f"codex exec exited with {result.exit_code}; inspect {artifact_dir}")
    require_computer_use_tool_call(
        result.transcript_path,
        artifact_dir=result.artifact_dir,
        model=args.model,
    )
    message = read_last_message(result.last_message_path)
    validate_tidal_result(message, artifact_dir=artifact_dir)
    print(f"tidal workflow passed: {artifact_dir}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
