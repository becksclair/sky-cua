"""Regression tests for project skill prompts and app-guidance projections."""

from __future__ import annotations

import ast
import json
import re
from pathlib import Path

import pytest

from _plugin_bundle import SKY_CUA_SKILLS

REPO_ROOT = Path(__file__).resolve().parents[1]
SKILLS_ROOT = REPO_ROOT / "skills"
LOCAL_SKILLS_ROOT = REPO_ROOT / ".agents" / "skills"
LOCAL_SKILL_NAMES = (
    "agent-cursor-debug",
    "cua-deploy",
    "overlay-pointer-animations",
    "vm-tests",
)
PROJECT_SKILLS = tuple(
    (SKILLS_ROOT / skill_name, skill_name) for skill_name in SKY_CUA_SKILLS
) + tuple((LOCAL_SKILLS_ROOT / skill_name, skill_name) for skill_name in LOCAL_SKILL_NAMES)
APP_INSTRUCTIONS_ROOT = REPO_ROOT / "resources" / "app-instructions"
APP_REFERENCES_ROOT = SKILLS_ROOT / "computer-use" / "references" / "apps"


def _parse_block_scalar(
    lines: list[str], start: int, end: int, path: Path, marker: str
) -> tuple[str, int]:
    match = re.fullmatch(r"(?P<style>[|>])(?P<indent>[1-9]?)(?P<chomp>[-+]?)", marker)
    assert match is not None, f"{path} has an unsupported block scalar marker: {marker!r}"

    raw_lines: list[str] = []
    index = start
    while index < end:
        line = lines[index]
        if line.strip() and not line.startswith(" "):
            break
        raw_lines.append(line)
        index += 1

    non_empty_indents = [len(line) - len(line.lstrip(" ")) for line in raw_lines if line.strip()]
    indentation = int(match.group("indent") or 0)
    if indentation == 0 and non_empty_indents:
        indentation = min(non_empty_indents)

    content_lines: list[str] = []
    for line in raw_lines:
        if not line.strip():
            content_lines.append("")
            continue
        actual_indent = len(line) - len(line.lstrip(" "))
        assert actual_indent >= indentation > 0, (
            f"{path} has invalid indentation in its {marker!r} frontmatter block"
        )
        content_lines.append(line[indentation:])

    if match.group("style") == "|":
        value = "\n".join(content_lines)
    else:
        folded: list[str] = []
        for line_number, line in enumerate(content_lines):
            folded.append(line)
            if line_number == len(content_lines) - 1:
                continue
            folded.append(" " if line and content_lines[line_number + 1] else "\n")
        value = "".join(folded)

    chomp = match.group("chomp")
    if chomp == "-":
        value = value.rstrip("\n")
    elif chomp != "+":
        value = value.rstrip("\n") + ("\n" if content_lines else "")
    return value, index


def _parse_skill_frontmatter(path: Path) -> dict[str, str]:
    lines = path.read_text(encoding="utf-8").splitlines()
    assert lines and lines[0] == "---", f"{path} must start with YAML frontmatter"
    try:
        closing_marker = lines.index("---", 1)
    except ValueError:
        raise AssertionError(f"{path} has unterminated YAML frontmatter") from None

    metadata: dict[str, str] = {}
    index = 1
    while index < closing_marker:
        line = lines[index]
        if not line.strip():
            index += 1
            continue
        key, separator, raw_value = line.partition(":")
        assert separator and key.strip(), f"{path} has an invalid frontmatter line: {line!r}"
        key = key.strip()
        assert key not in metadata, f"{path} repeats frontmatter key {key!r}"
        raw_value = raw_value.strip()
        assert raw_value, f"{path} has an empty frontmatter value for {key!r}"

        if raw_value[0] in "|>":
            value, index = _parse_block_scalar(lines, index + 1, closing_marker, path, raw_value)
            metadata[key] = value
            continue
        if raw_value[0] in {'"', "'"}:
            try:
                value = ast.literal_eval(raw_value)
            except (SyntaxError, ValueError) as exc:
                raise AssertionError(
                    f"{path} has an invalid quoted frontmatter value for {key!r}"
                ) from exc
            assert isinstance(value, str), f"{path} has a non-string value for {key!r}"
        else:
            value = raw_value
        metadata[key] = value
        index += 1
    return metadata


def _without_blank_lines(text: str) -> str:
    return "\n".join(line for line in text.splitlines() if line.strip())


def _markdown_projection(root: Path, excluded_names: set[str]) -> dict[str, str]:
    return {
        path.name: _without_blank_lines(path.read_text(encoding="utf-8"))
        for path in root.glob("*.md")
        if path.name not in excluded_names
    }


@pytest.mark.parametrize(
    ("skill_root", "skill_name"),
    PROJECT_SKILLS,
    ids=[skill_name for _skill_root, skill_name in PROJECT_SKILLS],
)
def test_project_skill_prompt_assets_are_valid(skill_root: Path, skill_name: str) -> None:
    frontmatter = _parse_skill_frontmatter(skill_root / "SKILL.md")
    assert frontmatter["name"] == skill_name
    assert frontmatter.get("description", "").strip()

    trigger_evals = json.loads((skill_root / "trigger-evals.json").read_text(encoding="utf-8"))
    assert isinstance(trigger_evals, list)
    assert len(trigger_evals) == 20

    queries: list[str] = []
    trigger_counts = {True: 0, False: 0}
    for evaluation in trigger_evals:
        assert isinstance(evaluation, dict)
        query = evaluation.get("query")
        assert isinstance(query, str) and query.strip()
        queries.append(query.strip())
        should_trigger = evaluation.get("should_trigger")
        assert type(should_trigger) is bool
        trigger_counts[should_trigger] += 1

    assert len(queries) == len(set(queries))
    assert trigger_counts == {True: 10, False: 10}

    eval_file = skill_root / "evals" / "evals.json"
    eval_payload = json.loads(eval_file.read_text(encoding="utf-8"))
    assert isinstance(eval_payload, dict)
    assert eval_payload.get("skill_name") == skill_name
    evals = eval_payload.get("evals")
    assert isinstance(evals, list) and evals

    ids: list[object] = []
    for evaluation in evals:
        assert isinstance(evaluation, dict)
        evaluation_id = evaluation.get("id")
        assert evaluation_id is not None
        assert evaluation_id not in ids
        ids.append(evaluation_id)

        prompt = evaluation.get("prompt")
        expected_output = evaluation.get("expected_output")
        assert isinstance(prompt, str) and prompt.strip()
        assert isinstance(expected_output, str) and expected_output.strip()

        expectations = evaluation.get("expectations")
        assert isinstance(expectations, list) and len(expectations) >= 3
        assert all(
            isinstance(expectation, str) and expectation.strip() for expectation in expectations
        )


def test_app_instruction_projections_are_in_sync() -> None:
    resource_index = json.loads((APP_INSTRUCTIONS_ROOT / "index.json").read_text(encoding="utf-8"))
    reference_index = json.loads((APP_REFERENCES_ROOT / "index.json").read_text(encoding="utf-8"))
    assert resource_index == reference_index

    assert _markdown_projection(APP_INSTRUCTIONS_ROOT, {"AGENTS.md"}) == _markdown_projection(
        APP_REFERENCES_ROOT, {"README.md"}
    )
