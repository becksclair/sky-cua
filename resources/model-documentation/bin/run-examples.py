#!/usr/bin/env python3
"""Execute every installed model-documentation example through bundled node_repl."""

from __future__ import annotations

import argparse
import base64
import json
import os
import selectors
import subprocess
import tempfile
from pathlib import Path

PDF = "JVBERi0xLjQKMSAwIG9iajw8L1R5cGUvQ2F0YWxvZy9QYWdlcyAyIDAgUj4+ZW5kb2JqCjIgMCBvYmo8PC9UeXBlL1BhZ2VzL0NvdW50IDEvS2lkc1szIDAgUl0+PmVuZG9iagozIDAgb2JqPDwvVHlwZS9QYWdlL1BhcmVudCAyIDAgUi9NZWRpYUJveFswIDAgMjQwIDEyMF0vQ29udGVudHMgNCAwIFIvUmVzb3VyY2VzPDwvWE9iamVjdDw8L0ltMSA1IDAgUj4+Pj4+PmVuZG9iago0IDAgb2JqPDwvTGVuZ3RoIDkyPj5zdHJlYW0KQlQgL0YxIDEyIFRmIDIwIDk1IFRkIChDdWEgTm9kZSBhY2NlcHRhbmNlIFBERikgVGogRVQKMCAwIDI0MCAxMjAgcmUgUwpxIDQwIDAgNDAgNDAgY20gL0ltMSBEbyBRCmVuZHN0cmVhbQplbmRvYmoKNSAwIG9iajw8L1R5cGUvWE9iamVjdC9TdWJ0eXBlL0ltYWdlL1dpZHRoIDEvSGVpZ2h0IDEvQ29sb3JTcGFjZS9EZXZpY2VSR0IvQml0c1BlckNvbXBvbmVudCA4L0ZpbHRlci9EQ1REZWNvZGUvTGVuZ3RoIDA+PnN0cmVhbQplbmRzdHJlYW0KZW5kb2JqCnRyYWlsZXI8PC9Sb290IDEgMCBSPj4KJSVFT0YK"
PNG = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII="


class Mcp:
    def __init__(self, executable: Path, cwd: Path, env: dict[str, str]) -> None:
        self.process = subprocess.Popen(
            [str(executable)],
            cwd=cwd,
            env=env,
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            bufsize=1,
        )
        self.next_id = 1
        self.selector = selectors.DefaultSelector()
        assert self.process.stdout is not None
        self.selector.register(self.process.stdout, selectors.EVENT_READ)

    def request(self, method: str, params: dict[str, object]) -> dict[str, object]:
        request_id = self.next_id
        self.next_id += 1
        assert self.process.stdin is not None
        self.process.stdin.write(
            json.dumps({"jsonrpc": "2.0", "id": request_id, "method": method, "params": params})
            + "\n"
        )
        self.process.stdin.flush()
        while True:
            if not self.selector.select(120):
                raise RuntimeError(f"timeout waiting for {method}")
            assert self.process.stdout is not None
            line = self.process.stdout.readline()
            if not line:
                stderr = self.process.stderr.read() if self.process.stderr is not None else ""
                raise RuntimeError(f"node_repl exited during {method}: {stderr}")
            response = json.loads(line)
            if response.get("id") == request_id:
                return response

    def notify(self, method: str, params: dict[str, object]) -> None:
        assert self.process.stdin is not None
        self.process.stdin.write(
            json.dumps({"jsonrpc": "2.0", "method": method, "params": params}) + "\n"
        )
        self.process.stdin.flush()

    def close(self) -> None:
        try:
            self.request("shutdown", {})
        finally:
            self.process.wait(timeout=5)


def component(release: Path, name: str) -> Path:
    manifest = json.loads((release / "RELEASE.json").read_text(encoding="utf-8"))
    record = next(item for item in manifest["components"] if item["name"] == name)
    return (release / record["path"]).resolve(strict=True)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--release-root", type=Path, required=True)
    args = parser.parse_args()
    release = args.release_root.expanduser().resolve(strict=True)
    runtime = component(release, "cua-node-linux-x64-glibc")
    docs = component(release, "documentation")
    manifest = json.loads((runtime / "manifest.json").read_text(encoding="utf-8"))
    trust = ",".join(manifest["trusted_browser_client_sha256s"])
    results: list[dict[str, object]] = []
    with tempfile.TemporaryDirectory(prefix="sky-cua-doc-examples-") as temp_text:
        temp = Path(temp_text)
        image = temp / "input.png"
        pdf = temp / "input.pdf"
        binary = temp / "input.bin"
        image.write_bytes(base64.b64decode(PNG))
        pdf.write_bytes(base64.b64decode(PDF))
        binary.write_bytes(b"installed documentation example\n")
        env = {
            **os.environ,
            "NODE_REPL_NODE_PATH": str(runtime / "bin/node"),
            "NODE_REPL_NODE_MODULE_DIRS": str(runtime / "lib/node_modules"),
            "NODE_REPL_PUBLIC_ENV": ",".join(
                (
                    "SKY_CUA_EXAMPLE_INPUT_FILE",
                    "SKY_CUA_EXAMPLE_IMAGE",
                    "SKY_CUA_EXAMPLE_PDF",
                )
            ),
            "NODE_REPL_TRUSTED_BROWSER_CLIENT_SHA256S": trust,
            "SKY_CUA_DOCUMENTATION_ROOT": str(docs),
            "SKY_CUA_EXAMPLE_INPUT_FILE": binary.as_uri(),
            "SKY_CUA_EXAMPLE_IMAGE": str(image),
            "SKY_CUA_EXAMPLE_PDF": str(pdf),
            "SKY_CUA_MCP_CALLER_PROVENANCE": "direct_mcp",
        }
        mcp = Mcp(runtime / "bin/node_repl", temp, env)
        try:
            initialized = mcp.request(
                "initialize",
                {
                    "protocolVersion": "2025-11-25",
                    "capabilities": {},
                    "clientInfo": {"name": "sky-cua-doc-examples", "version": "1"},
                },
            )
            if "error" in initialized:
                raise RuntimeError(f"initialize failed: {initialized}")
            mcp.notify("notifications/initialized", {})
            for example in sorted((docs / "examples").rglob("*.mjs")):
                response = mcp.request(
                    "tools/call",
                    {"name": "js", "arguments": {"code": example.read_text(encoding="utf-8")}},
                )
                result = response.get("result")
                passed = isinstance(result, dict) and result.get("isError") is False
                results.append({"path": example.relative_to(docs).as_posix(), "passed": passed})
                if not passed:
                    raise RuntimeError(f"documentation example failed: {example}: {response}")
        finally:
            mcp.close()
    print(
        json.dumps(
            {"status": "passed", "release_root": str(release), "examples": results}, sort_keys=True
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
