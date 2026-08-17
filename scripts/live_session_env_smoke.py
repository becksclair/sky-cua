#!/usr/bin/env python3
from __future__ import annotations

import json
import time
from pathlib import Path
from typing import Any

from _session_env_smoke import (
    TITLE,
    close_dialog_if_running,
    import_current_env_to_systemd,
    require_session_env_doctor,
    require_submitted_value,
    run_session_env_dialog,
    stripped_desktop_env,
)
from live_desktop_smoke import (
    CLIENT,
    McpClient,
    appshot_action_fences,
    find_button,
    find_editable,
    isolated_daemon_env,
    require_ok,
    wait_for_app_snapshot_result,
)


def grouped_structured_result(result: dict[str, Any]) -> dict[str, Any]:
    structured = result.get("structuredContent") or {}
    if not isinstance(structured, dict):
        return {}
    nested = structured.get("result")
    return nested if isinstance(nested, dict) else structured


def main() -> int:
    artifact_dir = Path("artifacts/session-env-smoke") / time.strftime("%Y%m%dT%H%M%SZ")
    artifact_dir.mkdir(parents=True, exist_ok=True)
    import_current_env_to_systemd()
    dialog = run_session_env_dialog()
    base_env = stripped_desktop_env(isolated_daemon_env())
    client = McpClient([str(CLIENT), "mcp"], base_env=base_env)
    try:
        client.initialize()
        doctor = client.tools_call(3, "doctor", {})
        doctor_report = grouped_structured_result(doctor)
        require_session_env_doctor(doctor_report, artifact_dir)

        apps = client.tools_call(4, "list_resources", {"surface": "desktop", "resource": "apps"})
        (artifact_dir / "doctor.json").write_text(
            json.dumps(doctor, indent=2, sort_keys=True), encoding="utf-8"
        )
        (artifact_dir / "list_apps.json").write_text(
            json.dumps(apps, indent=2, sort_keys=True), encoding="utf-8"
        )
        state = wait_for_app_snapshot_result(client, TITLE, deadline=time.time() + 10)
        snapshot = state["structuredContent"]
        (artifact_dir / "snapshot.json").write_text(
            json.dumps(state, indent=2, sort_keys=True), encoding="utf-8"
        )
        if doctor_report.get("environment", {}).get("session_kind") == "unsupported":
            raise RuntimeError(
                "environment stayed unsupported after session-env repair.\n"
                f"doctor={json.dumps(doctor_report, indent=2, sort_keys=True)}"
            )
        try:
            editable = find_editable(snapshot)
        except RuntimeError:
            target_x, target_y = fallback_entry_point(snapshot)
            require_ok(
                client.tools_call(
                    6,
                    "desktop_pointer",
                    {
                        "operation": "click",
                        **appshot_action_fences(snapshot),
                        "x": target_x,
                        "y": target_y,
                    },
                ),
                "fallback entry click",
            )
            require_ok(
                client.tools_call(
                    7,
                    "desktop_keyboard",
                    {"operation": "type_text", "text": "session-env-ok"},
                ),
                "fallback type_text",
            )
            require_ok(
                client.tools_call(
                    8,
                    "desktop_keyboard",
                    {"operation": "press_key", "key": "Enter"},
                ),
                "fallback press_key",
            )
        else:
            require_ok(
                client.tools_call(
                    6,
                    "desktop_set_value",
                    {
                        **appshot_action_fences(snapshot),
                        "element_index": editable["element_index"],
                        "value": "session-env-ok",
                    },
                ),
                "set_value",
            )
            updated = wait_for_app_snapshot_result(client, TITLE, deadline=time.time() + 10)[
                "structuredContent"
            ]
            ok_button = find_button(updated, "OK")
            require_ok(
                client.tools_call(
                    8,
                    "desktop_pointer",
                    {
                        "operation": "click",
                        **appshot_action_fences(updated),
                        "element_index": ok_button["element_index"],
                    },
                ),
                "click",
            )
        require_submitted_value(dialog, artifact_dir)
        print(f"session env smoke passed: {artifact_dir}")
        return 0
    finally:
        client.close()
        close_dialog_if_running(dialog)


def fallback_entry_point(snapshot: dict[str, Any]) -> tuple[float, float]:
    elements_raw = snapshot.get("elements", [])
    elements = [element for element in elements_raw if isinstance(element, dict)]
    rows = [element for element in elements if element.get("role") == "wayland_row_band_candidate"]
    if len(rows) >= 2:
        return element_point(snapshot, rows[1], y_fraction=0.15)
    for role in ("wayland_list_candidate", "wayland_main_region", "window"):
        for element in elements:
            if element.get("role") == role:
                return element_point(snapshot, element)
    raise RuntimeError(
        "session env smoke had no semantic editable element and no fallback entry anchor.\n"
        f"snapshot={json.dumps(snapshot, indent=2, sort_keys=True)}"
    )


def element_point(
    snapshot: dict[str, Any],
    element: dict[str, Any],
    *,
    y_fraction: float = 0.5,
) -> tuple[float, float]:
    bounds = element.get("bounds")
    capture = snapshot.get("capture")
    if not isinstance(bounds, dict) or not isinstance(capture, dict):
        raise RuntimeError(
            "fallback element did not include bounds or capture scale.\n"
            f"element={json.dumps(element, indent=2, sort_keys=True)}"
        )
    scale = capture.get("logical_to_pixel_scale")
    if not isinstance(scale, int | float):
        scale = 1.0
    x = (float(bounds["x"]) + (float(bounds["width"]) / 2.0)) * float(scale)
    y = (float(bounds["y"]) + (float(bounds["height"]) * y_fraction)) * float(scale)
    return x, y


if __name__ == "__main__":
    raise SystemExit(main())
