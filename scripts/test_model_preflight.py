"""Tests for the opencode model pre-flight helper (no real opencode calls)."""

from __future__ import annotations

import subprocess
import threading
from typing import cast

import pytest

import _model_preflight
import live_fallback_anchor_smoke
from _model_preflight import (
    DEFAULT_FALLBACK_ANCHOR_MODELS,
    ModelProbeResult,
    format_probe_table,
    probe_model,
    select_working_model,
)


def _completed(
    argv: list[str], *, returncode: int = 0, stdout: str = "", stderr: str = ""
) -> subprocess.CompletedProcess[str]:
    return subprocess.CompletedProcess(argv, returncode=returncode, stdout=stdout, stderr=stderr)


def _fake_runner(response: subprocess.CompletedProcess[str]):
    def runner(_model: str, _timeout_s: float) -> subprocess.CompletedProcess[str]:
        return response

    return runner


@pytest.mark.parametrize(
    ("proc", "expected_ok", "expected_reason"),
    [
        pytest.param(
            _completed(
                ["opencode"],
                returncode=0,
                stdout='{"type":"step_start"}\n{"type":"text","text":"READY"}\n'
                '{"type":"step_finish"}\n',
            ),
            True,
            "ok",
            id="ok",
        ),
        pytest.param(
            _completed(
                ["opencode"],
                returncode=1,
                stderr="Error: CreditsError: Insufficient balance",
            ),
            False,
            "no_credits",
            id="no_credits",
        ),
        pytest.param(
            _completed(
                ["opencode"],
                returncode=1,
                stderr="Error: Unexpected error / database is locked",
            ),
            False,
            "db_locked",
            id="db_locked",
        ),
        pytest.param(
            _completed(
                ["opencode"],
                returncode=1,
                stderr='Model "bogus/model" not found',
            ),
            False,
            "not_found",
            id="not_found",
        ),
        pytest.param(
            _completed(["opencode"], returncode=1, stderr="No models match pattern 'bogus/*'"),
            False,
            "not_found",
            id="no_models_match_pattern",
        ),
        pytest.param(
            _completed(["opencode"], returncode=17, stderr="kaboom, connection reset"),
            False,
            "error: kaboom, connection reset",
            id="generic_error",
        ),
    ],
)
def test_probe_model_classifies_outcomes(
    proc: subprocess.CompletedProcess[str], expected_ok: bool, expected_reason: str
) -> None:
    result = probe_model("opencode-go/deepseek-v4-pro", runner=_fake_runner(proc))

    assert result.ok is expected_ok
    assert result.reason == expected_reason
    assert result.model == "opencode-go/deepseek-v4-pro"


def test_probe_model_classifies_timeout_from_sentinel_returncode() -> None:
    proc = _completed(["opencode"], returncode=-9, stdout="", stderr="")

    result = probe_model("opencode-go/deepseek-v4-pro", runner=_fake_runner(proc))

    assert result.ok is False
    assert result.reason == "timeout"


def test_probe_model_classifies_raised_timeout_expired() -> None:
    def runner(_model: str, timeout_s: float) -> subprocess.CompletedProcess[str]:
        raise subprocess.TimeoutExpired(cmd=["opencode"], timeout=timeout_s)

    result = probe_model("opencode-go/deepseek-v4-pro", runner=runner)

    assert result.ok is False
    assert result.reason == "timeout"


def test_select_working_model_picks_first_ok_in_preference_order() -> None:
    def runner(model: str, _timeout_s: float) -> subprocess.CompletedProcess[str]:
        if model == "b":
            return _completed(["opencode"], returncode=0, stdout='{"type":"text"}\n')
        if model == "c":
            return _completed(["opencode"], returncode=0, stdout='{"type":"text"}\n')
        return _completed(["opencode"], returncode=1, stderr="CreditsError")

    selected, results = select_working_model(
        ["a", "b", "c"], runner=runner, stagger_s=0.0, timeout_s=5.0
    )

    assert selected == "b"
    assert [r.model for r in results] == ["a", "b", "c"]
    assert [r.reason for r in results] == ["no_credits", "ok", "ok"]


def test_select_working_model_retries_db_locked_and_succeeds() -> None:
    attempts: dict[str, int] = {"x": 0}
    lock = threading.Lock()

    def runner(model: str, _timeout_s: float) -> subprocess.CompletedProcess[str]:
        with lock:
            attempts[model] = attempts.get(model, 0) + 1
            attempt = attempts[model]
        if model == "x" and attempt == 1:
            return _completed(["opencode"], returncode=1, stderr="database is locked")
        return _completed(["opencode"], returncode=0, stdout='{"type":"text"}\n')

    selected, results = select_working_model(
        ["x"],
        runner=runner,
        stagger_s=0.0,
        timeout_s=5.0,
        max_lock_retries=3,
        retry_backoff_s=0.0,
    )

    assert selected == "x"
    assert results[0].ok is True
    assert results[0].reason == "ok"
    assert attempts["x"] == 2


def test_select_working_model_exhausts_lock_retries_and_reports_locked() -> None:
    def runner(_model: str, _timeout_s: float) -> subprocess.CompletedProcess[str]:
        return _completed(["opencode"], returncode=1, stderr="database is locked")

    selected, results = select_working_model(
        ["x"],
        runner=runner,
        stagger_s=0.0,
        timeout_s=5.0,
        max_lock_retries=2,
        retry_backoff_s=0.0,
    )

    assert selected is None
    assert results[0].reason == "db_locked"


def test_select_working_model_returns_none_when_all_fail() -> None:
    def runner(_model: str, _timeout_s: float) -> subprocess.CompletedProcess[str]:
        return _completed(["opencode"], returncode=1, stderr="CreditsError")

    selected, results = select_working_model(
        ["a", "b"], runner=runner, stagger_s=0.0, timeout_s=5.0
    )

    assert selected is None
    assert len(results) == 2
    assert all(r.reason == "no_credits" for r in results)


def test_select_working_model_handles_empty_candidate_list() -> None:
    selected, results = select_working_model([], runner=lambda *_a, **_k: _completed(["x"]))

    assert selected is None
    assert results == []


def test_default_fallback_anchor_models_preference_order() -> None:
    assert DEFAULT_FALLBACK_ANCHOR_MODELS == (
        "opencode-go/deepseek-v4-pro",
        "opencode-go/deepseek-v4-flash",
        "opencode/deepseek-v4-flash-free",
    )


def test_format_probe_table_renders_each_result() -> None:
    results = [
        ModelProbeResult("a", True, "ok", 1.23),
        ModelProbeResult("b", False, "no_credits", 0.5),
    ]

    table = format_probe_table(results)

    assert "a" in table and "OK" in table
    assert "b" in table and "no_credits" in table


def test_format_probe_table_handles_no_results() -> None:
    assert format_probe_table([]) == "(no models probed)"


def test_run_fallback_anchor_smoke_uses_preflight_selected_model(
    tmp_path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    """When no --model is given for opencode, the fixture asks pre-flight and
    passes the selected model into run_agent instead of a hardcoded default.
    """
    used_models: list[str | None] = []

    class FakeLaunchProcess:
        def poll(self) -> int | None:
            return None

        def terminate(self) -> None:
            return None

    def fake_run_agent(
        agent: str,
        _prompt: str,
        artifact_dir,
        *,
        model: str | None = None,
        **_kwargs: object,
    ):
        used_models.append(model)
        stdout = artifact_dir / f"{agent}.stdout.log"
        stdout.write_text(
            '{"type": "tool_result", "result": {"elements": [{"element_index": 0, '
            '"role": "window", "state_flags": ["vision_anchor"]}]}}\n',
            encoding="utf-8",
        )
        return live_fallback_anchor_smoke.subprocess.CompletedProcess([agent], returncode=0)

    def fake_select_working_model(candidates, **_kwargs):
        assert list(candidates) == list(DEFAULT_FALLBACK_ANCHOR_MODELS)
        return "opencode-go/deepseek-v4-flash", [
            ModelProbeResult(candidates[0], False, "no_credits", 0.1),
            ModelProbeResult(candidates[1], True, "ok", 0.2),
        ]

    monkeypatch.setattr(live_fallback_anchor_smoke, "make_artifact_dir", lambda *_args: tmp_path)
    monkeypatch.setattr(
        live_fallback_anchor_smoke.subprocess, "Popen", lambda *_a, **_k: FakeLaunchProcess()
    )
    monkeypatch.setattr(live_fallback_anchor_smoke, "run_agent", fake_run_agent)
    monkeypatch.setattr(live_fallback_anchor_smoke.time, "sleep", lambda *_a: None)
    monkeypatch.setattr(live_fallback_anchor_smoke, "kill_fallback_anchor_mpv", lambda: None)
    monkeypatch.setattr(
        live_fallback_anchor_smoke, "select_working_model", fake_select_working_model
    )

    exit_code = live_fallback_anchor_smoke.run_fallback_anchor_smoke(agent="opencode")

    assert exit_code == 0
    assert used_models == ["opencode-go/deepseek-v4-flash"]


def test_run_fallback_anchor_smoke_fails_early_when_no_model_reachable(
    tmp_path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    """No candidate model reachable: fail before launching mpv or the agent."""
    launched: list[str] = []

    def fake_select_working_model(candidates, **_kwargs):
        return None, [ModelProbeResult(model, False, "no_credits", 0.1) for model in candidates]

    def fake_popen(*_args, **_kwargs):
        launched.append("mpv")
        raise AssertionError("mpv should not be launched when pre-flight finds no model")

    def fake_run_agent(*_args, **_kwargs):
        raise AssertionError("agent should not be launched when pre-flight finds no model")

    monkeypatch.setattr(live_fallback_anchor_smoke, "make_artifact_dir", lambda *_args: tmp_path)
    monkeypatch.setattr(live_fallback_anchor_smoke.subprocess, "Popen", fake_popen)
    monkeypatch.setattr(live_fallback_anchor_smoke, "run_agent", fake_run_agent)
    monkeypatch.setattr(
        live_fallback_anchor_smoke, "select_working_model", fake_select_working_model
    )

    exit_code = live_fallback_anchor_smoke.run_fallback_anchor_smoke(agent="opencode")

    assert exit_code == 1
    assert launched == []


def test_run_fallback_anchor_smoke_honors_explicit_model_skips_preflight(
    tmp_path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    preflight_calls: list[object] = []
    used_models: list[str | None] = []

    class FakeLaunchProcess:
        def poll(self) -> int | None:
            return None

        def terminate(self) -> None:
            return None

    def fake_run_agent(
        agent: str,
        _prompt: str,
        artifact_dir,
        *,
        model: str | None = None,
        **_kwargs: object,
    ):
        used_models.append(model)
        stdout = artifact_dir / f"{agent}.stdout.log"
        stdout.write_text(
            '{"type": "tool_result", "result": {"elements": [{"element_index": 0, '
            '"role": "window", "state_flags": ["vision_anchor"]}]}}\n',
            encoding="utf-8",
        )
        return live_fallback_anchor_smoke.subprocess.CompletedProcess([agent], returncode=0)

    def fake_select_working_model(*args, **kwargs):
        preflight_calls.append((args, kwargs))
        raise AssertionError("pre-flight must be skipped when --model is explicit")

    monkeypatch.setattr(live_fallback_anchor_smoke, "make_artifact_dir", lambda *_args: tmp_path)
    monkeypatch.setattr(
        live_fallback_anchor_smoke.subprocess, "Popen", lambda *_a, **_k: FakeLaunchProcess()
    )
    monkeypatch.setattr(live_fallback_anchor_smoke, "run_agent", fake_run_agent)
    monkeypatch.setattr(live_fallback_anchor_smoke.time, "sleep", lambda *_a: None)
    monkeypatch.setattr(live_fallback_anchor_smoke, "kill_fallback_anchor_mpv", lambda: None)
    monkeypatch.setattr(
        live_fallback_anchor_smoke, "select_working_model", fake_select_working_model
    )

    exit_code = live_fallback_anchor_smoke.run_fallback_anchor_smoke(
        agent="opencode", model="pinned/model"
    )

    assert exit_code == 0
    assert preflight_calls == []
    assert used_models == ["pinned/model"]


def test_run_fallback_anchor_smoke_pi_agent_skips_preflight(
    tmp_path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    """Pi selects its own model via run_agent's own default; pre-flight
    (opencode/opencode-go candidate ids) does not apply to it."""

    def fake_select_working_model(*_args, **_kwargs):
        raise AssertionError("pre-flight must not run for the pi agent")

    class FakeLaunchProcess:
        def poll(self) -> int | None:
            return None

        def terminate(self) -> None:
            return None

    def fake_run_agent(agent: str, _prompt: str, artifact_dir, **_kwargs: object):
        stdout = artifact_dir / f"{agent}.stdout.log"
        stdout.write_text(
            '{"type": "tool_result", "result": {"elements": [{"element_index": 0, '
            '"role": "window", "state_flags": ["vision_anchor"]}]}}\n',
            encoding="utf-8",
        )
        return live_fallback_anchor_smoke.subprocess.CompletedProcess([agent], returncode=0)

    monkeypatch.setattr(live_fallback_anchor_smoke, "make_artifact_dir", lambda *_args: tmp_path)
    monkeypatch.setattr(
        live_fallback_anchor_smoke.subprocess, "Popen", lambda *_a, **_k: FakeLaunchProcess()
    )
    monkeypatch.setattr(live_fallback_anchor_smoke, "run_agent", fake_run_agent)
    monkeypatch.setattr(live_fallback_anchor_smoke.time, "sleep", lambda *_a: None)
    monkeypatch.setattr(live_fallback_anchor_smoke, "kill_fallback_anchor_mpv", lambda: None)
    monkeypatch.setattr(
        live_fallback_anchor_smoke, "select_working_model", fake_select_working_model
    )

    exit_code = live_fallback_anchor_smoke.run_fallback_anchor_smoke(agent="pi")

    assert exit_code == 0


def test_probe_model_default_runner_invokes_expected_argv(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    captured: dict[str, object] = {}

    def fake_subprocess_run(argv, **kwargs):
        captured["argv"] = argv
        captured["kwargs"] = kwargs
        return _completed(argv, returncode=0, stdout='{"type":"text"}\n')

    monkeypatch.setattr(_model_preflight.subprocess, "run", fake_subprocess_run)

    result = probe_model("opencode-go/deepseek-v4-pro", timeout_s=42.0)

    assert result.ok is True
    assert captured["argv"] == [
        "opencode",
        "run",
        "--format",
        "json",
        "--model",
        "opencode-go/deepseek-v4-pro",
        _model_preflight.PROBE_PROMPT,
    ]
    kwargs = cast("dict[str, object]", captured["kwargs"])
    assert kwargs["stdin"] is subprocess.DEVNULL
    assert kwargs["timeout"] == 42.0
