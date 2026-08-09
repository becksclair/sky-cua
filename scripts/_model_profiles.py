#!/usr/bin/env python3
"""Validated access to the model defaults used by live test harnesses."""

from __future__ import annotations

import argparse
import json
from dataclasses import dataclass
from functools import cache
from pathlib import Path
from typing import cast

MODEL_PROFILES_PATH = Path(__file__).with_name("model_profiles.yaml")
_REASONING_EFFORTS = {"low", "medium", "high", "xhigh"}


@dataclass(frozen=True)
class ModelProfile:
    model: str
    reasoning_effort: str | None = None


def _nonempty_string(value: object, *, field: str) -> str:
    if not isinstance(value, str) or not value.strip():
        raise ValueError(f"{MODEL_PROFILES_PATH}: {field} must be a non-empty string")
    return value.strip()


@cache
def _load_harnesses() -> dict[str, dict[str, object]]:
    try:
        # JSON is a strict subset of YAML 1.2. Keeping this YAML source in flow
        # syntax lets plain-python VM harnesses use the standard library rather
        # than depending on a separately installed YAML package.
        raw = json.loads(MODEL_PROFILES_PATH.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        raise RuntimeError(
            f"failed to load model profiles from {MODEL_PROFILES_PATH}: {exc}"
        ) from exc
    if not isinstance(raw, dict) or raw.get("version") != 1:
        raise ValueError(f"{MODEL_PROFILES_PATH}: expected version: 1")
    harnesses = raw.get("harnesses")
    if not isinstance(harnesses, dict) or not harnesses:
        raise ValueError(f"{MODEL_PROFILES_PATH}: harnesses must be a non-empty mapping")

    validated: dict[str, dict[str, object]] = {}
    for raw_name, raw_profile in harnesses.items():
        name = _nonempty_string(raw_name, field="harness name")
        if not isinstance(raw_profile, dict):
            raise ValueError(f"{MODEL_PROFILES_PATH}: harnesses.{name} must be a mapping")
        profile = cast(dict[str, object], raw_profile)
        unknown = set(profile) - {"model", "reasoning_effort", "candidates"}
        if unknown:
            raise ValueError(
                f"{MODEL_PROFILES_PATH}: harnesses.{name} has unknown keys: {sorted(unknown)}"
            )
        has_model = "model" in profile
        has_candidates = "candidates" in profile
        if has_model == has_candidates:
            raise ValueError(
                f"{MODEL_PROFILES_PATH}: harnesses.{name} must define exactly one of model or candidates"
            )
        if has_model:
            validated_profile: dict[str, object] = {
                "model": _nonempty_string(profile["model"], field=f"harnesses.{name}.model")
            }
            effort = profile.get("reasoning_effort")
            if effort is not None:
                normalized_effort = _nonempty_string(
                    effort, field=f"harnesses.{name}.reasoning_effort"
                )
                if normalized_effort not in _REASONING_EFFORTS:
                    raise ValueError(
                        f"{MODEL_PROFILES_PATH}: harnesses.{name}.reasoning_effort must be one of "
                        f"{sorted(_REASONING_EFFORTS)}"
                    )
                validated_profile["reasoning_effort"] = normalized_effort
        else:
            candidates = profile["candidates"]
            if not isinstance(candidates, list) or not candidates:
                raise ValueError(
                    f"{MODEL_PROFILES_PATH}: harnesses.{name}.candidates must be a non-empty list"
                )
            normalized_candidates = [
                _nonempty_string(candidate, field=f"harnesses.{name}.candidates[{index}]")
                for index, candidate in enumerate(candidates)
            ]
            if "reasoning_effort" in profile:
                raise ValueError(
                    f"{MODEL_PROFILES_PATH}: harnesses.{name} candidates cannot set reasoning_effort"
                )
            validated_profile = {"candidates": normalized_candidates}
        if name in validated:
            raise ValueError(f"{MODEL_PROFILES_PATH}: duplicate normalized harness name: {name}")
        validated[name] = validated_profile
    return validated


def model_profile(harness: str) -> ModelProfile:
    try:
        raw = _load_harnesses()[harness]
    except KeyError as exc:
        raise KeyError(f"unknown model harness profile: {harness}") from exc
    if "model" not in raw:
        raise ValueError(f"model harness profile {harness!r} defines candidates, not one model")
    effort = raw.get("reasoning_effort")
    return ModelProfile(
        model=cast(str, raw["model"]),
        reasoning_effort=cast(str | None, effort),
    )


def model_candidates(harness: str) -> tuple[str, ...]:
    try:
        raw = _load_harnesses()[harness]
    except KeyError as exc:
        raise KeyError(f"unknown model harness profile: {harness}") from exc
    candidates = raw.get("candidates")
    if not isinstance(candidates, list):
        raise ValueError(f"model harness profile {harness!r} defines one model, not candidates")
    return tuple(cast(list[str], candidates))


def required_reasoning_effort(harness: str) -> str:
    effort = model_profile(harness).reasoning_effort
    if effort is None:
        raise ValueError(f"model harness profile {harness!r} requires reasoning_effort")
    return effort


def _main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("harness")
    parser.add_argument("--field", choices=("model", "reasoning_effort"), default="model")
    args = parser.parse_args()
    profile = model_profile(args.harness)
    value = getattr(profile, args.field)
    if value is None:
        parser.error(f"profile {args.harness!r} has no {args.field}")
    print(value)
    return 0


if __name__ == "__main__":
    raise SystemExit(_main())
