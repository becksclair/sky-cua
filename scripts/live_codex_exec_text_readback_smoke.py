#!/usr/bin/env python3
from __future__ import annotations

import time

from _codex_exec import (
    DESKTOP_E2E_EXEC_ARGS,
    make_artifact_dir,
    prepare_chatgpt_plugin_test_home,
    read_last_message,
    require_computer_use_tool_call,
    run_codex_exec,
    transcript_mcp_tool_calls,
    with_plugin_mention,
)
from _plugin_bundle import REPO_ROOT
from _text_readback_smoke import (
    close_dialog_if_running,
    require_submitted_value,
    require_text_readback_message,
    require_transcript_readback_values,
    run_zenity_readback_dialog,
    text_readback_prompt,
)


def main() -> int:
    artifact_dir = make_artifact_dir("codex-text-readback-smoke")
    codex_home = prepare_chatgpt_plugin_test_home(
        artifact_dir=artifact_dir,
        symlink=False,
    )
    dialog = run_zenity_readback_dialog()
    try:
        result = run_codex_exec(
            prompt=with_plugin_mention(text_readback_prompt(), codex_home),
            artifact_dir=artifact_dir,
            output_schema=REPO_ROOT / "scripts" / "schemas" / "text_readback_smoke_result.json",
            extra_env={"CODEX_HOME": str(codex_home)},
            extra_args=DESKTOP_E2E_EXEC_ARGS,
        )
        if result.exit_code != 0:
            raise SystemExit(f"codex exec exited with {result.exit_code}; inspect {artifact_dir}")
        require_computer_use_tool_call(result.transcript_path, artifact_dir=result.artifact_dir)
        require_transcript_readback_values(
            transcript_mcp_tool_calls(result.transcript_path),
            artifact_dir,
        )
        message = read_last_message(result.last_message_path)
        require_text_readback_message(message, artifact_dir)
        time.sleep(1)
        require_submitted_value(dialog, artifact_dir)
        print(f"codex text readback smoke passed: {artifact_dir}")
        return 0
    finally:
        close_dialog_if_running(dialog)


if __name__ == "__main__":
    raise SystemExit(main())
