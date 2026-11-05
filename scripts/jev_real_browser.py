"""Opt-in unmodified TodoMVC workflow through the real sky-cua browser bridge."""

from __future__ import annotations

import argparse
import functools
import hashlib
import http.server
import json
import shutil
import tempfile
import threading
import time
from pathlib import Path
from typing import Any

from _chrome_bridge import (
    DEFAULT_EXTENSION_ID,
    HOST_NAME,
    browser_command,
    default_extension_dir,
    host_pid_from_socket,
    launch_browser,
    load_targets,
    terminate_browser,
    wait_for_devtools_port,
    wait_for_extension_target,
    wait_for_host_process,
    wait_for_socket,
)
from _jev_desktop_models import DesktopJudges
from _mcp_stdio import McpClient, isolated_daemon_env, stop_service_processes_for_socket
from jev_ui_experiment import dump
from live_chrome_host_client_smoke import evaluate
from live_desktop_smoke import grouped_structured_result, require_ok

ROOT = Path(__file__).resolve().parents[1]
STAGES = {
    "add": "Choose the field used to add a new task to the task list.",
    "complete": "Use the individual task checkbox to mark the existing task completed; leave the bulk-toggle control alone.",
    "filter_active": "Show only unfinished tasks and hide completed tasks.",
}


class QuietHandler(http.server.SimpleHTTPRequestHandler):
    def log_message(self, format: str, *args: Any) -> None:
        pass


class Browser:
    def __init__(
        self,
        out: Path,
        vendor: Path,
        *,
        client_path: Path | None = None,
        service_path: Path | None = None,
        ranking_enabled: bool | None = False,
    ):
        self.out = out
        self.vendor = vendor
        self.servers = []
        self.seq = 10
        self.client = None
        self.proc = None
        self.temp = tempfile.TemporaryDirectory(prefix="jev-")
        root = Path(self.temp.name)
        profile = root / "p"
        extension = root / "e"
        sockets = root / "s"
        sessions = root / "logs"
        sockets.mkdir()
        sessions.mkdir()
        self.socket = root / "service.sock"
        try:
            shutil.copytree(default_extension_dir(), extension)
            native = profile / "NativeMessagingHosts"
            native.mkdir(parents=True)
            host = Path.home() / ".local/bin/sky-cua-chrome-host"
            dump(
                native / f"{HOST_NAME}.json",
                {
                    "name": HOST_NAME,
                    "description": "Disposable TodoMVC pilot host",
                    "type": "stdio",
                    "path": str(host),
                    "allowed_origins": [f"chrome-extension://{DEFAULT_EXTENSION_ID}/"],
                },
            )
            selected = browser_command("chromium")
            self.proc = launch_browser(
                selected.command,
                user_data_dir=profile,
                extension_dir=extension,
                socket_dir=sockets,
                sessions_dir=sessions,
                stderr_path=out / "browser.stderr",
            )
            self.port = wait_for_devtools_port(profile, self.proc)
            wait_for_extension_target(self.port, DEFAULT_EXTENSION_ID)
            sock = wait_for_socket(sockets)
            pid = host_pid_from_socket(sock)
            hostproof = wait_for_host_process(host, pid)
            if hostproof is None:
                raise RuntimeError("Native host executable mismatch")
            dump(
                out / "bridge-proof.json",
                {
                    "host": hostproof,
                    "socket": str(sock),
                    "browser": "chromium",
                    "extension_id": DEFAULT_EXTENSION_ID,
                    "profile_manifest_only": True,
                },
            )
            client_env = {
                "SKY_CUA_BROWSER": "chromium",
                "CODEX_BROWSER_USE_SOCKET_DIR": str(sockets),
                "SKY_CUA_BROWSER_USE_SOCKET_DIR": str(sockets),
                "SKY_CUA_SERVICE_SOCKET_PATH": str(self.socket),
            }
            if ranking_enabled is not None:
                client_env["SKY_CUA_BROWSER_RANKING"] = "1" if ranking_enabled else "0"
            if service_path is not None:
                client_env["SKY_CUA_SERVICE_PATH"] = str(service_path.resolve())
            self.client = McpClient(
                [
                    str(client_path or Path.home() / ".local/share/sky-cua/bin/sky-cua-client"),
                    "mcp",
                ],
                extra_env=isolated_daemon_env(client_env),
                read_timeout=45 if ranking_enabled is False else 120,
            )
            self.client.initialize()
            dump(out / "schemas.json", self.client.tools_list())
        except BaseException:
            self.close()
            raise

    def call(self, tool: str, args: dict[str, Any]) -> dict[str, Any]:
        assert self.client is not None
        self.seq += 1
        r = self.client.tools_call(self.seq, tool, args)
        require_ok(r, tool)
        return r

    def origin(self, directory: Path | None = None) -> str:
        server = http.server.ThreadingHTTPServer(
            ("127.0.0.1", 0),
            functools.partial(QuietHandler, directory=str(directory or self.vendor)),
        )
        self.servers.append(server)
        threading.Thread(target=server.serve_forever, daemon=True).start()
        return f"http://127.0.0.1:{server.server_port}/"

    def observe(self, tab: str, folder: Path, name: str) -> dict[str, Any]:
        r = self.call("observe", {"surface": "browser", "target": "user_chrome", "tab_id": tab})
        dump(folder / f"{name}.json", r)
        return grouped_structured_result(r)

    def input(self, snapshot: dict[str, Any], op: str, **kwargs: Any) -> dict[str, Any]:
        return self.call(
            "browser_input",
            {
                "target": "user_chrome",
                "tab_id": snapshot["tab_id"],
                "appshot_id": snapshot["appshot_id"],
                "operation": op,
                **kwargs,
            },
        )

    def oracle(self, url: str) -> dict[str, Any]:
        targets = [t for t in load_targets(self.port) if str(t.get("url", "")).startswith(url)]
        if len(targets) != 1:
            raise RuntimeError("No unique owned app page for oracle")
        result = evaluate(
            str(targets[0]["webSocketDebuggerUrl"]),
            "({storage:Object.fromEntries(Object.entries(localStorage)),tasks:Array.from(document.querySelectorAll('.todo-list li')).map(e=>({title:e.querySelector('label')?.textContent,completed:e.querySelector('input.toggle')?.checked})),complete_bounds:(()=>{const e=document.querySelector('.todo-list input.toggle');if(!e)return null;const r=e.getBoundingClientRect();return {x:r.x,y:r.y,width:r.width,height:r.height}})(),page:{visible_tasks:Array.from(document.querySelectorAll('.todo-list li')).filter(e=>e.getBoundingClientRect().height>0).map(e=>e.querySelector('label')?.textContent),hash:location.hash}})",
        )
        if not isinstance(result, dict):
            raise RuntimeError("Bad oracle response")
        return result

    def close(self) -> None:
        if self.client is not None:
            self.client.close()
        stop_service_processes_for_socket(self.socket)
        if self.proc is not None:
            terminate_browser(self.proc, False)
        for s in self.servers:
            s.shutdown()
            s.server_close()
        self.temp.cleanup()


def flow(
    browser: Browser, judges: Any, folder: Path, route: str, dry: bool = False
) -> dict[str, Any]:
    from _jev_browser_workflow import (
        candidates_for,
        page_text_for,
        public_candidates,
        target_correct,
        workflow_proof,
    )

    folder.mkdir(parents=True, exist_ok=False)
    record: dict[str, Any] = {
        "route": route,
        "steps": [],
        "terra_calls": 0,
        "jev_calls": 0,
        "wrong_targets": 0,
        "success": False,
        "status": "running",
    }
    a = "Pack travel bag"
    b = "Buy train ticket"
    url = browser.origin()
    started = time.perf_counter()

    def persist() -> None:
        dump(folder / "progress.json", record)

    def ranked(stage: str, s: dict[str, Any], name: str) -> tuple[dict[str, Any], dict[str, Any]]:
        sd = folder / name
        sd.mkdir()
        cs = candidates_for(s["semantic_snapshot"])
        page_text = page_text_for(s["semantic_snapshot"])
        dump(sd / "candidates.json", cs)
        oracle_bounds = browser.oracle(url).get("complete_bounds") if stage == "complete" else None
        step: dict[str, Any] = {"stage": stage, "intent": STAGES[stage]}
        record["steps"].append(step)
        persist()
        if dry:
            for c in cs:
                print(stage, c["id"], c["tag"], repr(c["label"]), flush=True)
            good = [c for c in cs if target_correct(stage, c, complete_bounds=oracle_bounds)]
            if len(good) != 1:
                raise RuntimeError("Dry oracle needs inspection")
            choice = good[0]["id"]
            step["source"] = "oracle_preflight"
        else:
            choice = None
            if route == "hybrid":
                record["jev_calls"] += 1
                persist()
                step["jev"] = judges.select(
                    "jev",
                    STAGES[stage],
                    public_candidates(cs),
                    sd / "jev",
                    page_text=page_text,
                )
                persist()
                j = step["jev"]
                if j["choice"] != "abstain" and j.get("probability", 0) >= 0.8:
                    choice = j["choice"]
                    step["source"] = "jev"
            if choice is None:
                record["terra_calls"] += 1
                persist()
                step["terra"] = judges.select(
                    "terra",
                    STAGES[stage],
                    public_candidates(cs),
                    sd / "terra",
                    page_text=page_text,
                )
                choice = step["terra"]["choice"]
                step["source"] = "terra"
        c = next((c for c in cs if c["id"] == choice), None)
        step["choice"] = choice
        step["target_correct"] = target_correct(stage, c, complete_bounds=oracle_bounds)
        persist()
        if not step["target_correct"]:
            record["wrong_targets"] += 1
            persist()
            raise RuntimeError("Wrong selected target; rejected before action")
        assert c is not None
        fresh = browser.observe(s["tab_id"], sd, "fresh")
        live = [
            e
            for e in fresh["semantic_snapshot"]["elements"]
            if e["ref"] == c["ref"]
            and (e.get("name") or "") == c["label"]
            and not e.get("disabled")
        ]
        if len(live) != 1:
            raise RuntimeError("Reference identity changed")
        return fresh, c

    try:
        raw = browser.call("browser_open", {"target": "user_chrome", "url": url})
        dump(folder / "open.json", raw)
        opened = grouped_structured_result(raw)
        tab = str(opened["tab"]["tab_id"])
        s = browser.observe(tab, folder, "initial")
        s, c = ranked("add", s, "add-first")
        dump(folder / "type-first.json", browser.input(s, "type_text", ref=c["ref"], text=a))
        s = browser.observe(tab, folder, "typed-first")
        dump(folder / "enter-first.json", browser.input(s, "press_key", key="Enter"))
        s = browser.observe(tab, folder, "added-first")
        s, c = ranked("complete", s, "complete-first")
        dump(folder / "complete-click.json", browser.input(s, "click", ref=c["ref"]))
        s = browser.observe(tab, folder, "completed-first")
        # Reusing the already established add-field identity is a common deterministic step.
        fields = [
            c
            for c in candidates_for(s["semantic_snapshot"])
            if c["label"] == "New Todo Input" and not c["disabled"]
        ]
        if len(fields) != 1:
            raise RuntimeError("Add field no longer unique")
        dump(
            folder / "type-second.json", browser.input(s, "type_text", ref=fields[0]["ref"], text=b)
        )
        s = browser.observe(tab, folder, "typed-second")
        dump(folder / "enter-second.json", browser.input(s, "press_key", key="Enter"))
        s = browser.observe(tab, folder, "added-second")
        before_filter = browser.oracle(url)
        dump(folder / "before-filter-oracle.json", before_filter)
        s, c = ranked("filter_active", s, "filter-active")
        dump(folder / "filter-click.json", browser.input(s, "click", ref=c["ref"]))
        browser.observe(tab, folder, "final")
        oracle = browser.oracle(url)
        dump(folder / "oracle.json", oracle)
        proof = workflow_proof(before_filter["tasks"], oracle["page"], a, b)
        dump(folder / "state-proof.json", proof)
        record["success"] = proof["success"]
        record["status"] = "ok"
        record["total_ms"] = (time.perf_counter() - started) * 1000
    except Exception as exc:
        (folder / "error.txt").write_text(str(exc))
        record.update(
            status="error", error=str(exc)[:300], total_ms=(time.perf_counter() - started) * 1000
        )
    persist()
    dump(folder / "result.json", record)
    print(json.dumps({k: v for k, v in record.items() if k != "steps"}), flush=True)
    return record


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("mode", choices=["dry", "run"])
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    out = args.out.resolve()
    out.mkdir(parents=True, exist_ok=True)
    vendor = ROOT / "artifacts/jev-real-browser/vendor"
    protocol = {
        "app": "Unmodified TodoMVC React distributed build",
        "source_url": "https://todomvc.com/examples/react/dist/",
        "source_hashes": {
            str(p.relative_to(ROOT)): hashlib.sha256(p.read_bytes()).hexdigest()
            for p in [
                Path(__file__),
                ROOT / "scripts/_jev_browser_workflow.py",
                ROOT / "scripts/_jev_desktop_models.py",
            ]
        },
        "asset_hashes": {
            p.name: hashlib.sha256(p.read_bytes()).hexdigest()
            for p in vendor.iterdir()
            if p.is_file()
        },
        "stages": STAGES,
        "threshold": 0.8,
        "payload": "page text once in state.page_text; candidate context is the link destination only",
        "model": "gpt-6.1-terra",
        "effort": "medium",
        "warmups": "one complete flow per route excluded",
        "pairs": 3,
        "order": [["baseline", "hybrid"], ["hybrid", "baseline"], ["baseline", "hybrid"]],
        "timing": "browser_open through independent DOM checkbox/task state and active-view readback; browser/bridge startup excluded",
        "gate": "all task proofs correct; zero wrong targets; >=20% total workflow time reduction",
        "scope": "caller-known workflow, model target selection before each semantic action; no planning turn before routing; no general autonomous-agent claim",
    }
    if (out / "frozen.json").exists():
        raise FileExistsError("Fresh output directory required")
    dump(out / "frozen.json", protocol)
    (out / "frozen.sha256").write_text(
        hashlib.sha256((out / "frozen.json").read_bytes()).hexdigest()
    )
    browser = Browser(out, vendor)
    try:
        if args.mode == "dry":
            return 0 if flow(browser, None, out / "dry", "baseline", True)["success"] else 1
        from _jev_browser_workflow import paired_summary

        with DesktopJudges(out) as judges:
            for route in ["baseline", "hybrid"]:
                if not flow(browser, judges, out / f"warmup-{route}", route)["success"]:
                    return 1
            records = []
            for i, routes in enumerate(protocol["order"]):
                for route in routes:
                    r = flow(browser, judges, out / f"pair-{i}-{route}", route)
                    r["pair"] = i
                    records.append(r)
                    dump(out / "results.json", records)
                    if not r["success"]:
                        return 1
            summary = paired_summary(records)
            dump(out / "summary.json", summary)
            print(json.dumps(summary), flush=True)
            return 0 if summary["passed"] else 1
    finally:
        browser.close()


if __name__ == "__main__":
    raise SystemExit(main())
