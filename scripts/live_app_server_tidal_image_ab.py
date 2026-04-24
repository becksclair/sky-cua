#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
from dataclasses import dataclass
from datetime import UTC, datetime
from pathlib import Path
from typing import Any, Literal

from _plugin_bundle import REPO_ROOT
from _tidal_workflow import TidalAppServerWorkflowFailure, run_tidal_app_server_workflow


@dataclass(frozen=True)
class Variant:
    name: str
    image_format: Literal["jpeg", "webp"]
    quality: int


DEFAULT_VARIANTS = (
    Variant("jpeg-q85", "jpeg", 85),
    Variant("webp-q80", "webp", 80),
    Variant("webp-q85", "webp", 85),
    Variant("webp-q95", "webp", 95),
)
DEFAULT_VARIANT_NAMES = ("jpeg-q85", "webp-q85")


def load_json(path: Path) -> dict[str, Any]:
    if not path.exists():
        return {}
    return json.loads(path.read_text())


def screenshot_stat(message: dict[str, Any]) -> dict[str, Any]:
    screenshot_path = message.get("screenshot_path")
    if not isinstance(screenshot_path, str):
        return {}
    path = Path(screenshot_path)
    stat: dict[str, Any] = {
        "screenshot_path": screenshot_path,
        "extension": path.suffix.lstrip("."),
    }
    if path.exists():
        stat["bytes"] = path.stat().st_size
    return stat


def playlist_name_for_variant(run_id: str, variant: Variant) -> str:
    return f"Codex Favorites AB {run_id} {variant.name}"


def run_variant(variant: Variant, *, model: str | None, run_id: str) -> dict[str, Any]:
    exit_code = 0
    error: str | None = None
    playlist_name = playlist_name_for_variant(run_id, variant)
    try:
        result, message = run_tidal_app_server_workflow(
            model=model,
            playlist_name=playlist_name,
            image_format=variant.image_format,
            jpeg_quality=variant.quality if variant.image_format == "jpeg" else None,
            webp_quality=variant.quality if variant.image_format == "webp" else None,
        )
    except TidalAppServerWorkflowFailure as exc:
        exit_code = 1
        error = str(exc)
        result = exc.result
        message = exc.final_message or {}
    artifact_dir = str(result.artifact_dir) if result else None
    timing = load_json(result.timing_summary_path) if result else {}
    return {
        "variant": variant.name,
        "playlist_name": playlist_name,
        "image_format": variant.image_format,
        "quality": variant.quality,
        "exit_code": exit_code,
        "artifact_dir": artifact_dir,
        "error": error,
        "status": message.get("status"),
        "playlist_action": message.get("playlist_action"),
        "screenshot": screenshot_stat(message),
        "elapsed_ms": timing.get("elapsed_ms"),
        "completed_mcp_tool_calls": timing.get("completed_mcp_tool_calls"),
        "mcp_tool_duration_total_ms": timing.get("mcp_tool_duration_total_ms"),
        "last_uncached_input_tokens": timing.get("last_uncached_input_tokens"),
        "max_uncached_input_tokens": timing.get("max_uncached_input_tokens"),
        "avg_uncached_input_tokens": timing.get("avg_uncached_input_tokens"),
    }


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Run TIDAL app-server workflow variants for screenshot image-format A/B profiling."
    )
    parser.add_argument("--model", help="Codex model override passed to each workflow run.")
    parser.add_argument(
        "--variants",
        nargs="+",
        choices=[variant.name for variant in DEFAULT_VARIANTS],
        default=list(DEFAULT_VARIANT_NAMES),
        help="Variant names to run in order.",
    )
    parser.add_argument(
        "--all-variants",
        action="store_true",
        help="Run the full configured sweep instead of the default JPEG/WebP pair.",
    )
    parser.add_argument(
        "--output",
        default=str(REPO_ROOT / "artifacts" / "codex-e2e" / "tidal-image-ab-summary.json"),
        help="Path for the JSON summary.",
    )
    args = parser.parse_args()

    variant_names = (
        [variant.name for variant in DEFAULT_VARIANTS] if args.all_variants else args.variants
    )
    variants_by_name = {variant.name: variant for variant in DEFAULT_VARIANTS}
    run_id = datetime.now(UTC).strftime("%Y%m%dT%H%M%SZ")
    results = [
        run_variant(variants_by_name[name], model=args.model, run_id=run_id)
        for name in variant_names
    ]
    output_path = Path(args.output)
    output_path.parent.mkdir(parents=True, exist_ok=True)
    output_path.write_text(json.dumps({"run_id": run_id, "results": results}, indent=2))
    print(f"tidal image A/B summary: {output_path}")
    print(json.dumps({"run_id": run_id, "results": results}, indent=2))
    return 1 if any(result["exit_code"] != 0 for result in results) else 0


if __name__ == "__main__":
    raise SystemExit(main())
