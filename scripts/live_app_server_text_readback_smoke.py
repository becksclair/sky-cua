#!/usr/bin/env python3
from __future__ import annotations

import json
import time

from _app_server_harness import (
    require_computer_use_item,
    run_rich_app_server_turn,
    transcript_computer_use_items,
    with_plugin_mention,
)
from _codex_exec import make_artifact_dir
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
    artifact_dir = make_artifact_dir("app-server-text-readback-smoke")
    dialog = run_zenity_readback_dialog()
    try:
        result = run_rich_app_server_turn(
            prompt=with_plugin_mention(text_readback_prompt()),
            artifact_dir=artifact_dir,
            output_schema=REPO_ROOT / "scripts" / "schemas" / "text_readback_smoke_result.json",
        )
        require_computer_use_item(result.transcript_path)
        require_transcript_readback_values(
            transcript_computer_use_items(result.transcript_path),
            artifact_dir,
        )
        message = json.loads(result.last_message_path.read_text())
        require_text_readback_message(message, artifact_dir)
        time.sleep(1)
        require_submitted_value(dialog, artifact_dir)
        print(f"rich app-server text readback smoke passed: {artifact_dir}")
        print(json.dumps(message, indent=2))
        return 0
    finally:
        close_dialog_if_running(dialog)


if __name__ == "__main__":
    raise SystemExit(main())
