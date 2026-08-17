#!/usr/bin/env python3
"""Shared fixture-acknowledgement retry helpers for live smokes.

The pointer fixture acknowledges keyboard actions by flipping a field in its
state file (``entry_text`` for type_text, ``submitted_text`` for press_key).
Under desktop load the acknowledgement can lag past a fixed wait, so both waits
share one retry-once engine: on timeout they re-issue the action a single time,
but only while the observed field is still empty — meaning the original
keystrokes never landed. A non-empty wrong value is a hard failure, because
re-sending type_text would double-append and re-pressing Enter cannot repair a
different submitted value.
"""

from __future__ import annotations

import json
import time
from collections.abc import Callable
from pathlib import Path
from typing import Any

from live_desktop_smoke import (  # type: ignore[import-not-found]
    McpClient,
    load_state,
    require_ok,
    wait_for_state,
)


def write_json(path: Path, value: Any) -> None:
    path.write_text(json.dumps(value, indent=2, sort_keys=True), encoding="utf-8")


def wait_for_acknowledgement(
    client: McpClient,
    state_path: Path,
    *,
    field: str,
    expected: str,
    appshot_id: str,
    artifact_dir: Path,
    description: str,
    retry_request: Callable[[], dict[str, Any]],
    retry_artifact: str,
    retry_label: str,
    deadline_s: float = 20.0,
) -> dict[str, Any]:
    """Wait for the fixture to acknowledge a keyboard action, retrying once.

    On timeout the action is re-issued only while ``field`` is still empty (the
    original keystrokes never landed). Partial or unknown values are reported as
    a hard failure instead of being corrupted, with the original timeout chained
    as the cause.
    """

    def field_matches(current: dict[str, Any]) -> bool:
        return current.get(field) == expected

    timeout_error: RuntimeError | None = None
    try:
        return wait_for_state(
            state_path,
            field_matches,
            deadline=time.time() + deadline_s,
            description=description,
        )
    except RuntimeError as error:
        timeout_error = error

    last_state = load_state(state_path)
    if last_state is None or last_state.get(field) != "":
        raise RuntimeError(
            f"timed out waiting for fixture state: {description}\n"
            f"last_state={json.dumps(last_state, indent=2, sort_keys=True)}\n"
            f"original={timeout_error}"
        ) from timeout_error

    retry_result = retry_request()
    write_json(artifact_dir / retry_artifact, retry_result)
    require_ok(retry_result, retry_label)
    return wait_for_state(
        state_path,
        field_matches,
        deadline=time.time() + deadline_s,
        description=f"{description} (after retry)",
    )


def wait_for_type_text_acknowledgement(
    client: McpClient,
    call_id: int,
    state_path: Path,
    *,
    text_value: str,
    appshot_id: str,
    artifact_dir: Path,
    deadline_s: float = 20.0,
) -> dict[str, Any]:
    """Wait for the fixture to acknowledge type_text, retrying once under load.

    The fixture appends keystrokes to the entry, so a blind re-send could
    double the text. The retry only re-issues type_text when the entry is still
    empty, which means the original keystrokes never landed. Partial or unknown
    text is reported as a hard failure instead of being corrupted.
    """
    return wait_for_acknowledgement(
        client,
        state_path,
        field="entry_text",
        expected=text_value,
        appshot_id=appshot_id,
        artifact_dir=artifact_dir,
        description="visible Wayland type_text acknowledgement",
        retry_request=lambda: client.tools_call(
            call_id,
            "desktop_keyboard",
            {"operation": "type_text", "text": text_value, "appshot_id": appshot_id},
        ),
        retry_artifact="type-retry-result.json",
        retry_label="visible Wayland type_text retry",
        deadline_s=deadline_s,
    )


def wait_for_press_key_acknowledgement(
    client: McpClient,
    call_id: int,
    state_path: Path,
    *,
    text_value: str,
    appshot_id: str,
    artifact_dir: Path,
    deadline_s: float = 20.0,
) -> dict[str, Any]:
    """Wait for the fixture to acknowledge press_key, retrying once under load.

    Enter only ever sets ``submitted_text`` to the current entry contents, so a
    retry is idempotent: it can never double-append. The retry re-issues
    press_key only while ``submitted_text`` is still empty, meaning the original
    Enter never activated the entry. A non-empty wrong value is a hard failure.
    """
    return wait_for_acknowledgement(
        client,
        state_path,
        field="submitted_text",
        expected=text_value,
        appshot_id=appshot_id,
        artifact_dir=artifact_dir,
        description="visible Wayland press_key acknowledgement",
        retry_request=lambda: client.tools_call(
            call_id,
            "desktop_keyboard",
            {"operation": "press_key", "key": "Enter", "appshot_id": appshot_id},
        ),
        retry_artifact="press-key-retry-result.json",
        retry_label="visible Wayland press_key retry",
        deadline_s=deadline_s,
    )
