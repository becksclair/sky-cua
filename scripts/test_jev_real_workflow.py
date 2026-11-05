"""Offline tests for the real-workflow protocol and failure evidence."""

from __future__ import annotations

import json
from pathlib import Path
from typing import Any

import pytest

import jev_real_workflow as workflow


def node(index: int, label: str, role: str, **extra: Any) -> dict[str, Any]:
    return {
        "element_index": index,
        "backend_ref": f"native-{index}",
        "name": label,
        "role": role,
        "bounds": {"x": -30, "y": -10, "width": 100, "height": 30},
        "state_flags": ["showing", "enabled"] + (["editable"] if role == "text" else []),
        **extra,
    }


def projected(*nodes: dict[str, Any]) -> list[dict[str, Any]]:
    return workflow.candidates_for({"elements": list(nodes)})


def test_projection_preserves_native_semantics_and_excludes_hidden() -> None:
    candidates = projected(
        node(1, "Native name", "text", description="Native description"),
        node(2, "Hidden", "text", state_flags=["enabled"]),
        node(3, "Explicit hidden", "button", state_flags=["showing", "enabled", "hidden"]),
        node(4, "Tiny", "button", bounds={"width": 100, "height": 1}),
        node(5, "Unavailable", "button", state_flags=["showing"]),
    )
    assert [c["label"] for c in candidates] == ["Native name", "Unavailable"]
    assert candidates[0]["role"] == "text"
    assert candidates[0]["description"] == "Native description"
    assert candidates[0]["state_flags"] == ["showing", "enabled", "editable"]
    assert candidates[0]["bounds"]["x"] == -30  # Valid on an adjacent monitor.
    assert candidates[1]["disabled"] is True
    public = workflow.public_candidates(candidates)
    assert all("backend_ref" not in c and "element_index" not in c for c in public)
    assert all("oracle" not in c and "target_correct" not in c for c in public)


@pytest.mark.parametrize(
    ("step", "label", "role", "expected"),
    [
        ("editor", "source.txt", "text", True),
        ("editor", "source.txt", "entry", False),
        ("editor", "Other document", "text", False),
        ("editor", "source.txt", "button", False),
    ],
)
def test_target_oracle_checks_both_native_label_and_role(
    step: str, label: str, role: str, expected: bool
) -> None:
    candidate = {
        "label": label,
        "role": role,
        "visible": True,
        "disabled": False,
        "state_flags": ["editable"],
    }
    assert workflow.target_correct(step, candidate) is expected
    assert not workflow.target_correct(step, {**candidate, "disabled": True})
    assert not workflow.target_correct(step, {**candidate, "visible": False})
    assert not workflow.target_correct(step, {**candidate, "state_flags": []})


class Judges:
    def __init__(self, probability: Any = 0.8, choice: str = "e1"):
        self.probability = probability
        self.choice = choice
        self.calls: list[dict[str, Any]] = []

    def select(self, model: str, intent: str, candidates: Any, folder: Path) -> dict[str, Any]:
        self.calls.append({"model": model, "intent": intent, "candidates": candidates})
        return {"choice": self.choice, "probability": self.probability}


@pytest.mark.parametrize("probability", [0.799, float("nan"), True, 1.1, "0.9"])
def test_hybrid_falls_back_for_unusable_confidence(tmp_path: Path, probability: Any) -> None:
    judges = Judges(probability)
    record = {"jev_calls": 0, "terra_calls": 0}
    st: dict[str, Any] = {}
    checkpoints = []
    selected = workflow.select_target(
        judges,
        workflow.STEPS[0],
        projected(node(1, "source.txt", "text")),
        "hybrid",
        tmp_path,
        record,
        st,
        lambda: checkpoints.append(json.loads(json.dumps(record))),
    )
    assert selected == "e1"
    assert [call["model"] for call in judges.calls] == ["jev", "terra"]
    assert checkpoints[0]["jev_calls"] == 1
    assert st["source"] == "terra"
    assert all("oracle" not in call for call in judges.calls)
    assert all("target_correct" not in c for call in judges.calls for c in call["candidates"])


def test_threshold_boundary_accepts_jev_and_baseline_uses_terra(tmp_path: Path) -> None:
    for route, expected in (("hybrid", "jev"), ("baseline", "terra")):
        judges = Judges(0.8)
        st: dict[str, Any] = {}
        workflow.select_target(
            judges,
            workflow.STEPS[0],
            projected(node(1, "source.txt", "text")),
            route,
            tmp_path,
            {"jev_calls": 0, "terra_calls": 0},
            st,
            lambda: None,
        )
        assert [call["model"] for call in judges.calls] == [expected]
        assert st["source"] == expected


def test_byte_proof_requires_changed_exact_bytes(tmp_path: Path) -> None:
    source = tmp_path / "source"
    initial = b"Original input\n"
    expected = b"Replacement\r\n\x00bytes\n"
    source.write_bytes(initial)
    proof = workflow.file_proof(source, expected, initial)
    assert not proof["source_changed"]
    assert not proof["source_matches"]
    source.write_bytes(expected)
    proof = workflow.file_proof(source, expected, initial)
    assert proof["source_changed"] and proof["source_matches"]
    source.write_bytes(expected.replace(b"\r\n", b"\n"))
    proof = workflow.file_proof(source, expected, initial)
    assert proof["source_changed"] and not proof["source_matches"]
    assert proof["source_sha256"] != proof["expected_sha256"]


class Process:
    pid = 12345

    def __init__(self, command: list[str], **kwargs: Any):
        self.source = Path(command[1])
        self.env = kwargs["env"]
        self.stopped = False

    def poll(self) -> int | None:
        return 0 if self.stopped else None

    def terminate(self) -> None:
        self.stopped = True

    def wait(self, timeout: int) -> int:
        return 0


class FakeDesktop:
    def __init__(self, folder: Path, wrong: bool = False, action_error: bool = False):
        self.folder = folder
        self.stage = 0
        self.wrong = wrong
        self.action_error = action_error
        self.edited: str | None = None
        self.calls: list[str] = []
        self.action_checkpoints = []

    def call(self, tool: str, args: dict[str, Any]) -> dict[str, Any]:
        self.calls.append(tool)
        if tool == "list_resources":
            return {"windows": [{"pid": Process.pid, "window_id": "owned", "title": "Save As"}]}
        if tool in {"desktop_keyboard", "desktop_set_value"}:
            checkpoint = json.loads((self.folder / "progress.json").read_text())
            self.action_checkpoints.append(checkpoint)
            assert checkpoint["steps"][-1]["status"] == "acting"
            assert checkpoint["steps"][-1]["action"]["tool"] == tool
            # Activation invalidates snapshots; it must precede the last observation.
            assert self.calls[-4:-1] == ["activate_window", "observe", "list_resources"]
            if self.action_error:
                raise RuntimeError("injected native action failure")
            if tool == "desktop_set_value":
                self.edited = args["value"]
            if self.stage == 1:
                assert self.edited is not None
                assert tool == "desktop_keyboard"
                assert args["key"] == "Ctrl+S"
                (self.folder / "source.txt").write_bytes(self.edited.encode())
            self.stage += 1
        return {}

    def observe(self, wid: str) -> tuple[dict[str, Any], dict[str, Any]]:
        self.calls.append("observe")
        elements = [node(1, "source.txt", "text")]
        if self.wrong and self.stage == 0:
            elements.append(node(2, "Other text", "text"))
        snapshot = {"elements": elements}
        return snapshot, snapshot


@pytest.fixture
def offline_runtime(monkeypatch: pytest.MonkeyPatch) -> list[Process]:
    processes = []

    def launch(command: list[str], **kwargs: Any) -> Process:
        proc = Process(command, **kwargs)
        processes.append(proc)
        return proc

    monkeypatch.setattr(workflow.subprocess, "Popen", launch)
    monkeypatch.setattr(workflow, "grouped_structured_result", lambda raw: raw)
    monkeypatch.setattr(workflow, "appshot_action_fences", lambda snapshot: {"fence": "fresh"})
    return processes


def test_full_flow_proves_bytes_and_private_profiles(
    tmp_path: Path, offline_runtime: list[Process]
) -> None:
    folder = tmp_path / "flow"
    desktop = FakeDesktop(folder)
    result = workflow.run_flow(desktop, None, folder, "baseline", dry=True)  # type: ignore[arg-type]
    assert result["success"]
    assert result["wrong_targets"] == 0
    assert result["total_ms"] > 0
    assert len(desktop.action_checkpoints) == 2
    assert result["steps"][0]["target_correct"]
    assert result["steps"][1]["source"] == "deterministic_keyboard"
    assert all(st["status"] == "ok" for st in result["steps"])
    assert offline_runtime[0].stopped
    for name in ("CONFIG", "DATA", "STATE"):
        path = Path(offline_runtime[0].env[f"XDG_{name}_HOME"])
        assert path.parent == folder
        assert path.stat().st_mode & 0o777 == 0o700
    assert json.loads((folder / "file-proof.json").read_text())["source_matches"]


def test_wrong_target_is_counted_persisted_and_never_acted_on(
    tmp_path: Path, offline_runtime: list[Process]
) -> None:
    folder = tmp_path / "wrong"
    desktop = FakeDesktop(folder, wrong=True)
    result = workflow.run_flow(desktop, Judges(choice="e2"), folder, "hybrid")  # type: ignore[arg-type]
    assert not result["success"]
    assert result["wrong_targets"] == 1
    assert len(desktop.action_checkpoints) == 0
    failed = result["steps"][-1]
    assert failed["selected"]["label"] == "Other text"
    assert failed["status"] == "error"
    assert failed["target_correct"] is False
    assert json.loads((folder / "progress.json").read_text())["wrong_targets"] == 1


def test_action_error_preserves_attempted_step_before_exception(
    tmp_path: Path, offline_runtime: list[Process]
) -> None:
    folder = tmp_path / "error"
    result = workflow.run_flow(
        FakeDesktop(folder, action_error=True),  # type: ignore[arg-type]
        None,
        folder,
        "baseline",
        True,
    )
    assert not result["success"]
    assert result["steps"][0]["status"] == "error"
    assert result["steps"][0]["action"]["tool"] == "desktop_set_value"
    assert "injected" in result["error"]
    assert json.loads((folder / "result.json").read_text()) == result


def test_frozen_protocol_and_paired_gate() -> None:
    protocol = workflow.frozen_protocol()
    assert protocol["threshold"] == 0.8
    assert protocol["warmups_per_route"] == 1
    assert protocol["order"] == [
        ["baseline", "hybrid"],
        ["hybrid", "baseline"],
        ["baseline", "hybrid"],
    ]
    records = [
        {
            "pair": pair,
            "route": route,
            "total_ms": timing,
            "wrong_targets": 0,
            "status": "ok",
            "success": True,
        }
        for pair in range(3)
        for route, timing in (("baseline", 100), ("hybrid", 70))
    ]
    summary = workflow.paired_summary(records)
    assert summary["passed"]
    assert summary["savings_fraction"] == pytest.approx(0.3)
    for record in records:
        if record["route"] == "hybrid":
            record["total_ms"] = 80
    assert workflow.paired_summary(records)["passed"]  # Exactly 20%, despite float rounding.
    records[0]["wrong_targets"] = 1
    assert not workflow.paired_summary(records)["passed"]
    with pytest.raises(ValueError, match="Incomplete"):
        workflow.paired_summary(records[:-1])


def test_cli_freezes_before_opening_desktop_and_runs_exact_order(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    order = []

    class Desktop:
        def __init__(self, out: Path, **kwargs: Any):
            assert json.loads((out / "frozen.json").read_text())["threshold"] == 0.8
            assert (out / "frozen.sha256").exists()
            assert kwargs == {
                "client_path": Path("/local/client"),
                "service_path": Path("/local/service"),
            }

        def close(self) -> None:
            pass

    class Context:
        def __init__(self, out: Path):
            pass

        def __enter__(self) -> None:
            return None

        def __exit__(self, *args: Any) -> None:
            pass

    def run(d: Any, judges: Any, folder: Path, route: str) -> dict[str, Any]:
        order.append(folder.name)
        return {
            "route": route,
            "total_ms": 100 if route == "baseline" else 70,
            "wrong_targets": 0,
            "success": True,
            "status": "ok",
        }

    monkeypatch.setattr(workflow, "Desktop", Desktop)
    monkeypatch.setattr(workflow, "DesktopJudges", Context)
    monkeypatch.setattr(workflow, "run_flow", run)
    monkeypatch.setattr(
        "sys.argv",
        [
            "workflow",
            "run",
            "--out",
            str(tmp_path),
            "--client-path",
            "/local/client",
            "--service-path",
            "/local/service",
        ],
    )
    assert workflow.main() == 0
    assert order == [
        "warmup-baseline",
        "warmup-hybrid",
        "pair-0-baseline",
        "pair-0-hybrid",
        "pair-1-hybrid",
        "pair-1-baseline",
        "pair-2-baseline",
        "pair-2-hybrid",
    ]


def test_failed_dry_cli_is_nonzero(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    class Desktop:
        def __init__(self, out: Path):
            pass

        def close(self) -> None:
            pass

    monkeypatch.setattr(workflow, "Desktop", Desktop)
    monkeypatch.setattr(workflow, "run_flow", lambda *args: {"success": False})
    monkeypatch.setattr("sys.argv", ["workflow", "dry", "--out", str(tmp_path)])
    assert workflow.main() == 1


def test_provider_error_keeps_pending_step_and_attempt_count(
    tmp_path: Path, offline_runtime: list[Process]
) -> None:
    class FailingJudges:
        def select(self, *args: Any) -> None:
            raise RuntimeError("injected provider failure")

    folder = tmp_path / "provider-error"
    desktop = FakeDesktop(folder)
    result = workflow.run_flow(desktop, FailingJudges(), folder, "hybrid")  # type: ignore[arg-type]
    assert not result["success"]
    assert result["jev_calls"] == 1 and result["terra_calls"] == 0
    assert result["steps"][0]["pending_model"] == "jev"
    assert result["steps"][0]["status"] == "error"
    assert not desktop.action_checkpoints
    assert json.loads((folder / "progress.json").read_text()) == result
