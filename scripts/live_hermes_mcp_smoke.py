#!/usr/bin/env python3
"""Validate Hermes Agent discovery and real use of both Sky CUA MCP servers."""

from __future__ import annotations

import argparse
import json
import re
import shutil
import subprocess
import sys
import uuid
from collections.abc import Mapping
from pathlib import Path

import deploy_freshness

DEFAULT_ARTIFACT_DIR = Path("artifacts/live-hermes-mcp-smoke")


def run(command: list[str], *, timeout: int) -> subprocess.CompletedProcess[str]:
    return subprocess.run(command, check=False, capture_output=True, text=True, timeout=timeout)


def write_process_artifacts(
    artifact_dir: Path,
    name: str,
    result: subprocess.CompletedProcess[str],
) -> None:
    (artifact_dir / f"{name}.stdout.txt").write_text(result.stdout, encoding="utf-8")
    (artifact_dir / f"{name}.stderr.txt").write_text(result.stderr, encoding="utf-8")


def require_transport_success(server: str, result: subprocess.CompletedProcess[str]) -> None:
    connected = "✓ Connected" in result.stdout
    discovered = re.search(r"✓ Tools discovered:\s*([1-9][0-9]*)", result.stdout)
    if result.returncode == 0 and connected and discovered is not None:
        return
    detail = (result.stderr or result.stdout).strip()
    raise RuntimeError(f"hermes mcp test {server} failed: {detail}")


def tool_calls_from_debug(stderr: str) -> list[str]:
    return re.findall(r"Tool call: ([A-Za-z0-9_.-]+)\b", stderr)


def tool_call_arguments_from_debug(stderr: str) -> list[str]:
    return re.findall(r"Tool call: [A-Za-z0-9_.-]+ with args: (.+)$", stderr, re.MULTILINE)


def tool_results_from_debug(stderr: str) -> list[dict[str, object]]:
    payloads = re.findall(r"Tool result \([0-9]+ chars\): (.+)$", stderr, re.MULTILINE)
    return [json.loads(payload) for payload in payloads]


def final_assistant_response(stdout: str) -> str:
    match = re.search(r"Conversation completed[^\n]*\n\s+([^\n]+)\n╰", stdout)
    if match is None:
        raise RuntimeError("Hermes transcript does not contain a final assistant response")
    return match.group(1).strip()


def configured_sky_cua_client(hermes_bin: str, *, timeout: int) -> Path:
    result = run(
        [hermes_bin, "config", "get", "mcp_servers.sky_cua.command", "--json"],
        timeout=timeout,
    )
    if result.returncode != 0:
        raise RuntimeError(f"could not read Hermes sky_cua command: {result.stderr.strip()}")
    try:
        command = json.loads(result.stdout)
    except json.JSONDecodeError as error:
        raise RuntimeError("Hermes sky_cua command is not valid JSON") from error
    if not isinstance(command, str) or not command.strip():
        raise RuntimeError(f"Hermes sky_cua command is invalid: {command!r}")
    return Path(command).expanduser()


def require_fresh_runtime(hermes_bin: str, *, timeout: int) -> dict[str, object]:
    configured_client = configured_sky_cua_client(hermes_bin, timeout=timeout)
    freshness = deploy_freshness.check_client_freshness(configured_client)
    deploy_freshness.assert_runtime_fresh(configured_client)
    return {
        "configured_client": str(configured_client),
        "verified_client": str(freshness.client_path),
        "fresh": freshness.fresh,
        "source_fingerprint": deploy_freshness.runtime_source_fingerprint(),
    }


def require_agent_evidence(
    stderr: str,
    *,
    nonce: str,
    evidence: Mapping[str, object],
) -> str:
    expected_calls = ["mcp__sky_cua__status", "mcp__node_repl__js"]
    actual_calls = tool_calls_from_debug(stderr)
    if actual_calls != expected_calls:
        raise RuntimeError(
            f"Hermes tool-call sequence mismatch: expected {expected_calls}, got {actual_calls}"
        )
    arguments = tool_call_arguments_from_debug(stderr)
    if len(arguments) != 2:
        raise RuntimeError(f"Hermes tool arguments are incomplete: {arguments!r}")
    try:
        status_args = json.loads(arguments[0].removesuffix("..."))
    except json.JSONDecodeError as error:
        raise RuntimeError("Hermes status arguments are not valid JSON") from error
    if status_args != {"component": "session_presence"}:
        raise RuntimeError(f"Hermes status arguments are wrong: {status_args!r}")
    title_match = re.match(r'\{"title":("(?:\\.|[^"\\])*")', arguments[1])
    if title_match is None or json.loads(title_match.group(1)) != "Hermes Sky CUA acceptance":
        raise RuntimeError("Hermes node_repl title is wrong")
    if nonce[:32] not in arguments[1]:
        raise RuntimeError("Hermes node_repl arguments do not contain the visible nonce prefix")

    results = tool_results_from_debug(stderr)
    if len(results) != 2:
        raise RuntimeError(f"Hermes tool results are incomplete: {results!r}")
    if any(result.get("isError") is True for result in results):
        raise RuntimeError(f"Hermes MCP tool returned an error result: {results!r}")
    status_content = results[0].get("structuredContent")
    if not isinstance(status_content, dict):
        raise RuntimeError(f"Hermes status result has no structured content: {results[0]!r}")
    if status_content.get("tool") != "status" or status_content.get("branch") != "session_presence":
        raise RuntimeError(f"Hermes status result is wrong: {status_content!r}")
    node_result = results[1].get("result")
    if not isinstance(node_result, str):
        raise RuntimeError(f"Hermes node_repl result is wrong: {results[1]!r}")
    try:
        returned_evidence = json.loads(node_result)
    except json.JSONDecodeError as error:
        raise RuntimeError("Hermes node_repl result is not valid JSON") from error
    if returned_evidence != evidence:
        raise RuntimeError(
            f"Hermes node_repl result does not match file evidence: {returned_evidence!r}"
        )
    invocation_id = evidence.get("invocation_id")
    if not isinstance(invocation_id, str):
        raise RuntimeError(f"Hermes invocation ID is missing: {invocation_id!r}")
    try:
        parsed_id = uuid.UUID(invocation_id)
    except ValueError as error:
        raise RuntimeError(f"Hermes invocation ID is not a UUID: {invocation_id!r}") from error
    if str(parsed_id) != invocation_id:
        raise RuntimeError(f"Hermes invocation ID is not canonical: {invocation_id!r}")
    return invocation_id


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Prove Hermes Agent can discover and use Sky CUA plus Node REPL MCP tools."
    )
    parser.add_argument("--hermes-bin", default=shutil.which("hermes") or "hermes")
    parser.add_argument("--artifact-dir", type=Path, default=DEFAULT_ARTIFACT_DIR)
    parser.add_argument("--model", default=None)
    parser.add_argument("--provider", default=None)
    parser.add_argument("--timeout", type=int, default=240)
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    artifact_dir = args.artifact_dir.expanduser().resolve()
    artifact_dir.mkdir(parents=True, exist_ok=True)
    checks: dict[str, object] = {}
    try:
        checks["deploy_freshness"] = require_fresh_runtime(args.hermes_bin, timeout=args.timeout)
        for server in ("sky_cua", "node_repl"):
            result = run([args.hermes_bin, "mcp", "test", server], timeout=args.timeout)
            write_process_artifacts(artifact_dir, f"mcp-test-{server}", result)
            require_transport_success(server, result)
            checks[f"mcp_test_{server}"] = True

        nonce = f"sky-cua-hermes-{uuid.uuid4()}"
        evidence_path = artifact_dir / f"node-repl-evidence-{nonce}.json"
        evidence_path.unlink(missing_ok=True)
        code = (
            'var hermesFs = await import("node:fs/promises"); '
            'var hermesCrypto = await import("node:crypto"); '
            f"var hermesNonce = {json.dumps(nonce)}; "
            "var hermesInvocationId = hermesCrypto.randomUUID(); "
            "var hermesEvidence = {nonce:hermesNonce,invocation_id:hermesInvocationId}; "
            f"await hermesFs.writeFile({json.dumps(str(evidence_path))}, "
            "JSON.stringify(hermesEvidence)); "
            "nodeRepl.write(JSON.stringify(hermesEvidence))"
        )
        prompt = (
            "Use exactly two MCP tools and no built-in tools. First call "
            "mcp__sky_cua__status with component session_presence. Then call "
            "mcp__node_repl__js exactly once with title 'Hermes Sky CUA acceptance' "
            f"and this exact code: {json.dumps(code)}. After both tool results, reply "
            "with only the invocation_id returned by node_repl."
        )
        command = [
            args.hermes_bin,
            "chat",
            "-q",
            prompt,
            "--ignore-rules",
            "--yolo",
            "--max-turns",
            "8",
            "--source",
            "tool",
            "-v",
        ]
        if args.model:
            command.extend(["--model", args.model])
        if args.provider:
            command.extend(["--provider", args.provider])
        result = run(command, timeout=args.timeout)
        write_process_artifacts(artifact_dir, "agent-turn", result)
        if result.returncode != 0:
            detail = (result.stderr or result.stdout).strip()
            raise RuntimeError(f"Hermes agent turn failed: {detail}")
        if not evidence_path.is_file():
            raise RuntimeError("node_repl did not create nonce-bound evidence")
        evidence = json.loads(evidence_path.read_text(encoding="utf-8"))
        if evidence.get("nonce") != nonce or not evidence.get("invocation_id"):
            raise RuntimeError(f"node_repl evidence is invalid: {evidence!r}")
        invocation_id = require_agent_evidence(
            result.stderr,
            nonce=nonce,
            evidence=evidence,
        )
        final_response = final_assistant_response(result.stdout)
        if final_response != invocation_id:
            raise RuntimeError(
                f"Hermes final response did not equal the node_repl invocation ID: {final_response!r}"
            )
        checks.update(
            {
                "agent_turn": True,
                "sky_cua_tool": "mcp__sky_cua__status",
                "node_repl_tool": "mcp__node_repl__js",
                "nonce": nonce,
                "invocation_id": invocation_id,
            }
        )
        report = {
            "status": "passed",
            "target": "Hermes Agent",
            "artifact_dir": str(artifact_dir),
            "proof": "Both MCP transports connected and a real Hermes turn used both tools.",
            "checks": checks,
        }
        print(json.dumps(report, sort_keys=True))
        return 0
    except Exception as error:
        print(
            json.dumps(
                {
                    "status": "failed",
                    "target": "Hermes Agent",
                    "artifact_dir": str(artifact_dir),
                    "error": str(error),
                    "checks": checks,
                },
                sort_keys=True,
            ),
            file=sys.stderr,
        )
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
