"""Frozen synthetic text fixtures for the offline Jev/Luna comparison.

These are constructed application models, not recordings, real smoke replays,
or evidence of browser/desktop/phone integration. Private fixture state defines
what the simulated application did; observation completeness and consistency
separately define whether a classifier may conclude that from the supplied UI.
Only ``state`` is model input. IDs, expected labels, baseline predictions and
``ground_truth`` must remain outside prompts. Synthetic wording and deliberately
bounded evidence make this a feasibility test, not an estimate of live accuracy.
No fixtures are mined from model outputs and this module makes no network calls.
"""

from __future__ import annotations

import random
from dataclasses import dataclass
from typing import Any


@dataclass(frozen=True)
class _Family:
    name: str
    title: str
    field: str
    original: str
    requested: str
    success: str
    rejection: str
    progress: str


_FAMILIES = (
    _Family(
        "profile",
        "Account profile",
        "Display name",
        "Aster",
        "Birch",
        "Profile updated.",
        "Profile update rejected.",
        "Updating profile…",
    ),
    _Family(
        "notifications",
        "Notification preferences",
        "Delivery",
        "Daily",
        "Weekly",
        "Preferences saved.",
        "Preferences could not be saved.",
        "Saving preferences…",
    ),
    _Family(
        "invoice",
        "Invoice INV-42",
        "Reference",
        "REF-OLD",
        "REF-NEW",
        "Invoice changes recorded.",
        "Invoice update declined.",
        "Recording invoice changes…",
    ),
    _Family(
        "issue",
        "Issue 42",
        "Title",
        "Old title",
        "New title",
        "Issue updated.",
        "Issue update failed.",
        "Updating issue…",
    ),
    _Family(
        "calendar",
        "Planning meeting",
        "Location",
        "Room A",
        "Room B",
        "Event changes saved.",
        "Event changes rejected.",
        "Saving event…",
    ),
    _Family(
        "publishing",
        "Draft article",
        "Headline",
        "First draft",
        "Final headline",
        "Draft stored.",
        "Draft was not stored.",
        "Storing draft…",
    ),
    _Family(
        "roster",
        "Team member",
        "Role",
        "Reader",
        "Editor",
        "Member role updated.",
        "Role change denied.",
        "Applying role change…",
    ),
    _Family(
        "files",
        "Document properties",
        "Name",
        "draft.txt",
        "final.txt",
        "File renamed.",
        "Rename failed.",
        "Renaming file…",
    ),
)

_SUCCESS = frozenset(f.success for f in _FAMILIES)
_REJECTION = frozenset(f.rejection for f in _FAMILIES)
_PROGRESS = frozenset(f.progress for f in _FAMILIES) | {
    "Waiting for connection. Changes are queued."
}


def _split(index: int) -> str:
    return "development" if index < 2 else "heldout"


def _snapshot(family: _Family, value: str, *, record: bool = False) -> dict[str, Any]:
    return {
        "title": family.title,
        "observation": "complete",
        "text": [],
        "fields": [{"label": family.field, "value": value, "read_only": record}],
        "buttons": [{"label": "Save", "enabled": not record}],
        "view": "record details" if record else "edit form",
    }


def _outcome_truth(private: dict[str, Any]) -> str:
    if not private["evidence_sufficient"]:
        return "insufficient"
    return {
        "committed": "saved",
        "rejected": "rejected",
        "in_flight": "pending",
        "queued": "pending",
    }[private["request_state"]]


def _outcome_rules(state: dict[str, Any]) -> str:
    before, after = state["before"], state["after"]
    if after["observation"] != "complete":
        return "insufficient"
    lines = set(after["text"])
    success, rejection = bool(lines & _SUCCESS), bool(lines & _REJECTION)
    if success and rejection:
        return "insufficient"
    intent = state["intent"]
    readback = any(
        field["label"] == intent["field"]
        and field["value"] == intent["value"]
        and field["read_only"]
        for field in after["fields"]
    )
    if readback and not rejection:
        return "saved"
    if success and lines & set(before["text"]):
        return "insufficient"
    if rejection or any(
        error["field"] == intent["field"] for error in after.get("field_errors", [])
    ):
        return "rejected"
    if lines & _PROGRESS:
        return "pending"
    return "insufficient"


def _outcome_cases() -> list[dict[str, Any]]:
    cases = []
    for family_index, family in enumerate(_FAMILIES):
        for condition in range(8):
            before = _snapshot(family, family.original)
            after = _snapshot(family, family.requested)
            actions: list[dict[str, Any]] = [
                {
                    "tool": "click",
                    "target": "Save",
                    "result": {"ok": True, "input_dispatched": True},
                }
            ]
            private: dict[str, Any] = {
                "stored_before": family.original,
                "stored_after": family.original,
                "request_state": "rejected",
                "evidence_sufficient": True,
                "synthetic": True,
            }
            if condition in (0, 1):
                private.update(stored_after=family.requested, request_state="committed")
                after = _snapshot(family, family.requested, record=True)
                if condition == 0:
                    after["text"] = [family.success]
                else:
                    actions.append(
                        {"tool": "open", "target": "record details", "result": {"ok": True}}
                    )
            elif condition == 2:
                after["field_errors"] = [
                    {"field": family.field, "text": "This value is not allowed."}
                ]
                after["text"] = ["Correct the highlighted field before continuing."]
            elif condition == 3:
                after["text"] = [family.rejection]
            elif condition == 4:
                private["request_state"] = "in_flight"
                after["text"] = [family.progress]
                after["buttons"][0]["enabled"] = False
                after["progress_indicator"] = "visible"
            elif condition == 5:
                private["request_state"] = "queued"
                after["text"] = ["Waiting for connection. Changes are queued."]
            elif condition == 6:
                private.update(request_state="not_submitted", evidence_sufficient=False)
                if family_index % 2:
                    before["text"] = [family.success]
                    after["text"] = [family.success]
                    private["evidence_issue"] = "success banner predates this click"
                else:
                    private["evidence_issue"] = (
                        "input dispatch acknowledged without application response"
                    )
            else:
                private.update(
                    request_state="committed",
                    stored_after=family.requested,
                    evidence_sufficient=False,
                )
                variant = family_index % 3
                if variant == 0:
                    after["text"] = [family.success, family.rejection]
                    private["evidence_issue"] = "mutually contradictory current messages"
                elif variant == 1:
                    after = {"observation": "unavailable", "text": [], "fields": []}
                    private["evidence_issue"] = "post-action observation unavailable"
                else:
                    after["observation"] = "truncated"
                    after["text"] = [family.success[:12]]
                    after["fields"] = []
                    private["evidence_issue"] = "post-action observation truncated"
            state = {
                "intent": {"action": "save", "field": family.field, "value": family.requested},
                "before": before,
                "actions": actions,
                "after": after,
            }
            cases.append(
                {
                    "id": f"outcome-{family_index + 1:02d}-{condition + 1:02d}",
                    "family": family.name,
                    "split": _split(family_index),
                    "task": "outcome",
                    "state": state,
                    "expected": _outcome_truth(private),
                    "rules_prediction": _outcome_rules(state),
                    "ground_truth": private,
                }
            )
    return cases


def _ranking_rules(state: dict[str, Any]) -> str:
    intent = state["intent"]
    matches = [
        candidate["id"]
        for candidate in state["candidates"]
        if candidate["enabled"]
        and candidate["role"] == intent["role"]
        and candidate["label"] == intent["label"]
        and candidate["context"] == intent["context"]
    ]
    return matches[0] if len(matches) == 1 else "abstain"


def _ranking_cases() -> list[dict[str, Any]]:
    rng = random.Random(481516)
    cases = []
    contexts = ("Primary record", "Archived record", "Related record", "Preview pane")
    for family_index, family in enumerate(_FAMILIES):
        for scenario in range(3):
            # Logical widget keys belong only to the fixture oracle, never the snapshot.
            widgets = [
                {
                    "key": "target",
                    "role": "button",
                    "label": "Edit",
                    "context": contexts[0],
                    "enabled": True,
                },
                {
                    "key": "archive",
                    "role": "button",
                    "label": "Edit",
                    "context": contexts[1],
                    "enabled": True,
                },
                {
                    "key": "related",
                    "role": "button",
                    "label": "Edit",
                    "context": contexts[2],
                    "enabled": True,
                },
                {
                    "key": "preview",
                    "role": "link",
                    "label": "Edit",
                    "context": contexts[0],
                    "enabled": True,
                },
                {
                    "key": "dismiss",
                    "role": "button",
                    "label": "Cancel",
                    "context": contexts[0],
                    "enabled": True,
                },
            ]
            intended_keys = ["target"]
            request_kind = "unique_target"
            intent = {
                "action": "choose one control",
                "role": "button",
                "label": "Edit",
                "context": contexts[0],
            }
            if scenario == 1:
                intent.update(
                    action="dismiss the primary record editor without saving its changes",
                    label="dismiss without saving",
                )
                widgets[0]["label"] = "Discard edits"
                widgets[1]["label"] = "Discard edits"
                widgets[2]["label"] = "Keep editing"
                widgets[4]["label"] = "Save and exit"
                widgets.append(
                    {
                        "key": "disabled",
                        "role": "button",
                        "label": "Dismiss without saving",
                        "context": contexts[0],
                        "enabled": False,
                    }
                )
            elif scenario == 2:
                if family_index % 2:
                    widgets.append(
                        {
                            "key": "duplicate",
                            "role": "button",
                            "label": "Edit",
                            "context": contexts[0],
                            "enabled": True,
                        }
                    )
                    intended_keys = ["target", "duplicate"]
                    request_kind = "multi_target"
                    intent["action"] = "choose both Edit buttons for the primary record"
                else:
                    widgets[0]["enabled"] = False
                    intended_keys = []
                    request_kind = "no_match"
            # Include longer candidate lists in the held-out families.
            for distractor in range(family_index * 2):
                widgets.append(
                    {
                        "key": f"extra-{distractor}",
                        "role": "button",
                        "label": "Edit",
                        "context": f"Other record {distractor + 1}",
                        "enabled": True,
                    }
                )
            rng.shuffle(widgets)
            ids = rng.sample(range(10000, 99999), len(widgets))
            key_to_id = {
                widget["key"]: f"c{opaque}" for widget, opaque in zip(widgets, ids, strict=True)
            }
            candidates = [
                {
                    "id": key_to_id[widget["key"]],
                    **{key: value for key, value in widget.items() if key != "key"},
                }
                for widget in widgets
            ]
            target_ids = [key_to_id[key] for key in intended_keys]
            state = {
                "title": family.title,
                "intent": intent,
                "candidates": candidates,
            }
            cases.append(
                {
                    "id": f"ranking-{family_index + 1:02d}-{scenario + 1:02d}",
                    "family": family.name,
                    "split": _split(family_index),
                    "task": "ranking",
                    "state": state,
                    "expected": target_ids[0] if len(target_ids) == 1 else "abstain",
                    "rules_prediction": _ranking_rules(state),
                    "ground_truth": {
                        "synthetic": True,
                        "request_kind": request_kind,
                        "intended_widget_keys": intended_keys,
                        "eligible_target_ids": target_ids,
                        "widget_ids": key_to_id,
                    },
                }
            )
    return cases


def _annotation_rules(state: dict[str, Any]) -> str:
    episode = state["episode"]
    final = episode[-1]
    result = final["result"]
    if state["claim"]["kind"] == "observation_recovered":
        if final["tool"] == "observe" and not result["ok"]:
            return "contradicted"
        payload = result.get("payload")
        if (
            payload
            and payload.get("observation") == "complete"
            and payload.get("title") == state["claim"]["target"]
        ):
            return "supported"
        return "insufficient"
    payload = result.get("payload")
    if not payload or payload.get("observation") != "complete":
        return "insufficient"
    if payload.get("message") == "Update rejected.":
        return "contradicted"
    if payload.get("view") == "record details" and payload.get("value") == state["claim"]["value"]:
        return "supported"
    return "insufficient"


def _annotation_cases() -> list[dict[str, Any]]:
    cases = []
    for family_index, kind in enumerate(("completion", "completion", "recovery", "recovery")):
        target = ("Budget note", "Release note", "Issue board", "Team roster")[family_index]
        for condition in range(3):
            private = {
                "synthetic": True,
                "source": "constructed bounded episode; not a real smoke replay",
                "evidence_sufficient": condition != 2,
                "claim_true": condition == 0,
            }
            if kind == "completion":
                claim = {
                    "kind": "save_completed",
                    "target": target,
                    "value": "Reviewed",
                    "text": f"The {target} change to Reviewed was saved.",
                }
                episode = [
                    {
                        "tool": "click",
                        "target": "Save",
                        "result": {"ok": True, "input_dispatched": True},
                    }
                ]
                if condition == 0:
                    result = {
                        "ok": True,
                        "payload": {
                            "title": target,
                            "observation": "complete",
                            "view": "record details",
                            "value": "Reviewed",
                        },
                    }
                elif condition == 1:
                    result = {
                        "ok": True,
                        "payload": {
                            "title": target,
                            "observation": "complete",
                            "view": "edit form",
                            "value": "Original",
                            "message": "Update rejected.",
                        },
                    }
                else:
                    result = {
                        "ok": True,
                        "payload": {"title": target, "observation": "truncated", "text": "Upda…"},
                    }
                episode.append({"tool": "observe", "result": result})
            else:
                claim = {
                    "kind": "observation_recovered",
                    "target": target,
                    "text": f"Observation recovered and returned the {target} UI.",
                }
                episode = [
                    {"tool": "observe", "result": {"ok": False, "error": "STALE_REFERENCE"}},
                    {"tool": "refresh", "result": {"ok": True}},
                ]
                if condition == 0:
                    result = {
                        "ok": True,
                        "payload": {
                            "title": target,
                            "observation": "complete",
                            "text": ["Overview", "Members", "Settings"],
                            "controls": [{"role": "button", "label": "Add"}],
                        },
                    }
                elif condition == 1:
                    result = {"ok": False, "error": "OBSERVATION_UNAVAILABLE"}
                else:
                    # A successful transport status with the observation payload omitted
                    # cannot establish recovery. This models lossy judge condensation.
                    result = {"ok": True, "payload_omitted": True}
                episode.append({"tool": "observe", "result": result})
            state = {
                "source": "constructed synthetic episode, not a real smoke replay",
                "claim": claim,
                "episode": episode,
            }
            expected = (
                "insufficient"
                if not private["evidence_sufficient"]
                else ("supported" if private["claim_true"] else "contradicted")
            )
            cases.append(
                {
                    "id": f"annotation-{family_index + 1:02d}-{condition + 1:02d}",
                    "family": f"synthetic_{kind}_{family_index + 1}",
                    "split": "development" if family_index in (0, 2) else "heldout",
                    "task": "annotation",
                    "state": state,
                    "expected": expected,
                    "rules_prediction": _annotation_rules(state),
                    "ground_truth": private,
                }
            )
    return cases


def build_cases() -> list[dict[str, Any]]:
    """Return fresh deterministic cases; callers must prompt with ``state`` only."""
    return _outcome_cases() + _ranking_cases() + _annotation_cases()
