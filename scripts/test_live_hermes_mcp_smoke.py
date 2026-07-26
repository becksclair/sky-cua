from __future__ import annotations

import subprocess
from pathlib import Path

import pytest

import live_hermes_mcp_smoke


def _result(stdout: str, stderr: str = "", returncode: int = 0) -> subprocess.CompletedProcess[str]:
    return subprocess.CompletedProcess(["hermes"], returncode, stdout, stderr)


def test_transport_requires_connected_and_nonzero_tool_discovery() -> None:
    live_hermes_mcp_smoke.require_transport_success(
        "node_repl", _result("✓ Connected (10ms)\n✓ Tools discovered: 3\n")
    )

    with pytest.raises(RuntimeError, match="hermes mcp test node_repl failed"):
        live_hermes_mcp_smoke.require_transport_success(
            "node_repl", _result("Unknown MCP server: node_repl\n")
        )


def test_tool_calls_ignore_prompt_echoes() -> None:
    stderr = (
        "prompt mentions mcp__sky_cua__status and mcp__node_repl__js\n"
        "Tool call: mcp__sky_cua__status with args: {}\n"
        "Tool call: mcp__node_repl__js with args: {}\n"
    )

    assert live_hermes_mcp_smoke.tool_calls_from_debug(stderr) == [
        "mcp__sky_cua__status",
        "mcp__node_repl__js",
    ]


def test_agent_evidence_rejects_builtins_wrong_arguments_and_failed_results() -> None:
    nonce = "sky-cua-hermes-fixture"
    evidence = {"nonce": nonce, "invocation_id": "d01f5b84-cc50-4403-aaf4-e4f8c1f26e42"}
    valid = (
        'Tool call: mcp__sky_cua__status with args: {"component":"session_presence"}...\n'
        'Tool result (100 chars): {"structuredContent":{"branch":"session_presence","tool":"status"}}\n'
        f'Tool call: mcp__node_repl__js with args: {{"title":"Hermes Sky CUA acceptance","code":"{nonce}"}}...\n'
        f'Tool result (100 chars): {{"result":"{{\\"nonce\\":\\"{nonce}\\",\\"invocation_id\\":\\"d01f5b84-cc50-4403-aaf4-e4f8c1f26e42\\"}}"}}\n'
    )
    assert (
        live_hermes_mcp_smoke.require_agent_evidence(
            valid,
            nonce=nonce,
            evidence=evidence,
        )
        == evidence["invocation_id"]
    )

    with pytest.raises(RuntimeError, match="tool-call sequence mismatch"):
        live_hermes_mcp_smoke.require_agent_evidence(
            "Tool call: write_file with args: {}\n" + valid,
            nonce=nonce,
            evidence=evidence,
        )
    with pytest.raises(RuntimeError, match="status arguments are wrong"):
        live_hermes_mcp_smoke.require_agent_evidence(
            valid.replace('"session_presence"', '"browser"', 1),
            nonce=nonce,
            evidence=evidence,
        )
    with pytest.raises(RuntimeError, match="node_repl title is wrong"):
        live_hermes_mcp_smoke.require_agent_evidence(
            valid.replace(
                '"title":"Hermes Sky CUA acceptance"',
                '"title":"wrong","code":"Hermes Sky CUA acceptance ',
            ),
            nonce=nonce,
            evidence=evidence,
        )
    with pytest.raises(RuntimeError, match="returned an error result"):
        live_hermes_mcp_smoke.require_agent_evidence(
            valid.replace(
                '"structuredContent":',
                '"isError":true,"structuredContent":',
                1,
            ),
            nonce=nonce,
            evidence=evidence,
        )
    with pytest.raises(RuntimeError, match="tool results are incomplete"):
        live_hermes_mcp_smoke.require_agent_evidence(
            "\n".join(line for line in valid.splitlines() if 'status"}}' not in line),
            nonce=nonce,
            evidence=evidence,
        )


def test_freshness_uses_hermes_configured_client(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    client = tmp_path / "sky-cua-client"
    monkeypatch.setattr(
        live_hermes_mcp_smoke,
        "run",
        lambda _command, *, timeout: _result(f'"{client}"\n'),
    )
    checked: list[Path] = []
    monkeypatch.setattr(
        live_hermes_mcp_smoke.deploy_freshness,
        "check_client_freshness",
        lambda path: live_hermes_mcp_smoke.deploy_freshness.Freshness(True, path, "fixture", ""),
    )
    monkeypatch.setattr(
        live_hermes_mcp_smoke.deploy_freshness,
        "assert_runtime_fresh",
        checked.append,
    )
    monkeypatch.setattr(
        live_hermes_mcp_smoke.deploy_freshness,
        "runtime_source_fingerprint",
        lambda: "fingerprint",
    )

    report = live_hermes_mcp_smoke.require_fresh_runtime("hermes", timeout=30)

    assert checked == [client]
    assert report == {
        "configured_client": str(client),
        "verified_client": str(client),
        "fresh": True,
        "source_fingerprint": "fingerprint",
    }


def test_final_response_isolated_from_tool_results() -> None:
    stdout = (
        "Result: invocation-from-tool\n"
        "Conversation completed after 3 calls\n"
        "    invocation-from-assistant\n"
        "╰────────────────\n"
    )

    assert live_hermes_mcp_smoke.final_assistant_response(stdout) == "invocation-from-assistant"
