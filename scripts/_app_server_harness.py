from __future__ import annotations

import json
import os
import time
from dataclasses import dataclass
from datetime import UTC, datetime
from itertools import pairwise
from pathlib import Path
from typing import Any

from _codex_app_server import CodexAppServerClient
from _codex_exec import (
    DEFAULT_MODEL,
    DEFAULT_REASONING_EFFORT,
    FAST_SERVICE_TIER,
    PLUGIN_MENTION,
    prepare_chatgpt_plugin_test_home,
)
from _plugin_bundle import REPO_ROOT

_READ_POLL_SECONDS = 0.25


def with_plugin_mention(prompt: str) -> str:
    return (
        f"Use {PLUGIN_MENTION} and its bundled computer-use skill.\n"
        "When `get_app_state` returns a `screenshot_path`, inspect that image with `view_image` and"
        " treat it as the visual source of truth. If the control tree is sparse or fallback-only,"
        " you may still act by confirmed on-screen coordinates through the computer-use tools."
        ' Use full `get_app_state` for the first orientation pass, then prefer `detail: "compact"`'
        " for repeated screenshot/action verification loops unless verbose element details are needed."
        " If you know the target app or window, visually focus on that region of the full screenshot"
        " while keeping all click and drag coordinates in the current screenshot's pixel coordinate space."
        " When the next move is unclear, look for visible text-entry controls, action clusters,"
        " and right-click/context-menu paths rather than waiting for perfect semantics. Before typing,"
        " inspect the current screenshot and any element `value` or `text.content` readback to confirm"
        " the target field and whether it already contains text; clear or select stale contents before `type_text` when replacement is intended. Re-run"
        " `get_app_state` after opening menus, submenus, dialogs, text entry, clicks, scrolls, keypresses,"
        " or any other visually meaningful action before you commit to the next move. If two successive fresh screenshots show no material"
        " progress after exploratory actions, change strategy or classify the run honestly instead of"
        " looping forever.\n\n"
        f"{prompt}"
    )


@dataclass
class AppServerHarnessPolicy:
    allow_command_approvals: bool = False
    allow_file_change_approvals: bool = False
    allow_permissions_approvals: bool = False


@dataclass
class AppServerTurnResult:
    artifact_dir: Path
    codex_home: Path
    command: list[str]
    transcript_path: Path
    stderr_path: Path
    init_path: Path
    mcp_status_path: Path
    thread_start_path: Path
    turn_start_path: Path
    turn_completed_path: Path
    last_message_path: Path
    thread_id: str
    turn_id: str
    request_log_path: Path
    timing_path: Path
    timing_summary_path: Path


def choose_user_input_answers(params: dict[str, Any]) -> dict[str, Any]:
    answers: dict[str, dict[str, list[str]]] = {}
    for question in params.get("questions", []):
        question_id = question["id"]
        options = question.get("options") or []
        labels = [option.get("label") for option in options if isinstance(option.get("label"), str)]
        chosen: list[str]
        allow = next((label for label in labels if label.lower().startswith("allow")), None)
        if allow is not None:
            chosen = [allow]
        elif labels:
            chosen = [labels[0]]
        else:
            chosen = ["yes"]
        answers[question_id] = {"answers": chosen}
    return {"answers": answers}


def build_schema_accept_value(schema: dict[str, Any]) -> Any:
    schema_type = schema.get("type")
    if schema_type == "object" or ("properties" in schema and schema_type is None):
        properties = schema.get("properties") or {}
        required = schema.get("required") or list(properties.keys())
        value: dict[str, Any] = {}
        for key in required:
            child = properties.get(key, {})
            value[key] = build_schema_accept_value(child)
        return value
    if schema_type == "array":
        items = schema.get("items") or {}
        min_items = schema.get("minItems", 1)
        return [build_schema_accept_value(items) for _ in range(max(1, int(min_items)))]
    if schema_type == "boolean":
        return True
    if schema_type in {"integer", "number"}:
        return 1
    if schema_type == "string":
        enum_values = schema.get("enum")
        if isinstance(enum_values, list) and enum_values:
            return enum_values[0]
        any_of = schema.get("anyOf") or schema.get("oneOf") or []
        if any_of:
            option = any_of[0]
            if isinstance(option, dict):
                if "const" in option:
                    return option["const"]
                return build_schema_accept_value(option)
        return "yes"
    if "const" in schema:
        return schema["const"]
    if "enum" in schema and isinstance(schema["enum"], list) and schema["enum"]:
        return schema["enum"][0]
    any_of = schema.get("anyOf") or schema.get("oneOf") or []
    if any_of:
        option = any_of[0]
        if isinstance(option, dict):
            return build_schema_accept_value(option)
    return True


def choose_mcp_elicitation_response(params: dict[str, Any]) -> dict[str, Any]:
    request = params.get("request") or {}
    mode = request.get("mode")
    if mode == "form":
        schema = request.get("requestedSchema") or {}
        return {
            "action": "accept",
            "content": build_schema_accept_value(schema),
        }
    return {
        "action": "accept",
        "content": None,
    }


def choose_approval_decision(method: str, policy: AppServerHarnessPolicy) -> str:
    if method == "item/commandExecution/requestApproval":
        return "accept" if policy.allow_command_approvals else "decline"
    if method == "item/fileChange/requestApproval":
        return "accept" if policy.allow_file_change_approvals else "decline"
    if method == "item/permissions/requestApproval":
        return "accept" if policy.allow_permissions_approvals else "decline"
    return "cancel"


def extract_agent_message(notification: dict[str, Any]) -> str | None:
    params = notification.get("params") or {}
    item = params.get("item") or {}
    if item.get("type") != "agentMessage":
        return None
    text = item.get("text")
    return text if isinstance(text, str) else None


def transcript_computer_use_items(transcript_path: Path) -> list[dict[str, Any]]:
    items: list[dict[str, Any]] = []
    for raw_line in transcript_path.read_text().splitlines():
        if not raw_line.strip():
            continue
        try:
            message = json.loads(raw_line)
        except json.JSONDecodeError:
            continue
        if message.get("method") not in {"item/started", "item/completed"}:
            continue
        params = message.get("params") or {}
        item = params.get("item") or {}
        if item.get("type") != "mcpToolCall":
            continue
        if item.get("server") != "computer-use":
            continue
        items.append(item)
    return items


def require_computer_use_item(transcript_path: Path) -> None:
    items = transcript_computer_use_items(transcript_path)
    if not items:
        raise RuntimeError("rich app-server turn never emitted a computer-use mcpToolCall item")


def response_contains_computer_use_server(response: dict[str, Any]) -> bool:
    result = response.get("result")
    candidates: list[Any] = []
    if isinstance(result, list):
        candidates.extend(result)
    elif isinstance(result, dict):
        for key in ("data", "servers", "mcpServers", "items"):
            value = result.get(key)
            if isinstance(value, list):
                candidates.extend(value)
    for candidate in candidates:
        if not isinstance(candidate, dict):
            continue
        name = candidate.get("name") or candidate.get("server") or candidate.get("id")
        if name == "computer-use":
            return True
    return False


def describe_inbound_message(message: dict[str, Any]) -> dict[str, Any]:
    event: dict[str, Any] = {}
    method = message.get("method")
    if method is not None:
        event["method"] = method
    if "id" in message:
        event["id"] = message["id"]
    params = message.get("params") or {}
    item = params.get("item")
    if isinstance(item, dict):
        event["item_type"] = item.get("type")
        event["item_status"] = item.get("status")
        event["item_id"] = item.get("id")
        if item.get("type") == "mcpToolCall":
            event["server"] = item.get("server")
            event["tool"] = item.get("tool")
            event["duration_ms"] = item.get("durationMs")
    token_usage = params.get("tokenUsage")
    if isinstance(token_usage, dict):
        event["token_usage"] = token_usage
    return event


def summarize_timing(events: list[dict[str, Any]], transcript_lines: list[str]) -> dict[str, Any]:
    elapsed_values = [
        float(event["elapsed_ms"])
        for event in events
        if isinstance(event.get("elapsed_ms"), int | float)
    ]
    summary: dict[str, Any] = {
        "elapsed_ms": max(elapsed_values) if elapsed_values else 0,
        "event_count": len(events),
        "transcript_event_count": len(transcript_lines),
        "inbound_count": sum(1 for event in events if event.get("event") == "inbound"),
        "client_request_count": sum(
            1 for event in events if event.get("event") == "client_request_sent"
        ),
        "server_request_count": sum(
            1 for event in events if event.get("event") == "server_request_received"
        ),
        "server_request_answer_count": sum(
            1 for event in events if event.get("event") == "server_request_answered"
        ),
    }
    mcp_tool_counts: dict[str, int] = {}
    mcp_tool_duration_ms: dict[str, int] = {}
    completed_mcp_calls = 0
    token_updates: list[dict[str, Any]] = []
    token_update_deltas: list[dict[str, int]] = []
    item_starts: dict[str, dict[str, Any]] = {}
    item_wall_time_ms: dict[str, int] = {}
    item_completed_counts: dict[str, int] = {}
    previous_total_tokens: int | None = None
    for event in events:
        item_id = event.get("item_id")
        item_type = event.get("item_type")
        if (
            event.get("event") == "inbound"
            and event.get("method") == "item/started"
            and isinstance(item_id, str)
            and isinstance(item_type, str)
        ):
            item_starts[item_id] = event
        if (
            event.get("event") == "inbound"
            and event.get("method") == "item/completed"
            and isinstance(item_id, str)
            and isinstance(item_type, str)
        ):
            item_completed_counts[item_type] = item_completed_counts.get(item_type, 0) + 1
            start = item_starts.get(item_id)
            start_ms = start.get("elapsed_ms") if start else None
            end_ms = event.get("elapsed_ms")
            if isinstance(start_ms, int | float) and isinstance(end_ms, int | float):
                item_wall_time_ms[item_type] = item_wall_time_ms.get(item_type, 0) + int(
                    end_ms - start_ms
                )
        if event.get("event") == "inbound" and event.get("method") == "thread/tokenUsage/updated":
            token_usage = event.get("token_usage")
            if isinstance(token_usage, dict):
                token_updates.append(token_usage)
                total = token_usage.get("total") or {}
                total_tokens = total.get("totalTokens")
                if isinstance(total_tokens, int):
                    delta: dict[str, int] = {"totalTokens": total_tokens}
                    if previous_total_tokens is not None:
                        delta["deltaTotalTokens"] = total_tokens - previous_total_tokens
                    previous_total_tokens = total_tokens
                    last = token_usage.get("last") or {}
                    for key in (
                        "totalTokens",
                        "inputTokens",
                        "cachedInputTokens",
                        "outputTokens",
                        "reasoningOutputTokens",
                    ):
                        value = last.get(key)
                        if isinstance(value, int):
                            delta[f"last{key[0].upper()}{key[1:]}"] = value
                    token_update_deltas.append(delta)
        if (
            event.get("event") == "inbound"
            and event.get("method") == "item/completed"
            and event.get("item_type") == "mcpToolCall"
        ):
            tool = str(event.get("tool") or "unknown")
            completed_mcp_calls += 1
            mcp_tool_counts[tool] = mcp_tool_counts.get(tool, 0) + 1
            duration_ms = event.get("duration_ms")
            if isinstance(duration_ms, int | float):
                mcp_tool_duration_ms[tool] = mcp_tool_duration_ms.get(tool, 0) + int(duration_ms)
    summary["completed_mcp_tool_calls"] = completed_mcp_calls
    summary["mcp_tool_counts"] = dict(sorted(mcp_tool_counts.items()))
    summary["mcp_tool_duration_ms"] = dict(sorted(mcp_tool_duration_ms.items()))
    summary["mcp_tool_duration_total_ms"] = sum(mcp_tool_duration_ms.values())
    summary["item_completed_counts"] = dict(sorted(item_completed_counts.items()))
    summary["item_wall_time_ms"] = dict(sorted(item_wall_time_ms.items()))
    summary["non_mcp_elapsed_estimate_ms"] = max(
        0, int(summary["elapsed_ms"]) - summary["mcp_tool_duration_total_ms"]
    )
    inbound_events = [event for event in events if event.get("event") == "inbound"]
    gaps: list[dict[str, Any]] = []
    for previous, current in pairwise(inbound_events):
        previous_elapsed = previous.get("elapsed_ms")
        current_elapsed = current.get("elapsed_ms")
        if not isinstance(previous_elapsed, int | float) or not isinstance(
            current_elapsed, int | float
        ):
            continue
        gap_ms = int(current_elapsed - previous_elapsed)
        if gap_ms <= 0:
            continue
        gaps.append(
            {
                "gap_ms": gap_ms,
                "from": {
                    "method": previous.get("method"),
                    "item_type": previous.get("item_type"),
                    "tool": previous.get("tool"),
                },
                "to": {
                    "method": current.get("method"),
                    "item_type": current.get("item_type"),
                    "tool": current.get("tool"),
                },
            }
        )
    if gaps:
        summary["largest_inbound_gaps_ms"] = sorted(
            gaps, key=lambda event: event["gap_ms"], reverse=True
        )[:10]
    if token_updates:
        summary["token_usage_update_count"] = len(token_updates)
        summary["last_token_usage"] = token_updates[-1]
    if token_update_deltas:
        summary["last_token_usage_deltas"] = token_update_deltas[-10:]
        uncached_inputs = [
            delta["lastInputTokens"] - delta["lastCachedInputTokens"]
            for delta in token_update_deltas
            if "lastInputTokens" in delta and "lastCachedInputTokens" in delta
        ]
        if uncached_inputs:
            summary["last_uncached_input_tokens"] = uncached_inputs[-1]
            summary["max_uncached_input_tokens"] = max(uncached_inputs)
            summary["avg_uncached_input_tokens"] = int(sum(uncached_inputs) / len(uncached_inputs))
    outbound_latencies = [
        event
        for event in events
        if event.get("event") == "client_response_received"
        and isinstance(event.get("latency_ms"), int | float)
    ]
    if outbound_latencies:
        summary["client_request_latencies_ms"] = [
            {
                "method": event.get("request_method"),
                "id": event.get("id"),
                "latency_ms": event.get("latency_ms"),
            }
            for event in outbound_latencies
        ]
    return summary


def run_rich_app_server_turn(
    *,
    prompt: str,
    artifact_dir: Path,
    output_schema: Path,
    model: str = DEFAULT_MODEL,
    reasoning_effort: str = DEFAULT_REASONING_EFFORT,
    max_turn_seconds: float | None = 180.0,
    policy: AppServerHarnessPolicy | None = None,
    extra_env: dict[str, str] | None = None,
) -> AppServerTurnResult:
    policy = policy or AppServerHarnessPolicy()
    codex_home = prepare_chatgpt_plugin_test_home(artifact_dir=artifact_dir)
    service_socket_path = (artifact_dir / "service.sock").resolve()
    if service_socket_path.exists():
        service_socket_path.unlink()
    transcript_path = artifact_dir / "app-server-output.jsonl"
    stderr_path = artifact_dir / "app-server-stderr.txt"
    init_path = artifact_dir / "init.json"
    mcp_status_path = artifact_dir / "mcp-server-status.json"
    thread_start_path = artifact_dir / "thread_start.json"
    turn_start_path = artifact_dir / "turn_start.json"
    turn_completed_path = artifact_dir / "turn_completed.json"
    last_message_path = artifact_dir / "last-message.json"
    request_log_path = artifact_dir / "server-requests.jsonl"
    timing_path = artifact_dir / "timing.jsonl"
    timing_summary_path = artifact_dir / "timing-summary.json"
    prompt_path = artifact_dir / "prompt.txt"
    prompt_path.write_text(prompt)
    transcript_path.write_text("")
    request_log_path.write_text("")
    timing_path.write_text("")

    command = ["codex", "app-server"]
    env = dict(os.environ)
    env["CODEX_HOME"] = str(codex_home)
    env["SKY_CUA_SERVICE_SOCKET_PATH"] = str(service_socket_path)
    env.setdefault("RUST_LOG", "error")
    if extra_env:
        env.update(extra_env)

    rpc = CodexAppServerClient(command, env=env, cwd=REPO_ROOT)
    transcript_lines: list[str] = []
    request_lines: list[str] = []
    timing_events: list[dict[str, Any]] = []
    last_agent_message: dict[str, Any] | None = None
    thread_id = ""
    turn_id = ""
    start_monotonic = time.monotonic()
    pending_client_requests: dict[int, dict[str, Any]] = {}
    pending_server_requests: dict[int, dict[str, Any]] = {}
    deadline = time.monotonic() + max_turn_seconds if max_turn_seconds is not None else None

    def elapsed_ms() -> int:
        return int((time.monotonic() - start_monotonic) * 1000)

    def record_timing(event: str, **fields: Any) -> None:
        entry = {
            "event": event,
            "elapsed_ms": elapsed_ms(),
            "timestamp": datetime.now(UTC).isoformat(),
            **fields,
        }
        timing_events.append(entry)
        with timing_path.open("a") as handle:
            handle.write(json.dumps(entry) + "\n")

    def record(message: dict[str, Any]) -> None:
        encoded = json.dumps(message)
        transcript_lines.append(encoded)
        with transcript_path.open("a") as handle:
            handle.write(encoded + "\n")
        record_timing("inbound", **describe_inbound_message(message))

    def record_request(message: dict[str, Any]) -> None:
        encoded = json.dumps(message)
        request_lines.append(encoded)
        with request_log_path.open("a") as handle:
            handle.write(encoded + "\n")

    def ensure_not_timed_out(phase: str) -> None:
        if deadline is not None and time.monotonic() > deadline:
            record_timing("timeout", phase=phase, max_turn_seconds=max_turn_seconds)
            raise TimeoutError(
                f"rich app-server turn exceeded {max_turn_seconds:.0f}s while waiting for {phase}; inspect {artifact_dir}"
            )

    def send_request(method: str, params: dict[str, Any]) -> int:
        request_id = rpc.send_request(method, params)
        pending_client_requests[request_id] = {
            "method": method,
            "start_ms": elapsed_ms(),
        }
        record_timing("client_request_sent", id=request_id, method=method)
        return request_id

    def send_notification(method: str, params: dict[str, Any]) -> None:
        record_timing("client_notification_sent", method=method)
        rpc.notify(method, params)

    def read_message(phase: str) -> dict[str, Any]:
        while True:
            ensure_not_timed_out(phase)
            try:
                return rpc.read_message(timeout=_READ_POLL_SECONDS)
            except TimeoutError:
                continue

    def handle_server_request(message: dict[str, Any]) -> None:
        record_request(message)
        method = message["method"]
        request_id = message["id"]
        params = message.get("params") or {}
        pending_server_requests[request_id] = {
            "method": method,
            "start_ms": elapsed_ms(),
        }
        record_timing(
            "server_request_received",
            id=request_id,
            method=method,
            server_name=params.get("serverName"),
        )

        if method == "item/tool/requestUserInput":
            rpc.respond(request_id, choose_user_input_answers(params))
            record_server_request_answer(request_id)
            return
        if method == "mcpServer/elicitation/request":
            rpc.respond(request_id, choose_mcp_elicitation_response(params))
            record_server_request_answer(request_id)
            return
        if method in {
            "item/commandExecution/requestApproval",
            "item/fileChange/requestApproval",
            "item/permissions/requestApproval",
        }:
            rpc.respond(request_id, {"decision": choose_approval_decision(method, policy)})
            record_server_request_answer(request_id)
            return

        rpc.respond(request_id, {"decision": "cancel"})
        record_server_request_answer(request_id)

    def record_server_request_answer(request_id: int) -> None:
        pending = pending_server_requests.pop(request_id, {})
        start_ms = pending.get("start_ms")
        latency_ms = elapsed_ms() - start_ms if isinstance(start_ms, int) else None
        record_timing(
            "server_request_answered",
            id=request_id,
            request_method=pending.get("method"),
            latency_ms=latency_ms,
        )

    def record_client_response(message: dict[str, Any]) -> None:
        request_id = message.get("id")
        if not isinstance(request_id, int):
            record_timing("client_response_received", id=request_id, latency_ms=None)
            return
        pending = pending_client_requests.pop(request_id, {})
        start_ms = pending.get("start_ms")
        latency_ms = elapsed_ms() - start_ms if isinstance(start_ms, int) else None
        record_timing(
            "client_response_received",
            id=request_id,
            request_method=pending.get("method"),
            latency_ms=latency_ms,
        )

    def read_until_response(target_id: int) -> dict[str, Any]:
        nonlocal last_agent_message
        while True:
            message = read_message(f"response {target_id}")
            record(message)
            if "id" in message and message.get("method") is None:
                record_client_response(message)
                if message["id"] == target_id:
                    return message
                continue
            if "id" in message and message.get("method"):
                handle_server_request(message)
                continue
            if message.get("method") == "item/completed":
                agent_text = extract_agent_message(message)
                if agent_text is not None:
                    try:
                        last_agent_message = json.loads(agent_text)
                    except json.JSONDecodeError:
                        last_agent_message = {"raw_text": agent_text}
                    last_message_path.write_text(json.dumps(last_agent_message, indent=2))

    def read_until_turn_completed() -> dict[str, Any]:
        nonlocal last_agent_message
        while True:
            message = read_message("turn completion")
            record(message)
            if "id" in message and message.get("method"):
                handle_server_request(message)
                continue
            if message.get("method") == "item/completed":
                agent_text = extract_agent_message(message)
                if agent_text is not None:
                    try:
                        last_agent_message = json.loads(agent_text)
                    except json.JSONDecodeError:
                        last_agent_message = {"raw_text": agent_text}
                    last_message_path.write_text(json.dumps(last_agent_message, indent=2))
            if message.get("method") == "turn/completed":
                return message

    try:
        init_id = send_request(
            "initialize",
            {
                "clientInfo": {
                    "name": "sky-cua-rich-harness",
                    "title": "sky-cua rich harness",
                    "version": "0.1.0",
                },
                "capabilities": {"experimentalApi": True},
            },
        )
        init_response = read_until_response(init_id)
        init_path.write_text(json.dumps(init_response, indent=2))
        send_notification("initialized", {})

        mcp_status_id = send_request("mcpServerStatus/list", {"detail": "full", "limit": 100})
        mcp_status_response = read_until_response(mcp_status_id)
        mcp_status_path.write_text(json.dumps(mcp_status_response, indent=2))
        if not response_contains_computer_use_server(mcp_status_response):
            raise RuntimeError(
                f"computer-use MCP server was not visible before thread/start; inspect {mcp_status_path}"
            )

        thread_start_id = send_request(
            "thread/start",
            {
                "model": model,
                "serviceTier": FAST_SERVICE_TIER,
                "cwd": str(REPO_ROOT),
            },
        )
        thread_start_response = read_until_response(thread_start_id)
        thread_start_path.write_text(json.dumps(thread_start_response, indent=2))
        thread_id = thread_start_response["result"]["thread"]["id"]

        turn_start_id = send_request(
            "turn/start",
            {
                "threadId": thread_id,
                "input": [{"type": "text", "text": prompt, "textElements": []}],
                "model": model,
                "serviceTier": FAST_SERVICE_TIER,
                "reasoning_effort": reasoning_effort,
                "outputSchema": json.loads(output_schema.read_text()),
            },
        )
        turn_start_response = read_until_response(turn_start_id)
        turn_start_path.write_text(json.dumps(turn_start_response, indent=2))
        turn_id = turn_start_response["result"]["turn"]["id"]

        turn_completed = read_until_turn_completed()
        turn_completed_path.write_text(json.dumps(turn_completed, indent=2))
    finally:
        timing_summary_path.write_text(
            json.dumps(summarize_timing(timing_events, transcript_lines), indent=2)
        )
        rpc.close()
        stderr_path.write_text(rpc.stderr_text())
        if last_agent_message is not None and not last_message_path.exists():
            last_message_path.write_text(json.dumps(last_agent_message, indent=2))

    if not last_message_path.exists():
        raise RuntimeError(
            f"rich app-server turn did not emit a final agent message; inspect {artifact_dir}"
        )

    return AppServerTurnResult(
        artifact_dir=artifact_dir,
        codex_home=codex_home,
        command=command,
        transcript_path=transcript_path,
        stderr_path=stderr_path,
        init_path=init_path,
        mcp_status_path=mcp_status_path,
        thread_start_path=thread_start_path,
        turn_start_path=turn_start_path,
        turn_completed_path=turn_completed_path,
        last_message_path=last_message_path,
        thread_id=thread_id,
        turn_id=turn_id,
        request_log_path=request_log_path,
        timing_path=timing_path,
        timing_summary_path=timing_summary_path,
    )
