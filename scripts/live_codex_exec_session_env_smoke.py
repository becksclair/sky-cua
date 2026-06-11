#!/usr/bin/env python3
from __future__ import annotations

import json
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
from _session_env_smoke import (
    SUBMITTED_VALUE,
    close_dialog_if_running,
    import_current_env_to_systemd,
    require_session_env_transcript,
    require_submitted_value,
    run_session_env_dialog,
    session_env_prompt,
    stripped_desktop_env,
)


def main() -> int:
    artifact_dir = make_artifact_dir("codex-session-env-smoke")
    codex_home = prepare_chatgpt_plugin_test_home(artifact_dir=artifact_dir, symlink=False)
    import_current_env_to_systemd()
    dialog = run_session_env_dialog()
    try:
        result = run_codex_exec(
            prompt=with_plugin_mention(session_env_prompt(), codex_home),
            artifact_dir=artifact_dir,
            output_schema=REPO_ROOT / "scripts" / "schemas" / "session_env_smoke_result.json",
            extra_env=stripped_desktop_env({"CODEX_HOME": str(codex_home)}),
            extra_args=DESKTOP_E2E_EXEC_ARGS,
        )
        if result.exit_code != 0:
            raise SystemExit(f"codex exec exited with {result.exit_code}; inspect {artifact_dir}")
        require_computer_use_tool_call(result.transcript_path, artifact_dir=result.artifact_dir)
        require_session_env_transcript(
            transcript_mcp_tool_calls(result.transcript_path), artifact_dir
        )
        message = read_last_message(result.last_message_path)
        if (
            message.get("status") != "completed"
            or not message.get("session_env_repair_seen")
            or message.get("submitted_value") != SUBMITTED_VALUE
        ):
            raise SystemExit(
                f"codex session env smoke returned unexpected message: {json.dumps(message)}; "
                f"inspect {artifact_dir}"
            )
        time.sleep(1)
        require_submitted_value(dialog, artifact_dir)
        print(f"codex session env smoke passed: {artifact_dir}")
        return 0
    finally:
        close_dialog_if_running(dialog)


if __name__ == "__main__":
    raise SystemExit(main())
