"""Offline adapter checks: no credentials, model calls, or desktop are needed."""

from __future__ import annotations

import base64
import json
from pathlib import Path
from types import SimpleNamespace
from typing import Any
from unittest.mock import Mock

import pytest
from PIL import Image

import _jev_desktop_models as models


def candidates() -> list[dict[str, Any]]:
    return [
        {
            "id": "k2",
            "label": "Apply",
            "role": "button",
            "context": "Display",
            "bounds": {"x": 25, "y": 50, "width": 80, "height": 30},
            "disabled": False,
            "visible": True,
            "oracle": "must never enter prompt",
        },
        {
            "id": "k9",
            "label": "Apply",
            "role": "button",
            "context": "Audio",
            "disabled": True,
            "visible": True,
        },
    ]


@pytest.mark.parametrize(
    ("size", "resized", "output", "offset"),
    [
        ((320, 240), (320, 240), (320, 280), (0, 20)),
        ((3200, 1600), (1568, 784), (1568, 784), (0, 0)),
        ((100, 4000), (39, 1568), (280, 1568), (120, 0)),
        ((10, 10), (10, 10), (280, 280), (135, 135)),
        ((1568, 280), (1568, 280), (1568, 280), (0, 0)),
    ],
)
def test_prepare_image_shared_transform(
    tmp_path: Path,
    size: tuple[int, int],
    resized: tuple[int, int],
    output: tuple[int, int],
    offset: tuple[int, int],
) -> None:
    source = tmp_path / "source.webp"
    Image.new("RGB", size, "red").save(source, "WEBP", lossless=True)
    out = tmp_path / "prepared.png"
    metadata = models.prepare_image(source, out)
    with Image.open(out) as prepared:
        assert prepared.format == "PNG"
        assert prepared.size == output
        assert all(280 <= side <= 1568 for side in prepared.size)
        assert prepared.getpixel(offset) == (255, 0, 0)
        if offset != (0, 0):
            assert prepared.getpixel((0, 0)) == (255, 255, 255)
    assert metadata["image_path"] == str(out.resolve())
    assert metadata["original_size"] == list(size)
    assert metadata["resized_size"] == list(resized)
    assert metadata["output_size"] == list(output)
    assert metadata["offset"] == list(offset)
    assert metadata["scale_x"] * size[0] == pytest.approx(resized[0])
    assert metadata["scale_y"] * size[1] == pytest.approx(resized[1])


def test_jev_reuses_client_and_does_not_gate_probability(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    response = {
        "answers": {"target": {"choice": "k2", "probabilities": {"k2": 0.89}}},
        "usage": {"tokens": 12},
    }
    client = Mock()
    client.call.return_value = response
    factory = Mock(return_value=client)
    auth = Mock(side_effect=AssertionError("Jev must not prepare Codex auth"))
    monkeypatch.setattr(models, "jev_client", factory)
    monkeypatch.setattr(models, "prepare_judge_codex_home", auth)
    with models.DesktopJudges(tmp_path) as judges:
        factory.assert_not_called()
        result = judges.select("jev", "Apply display settings", candidates(), tmp_path / "a")
        judges.select("jev", "Apply display settings", candidates(), tmp_path / "b")
        assert result["choice"] == "k2"
        assert result["probability"] == 0.89
        assert result["response"] == response
        assert result["usage"] == {"tokens": 12}
        assert result["model"] == "jev-1.13"
        assert result["elapsed_ms"] >= 0
        request = json.loads((tmp_path / "a/request.json").read_text())
        assert "oracle" not in json.dumps(request)
        assert request["state"]["candidates"][0]["bounds"] == candidates()[0]["bounds"]
        assert set(request["questions"]["target"]["criteria"]) == {"k2", "k9", "abstain"}
        assert request["questions"]["target"]["instructions"] == models.INSTRUCTION
        assert "page_text" not in request["state"]
        judges.select(
            "jev", "Apply display settings", candidates(), tmp_path / "c", page_text="Shared"
        )
        request = json.loads((tmp_path / "c/request.json").read_text())
        assert list(request["state"]) == ["intent", "page_text", "candidates"]
        assert request["state"]["page_text"] == "Shared"
        assert json.dumps(request).count("Shared") == 1
    factory.assert_called_once_with(experimental_images=True)
    auth.assert_not_called()
    client.close.assert_called_once()
    judges.close()
    client.close.assert_called_once()
    with pytest.raises(RuntimeError, match="closed"):
        judges.select("jev", "Apply", candidates(), tmp_path / "closed")


def test_both_routes_consume_identical_png_and_terra_auth_is_removed(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    source = tmp_path / "original.webp"
    Image.new("RGB", (320, 240), "blue").save(source, "WEBP", lossless=True)
    image = tmp_path / "shared.png"
    models.prepare_image(source, image)
    client = Mock()
    client.call.return_value = {"answers": {"target": {"choice": "abstain"}}}
    monkeypatch.setattr(models, "jev_client", Mock(return_value=client))
    homes: list[Path] = []

    def prepare_home(folder: Path) -> Path:
        home = folder / "judge-codex-home"
        home.mkdir()
        (home / "auth.json").write_text("fake test auth")
        homes.append(home)
        return home

    def run(**kwargs: Any) -> SimpleNamespace:
        folder = kwargs["artifact_dir"]
        final = folder / "last-message.json"
        final.write_text('{"target":"k2"}')
        transcript = folder / "codex-output.jsonl"
        transcript.write_text('{"type":"turn.completed","usage":{"input_tokens":7}}\n')
        return SimpleNamespace(exit_code=0, last_message_path=final, transcript_path=transcript)

    runner = Mock(side_effect=run)
    monkeypatch.setattr(models, "prepare_judge_codex_home", prepare_home)
    monkeypatch.setattr(models, "run_codex_exec", runner)
    with models.DesktopJudges(tmp_path) as judges:
        jev = judges.select("jev", "Apply display settings", candidates(), tmp_path / "jev", image)
        terra = judges.select(
            "terra", "Apply display settings", candidates(), tmp_path / "terra", image
        )
        judges.select("terra", "Apply display settings", candidates(), tmp_path / "terra2")
        assert len(homes) == 1
        assert (homes[0] / "auth.json").exists()
        assert "probability" not in jev
        assert "probability" not in terra
    assert not homes[0].exists()
    request = client.call.call_args.args[0]
    asset = request["assets"]["desktop-screen"]
    assert base64.b64decode(asset["data_base64"]) == image.read_bytes()
    args = runner.call_args_list[0].kwargs
    assert args["extra_args"] == ["--sandbox", "read-only", "--image", str(image.resolve()), "--"]
    # The shared harness appends its prompt after extra_args. Without --,
    # Codex's variadic image flag consumes that prompt as another image path.
    assert args["extra_args"][-3:] == ["--image", str(image.resolve()), "--"]
    assert args["extra_env"] == {"CODEX_HOME": str(homes[0])}
    assert args["model"] == "gpt-6.1-terra"
    assert args["reasoning_effort"] == "medium"
    assert "data_base64" not in args["prompt"]
    assert asset["data_base64"] not in args["prompt"]
    prompt_payload = json.loads(args["prompt"].split("\n", 1)[1])
    assert prompt_payload == {key: request[key] for key in ("state", "questions")}
    schema = json.loads(args["output_schema"].read_text())
    assert schema["properties"]["target"]["enum"] == ["k2", "k9", "abstain"]
    assert terra["usage"] == [{"input_tokens": 7}]
    assert terra["response"] == {"target": "k2"}
    assert terra["model"] == "gpt-6.1-terra"


@pytest.mark.parametrize("choice", ["invented", 1, None])
def test_invalid_model_choices_are_rejected_but_raw_response_retained(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
    choice: object,
) -> None:
    client = Mock()
    response = {"answers": {"target": {"choice": choice}}}
    client.call.return_value = response
    monkeypatch.setattr(models, "jev_client", Mock(return_value=client))
    with models.DesktopJudges(tmp_path) as judges, pytest.raises(ValueError, match="unobserved"):
        judges.select("jev", "Apply", candidates(), tmp_path / "call")
    assert json.loads((tmp_path / "call/response.json").read_text()) == response
    assert json.loads((tmp_path / "call/error.json").read_text()) == {"error_type": "ValueError"}
    client.close.assert_called_once()


def test_cleanup_survives_provider_close_failure(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    def prepare_home(folder: Path) -> Path:
        (folder / "auth.json").write_text("fake test auth")
        return folder

    monkeypatch.setattr(models, "prepare_judge_codex_home", prepare_home)
    judges = models.DesktopJudges(tmp_path)
    home = judges._terra_home()
    judges._client = Mock()
    judges._client.close.side_effect = RuntimeError("close failed")
    with pytest.raises(RuntimeError, match="close failed"):
        judges.close()
    assert not home.exists()


def test_failed_auth_preparation_removes_partial_auth_and_retains_only_safe_error(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    homes: list[Path] = []

    def prepare_home(folder: Path) -> Path:
        (folder / "auth.json").write_text("test secret")
        homes.append(folder)
        raise RuntimeError("provider body with test secret")

    runner = Mock(side_effect=AssertionError("No call without prepared auth"))
    monkeypatch.setattr(models, "prepare_judge_codex_home", prepare_home)
    monkeypatch.setattr(models, "run_codex_exec", runner)
    with models.DesktopJudges(tmp_path) as judges, pytest.raises(RuntimeError):
        judges.select("terra", "Apply", candidates(), tmp_path / "call")
    assert not homes[0].exists()
    assert json.loads((tmp_path / "call/error.json").read_text()) == {"error_type": "RuntimeError"}
    assert "test secret" not in (tmp_path / "call/error.json").read_text()
    runner.assert_not_called()


@pytest.mark.parametrize("probability", [-0.1, 1.1, True, "0.9", float("nan")])
def test_invalid_probabilities_are_rejected(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
    probability: object,
) -> None:
    client = Mock()
    client.call.return_value = {
        "answers": {"target": {"choice": "k2", "probabilities": {"k2": probability}}}
    }
    monkeypatch.setattr(models, "jev_client", Mock(return_value=client))
    with models.DesktopJudges(tmp_path) as judges, pytest.raises(ValueError, match="probability"):
        judges.select("jev", "Apply", candidates(), tmp_path / "call")


@pytest.mark.parametrize("ids", [["x", "x"], ["abstain"], [""]])
def test_invalid_candidate_ids_fail_before_clients(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
    ids: list[str],
) -> None:
    factory = Mock(side_effect=AssertionError("Must not initialize client"))
    monkeypatch.setattr(models, "jev_client", factory)
    with models.DesktopJudges(tmp_path) as judges, pytest.raises(ValueError):
        judges.select("jev", "Apply", [{"id": cid} for cid in ids], tmp_path / "call")
    factory.assert_not_called()


def test_rejects_unprepared_images_and_repeated_evidence(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    source = tmp_path / "too-small.png"
    Image.new("RGB", (200, 200)).save(source)
    client = Mock()
    client.call.return_value = {"answers": {"target": {"choice": "abstain"}}}
    monkeypatch.setattr(models, "jev_client", Mock(return_value=client))
    with models.DesktopJudges(tmp_path) as judges:
        with pytest.raises(ValueError, match="prepare_image"):
            judges.select("jev", "Apply", candidates(), tmp_path / "bad-image", source)
        client.call.assert_not_called()
        judges.select("jev", "Apply", candidates(), tmp_path / "call")
        with pytest.raises(FileExistsError):
            judges.select("jev", "Apply", candidates(), tmp_path / "call")
        client.call.assert_called_once()
