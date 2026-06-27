"""Deterministic tool-use coverage and no-error gate for the codex CUA smoke.

The codex CUA smoke drives a single agent run that must exercise every
computer-use and browser-use tool. These helpers parse the codex transcript's
``mcp_tool_call`` items (as returned by ``_codex_exec.transcript_mcp_tool_calls``)
and prove, without an LLM, that:

- every required grouped tool was called at least once,
- the grouped tools that take an ``operation``/``surface`` were called with each
  required value (e.g. ``desktop_pointer`` with click/secondary_click/drag), and
- no tool call returned an error.

This is the objective gate that runs in the VM before the qualitative host
judge. Tool names are matched on their bare grouped name, tolerating the
``mcp__computer_use__`` / server namespacing codex applies.
"""

from __future__ import annotations

import json
from collections.abc import Iterable, Mapping
from dataclasses import dataclass, field
from typing import Any

# Bare grouped tool names every codex CUA run must exercise. Desktop names come
# from the computer-use surface; browser names from the browser-use surface.
REQUIRED_DESKTOP_TOOLS: frozenset[str] = frozenset(
    {
        "observe",
        "capture_desktop",
        "activate_window",
        "desktop_pointer",
        "desktop_keyboard",
        "desktop_semantic",
        "desktop_action",
        "desktop_set_value",
        "desktop_scroll",
    }
)
REQUIRED_BROWSER_TOOLS: frozenset[str] = frozenset(
    {
        "browser_open",
        "browser_navigate",
        "browser_claim_tab",
        "browser_move_mouse",
        "browser_input",
        "browser_scroll",
        "capture_screen",
    }
)
REQUIRED_TOOLS: frozenset[str] = REQUIRED_DESKTOP_TOOLS | REQUIRED_BROWSER_TOOLS

# Grouped tools dispatch on an ``operation``; require each branch was exercised.
# Spellings are authoritative against crates/.../tool_contract.json.
REQUIRED_OPERATIONS: dict[str, frozenset[str]] = {
    "desktop_pointer": frozenset({"click", "secondary_click", "drag"}),
    "desktop_keyboard": frozenset({"type_text", "press_key"}),
    "browser_input": frozenset({"click", "type_text", "press_key"}),
}

# Tools that take a ``surface``; require the listed surfaces were exercised.
# ``observe`` covers desktop implicitly by appearing in REQUIRED_DESKTOP_TOOLS;
# the browser surface must be proven explicitly. Desktop screenshots use
# ``capture_desktop``; ``capture_screen`` is the browser/phone screenshot tool.
REQUIRED_SURFACES: dict[str, frozenset[str]] = {
    "observe": frozenset({"browser"}),
    "capture_screen": frozenset({"browser"}),
}

FAILURE_STATUSES: frozenset[str] = frozenset(
    {"canceled", "cancelled", "error", "failed", "failure", "timeout"}
)

_TOOL_NAME_PREFIXES: tuple[str, ...] = (
    "mcp__computer_use__",
    "mcp__sky-cua__",
    "mcp__sky_cua__",
    "sky_cua_",
    "sky-cua_",
    "computer-use_",
    "computer_use_",
)


def bare_tool_name(name: str) -> str:
    """Strip an MCP/server namespace prefix to the bare grouped tool name."""
    token = name.strip()
    if token.startswith("mcp__") and "__" in token[len("mcp__") :]:
        # mcp__<server>__<tool> -> <tool>
        token = token.split("__", 2)[-1]
    for prefix in _TOOL_NAME_PREFIXES:
        if token.startswith(prefix):
            return token[len(prefix) :]
    return token


def _tool_name_of(item: Mapping[str, Any]) -> str | None:
    for key in ("tool", "tool_name", "toolName", "name"):
        value = item.get(key)
        if isinstance(value, str) and value:
            return bare_tool_name(value)
    return None


def _arguments_of(item: Mapping[str, Any]) -> dict[str, Any]:
    raw = item.get("arguments")
    if raw is None:
        raw = item.get("args")
    if isinstance(raw, str):
        try:
            raw = json.loads(raw)
        except json.JSONDecodeError:
            return {}
    return raw if isinstance(raw, dict) else {}


def _call_id_of(item: Mapping[str, Any]) -> str | None:
    for key in ("id", "call_id", "callId", "tool_call_id", "toolCallId"):
        value = item.get(key)
        if isinstance(value, str) and value:
            return value
    return None


@dataclass
class _MergedCall:
    tool: str
    arguments: dict[str, Any] = field(default_factory=dict)
    failed: bool = False
    error: str = ""


def _merge_calls(calls: Iterable[Mapping[str, Any]]) -> list[_MergedCall]:
    """Merge paired started/completed items into one record per call id.

    codex emits ``item.started`` (carrying ``arguments``) and ``item.completed``
    (carrying status/result) for the same call id. Coverage needs the arguments
    from the started item and the failure signal from the completed item, so the
    two are unioned rather than letting the later event clobber the earlier.
    """
    order: list[str] = []
    by_id: dict[str, _MergedCall] = {}
    anonymous: list[_MergedCall] = []
    for item in calls:
        tool = _tool_name_of(item)
        if tool is None:
            continue
        args = _arguments_of(item)
        failed = call_failed(item)
        error = _error_excerpt(item) if failed else ""
        call_id = _call_id_of(item)
        if call_id is None:
            anonymous.append(
                _MergedCall(tool=tool, arguments=dict(args), failed=failed, error=error)
            )
            continue
        record = by_id.get(call_id)
        if record is None:
            by_id[call_id] = _MergedCall(
                tool=tool, arguments=dict(args), failed=failed, error=error
            )
            order.append(call_id)
            continue
        record.arguments.update(args)
        if failed:
            record.failed = True
            record.error = record.error or error
    return [by_id[cid] for cid in order] + anonymous


def call_failed(item: Mapping[str, Any]) -> bool:
    """True when a tool-call item declares failure anywhere in its envelope."""
    if item.get("is_error") is True or item.get("isError") is True:
        return True
    error = item.get("error")
    if isinstance(error, str) and error.strip():
        return True
    if isinstance(error, (dict, list)) and error:
        return True
    for key in ("status", "phase", "state"):
        value = item.get(key)
        if isinstance(value, str) and value.lower() in FAILURE_STATUSES:
            return True
    for key in ("result", "output", "content", "structuredContent", "structured_content"):
        nested = item.get(key)
        if isinstance(nested, dict) and call_failed(nested):
            return True
        if isinstance(nested, list) and any(
            isinstance(entry, dict) and call_failed(entry) for entry in nested
        ):
            return True
    return False


def _error_excerpt(item: Mapping[str, Any], limit: int = 300) -> str:
    error = item.get("error")
    if isinstance(error, str) and error.strip():
        return error.strip()[:limit]
    text = json.dumps(item, default=str)
    return text[:limit]


@dataclass
class CoverageReport:
    tools_seen: dict[str, int] = field(default_factory=dict)
    operations_seen: dict[str, list[str]] = field(default_factory=dict)
    surfaces_seen: dict[str, list[str]] = field(default_factory=dict)
    errors: list[dict[str, Any]] = field(default_factory=list)
    missing_tools: list[str] = field(default_factory=list)
    missing_operations: list[str] = field(default_factory=list)
    missing_surfaces: list[str] = field(default_factory=list)

    @property
    def unrecovered_errors(self) -> list[dict[str, Any]]:
        """Errors whose ``(tool, operation)`` never succeeded later in the run.

        A transient error followed by a successful retry of the same operation
        is the recovery behavior the smoke wants to exercise, not a failure;
        every consequential failure still surfaces as a missing tool, operation,
        or surface, or as an unrecovered error here.

        Granularity ceiling: recovery is keyed on ``(tool, operation/surface)``,
        not on call arguments. For argument-distinguished calls of the same tool
        and operation (e.g. two ``browser_navigate`` calls to different URLs, or
        two ``desktop_pointer`` drags), a permanent failure of one is treated as
        recovered if any later call with the same key succeeds. This gate proves
        tool/operation *coverage*; per-action correctness is the ground-truth
        check's job, so the two together still catch a genuinely failing run.
        """
        return [entry for entry in self.errors if not entry.get("recovered")]

    @property
    def ok(self) -> bool:
        return not (
            self.missing_tools
            or self.missing_operations
            or self.missing_surfaces
            or self.unrecovered_errors
        )

    def problems(self) -> list[str]:
        problems: list[str] = []
        if self.missing_tools:
            problems.append(f"missing tools: {', '.join(self.missing_tools)}")
        if self.missing_operations:
            problems.append(f"missing operations: {', '.join(self.missing_operations)}")
        if self.missing_surfaces:
            problems.append(f"missing surfaces: {', '.join(self.missing_surfaces)}")
        unrecovered = self.unrecovered_errors
        if unrecovered:
            failed = ", ".join(sorted({str(entry["tool"]) for entry in unrecovered}))
            problems.append(f"tool calls returned unrecovered errors: {failed}")
        return problems

    def to_summary(self) -> dict[str, Any]:
        return {
            "ok": self.ok,
            "tools_required": sorted(REQUIRED_TOOLS),
            "tools_seen": dict(sorted(self.tools_seen.items())),
            "operations_seen": {k: sorted(v) for k, v in sorted(self.operations_seen.items())},
            "surfaces_seen": {k: sorted(v) for k, v in sorted(self.surfaces_seen.items())},
            "missing_tools": self.missing_tools,
            "missing_operations": self.missing_operations,
            "missing_surfaces": self.missing_surfaces,
            "errors": self.errors,
            "unrecovered_errors": self.unrecovered_errors,
        }


def analyze_coverage(calls: Iterable[Mapping[str, Any]]) -> CoverageReport:
    """Build a coverage report from codex ``mcp_tool_call`` items."""
    report = CoverageReport()
    operations: dict[str, set[str]] = {}
    surfaces: dict[str, set[str]] = {}
    # A failure is only fatal when the same (tool, operation/surface) never
    # succeeds later in the run; track success positions so recovered retries
    # stay informational instead of flipping the gate.
    success_positions: dict[tuple[str, str], list[int]] = {}
    pending_errors: list[tuple[int, str, str, tuple[str, str]]] = []
    for index, record in enumerate(_merge_calls(calls)):
        tool = record.tool
        report.tools_seen[tool] = report.tools_seen.get(tool, 0) + 1
        operation = record.arguments.get("operation")
        operation_str = operation if isinstance(operation, str) and operation else ""
        if operation_str:
            operations.setdefault(tool, set()).add(operation_str)
        surface = record.arguments.get("surface")
        surface_str = surface if isinstance(surface, str) and surface else ""
        if surface_str:
            surfaces.setdefault(tool, set()).add(surface_str)
        key = (tool, operation_str or surface_str)
        if record.failed:
            pending_errors.append((index, tool, record.error, key))
        else:
            success_positions.setdefault(key, []).append(index)

    for index, tool, excerpt, key in pending_errors:
        recovered = any(position > index for position in success_positions.get(key, ()))
        report.errors.append({"tool": tool, "excerpt": excerpt, "recovered": recovered})

    report.operations_seen = {tool: sorted(ops) for tool, ops in operations.items()}
    report.surfaces_seen = {tool: sorted(values) for tool, values in surfaces.items()}

    report.missing_tools = sorted(name for name in REQUIRED_TOOLS if name not in report.tools_seen)
    missing_ops: list[str] = []
    for tool, required in sorted(REQUIRED_OPERATIONS.items()):
        seen = operations.get(tool, set())
        for op in sorted(required - seen):
            missing_ops.append(f"{tool}:{op}")
    report.missing_operations = missing_ops
    missing_surfaces: list[str] = []
    for tool, required in sorted(REQUIRED_SURFACES.items()):
        seen = surfaces.get(tool, set())
        for value in sorted(required - seen):
            missing_surfaces.append(f"{tool}:{value}")
    report.missing_surfaces = missing_surfaces
    return report
