"""Opt-in desktop target judges; image preparation is shared by both routes."""

from __future__ import annotations

import base64
import json
import math
import tempfile
import time
from pathlib import Path
from types import TracebackType
from typing import Any, Literal

from PIL import Image

from _agent_perf_judge import prepare_judge_codex_home
from _codex_exec import read_last_message, run_codex_exec
from jev_ui_experiment import dump, jev_client

JEV_MODEL = "jev-1.13"
TERRA_MODEL = "gpt-6.1-terra"
TERRA_EFFORT = "medium"
INSTRUCTION = (
    "Choose the single observed enabled visible target that fulfills the user intent. "
    "Use its label, role, surrounding context, bounds, and the screenshot when supplied. "
    "Candidate IDs are opaque references, not semantic clues. Do not choose disabled or hidden "
    "controls. If the target is absent, multiple targets are equally valid, or the intent "
    "requires more than one target, choose abstain. Return an observed candidate ID only; "
    "never invent IDs or coordinates."
)


def prepare_image(source: Path, out: Path) -> dict[str, Any]:
    """Write one PNG for both judges, shrinking to 1568px and padding sides to 280px.

    Bounds remain in the original observation space. The returned transform maps a
    source point to the image as ``(x * scale_x + offset[0], y * scale_y + offset[1])``.
    Small images are padded rather than enlarged; all padding is white.
    """
    with Image.open(source) as original:
        original_size = original.size
        scale = min(1.0, 1568 / max(original_size))
        resized_size = tuple(max(1, round(side * scale)) for side in original_size)
        image = original.convert("RGB")
        if resized_size != original_size:
            image = image.resize(resized_size, Image.Resampling.LANCZOS)
        output_size = (max(280, image.width), max(280, image.height))
        offset = ((output_size[0] - image.width) // 2, (output_size[1] - image.height) // 2)
        padded = Image.new("RGB", output_size, "white")
        padded.paste(image, offset)
        out.parent.mkdir(parents=True, exist_ok=True)
        padded.save(out, format="PNG")
    return {
        "image_path": str(out.resolve()),
        "original_size": list(original_size),
        "resized_size": list(resized_size),
        "output_size": list(output_size),
        "scale": scale,
        "scale_x": resized_size[0] / original_size[0],
        "scale_y": resized_size[1] / original_size[1],
        "offset": list(offset),
    }


def _payload(
    intent: str, candidates: list[dict[str, Any]], page_text: str | None = None
) -> dict[str, Any]:
    observed = [
        {
            key: candidate[key]
            for key in (
                "id",
                "label",
                "role",
                "context",
                "bounds",
                "enabled",
                "disabled",
                "visible",
            )
            if key in candidate
        }
        for candidate in candidates
    ]
    ids = [candidate.get("id") for candidate in observed]
    if any(not isinstance(cid, str) or not cid or cid == "abstain" for cid in ids):
        raise ValueError("Candidate IDs must be nonempty opaque strings other than abstain")
    if len(ids) != len(set(ids)):
        raise ValueError("Candidate IDs must be unique")
    criteria: dict[str, Any] = {candidate["id"]: candidate for candidate in observed}
    criteria["abstain"] = "No unique enabled visible target fulfills the intent."
    state: dict[str, Any] = {"intent": intent}
    if page_text is not None:
        state["page_text"] = page_text
    state["candidates"] = observed
    return {
        "state": state,
        "questions": {
            "target": {"type": "choice", "instructions": INSTRUCTION, "criteria": criteria}
        },
    }


class DesktopJudges:
    """Reuse the image-enabled Jev client and clean temporary Terra auth on close.

    Neither constructor nor image preparation discovers credentials or calls a
    provider. Judge clients/auth are prepared lazily on the selected live route.
    """

    def __init__(self, out: Path):
        self.out = out
        self._client: Any = None
        self._temporary: tempfile.TemporaryDirectory[str] | None = None
        self._home: Path | None = None
        self._closed = False

    def __enter__(self) -> DesktopJudges:
        if self._closed:
            raise RuntimeError("Desktop judges are closed")
        return self

    def __exit__(
        self,
        exc_type: type[BaseException] | None,
        exc: BaseException | None,
        traceback: TracebackType | None,
    ) -> None:
        self.close()

    def close(self) -> None:
        if self._closed:
            return
        self._closed = True
        try:
            if self._client is not None:
                self._client.close()
        finally:
            if self._temporary is not None:
                self._temporary.cleanup()

    def _terra_home(self) -> Path:
        if self._home is None:
            self.out.mkdir(parents=True, exist_ok=True)
            self._temporary = tempfile.TemporaryDirectory(prefix="desktop-judges-", dir=self.out)
            try:
                self._home = prepare_judge_codex_home(Path(self._temporary.name))
            except BaseException:
                self._temporary.cleanup()
                raise
        return self._home

    def select(
        self,
        model: Literal["jev", "terra"],
        intent: str,
        candidates: list[dict[str, Any]],
        folder: Path,
        image_path: Path | None = None,
        page_text: str | None = None,
    ) -> dict[str, Any]:
        """Select a target, retaining raw evidence; confidence gating belongs to caller.

        ``image_path`` must be the shared PNG returned by ``prepare_image``.
        The adapters validate it but never independently transform its pixels.
        """
        if self._closed:
            raise RuntimeError("Desktop judges are closed")
        if model not in ("jev", "terra"):
            raise ValueError("Unknown desktop judge")
        payload = _payload(intent, candidates, page_text)
        image_bytes: bytes | None = None
        if image_path is not None:
            with Image.open(image_path) as image:
                if image.format != "PNG" or any(not 280 <= side <= 1568 for side in image.size):
                    raise ValueError(
                        "Use prepare_image to supply a PNG with sides from 280 to 1568"
                    )
            image_path = image_path.resolve()
            image_bytes = image_path.read_bytes()
            payload["state"]["screen"] = "desktop-screen"
        folder.mkdir(parents=True, exist_ok=True)
        if (folder / "request.json").exists():
            raise FileExistsError("Refusing to overwrite existing judge evidence")
        started = time.perf_counter()
        try:
            if model == "jev":
                request = {"model": JEV_MODEL, **payload}
                if image_bytes is not None:
                    request["assets"] = {
                        "desktop-screen": {
                            "type": "image",
                            "format": "png",
                            "data_base64": base64.b64encode(image_bytes).decode("ascii"),
                        }
                    }
                dump(folder / "request.json", request)
                if self._client is None:
                    self._client = jev_client(experimental_images=True)
                response = self._client.call(request)
                dump(folder / "response.json", response)
                answer = response["answers"]["target"]
                choice = answer["choice"]
                result: dict[str, Any] = {
                    "choice": choice,
                    "usage": response.get("usage"),
                    "response": response,
                    "model": JEV_MODEL,
                    "reasoning_effort": None,
                }
                probability = answer.get("probabilities", {}).get(choice)
                if probability is not None:
                    if (
                        isinstance(probability, bool)
                        or not isinstance(probability, (int, float))
                        or not math.isfinite(probability)
                        or not 0 <= probability <= 1
                    ):
                        raise ValueError("Invalid selected probability")
                    result["probability"] = probability
            else:
                dump(
                    folder / "request.json",
                    {
                        "model": TERRA_MODEL,
                        "reasoning_effort": TERRA_EFFORT,
                        **payload,
                        "image_path": str(image_path) if image_path is not None else None,
                    },
                )
                schema_path = folder / "schema.json"
                dump(
                    schema_path,
                    {
                        "type": "object",
                        "properties": {
                            "target": {
                                "type": "string",
                                "enum": list(payload["questions"]["target"]["criteria"]),
                            }
                        },
                        "required": ["target"],
                        "additionalProperties": False,
                    },
                )
                run = run_codex_exec(
                    prompt="Do not use tools. Return only JSON matching the schema. "
                    "Use the supplied observed desktop and attached screenshot, if present.\n"
                    + json.dumps(payload),
                    artifact_dir=folder,
                    output_schema=schema_path,
                    model=TERRA_MODEL,
                    reasoning_effort=TERRA_EFFORT,
                    extra_env={"CODEX_HOME": str(self._terra_home())},
                    extra_args=[
                        "--sandbox",
                        "read-only",
                        # --image accepts multiple files; terminate options so
                        # the shared harness's trailing prompt stays positional.
                        *(["--image", str(image_path), "--"] if image_path is not None else []),
                    ],
                )
                if run.exit_code != 0 or not run.last_message_path.exists():
                    raise RuntimeError("Terra request failed; inspect retained trace")
                response = read_last_message(run.last_message_path)
                dump(folder / "response.json", response)
                choice = response["target"]
                events = [
                    json.loads(line)
                    for line in run.transcript_path.read_text().splitlines()
                    if line.strip()
                ]
                result = {
                    "choice": choice,
                    "usage": [e.get("usage") for e in events if e.get("type") == "turn.completed"],
                    "response": response,
                    "model": TERRA_MODEL,
                    "reasoning_effort": TERRA_EFFORT,
                }
            if (
                not isinstance(choice, str)
                or choice not in payload["questions"]["target"]["criteria"]
            ):
                raise ValueError("Judge returned an unobserved candidate ID")
            result["elapsed_ms"] = (time.perf_counter() - started) * 1000
            dump(folder / "result.json", result)
            return result
        except Exception as exc:
            # External exceptions may contain credentials or provider bodies.
            dump(folder / "error.json", {"error_type": type(exc).__name__})
            raise
