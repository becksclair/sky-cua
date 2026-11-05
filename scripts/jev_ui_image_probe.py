"""Frozen synthetic PNG image transport probe; no personal screenshots."""

from __future__ import annotations

import argparse
import base64
import io
import json
import time
from pathlib import Path

from PIL import Image, ImageDraw

from jev_ui_experiment import dump, jev_client


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--out", type=Path, default=Path(__file__).resolve().parents[1] / "artifacts/jev-ui/images"
    )
    out = parser.parse_args().out.resolve()
    out.mkdir(parents=True, exist_ok=True)
    if (out / "frozen.json").exists():
        raise FileExistsError("Refusing to overwrite image probe evidence")
    cases = []
    for color in ["red", "blue"]:
        started = time.perf_counter()
        im = Image.new("RGB", (320, 240), "white")
        ImageDraw.Draw(im).rectangle((60, 20, 260, 220), fill=color)
        # Exercise the normal WebP -> supported PNG conversion boundary.
        webp = io.BytesIO()
        im.save(webp, format="WEBP", lossless=True)
        png = io.BytesIO()
        # Preserve pixels and pad 20px above/below to meet the 280px minimum side.
        padded = Image.new("RGB", (320, 280), "white")
        padded.paste(Image.open(io.BytesIO(webp.getvalue())), (0, 20))
        padded.save(png, format="PNG")
        (out / f"{color}.webp").write_bytes(webp.getvalue())
        (out / f"{color}.png").write_bytes(png.getvalue())
        payload = {
            "model": "jev-1.13",
            "state": {"screen": "fixture-image"},
            "assets": {
                "fixture-image": {
                    "type": "image",
                    "format": "png",
                    "data_base64": base64.b64encode(png.getvalue()).decode(),
                }
            },
            "questions": {
                "red": {
                    "type": "noul",
                    "instructions": "Does the image contain a large red square?",
                },
                "blue": {
                    "type": "noul",
                    "instructions": "Does the image contain a large blue square?",
                },
            },
        }
        cases.append(
            {
                "color": color,
                "expected": {"red": color == "red", "blue": color == "blue"},
                "preprocess_ms": (time.perf_counter() - started) * 1000,
                "payload": payload,
            }
        )
    dump(out / "frozen.json", cases)
    records = []
    with jev_client(experimental_images=True) as client:
        for case in cases:
            started = time.perf_counter()
            record = {"color": case["color"]}
            try:
                record["response"] = client.call(case["payload"])
                record["status"] = "ok"
            except Exception as exc:
                record["status"] = "error"
                record["error_type"] = type(exc).__name__
                # The reused helper guarantees this exception contains no provider body or key.
                record["safe_error"] = (
                    str(exc) if type(exc).__name__ == "JevError" else "probe failed"
                )
            record["request_ms"] = client.last_elapsed_ms
            record["workflow_ms"] = (time.perf_counter() - started) * 1000 + case["preprocess_ms"]
            records.append(record)
            dump(out / "results.json", records)
            if record["status"] == "error":
                break
    print(json.dumps(records, indent=2))


if __name__ == "__main__":
    main()
