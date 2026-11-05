"""Opt-in paired Jev/Terra ranking through the actual sky-cua desktop MCP path."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import subprocess
import sys
import time
from pathlib import Path
from typing import Any

from _mcp_stdio import McpClient, isolated_daemon_env, stop_service_processes_for_socket
from jev_ui_experiment import dump
from live_desktop_smoke import (
    appshot_action_fences,
    grouped_structured_result,
    load_state,
    require_ok,
    wait_for_stable_pointer_fixture,
)

ROOT = Path(__file__).resolve().parents[1]
CASES = [
    {
        "id": "exact",
        "intent": "Click Physical click target.",
        "exact": "Physical click target",
        "operation": "click",
        "field": "clicked",
    },
    {
        "id": "click",
        "intent": "Trigger the main button that acknowledges a physical mouse click.",
        "operation": "click",
        "field": "clicked",
    },
    {
        "id": "checkbox",
        "intent": "Enable the optional smoke setting using its checkbox.",
        "operation": "click",
        "field": "checkbox_toggled",
    },
    {
        "id": "expand",
        "intent": "Reveal the hidden smoke details.",
        "operation": "expand",
        "field": "expander_expanded",
    },
    {
        "id": "entry",
        "intent": "Place the caret in the free-form text entry field, ready for typing.",
        "operation": "focus",
        "field": "entry_focused",
    },
    {
        "id": "scroll",
        "intent": "Move the long list of smoke lines downward to see later lines.",
        "operation": "scroll",
        "field": "scroll_events",
    },
    {
        "id": "slider",
        "intent": "Set the horizontal slider to 75, leaving the vertical slider alone.",
        "operation": "set_value",
        "value": "75",
        "field": "slider_h_value",
    },
    {
        "id": "switch",
        "intent": "Turn on the on/off switch, leaving the checkbox alone.",
        "operation": "click",
        "field": "switch_active",
    },
    {
        "id": "absent",
        "intent": "Print the current document using its print control.",
        "operation": "click",
        "field": None,
    },
    {
        "id": "ambiguous",
        "intent": "Adjust a slider; either slider is equally suitable and no orientation is specified. Choose only if a unique target is justified.",
        "operation": "set_value",
        "value": "75",
        "field": None,
    },
]
ROLES = {
    "button",
    "text",
    "check box",
    "toggle button",
    "slider",
    "spin button",
    "combo box",
    "scroll pane",
}
FIELDS = [
    "clicked",
    "secondary_clicked",
    "drag_completed",
    "checkbox_toggled",
    "expander_expanded",
    "entry_focused",
    "scroll_events",
    "horizontal_scroll_events",
    "slider_h_value",
    "slider_v_value",
    "spin_value",
    "combo_index",
    "switch_active",
    "xy_pad_dragged",
    "dnd_dropped",
]


def candidates_for(snapshot: dict[str, Any]) -> list[dict[str, Any]]:
    result = []
    for e in snapshot["elements"]:
        b = e.get("bounds") or {}
        flags = e.get("state_flags", [])
        if (
            e.get("role") not in ROLES
            or "showing" not in flags
            or b.get("x", -1) < 0
            or b.get("y", -1) < 0
        ):
            continue
        if b.get("width", 0) <= 1 or b.get("height", 0) <= 1:
            continue
        result.append(
            {
                "id": f"e{e['element_index']}",
                "label": e.get("name") or "",
                "role": e["role"],
                "bounds": b,
                "disabled": "enabled" not in flags,
                "visible": True,
                "context": e.get("description") or "",
                "backend_ref": e["backend_ref"],
                "element_index": e["element_index"],
            }
        )
    return result


def public_candidates(candidates: list[dict[str, Any]]) -> list[dict[str, Any]]:
    return [
        {k: v for k, v in c.items() if k not in {"backend_ref", "element_index"}}
        for c in candidates
    ]


def exact_choice(case: dict[str, Any], candidates: list[dict[str, Any]]) -> str | None:
    matches = [
        c
        for c in candidates
        if case.get("exact") and c["label"] == case["exact"] and not c["disabled"]
    ]
    return matches[0]["id"] if len(matches) == 1 else None


class Desktop:
    def __init__(
        self, out: Path, *, client_path: Path | None = None, service_path: Path | None = None
    ):
        self.out = out
        self.socket = out / "service.sock"
        client_path = client_path or Path(
            os.environ.get(
                "SKY_CUA_CLIENT_PATH", str(Path.home() / ".local/share/sky-cua/bin/sky-cua-client")
            )
        )
        env = {"SKY_CUA_SERVICE_SOCKET_PATH": str(self.socket)}
        if service_path is not None:
            env["SKY_CUA_SERVICE_PATH"] = str(service_path.resolve())
        self.client = McpClient(
            [str(client_path.resolve()), "mcp"],
            extra_env=isolated_daemon_env(env),
            read_timeout=45,
        )
        self.client.initialize()
        dump(out / "tool-schemas.json", self.client.tools_list())
        self.sequence = 0

    def call(self, tool: str, args: dict[str, Any]) -> dict[str, Any]:
        self.sequence += 1
        r = self.client.tools_call(self.sequence, tool, args)
        require_ok(r, tool)
        return r

    def observe(self, wid: str) -> tuple[dict[str, Any], dict[str, Any]]:
        raw = self.call("observe", {"surface": "desktop", "window_id": wid})
        return grouped_structured_result(raw), raw

    def close(self) -> None:
        self.client.close()
        stop_service_processes_for_socket(self.socket)


def start_fixture(folder: Path) -> tuple[subprocess.Popen[str], Path]:
    state = folder / "state.json"
    env = dict(os.environ, NO_AT_BRIDGE="0", GDK_BACKEND="wayland", SKY_CUA_POINTER_FULLSCREEN="1")
    proc = subprocess.Popen(
        [sys.executable, str(ROOT / "scripts/jev_desktop_fixture.py"), str(state)],
        env=env,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )
    return proc, state


def act(
    desktop: Desktop, snapshot: dict[str, Any], selected: dict[str, Any], case: dict[str, Any]
) -> dict[str, Any]:
    args: dict[str, Any] = {
        **appshot_action_fences(snapshot),
        "element_index": selected["element_index"],
    }
    op = case["operation"]
    if op == "click":
        return desktop.call("desktop_pointer", {**args, "operation": "click"})
    if op in {"expand", "focus"}:
        return desktop.call("desktop_semantic", {**args, "operation": op})
    if op == "scroll":
        return desktop.call("desktop_scroll", {**args, "direction": "down", "pages": 1})
    return desktop.call("desktop_set_value", {**args, "value": case["value"]})


def target_correct(case: dict[str, Any], choice: str, candidates: list[dict[str, Any]]) -> bool:
    if case["field"] is None:
        return choice == "abstain"
    c = next((c for c in candidates if c["id"] == choice), None)
    if c is None:
        return False
    field = case["field"]
    if field == "clicked":
        return c["role"] == "button" and c["label"] == "Physical click target"
    if field == "checkbox_toggled":
        return c["role"] == "check box" and c["label"] == "Enable smoke option"
    if field == "expander_expanded":
        return c["label"] == "Smoke details (expander)"
    if field == "entry_focused":
        return c["role"] == "text"
    if field == "scroll_events":
        return c["role"] == "scroll pane"
    if field == "slider_h_value":
        return c["role"] == "slider" and c["bounds"]["width"] > c["bounds"]["height"]
    return field == "switch_active" and c["role"] == "toggle button" and not c["label"]


def score(
    case: dict[str, Any], before: dict[str, Any], after: dict[str, Any], choice: str
) -> dict[str, Any]:
    field = case["field"]
    changed = [k for k in FIELDS if before.get(k) != after.get(k)]
    if field is None:
        return {
            "success": choice == "abstain" and not changed,
            "wrong_action": choice != "abstain",
            "changed_fields": changed,
        }
    success = (
        bool(after.get(field)) if field != "slider_h_value" else abs(after.get(field, 0) - 75) < 1
    )
    return {
        "success": success,
        "wrong_action": any(k not in {field, "entry_focused"} for k in changed),
        "changed_fields": changed,
    }


def run_trial(
    desktop: Desktop,
    judges: Any,
    case: dict[str, Any],
    route: str,
    lane: str,
    folder: Path,
    dry: bool = False,
) -> dict[str, Any]:
    if folder.exists():
        raise FileExistsError(folder)
    folder.mkdir(parents=True)
    record: dict[str, Any] = {"case_id": case["id"], "route": route, "lane": lane}
    proc, state_path = start_fixture(folder)
    try:
        wait_for_stable_pointer_fixture(state_path, deadline=time.time() + 30)
        windows = grouped_structured_result(
            desktop.call("list_resources", {"surface": "desktop", "resource": "windows"})
        )["windows"]
        matches = [w for w in windows if w.get("pid") == proc.pid]
        if len(matches) != 1:
            raise RuntimeError("Fixture window not uniquely identified by PID")
        wid = matches[0]["window_id"]
        desktop.call("activate_window", {"window_id": wid})
        before = wait_for_stable_pointer_fixture(state_path, deadline=time.time() + 10)
        dump(folder / "before.json", before)
        started = time.perf_counter()
        snapshot, raw = desktop.observe(wid)
        dump(folder / "observation.json", raw)
        candidates = candidates_for(snapshot)
        public = public_candidates(candidates)
        if any("points=" in str(c) for c in public):
            raise RuntimeError("Oracle leaked into candidates")
        record["candidates"] = public
        record["observation_ms"] = (time.perf_counter() - started) * 1000
        image_path = None
        if lane == "image":
            geometry = before["monitor"]["geometry"]
            if (
                before["window_size"]["width"] != geometry["width"]
                or before["window_size"]["height"] != geometry["height"]
            ):
                raise RuntimeError("Image lane requires the verified fullscreen fixture")
            image_path = capture_image(snapshot, folder, record)
        choice = exact_choice(case, candidates)
        record["source"] = "rules"
        if choice is None:
            if dry:
                raise RuntimeError("Dry run requires exact case")
            model_start = time.perf_counter()
            if route == "hybrid":
                jev = judges.select("jev", case["intent"], public, folder / "jev", image_path)
                record["jev"] = jev
                if jev["choice"] != "abstain" and jev.get("probability", 0) >= 0.9:
                    choice = jev["choice"]
                    record["source"] = "jev"
            if choice is None:
                terra = judges.select("terra", case["intent"], public, folder / "terra", image_path)
                record["terra"] = terra
                choice = terra["choice"]
                record["source"] = "terra"
            record["decision_ms"] = (time.perf_counter() - model_start) * 1000
        record["choice"] = choice
        record["target_correct"] = target_correct(case, choice, candidates)
        if record.get("jev"):
            record["jev_target_correct"] = target_correct(case, record["jev"]["choice"], candidates)
        if choice != "abstain":
            selected = next((c for c in candidates if c["id"] == choice), None)
            if selected is None:
                raise RuntimeError("Unknown target")
            fresh, fresh_raw = desktop.observe(wid)
            dump(folder / "pre-action-observation.json", fresh_raw)
            matches = [
                c
                for c in candidates_for(fresh)
                if c["backend_ref"] == selected["backend_ref"]
                and c["label"] == selected["label"]
                and c["role"] == selected["role"]
                and not c["disabled"]
            ]
            if len(matches) != 1:
                raise RuntimeError("Target identity changed")
            record["action"] = act(desktop, fresh, matches[0], case)
            _post, post_raw = desktop.observe(wid)
            dump(folder / "post-observation.json", post_raw)
        after = load_state(state_path) or {}
        dump(folder / "after.json", after)
        record.update(score(case, before, after, choice))
        record["wrong_action"] = record["wrong_action"] or (
            choice != "abstain" and not record["target_correct"]
        )
        record["success"] = record["success"] and record["target_correct"]
        record["total_ms"] = (time.perf_counter() - started) * 1000
        record["status"] = "ok"
    except Exception as exc:
        record.update(status="error", error=str(exc), success=False)
    finally:
        proc.terminate()
        _stdout, stderr = proc.communicate(timeout=5)
        (folder / "fixture.stderr").write_text(stderr)
        dump(folder / "result.json", record)
    print(
        json.dumps(
            {
                k: v
                for k, v in record.items()
                if k
                in {
                    "case_id",
                    "route",
                    "lane",
                    "status",
                    "success",
                    "wrong_action",
                    "total_ms",
                    "source",
                    "error",
                }
            }
        ),
        flush=True,
    )
    return record


def capture_image(snapshot: dict[str, Any], folder: Path, record: dict[str, Any]) -> Path:
    from _jev_desktop_models import prepare_image

    metadata = snapshot["image"]
    filename = metadata["filename"]
    if Path(filename).name != filename:
        raise ValueError("Invalid artifact filename")
    source = Path(os.environ["XDG_RUNTIME_DIR"]) / "sky-cua/appshots" / filename
    raw = source.read_bytes()
    if len(raw) != metadata["size_bytes"] or hashlib.sha256(raw).hexdigest() != metadata["sha256"]:
        raise ValueError("AppShot image integrity mismatch")
    copy = folder / "observation.webp"
    copy.write_bytes(raw)
    record["image_transform"] = prepare_image(copy, folder / "model-image.png")
    record["image_note"] = (
        "Candidate bounds retain toolkit coordinates; screenshot includes window decorations. Select IDs, not pixel coordinates."
    )
    return folder / "model-image.png"


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("mode", choices=["freeze", "dry", "dry-image", "semantic", "image"])
    parser.add_argument("--out", type=Path, default=ROOT / "artifacts/jev-desktop-pilot")
    args = parser.parse_args()
    out = args.out.resolve()
    out.mkdir(parents=True, exist_ok=True)
    if args.mode == "freeze":
        if (out / "frozen.json").exists():
            raise FileExistsError("Already frozen")
        dump(
            out / "frozen.json",
            {
                "cases": CASES,
                "threshold": 0.9,
                "fixture_mode": "fullscreen, oracle debug status hidden",
                "source_hashes": {
                    p: hashlib.sha256((ROOT / p).read_bytes()).hexdigest()
                    for p in [
                        "scripts/jev_desktop_fixture.py",
                        "scripts/jev_desktop_pilot.py",
                        "scripts/_jev_desktop_models.py",
                    ]
                },
                "baseline": "rules then Terra",
                "hybrid": "rules then Jev then Terra",
                "model": "gpt-6.1-terra",
                "instruction": __import__("_jev_desktop_models").INSTRUCTION,
                "effort": "medium",
                "lanes": ["semantic", "image"],
                "gate": "zero wrong actions; no correctness regression; >=20% total paired wall time reduction",
                "scope": "actual sky-cua desktop MCP; ranking with prescribed operation, not autonomous planning; image-assisted uses same semantics plus screenshot",
                "timing": "observe, preprocessing, inference, fresh validation, native action, post-observe and independent readback; fixture setup excluded",
            },
        )
        (out / "frozen.sha256").write_text(
            hashlib.sha256((out / "frozen.json").read_bytes()).hexdigest()
        )
        return
    if not args.mode.startswith("dry"):
        assert (
            hashlib.sha256((out / "frozen.json").read_bytes()).hexdigest()
            == (out / "frozen.sha256").read_text()
        )
        assert json.loads((out / "frozen.json").read_text())["cases"] == CASES
    desktop = Desktop(out)
    try:
        if args.mode.startswith("dry"):
            run_trial(
                desktop,
                None,
                CASES[0],
                "baseline",
                "image" if args.mode == "dry-image" else "semantic",
                out / args.mode,
            )
        else:
            from _jev_desktop_models import DesktopJudges

            with DesktopJudges(out) as judges:
                records = []
                for index, case in enumerate(CASES):
                    for route in (
                        ["baseline", "hybrid"] if index % 2 == 0 else ["hybrid", "baseline"]
                    ):
                        record = run_trial(
                            desktop,
                            judges,
                            case,
                            route,
                            args.mode,
                            out / args.mode / f"{case['id']}-{route}",
                        )
                        records.append(record)
                        dump(out / f"{args.mode}-results.json", records)
                        if record["status"] != "ok":
                            raise RuntimeError("Trial failed; preserved evidence, no retry")
    finally:
        desktop.close()


if __name__ == "__main__":
    main()
