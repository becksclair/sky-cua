"""Pure projection and independent proof for the real TodoMVC browser pilot."""

from __future__ import annotations

import json
import math
from typing import Any

PAIR_COUNT = 3
MIN_SAVINGS = 0.2
PAGE_TEXT_LIMIT = 4000
PUBLIC_FIELDS = ("id", "label", "role", "context", "bounds", "disabled", "visible")


def page_text_for(snapshot: dict[str, Any]) -> str:
    """Observed page text, sent once per request in the judge state."""
    return (snapshot.get("text") or "")[:PAGE_TEXT_LIMIT]


def candidates_for(snapshot: dict[str, Any]) -> list[dict[str, Any]]:
    """Retain actual DOM semantics and refs, with no inferred roles or ancestry.

    Per-candidate context holds only candidate-specific facts (the link
    destination); shared page text goes once into the payload state.
    """
    candidates = []
    for element in snapshot["elements"]:
        bounds = dict(element.get("bounds") or {})
        href = element.get("href")
        candidates.append(
            {
                "id": f"c{element['index']}",
                "index": element["index"],
                "ref": element["ref"],
                "tag": element.get("tag"),
                "href": href,
                "label": element.get("name") or "",
                "role": element.get("role") or element.get("tag") or "",
                "context": f"href: {href}" if href else "",
                "bounds": bounds,
                "disabled": element.get("disabled", False),
                "visible": bounds.get("width", 0) > 0 and bounds.get("height", 0) > 0,
            }
        )
    return candidates


def public_candidates(candidates: list[dict[str, Any]]) -> list[dict[str, Any]]:
    """Return only judge payload fields, excluding action refs and oracle metadata."""
    return [{key: candidate[key] for key in PUBLIC_FIELDS} for candidate in candidates]


def target_correct(
    stage: str,
    candidate: dict[str, Any] | None,
    *,
    complete_label: str | None = None,
    complete_bounds: dict[str, Any] | None = None,
) -> bool:
    """Check the selected observed control against the caller's frozen stage oracle.

    Completion requires a confirmed nonempty label or independently observed DOM
    bounds. The caller requires a unique matching candidate before acting. Bounds
    are private oracle input, never an addition to the model's observed payload.
    """
    if candidate is None or candidate.get("disabled") or not candidate.get("visible"):
        return False
    if stage == "add":
        return candidate.get("label") == "New Todo Input" and candidate.get("tag") == "input"
    if stage == "complete":
        if candidate.get("tag") != "input" or not (complete_label or complete_bounds):
            return False
        if complete_label and candidate.get("label") != complete_label:
            return False
        if complete_bounds is not None:
            actual_bounds = candidate.get("bounds") or {}
            for key in ("x", "y", "width", "height"):
                expected = complete_bounds.get(key)
                actual = actual_bounds.get(key)
                if (
                    isinstance(expected, bool)
                    or not isinstance(expected, (int, float))
                    or not math.isfinite(expected)
                    or isinstance(actual, bool)
                    or not isinstance(actual, (int, float))
                    or not math.isfinite(actual)
                ):
                    return False
                if key in ("width", "height") and (expected <= 0 or actual <= 0):
                    return False
                if abs(actual - expected) > 1:
                    return False
        return True
    if stage == "filter_active":
        href = candidate.get("href")
        return (
            candidate.get("label") == "Active"
            and candidate.get("tag") == "a"
            and isinstance(href, str)
            and href.endswith("#/active")
        )
    raise ValueError(f"Unknown browser workflow stage: {stage}")


def _tasks_match(tasks: list[Any], expected_a: str, expected_b: str) -> bool:
    if not expected_a or not expected_b or expected_a == expected_b:
        raise ValueError("Expected task titles must be nonempty and distinct")
    return (
        len(tasks) == 2
        and all(
            isinstance(task, dict)
            and isinstance(task.get("title"), str)
            and isinstance(task.get("completed"), bool)
            for task in tasks
        )
        and sorted((task["title"], task["completed"]) for task in tasks)
        == sorted(((expected_a, True), (expected_b, False)))
    )


def workflow_proof(
    tasks_before_filter: list[dict[str, Any]],
    final_page: dict[str, Any],
    expected_a: str,
    expected_b: str,
) -> dict[str, Any]:
    """Verify actual DOM task state before filtering and the final active view.

    The caller reads task titles and checkbox checked booleans from the DOM before
    the completed task is hidden. This proves observed workflow state and makes
    no claim about persistence in this in-memory application.
    """
    tasks_match = _tasks_match(tasks_before_filter, expected_a, expected_b)
    hash_matches = final_page.get("hash") == "#/active"
    visible_tasks_match = final_page.get("visible_tasks") == [expected_b]
    return {
        "tasks_before_filter": tasks_before_filter,
        "tasks_match": tasks_match,
        "hash_matches": hash_matches,
        "visible_tasks_match": visible_tasks_match,
        "success": tasks_match and hash_matches and visible_tasks_match,
    }


def state_proof(
    storage: dict[str, str], page: dict[str, Any], expected_a: str, expected_b: str
) -> dict[str, Any]:
    """Require exact persisted tasks and the independently observed active view.

    Storage key names are not assumed. Arrays containing TodoMVC task fields are
    collected in full, so duplicates, extra tasks, and malformed task entries fail.
    Unrelated local-storage values are ignored.
    """
    if not expected_a or not expected_b or expected_a == expected_b:
        raise ValueError("Expected task titles must be nonempty and distinct")
    tasks: list[Any] = []
    storage_keys = []
    for key, raw in storage.items():
        try:
            value = json.loads(raw)
        except (TypeError, ValueError):
            continue
        if not isinstance(value, list) or not any(
            isinstance(task, dict) and ("title" in task or "completed" in task) for task in value
        ):
            continue
        storage_keys.append(key)
        tasks.extend(value)
    storage_matches = _tasks_match(tasks, expected_a, expected_b)
    hash_matches = page.get("hash") == "#/active"
    visible_tasks_match = page.get("visible_tasks") == [expected_b]
    return {
        "storage_keys": storage_keys,
        "tasks": tasks,
        "storage_matches": storage_matches,
        "hash_matches": hash_matches,
        "visible_tasks_match": visible_tasks_match,
        "success": storage_matches and hash_matches and visible_tasks_match,
    }


def paired_summary(records: list[dict[str, Any]]) -> dict[str, Any]:
    """Score three measured pairs using wall time, task proof, and actual calls."""
    if len(records) != 2 * PAIR_COUNT:
        raise ValueError("Unexpected measured workflow count")
    pairs = []
    for pair in range(PAIR_COUNT):
        members = [record for record in records if record.get("pair") == pair]
        indexed = {record["route"]: record for record in members}
        if len(members) != 2 or set(indexed) != {"baseline", "hybrid"}:
            raise ValueError(f"Incomplete, duplicate, or invalid pair {pair}")
        times = [indexed[route]["total_ms"] for route in ("baseline", "hybrid")]
        if any(
            isinstance(value, bool)
            or not isinstance(value, (int, float))
            or not math.isfinite(value)
            or value <= 0
            for value in times
        ):
            raise ValueError("Workflow timing must be finite and positive")
        baseline, hybrid = times
        pairs.append(
            {
                "pair": pair,
                "baseline_ms": baseline,
                "hybrid_ms": hybrid,
                "savings_fraction": 1 - hybrid / baseline,
            }
        )
    calls_by_route = {}
    for route in ("baseline", "hybrid"):
        calls = {}
        for field in ("terra_calls", "jev_calls", "wrong_targets"):
            values = [record[field] for record in records if record["route"] == route]
            if any(type(value) is not int or value < 0 for value in values):
                raise ValueError("Call and wrong-target counts must be nonnegative integers")
            calls[field] = sum(values)
        calls_by_route[route] = calls
    baseline_total = sum(pair["baseline_ms"] for pair in pairs)
    hybrid_total = sum(pair["hybrid_ms"] for pair in pairs)
    correct_tasks = sum(
        record["success"] is True and record["status"] == "ok" for record in records
    )
    all_correct = correct_tasks == len(records)
    wrong = sum(calls["wrong_targets"] for calls in calls_by_route.values())
    return {
        "pairs": pairs,
        "baseline_total_ms": baseline_total,
        "hybrid_total_ms": hybrid_total,
        "savings_fraction": 1 - hybrid_total / baseline_total,
        "calls_by_route": calls_by_route,
        "terra_calls": sum(calls["terra_calls"] for calls in calls_by_route.values()),
        "jev_calls": sum(calls["jev_calls"] for calls in calls_by_route.values()),
        "wrong_targets": wrong,
        "correct_tasks": correct_tasks,
        "total_tasks": len(records),
        "all_task_proofs_correct": all_correct,
        "passed": all_correct and wrong == 0 and hybrid_total <= baseline_total * (1 - MIN_SAVINGS),
    }
