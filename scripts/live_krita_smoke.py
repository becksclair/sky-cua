#!/usr/bin/env python3
"""Operator-run hybrid smoke for a real Krita workflow.

This is deliberately a hybrid Computer Use proof rather than a pure AT-SPI drill.
It uses the accessibility tree to find the Krita app and stable high-level
surfaces, and uses physical pointer control for the dialog/canvas steps that are
more reliable visually than semantically in Krita.

Workflow proved:
- launch Krita on the live desktop
- click "New Image"
- create the default document
- draw a visible mark on the canvas
- open Save As and save a .kra file
- verify the saved archive exists and its merged preview image is not blank
"""

from __future__ import annotations

import io
import json
import os
import shutil
import subprocess
import tempfile
import time
import uuid
import zipfile
from pathlib import Path
from typing import Any

from PIL import Image, ImageChops

from live_desktop_smoke import (  # type: ignore[import-not-found]
    CLIENT,
    McpClient,
    require_no_portal_approval_pending,
    require_ok,
    wait_for_app_snapshot_result,
)

KRITA_TITLE = "Krita"
PICTURES_DIR = Path.home() / "Pictures"
CREATE_BUTTON_IN_FRAME = (0.597, 0.748)
NAME_FIELD_IN_FRAME = (0.449, 0.714)
CANVAS_DRAG_START = (0.48, 0.217)
CANVAS_DRAG_END = (0.631, 0.509)


def require_installed(binary: str) -> None:
    if shutil.which(binary) is None:
        raise RuntimeError(f"required binary is not installed: {binary}")


def ensure_no_existing_krita() -> None:
    existing = subprocess.run(
        ["pgrep", "-x", "krita"],
        stdout=subprocess.PIPE,
        stderr=subprocess.DEVNULL,
        text=True,
        check=False,
    )
    if existing.returncode == 0 and existing.stdout.strip():
        raise RuntimeError(
            "Krita already appears to be running. Close existing Krita windows before running "
            "the live smoke so it doesn't trample a real session."
        )


def launch_krita(workdir: Path) -> subprocess.Popen[str]:
    return subprocess.Popen(
        ["krita", "--nosplash"],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.PIPE,
        text=True,
        cwd=workdir,
        env=dict(os.environ),
    )


def terminate_process(process: subprocess.Popen[str], *, name: str) -> None:
    if process.poll() is None:
        process.terminate()
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait(timeout=5)
    stderr = process.stderr.read() if process.stderr is not None else ""
    if stderr.strip():
        print(f"{name} stderr: {stderr.strip()}")


def find_named_element(snapshot: dict[str, Any], name: str) -> dict[str, Any]:
    for element in snapshot.get("elements", []):
        if (element.get("name") or "") == name and element.get("bounds"):
            return element
    raise RuntimeError(
        f"did not find a visible element named {name!r}.\n"
        f"elements={json.dumps(snapshot.get('elements', []), indent=2, sort_keys=True)}"
    )


def find_krita_frame(snapshot: dict[str, Any]) -> dict[str, Any]:
    frames = [
        element
        for element in snapshot.get("elements", [])
        if element.get("role") == "frame"
        and element.get("bounds")
        and (element.get("name") or "") == "Krita"
    ]
    if not frames:
        raise RuntimeError(
            "did not find the main Krita frame with bounds.\n"
            f"snapshot={json.dumps(snapshot, indent=2, sort_keys=True)}"
        )
    return max(
        frames,
        key=lambda element: float(element["bounds"]["width"]) * float(element["bounds"]["height"]),
    )


def find_canvas_frame(snapshot: dict[str, Any]) -> dict[str, Any]:
    frames = [
        element
        for element in snapshot.get("elements", [])
        if element.get("role") == "frame"
        and element.get("bounds")
        and "[not saved]" in ((element.get("name") or "").lower())
    ]
    if not frames:
        raise RuntimeError(
            "did not find the active unsaved document frame after creating the canvas.\n"
            f"snapshot={json.dumps(snapshot, indent=2, sort_keys=True)}"
        )
    return max(
        frames,
        key=lambda element: float(element["bounds"]["width"]) * float(element["bounds"]["height"]),
    )


def point_within(
    bounds: dict[str, Any], x_fraction: float, y_fraction: float
) -> tuple[float, float]:
    return (
        float(bounds["x"]) + (float(bounds["width"]) * x_fraction),
        float(bounds["y"]) + (float(bounds["height"]) * y_fraction),
    )


def wait_for_file(path: Path, *, deadline: float) -> None:
    while time.time() < deadline:
        if path.exists():
            return
        time.sleep(0.5)
    raise RuntimeError(f"expected file {path} to appear, but it never did")


def verify_kra_has_nonwhite_preview(path: Path) -> None:
    with zipfile.ZipFile(path) as archive:
        merged_png = archive.read("mergedimage.png")
    image = Image.open(io.BytesIO(merged_png)).convert("RGB")
    diff = ImageChops.difference(image, Image.new("RGB", image.size, (255, 255, 255)))
    if diff.getbbox() is None:
        raise RuntimeError(f"saved file {path} exists, but the merged preview image is blank white")


def main() -> int:
    print("Starting live Krita workflow smoke.")
    print("If KDE shows a portal approval prompt, approve it so the test can continue.\n")

    require_installed("krita")
    ensure_no_existing_krita()
    PICTURES_DIR.mkdir(parents=True, exist_ok=True)

    filename = f"sky-cua-krita-smoke-{uuid.uuid4().hex[:8]}.kra"
    output_path = PICTURES_DIR / filename
    if output_path.exists():
        output_path.unlink()

    with tempfile.TemporaryDirectory(prefix="sky-cua-krita-smoke-") as tmpdir:
        tmpdir_path = Path(tmpdir)
        socket_path = tmpdir_path / "service.sock"

        krita = launch_krita(tmpdir_path)
        client = McpClient(
            [str(CLIENT), "mcp"],
            extra_env={"SKY_CUA_SERVICE_SOCKET_PATH": str(socket_path)},
        )
        try:
            client.initialize()
            tools = {tool["name"] for tool in client.tools_list()}
            missing = sorted(
                {"list_apps", "get_app_state", "click", "drag", "type_text", "press_key"} - tools
            )
            if missing:
                raise RuntimeError(f"MCP server did not advertise required tools: {missing}")

            initial = wait_for_app_snapshot_result(client, KRITA_TITLE, deadline=time.time() + 30)
            if initial.get("isError"):
                structured = initial.get("structuredContent") or {}
                code = structured.get("code")
                if code == "PortalApprovalPending":
                    raise RuntimeError(
                        "Krita smoke is still waiting on portal approval. Approve the KDE dialog and re-run.\n"
                        f"result={json.dumps(initial, indent=2, sort_keys=True)}"
                    )
                raise RuntimeError(
                    "Krita smoke failed before the first snapshot was produced.\n"
                    f"result={json.dumps(initial, indent=2, sort_keys=True)}"
                )

            initial_snapshot = initial["structuredContent"]
            require_no_portal_approval_pending(initial_snapshot, "Krita initial snapshot")
            focused_app = initial_snapshot.get("focused_app") or {}
            if focused_app.get("desktop_file_id") != "krita.desktop":
                raise RuntimeError(
                    "Krita smoke did not focus the expected desktop app.\n"
                    f"focused_app={json.dumps(focused_app, indent=2, sort_keys=True)}"
                )
            if str(focused_app.get("app_id") or "").startswith("x11:"):
                raise RuntimeError(
                    "Krita surfaced as X11/XWayland instead of a native desktop app.\n"
                    f"focused_app={json.dumps(focused_app, indent=2, sort_keys=True)}"
                )

            new_image = find_named_element(initial_snapshot, "New Image")
            new_image_bounds = new_image["bounds"]
            require_ok(
                client.tools_call(
                    20,
                    "click",
                    {
                        "x": float(new_image_bounds["x"])
                        + (float(new_image_bounds["width"]) / 2.0),
                        "y": float(new_image_bounds["y"])
                        + (float(new_image_bounds["height"]) / 2.0),
                    },
                ),
                "Krita New Image click",
            )
            time.sleep(2)

            custom_document = client.tools_call(
                21, "get_app_state", {"app_id": focused_app["app_id"]}
            )
            require_ok(custom_document, "Krita Custom Document snapshot")
            custom_snapshot = custom_document["structuredContent"]
            custom_focused = custom_snapshot.get("focused_app") or {}
            if "custom document" not in ((custom_focused.get("window_title") or "").lower()):
                raise RuntimeError(
                    "Krita did not open the Custom Document dialog after clicking New Image.\n"
                    f"focused_app={json.dumps(custom_focused, indent=2, sort_keys=True)}"
                )

            krita_frame = find_krita_frame(custom_snapshot)
            create_x, create_y = point_within(krita_frame["bounds"], *CREATE_BUTTON_IN_FRAME)
            require_ok(
                client.tools_call(22, "click", {"x": create_x, "y": create_y}),
                "Krita Custom Document Create click",
            )
            time.sleep(3)

            canvas_result = client.tools_call(
                23, "get_app_state", {"app_id": focused_app["app_id"]}
            )
            require_ok(canvas_result, "Krita canvas snapshot")
            canvas_snapshot = canvas_result["structuredContent"]
            canvas_frame = find_canvas_frame(canvas_snapshot)
            drag_from = point_within(canvas_frame["bounds"], *CANVAS_DRAG_START)
            drag_to = point_within(canvas_frame["bounds"], *CANVAS_DRAG_END)
            require_ok(
                client.tools_call(
                    24,
                    "drag",
                    {
                        "from_x": drag_from[0],
                        "from_y": drag_from[1],
                        "to_x": drag_to[0],
                        "to_y": drag_to[1],
                    },
                ),
                "Krita canvas drag",
            )
            time.sleep(1)

            require_ok(
                client.tools_call(
                    25,
                    "press_key",
                    {
                        "snapshot_id": canvas_snapshot["snapshot_id"],
                        "element_index": canvas_frame["element_index"],
                        "key": "Ctrl+S",
                    },
                ),
                "Krita save shortcut",
            )
            time.sleep(2)

            save_field_x, save_field_y = point_within(krita_frame["bounds"], *NAME_FIELD_IN_FRAME)
            require_ok(
                client.tools_call(26, "click", {"x": save_field_x, "y": save_field_y}),
                "Krita save-name field click",
            )
            time.sleep(0.5)
            require_ok(
                client.tools_call(
                    27,
                    "type_text",
                    {"x": save_field_x, "y": save_field_y, "text": filename},
                ),
                "Krita filename type_text",
            )
            require_ok(
                client.tools_call(
                    28,
                    "press_key",
                    {"x": save_field_x, "y": save_field_y, "key": "Enter"},
                ),
                "Krita save confirmation",
            )

            wait_for_file(output_path, deadline=time.time() + 20)
            verify_kra_has_nonwhite_preview(output_path)

            print(f"Focused app: {focused_app}")
            print(
                "New Image button: "
                + json.dumps(
                    {
                        "element_index": new_image["element_index"],
                        "bounds": new_image["bounds"],
                    },
                    sort_keys=True,
                )
            )
            print(
                "Canvas frame: "
                + json.dumps(
                    {
                        "element_index": canvas_frame["element_index"],
                        "name": canvas_frame.get("name"),
                        "bounds": canvas_frame.get("bounds"),
                    },
                    sort_keys=True,
                )
            )
            print(f"Saved file: {output_path}")
            print(f"File size: {output_path.stat().st_size} bytes")
            print("Verified: mergedimage.png inside the .kra archive contains non-white pixels.")
            print("\nLive Krita workflow smoke completed successfully.")
            return 0
        finally:
            client.close()
            terminate_process(krita, name="Krita")


if __name__ == "__main__":
    raise SystemExit(main())
