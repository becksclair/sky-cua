"""Opt-in loopback browser pilot: serve frozen fixtures and bounded model decisions.

Browser observation, validation and clicks are driven by jev_browser_pilot.mjs
through the existing browser tool. This is not installed Rust-runtime acceptance.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from typing import Any

from _agent_perf_judge import (
    DEFAULT_JUDGE_MODEL,
    DEFAULT_JUDGE_REASONING_EFFORT,
    prepare_judge_codex_home,
)
from _codex_exec import read_last_message, run_codex_exec
from jev_ui_experiment import dump, jev_client

ROOT = Path(__file__).resolve().parents[1]
INSTRUCTION = (
    "Choose the single observed enabled visible button that fulfills the user intent. "
    "Use its surrounding group context, not just similar words. Do not choose a disabled "
    "control. If no control fulfills the intent or multiple controls are equally valid, "
    "choose abstain. Return an observed candidate ID only; never invent coordinates."
)
THRESHOLD = 0.7


def payload_for(intent: str, observation: dict[str, Any]) -> dict[str, Any]:
    candidates = observation["candidates"]
    criteria: dict[str, Any] = {
        c["id"]: {
            "label": c["label"],
            "context": c["context"],
            "role": c["role"],
            "disabled": c["disabled"],
        }
        for c in candidates
    }
    criteria["abstain"] = "No unique enabled visible control fulfills the intent."
    return {
        "model": "jev-1.13",
        "state": {"intent": intent, "observation": observation},
        "questions": {
            "target": {"type": "choice", "instructions": INSTRUCTION, "criteria": criteria}
        },
    }


def exact_rule(case: dict[str, Any], observation: dict[str, Any]) -> str | None:
    name = case.get("exact_name")
    if not name:
        return None
    matches = [
        c
        for c in observation["candidates"]
        if c["label"] == name and not c["disabled"] and c["visible"]
    ]
    return matches[0]["id"] if len(matches) == 1 else None


def freeze(out: Path) -> None:
    from _jev_browser_fixture import build_cases

    if (out / "frozen.json").exists():
        raise FileExistsError("Frozen inputs already exist")
    manifest = {
        "version": 1,
        "cases": build_cases(),
        "instruction": INSTRUCTION,
        "threshold": THRESHOLD,
        "jev_model": "jev-1.13",
        "luna_model": DEFAULT_JUDGE_MODEL,
        "luna_effort": DEFAULT_JUDGE_REASONING_EFFORT,
        "protocol": {
            "routes": ["baseline", "hybrid"],
            "baseline": "exact rules then Luna",
            "hybrid": "exact rules then Jev; non-abstain selected probability >=0.7 skips Luna; all other responses fall back to Luna",
            "order": "alternate route order by case index",
            "unit": "one rendered fixture and one decision per request, independent fresh page per route",
            "gate": "zero wrong clicks; no task success regression; >=20% paired total wall-time reduction on semantic cases",
            "validation": "fresh visible DOM ID/label/context match, unique locator, enabled/visible, browser click actionability",
            "ground_truth": "server-owned action log, excluded from model inputs",
            "timing": "navigation through observation, decision, actual fallback, validation, click, post-action observation, server readback",
            "scope": "real Chrome fixture interactions; not production Rust-runtime or unrestricted agent-loop acceptance",
        },
    }
    dump(out / "frozen.json", manifest)
    (out / "frozen.sha256").write_text(
        hashlib.sha256((out / "frozen.json").read_bytes()).hexdigest() + "\n"
    )
    print("Frozen", len(manifest["cases"]), "cases", flush=True)


class Decisions:
    def __init__(self, out: Path, manifest: dict[str, Any]):
        self.out = out
        self.manifest = manifest
        self.cases = {c["id"]: c for c in manifest["cases"]}
        self.client = jev_client()
        self.home = prepare_judge_codex_home(out)
        self.lock = threading.Lock()

    def close(self) -> None:
        self.client.close()
        (self.home / "auth.json").unlink(missing_ok=True)

    def luna(self, payload: dict[str, Any], folder: Path) -> dict[str, Any]:
        folder.mkdir(parents=True, exist_ok=True)
        schema = {
            "type": "object",
            "properties": {
                "target": {
                    "type": "string",
                    "enum": list(payload["questions"]["target"]["criteria"]),
                }
            },
            "required": ["target"],
            "additionalProperties": False,
        }
        dump(folder / "schema.json", schema)
        start = time.perf_counter()
        result = run_codex_exec(
            prompt="Do not use tools. Return only the requested JSON decision from the supplied observed UI.\n"
            + json.dumps({"state": payload["state"], "questions": payload["questions"]}),
            artifact_dir=folder,
            output_schema=folder / "schema.json",
            model=self.manifest["luna_model"],
            reasoning_effort=self.manifest["luna_effort"],
            extra_env={"CODEX_HOME": str(self.home)},
            extra_args=["--sandbox", "read-only"],
        )
        if result.exit_code != 0 or not result.last_message_path.exists():
            raise RuntimeError("Luna request failed; inspect retained trace")
        answer = read_last_message(result.last_message_path)["target"]
        events = [
            json.loads(line)
            for line in result.transcript_path.read_text().splitlines()
            if line.strip()
        ]
        return {
            "choice": answer,
            "elapsed_ms": (time.perf_counter() - start) * 1000,
            "usage": [e.get("usage") for e in events if e.get("type") == "turn.completed"],
        }

    def decide(self, request: dict[str, Any]) -> dict[str, Any]:
        with self.lock:
            start = time.perf_counter()
            case = self.cases[request["case_id"]]
            route = request["route"]
            if route not in ("baseline", "hybrid"):
                raise ValueError("Unknown route")
            folder = self.out / "decisions" / f"{case['id']}-{route}"
            if folder.exists():
                raise FileExistsError("Refusing repeated inference for an existing case/route")
            folder.mkdir(parents=True)
            observation = request["observation"]
            payload = payload_for(case["intent"], observation)
            dump(folder / "request.json", payload)
            result: dict[str, Any] = {
                "case_id": case["id"],
                "route": route,
                "choice": exact_rule(case, observation),
                "source": "rules",
                "jev": None,
                "luna": None,
            }
            try:
                if result["choice"] is None and route == "hybrid":
                    response = self.client.call(payload)
                    answer = response["answers"]["target"]
                    result["jev"] = {
                        "response": response,
                        "elapsed_ms": self.client.last_elapsed_ms,
                    }
                    choice = answer["choice"]
                    if (
                        choice != "abstain"
                        and choice in payload["questions"]["target"]["criteria"]
                        and answer["probabilities"][choice] >= self.manifest["threshold"]
                    ):
                        result["choice"] = choice
                        result["source"] = "jev"
                if result["choice"] is None:
                    result["luna"] = self.luna(payload, folder / "luna")
                    result["choice"] = result["luna"]["choice"]
                    result["source"] = "luna"
                result["status"] = "ok"
            except Exception as exc:
                result["status"] = "error"
                result["error_type"] = type(exc).__name__
            result["elapsed_ms"] = (time.perf_counter() - start) * 1000
            dump(folder / "result.json", result)
            print(
                case["id"],
                route,
                result["status"],
                result["source"],
                round(result["elapsed_ms"]),
                flush=True,
            )
            return result


def serve(out: Path) -> None:
    from _jev_browser_fixture import Fixture

    raw = (out / "frozen.json").read_bytes()
    if hashlib.sha256(raw).hexdigest() != (out / "frozen.sha256").read_text().strip():
        raise ValueError("Frozen input hash mismatch")
    manifest = json.loads(raw)
    with Fixture() as fixture:
        if fixture.cases != manifest["cases"]:
            raise ValueError("Fixture source changed since freeze")
        decisions = Decisions(out, manifest)

        class Handler(BaseHTTPRequestHandler):
            def log_message(self, format: str, *args: Any) -> None:
                pass

            def reply(self, code: int, data: Any) -> None:
                encoded = json.dumps(data).encode()
                self.send_response(code)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(encoded)))
                self.end_headers()
                self.wfile.write(encoded)

            def do_GET(self) -> None:
                if self.path != "/manifest":
                    self.reply(404, {"error": "not found"})
                    return
                self.reply(
                    200,
                    {
                        "fixture_origin": fixture.origin,
                        "cases": [
                            {
                                k: v
                                for k, v in c.items()
                                if k in ("id", "family", "kind", "intent", "exact_name")
                            }
                            for c in manifest["cases"]
                        ],
                        "threshold": manifest["threshold"],
                    },
                )

            def do_POST(self) -> None:
                if self.path != "/decide":
                    self.reply(404, {"error": "not found"})
                    return
                try:
                    request = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
                    self.reply(200, decisions.decide(request))
                except Exception as exc:
                    self.reply(500, {"error_type": type(exc).__name__})

        server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        address = {
            "fixture_origin": fixture.origin,
            "decision_origin": f"http://127.0.0.1:{server.server_port}",
        }
        dump(out / "server.json", address)
        print(json.dumps(address), flush=True)
        try:
            server.serve_forever()
        except KeyboardInterrupt:
            pass
        finally:
            server.server_close()
            decisions.close()


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("mode", choices=["freeze", "serve"])
    parser.add_argument("--out", type=Path, default=ROOT / "artifacts/jev-browser-pilot")
    args = parser.parse_args()
    args.out = args.out.resolve()
    args.out.mkdir(parents=True, exist_ok=True)
    {"freeze": freeze, "serve": serve}[args.mode](args.out)


if __name__ == "__main__":
    main()
