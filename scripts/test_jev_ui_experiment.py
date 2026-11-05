"""Offline safeguards for the paid experiment's frozen input boundary."""

from __future__ import annotations

import hashlib
import json
from pathlib import Path

import pytest

from jev_ui_experiment import freeze


def test_freeze_excludes_oracle_and_binds_exact_input_bytes(tmp_path: Path) -> None:
    freeze(tmp_path)
    raw = (tmp_path / "frozen.json").read_bytes()
    manifest = json.loads(raw)
    assert (tmp_path / "frozen.sha256").read_text().strip() == hashlib.sha256(raw).hexdigest()
    cases = {case["id"]: case for case in manifest["cases"]}
    assert len(cases) == 100
    for batch in manifest["batches"]:
        wire = json.dumps(batch["payload"])
        for private in (
            "ground_truth",
            "expected",
            "rules_prediction",
            "stored_after",
            "intended_widget_keys",
        ):
            assert f'"{private}"' not in wire
        for i, case_id in enumerate(batch["ids"]):
            case = cases[case_id]
            assert batch["payload"]["state"][f"c{i}"] == case["state"]
            assert case["expected"] in batch["payload"]["questions"][f"q{i}"]["criteria"]
    with pytest.raises(FileExistsError):
        freeze(tmp_path)
    assert (tmp_path / "frozen.json").read_bytes() == raw


def test_ranking_contains_model_opportunity_and_no_invented_target(tmp_path: Path) -> None:
    freeze(tmp_path)
    cases = json.loads((tmp_path / "frozen.json").read_text())["cases"]
    ranking = [c for c in cases if c["task"] == "ranking"]
    assert any(c["rules_prediction"] == "abstain" and c["expected"] != "abstain" for c in ranking)
    for case in ranking:
        assert case["expected"] in {"abstain", *(c["id"] for c in case["state"]["candidates"])}


def test_changed_frozen_bytes_cannot_reach_paid_client(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    from unittest.mock import Mock

    import jev_ui_experiment

    freeze(tmp_path)
    path = tmp_path / "frozen.json"
    path.write_bytes(path.read_bytes() + b" ")
    client = Mock(side_effect=RuntimeError("paid client must not be constructed"))
    monkeypatch.setattr(jev_ui_experiment, "jev_client", client)
    with pytest.raises(AssertionError):
        jev_ui_experiment.run(tmp_path)
    client.assert_not_called()


def test_image_probe_obeys_vendor_minimum_dimensions(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    import base64
    import io
    from unittest.mock import MagicMock

    from PIL import Image

    import jev_ui_image_probe

    factory = MagicMock()
    client = factory.return_value.__enter__.return_value
    client.last_elapsed_ms = 0
    client.call.return_value = {"answers": {}}
    monkeypatch.setattr(jev_ui_image_probe, "jev_client", factory)
    monkeypatch.setattr("sys.argv", ["jev_ui_image_probe", "--out", str(tmp_path)])
    jev_ui_image_probe.main()
    factory.assert_called_once_with(experimental_images=True)
    assert client.call.call_count == 2
    for color, call in zip(("red", "blue"), client.call.call_args_list, strict=True):
        asset = call.args[0]["assets"]["fixture-image"]
        image = Image.open(io.BytesIO(base64.b64decode(asset["data_base64"])))
        original = Image.open(tmp_path / f"{color}.webp")
        assert image.crop((0, 20, 320, 260)).tobytes() == original.tobytes()
        assert image.size == (320, 280)
        assert 280 <= image.width <= 1568 and 280 <= image.height <= 1568
        assert image.width * image.height >= 78400
        assert image.getpixel((0, 279)) == (255, 255, 255)
