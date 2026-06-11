#!/usr/bin/env python3
from __future__ import annotations

import json
import time

from _app_server_harness import (
    require_computer_use_item,
    run_rich_app_server_turn,
    transcript_computer_use_items,
)
from _codex_exec import make_artifact_dir
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
    artifact_dir = make_artifact_dir("app-server-session-env-smoke")
    import_current_env_to_systemd()
    dialog = run_session_env_dialog()
    try:
        result = run_rich_app_server_turn(
            prompt=session_env_prompt(),
            artifact_dir=artifact_dir,
            output_schema=REPO_ROOT / "scripts" / "schemas" / "session_env_smoke_result.json",
            extra_env=stripped_desktop_env(),
        )
        require_computer_use_item(result.transcript_path)
        require_session_env_transcript(
            transcript_computer_use_items(result.transcript_path),
            artifact_dir,
        )
        message = json.loads(result.last_message_path.read_text())
        if (
            message.get("status") != "completed"
            or not message.get("session_env_repair_seen")
            or message.get("submitted_value") != SUBMITTED_VALUE
        ):
            raise SystemExit(
                f"app-server session env smoke returned unexpected message: {json.dumps(message)}; "
                f"inspect {artifact_dir}"
            )
        time.sleep(1)
        require_submitted_value(dialog, artifact_dir)
        print(f"rich app-server session env smoke passed: {artifact_dir}")
        return 0
    finally:
        close_dialog_if_running(dialog)


if __name__ == "__main__":
    raise SystemExit(main())
