#!/usr/bin/env python3
"""Focused direct MCP text-readback smoke against a zenity entry dialog.

Proves AT-SPI editable readback (initial stale value, post-`set_value`
replacement) and dialog submission. Unlike `live_desktop_smoke.py` it does
not require PipeWire frame capture, so it can run headless on sessions where
snapshot images legitimately downgrade to the Screenshot portal fallback
(for example COSMIC). The strict full-capture lane remains
`live_desktop_smoke.py`.
"""

from __future__ import annotations

import json
import time
from pathlib import Path
from typing import Any

from _text_readback_smoke import (
    INITIAL_VALUE,
    REPLACEMENT_VALUE,
    TITLE,
    close_dialog_if_running,
    require_submitted_value,
    run_zenity_readback_dialog,
)
from live_desktop_smoke import (
    CLIENT,
    McpClient,
    find_button,
    find_editable,
    require_editable_readback,
    require_ok,
    wait_for_app_snapshot_result,
)


def main() -> int:
    artifact_dir = Path("artifacts/text-readback-smoke") / time.strftime("%Y%m%dT%H%M%SZ")
    artifact_dir.mkdir(parents=True, exist_ok=True)
    dialog = run_zenity_readback_dialog()
    client = McpClient([str(CLIENT), "mcp"])
    try:
        client.initialize()
        state = wait_for_app_snapshot_result(client, TITLE, deadline=time.time() + 15)
        snapshot = state["structuredContent"]
        write_artifact(artifact_dir / "initial-snapshot.json", state)
        editable = find_editable(snapshot)
        require_editable_readback(
            editable,
            INITIAL_VALUE,
            snapshot=snapshot,
            label="initial zenity readback",
        )
        require_ok(
            client.tools_call(
                20,
                "desktop_set_value",
                {
                    "snapshot_id": snapshot["snapshot_id"],
                    "element_index": editable["element_index"],
                    "value": REPLACEMENT_VALUE,
                },
            ),
            "set_value",
        )
        updated_state = wait_for_app_snapshot_result(client, TITLE, deadline=time.time() + 15)
        updated = updated_state["structuredContent"]
        write_artifact(artifact_dir / "updated-snapshot.json", updated_state)
        updated_editable = find_editable(updated)
        require_editable_readback(
            updated_editable,
            REPLACEMENT_VALUE,
            snapshot=updated,
            label="post-set_value zenity readback",
        )
        ok_button = find_button(updated, "OK")
        require_ok(
            client.tools_call(
                22,
                "desktop_pointer",
                {
                    "operation": "click",
                    "snapshot_id": updated["snapshot_id"],
                    "element_index": ok_button["element_index"],
                },
            ),
            "click OK",
        )
        wait_for_dialog_exit(dialog, deadline=time.time() + 10)
        require_submitted_value(dialog, artifact_dir)
        print(f"text readback smoke passed: {artifact_dir}")
        return 0
    finally:
        client.close()
        close_dialog_if_running(dialog)


def wait_for_dialog_exit(dialog: Any, *, deadline: float) -> None:
    while time.time() < deadline:
        if dialog.poll() is not None:
            return
        time.sleep(0.25)


def write_artifact(path: Path, payload: dict[str, Any]) -> None:
    path.write_text(json.dumps(payload, indent=2, sort_keys=True), encoding="utf-8")


if __name__ == "__main__":
    raise SystemExit(main())
