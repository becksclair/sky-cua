#!/usr/bin/env python3
"""Operator-run smoke for forced Wayland capture downgrade.

This starts a fresh isolated sky-cua service, forces PipeWire image capture to
fail, and proves that get_app_state falls back to the Screenshot portal with
explicit downgrade diagnostics and MCP summary text.
"""

from __future__ import annotations

import json
import tempfile
import time
from pathlib import Path

from live_desktop_smoke import (
    CLIENT,
    McpClient,
    run_zenity_input,
    wait_for_app_snapshot_result,
)

DOWNGRADE_TITLE = "sky-cua forced downgrade smoke"
SESSION_DIAGNOSTIC_CODES = {"PortalSessionStarted", "PortalSessionRestored"}
SESSION_SUMMARY_MARKERS = (
    "Started a new combined RemoteDesktop and ScreenCast portal session.",
    "Reused a persisted RemoteDesktop approval token for the combined portal session.",
)


def diagnostic_codes(diagnostics: list[dict[str, object]]) -> set[str]:
    return {code for entry in diagnostics if isinstance(code := entry.get("code"), str)}


def has_portal_session_diagnostic(diagnostics: list[dict[str, object]]) -> bool:
    return bool(diagnostic_codes(diagnostics).intersection(SESSION_DIAGNOSTIC_CODES))


def summary_mentions_portal_session(summary_text: str) -> bool:
    return any(marker in summary_text for marker in SESSION_SUMMARY_MARKERS)


def main() -> int:
    print("Starting live portal downgrade smoke.")
    print("If KDE shows a portal approval prompt, approve it so the test can continue.\n")

    with tempfile.TemporaryDirectory(prefix="sky-cua-downgrade-smoke-") as tmpdir:
        socket_path = Path(tmpdir) / "service.sock"
        extra_env = {
            "SKY_CUA_FORCE_PIPEWIRE_CAPTURE_FAILURE": "1",
            "SKY_CUA_SERVICE_SOCKET_PATH": str(socket_path),
        }
        dialog = run_zenity_input(DOWNGRADE_TITLE)
        client = McpClient([str(CLIENT), "mcp"], extra_env=extra_env)
        try:
            client.initialize()
            tools = {tool["name"] for tool in client.tools_list()}
            missing = sorted({"list_apps", "get_app_state"} - tools)
            if missing:
                raise RuntimeError(f"MCP server did not advertise required tools: {missing}")

            result = wait_for_app_snapshot_result(
                client,
                DOWNGRADE_TITLE,
                deadline=time.time() + 30,
            )
            if result.get("isError"):
                structured = result.get("structuredContent") or {}
                code = structured.get("code")
                if code == "PortalApprovalPending":
                    raise RuntimeError(
                        "Forced downgrade smoke is still waiting on portal approval. "
                        "Approve the KDE dialog and re-run.\n"
                        f"result={json.dumps(result, indent=2, sort_keys=True)}"
                    )
                raise RuntimeError(
                    "Forced downgrade smoke failed before a snapshot was produced.\n"
                    f"result={json.dumps(result, indent=2, sort_keys=True)}"
                )

            snapshot = result["structuredContent"]
            summary_text = result["content"][0]["text"]
            capture = snapshot.get("capture") or {}
            diagnostics = snapshot.get("diagnostics") or []

            expected_codes = {
                "PipeWireStreamFailed",
                "CaptureBackendDowngraded",
            }
            seen_codes = diagnostic_codes(diagnostics)
            if not has_portal_session_diagnostic(diagnostics):
                raise RuntimeError(
                    "Forced downgrade snapshot did not report a portal session startup or restore diagnostic.\n"
                    f"diagnostics={json.dumps(diagnostics, indent=2, sort_keys=True)}"
                )
            missing_codes = sorted(expected_codes - seen_codes)
            if missing_codes:
                raise RuntimeError(
                    "Forced downgrade snapshot did not report the expected diagnostics.\n"
                    f"missing={missing_codes}\n"
                    f"diagnostics={json.dumps(diagnostics, indent=2, sort_keys=True)}"
                )

            if capture.get("backend") != "portal_pipe_wire":
                raise RuntimeError(
                    "Forced downgrade snapshot did not preserve PipeWire as the primary capture lane.\n"
                    f"capture={json.dumps(capture, indent=2, sort_keys=True)}"
                )
            if capture.get("image_backend") != "portal_screenshot":
                raise RuntimeError(
                    "Forced downgrade snapshot did not report Screenshot fallback as the actual image backend.\n"
                    f"capture={json.dumps(capture, indent=2, sort_keys=True)}"
                )

            screenshot_path = capture.get("screenshot_path")
            if not screenshot_path or not Path(screenshot_path).exists():
                raise RuntimeError(
                    "Forced downgrade snapshot did not produce a real fallback screenshot path.\n"
                    f"capture={json.dumps(capture, indent=2, sort_keys=True)}"
                )

            if not summary_mentions_portal_session(summary_text):
                raise RuntimeError(
                    "Forced downgrade summary did not mention portal session startup or restore.\n"
                    f"summary={summary_text!r}"
                )
            if (
                "Snapshot image capture downgraded from PipeWire to Screenshot portal fallback"
                not in summary_text
            ):
                raise RuntimeError(
                    "Forced downgrade summary did not mention the capture downgrade.\n"
                    f"summary={summary_text!r}"
                )
            if "image_backend=portal_screenshot" not in summary_text:
                raise RuntimeError(
                    "Forced downgrade summary did not include the image-backend detail.\n"
                    f"summary={summary_text!r}"
                )

            print(f"Focused app: {snapshot.get('focused_app')}")
            print(f"Summary: {summary_text}")
            print(f"Capture: {capture}")
            print(f"Diagnostics: {json.dumps(diagnostics, sort_keys=True)}")
            print("\nForced portal downgrade smoke completed successfully.")
            return 0
        finally:
            client.close()
            if dialog.poll() is None:
                dialog.terminate()
                try:
                    dialog.wait(timeout=5)
                except Exception:
                    dialog.kill()


if __name__ == "__main__":
    raise SystemExit(main())
