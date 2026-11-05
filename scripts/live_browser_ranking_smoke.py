"""Opt-in browser_rank acceptance through compiled MCP and real TodoMVC state."""

from __future__ import annotations

import argparse
import json
from pathlib import Path
from typing import Any

from _jev_browser_workflow import candidates_for, target_correct, workflow_proof
from _mcp_stdio import McpClient, isolated_daemon_env
from jev_real_browser import ROOT, STAGES, Browser
from jev_ui_experiment import dump
from live_desktop_smoke import grouped_structured_result


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--client", type=Path, required=True)
    parser.add_argument("--service", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--use-machine-config", action="store_true")
    args = parser.parse_args()
    out = args.out.resolve()
    out.mkdir(parents=True, exist_ok=False)
    browser = Browser(
        out,
        ROOT / "artifacts/jev-real-browser/vendor",
        client_path=args.client.resolve(),
        service_path=args.service.resolve(),
        ranking_enabled=None if args.use_machine_config else True,
    )
    results: list[dict[str, Any]] = []
    try:
        assert browser.client is not None
        names = {t["name"] for t in browser.client.tools_list()}
        assert "browser_rank" in names
        disabled = McpClient(
            [str(args.client.resolve()), "mcp"],
            extra_env=isolated_daemon_env(
                {
                    "SKY_CUA_SERVICE_PATH": str(args.service.resolve()),
                    "SKY_CUA_SERVICE_SOCKET_PATH": str(browser.socket),
                    "SKY_CUA_BROWSER": "chromium",
                    "SKY_CUA_BROWSER_RANKING": "0",
                }
            ),
        )
        try:
            disabled.initialize()
            assert "browser_rank" not in {t["name"] for t in disabled.tools_list()}
            denied = disabled.tools_call(
                20, "browser_rank", {"tab_id": "unused", "intent": "unused"}
            )
            assert denied.get("isError")
            dump(out / "disabled.json", denied)
        finally:
            disabled.close()
        hidden_fixture = Path(browser.temp.name) / "hidden-fixture"
        hidden_fixture.mkdir()
        (hidden_fixture / "index.html").write_text(
            '<button style="visibility:hidden;width:120px;height:30px">Hidden target</button>'
        )
        hidden_tab = grouped_structured_result(
            browser.call("browser_open", {"url": browser.origin(hidden_fixture)})
        )["tab"]["tab_id"]
        hidden_observed = browser.observe(hidden_tab, out, "hidden-control")
        hidden = hidden_observed["semantic_snapshot"]["elements"][0]
        assert hidden["bounds"]["width"] > 0 and hidden["visible"] is False
        hidden_result = grouped_structured_result(
            browser.call(
                "browser_rank",
                {
                    "tab_id": hidden_tab,
                    "intent": "Select Hidden target",
                    "exact_name": "Hidden target",
                },
            )
        )
        assert hidden_result["status"] == "unresolved" and "ref" not in hidden_result
        assert hidden_result["fallback_reason"] == "no_actionable_candidates"
        dump(out / "hidden-result.json", hidden_result)
        (hidden_fixture / "transparent.html").write_text(
            '<button style="opacity:0;width:120px;height:30px">Transparent hit target</button>'
        )
        transparent_url = browser.origin(hidden_fixture) + "transparent.html"
        transparent_tab = grouped_structured_result(
            browser.call("browser_open", {"url": transparent_url})
        )["tab"]["tab_id"]
        transparent_result = grouped_structured_result(
            browser.call(
                "browser_rank",
                {
                    "tab_id": transparent_tab,
                    "intent": "Select the transparent input hit target",
                    "exact_name": "Transparent hit target",
                },
            )
        )
        assert transparent_result["status"] == "selected"
        assert transparent_result["source"] == "deterministic"
        dump(out / "transparent-result.json", transparent_result)
        url = browser.origin()
        opened = grouped_structured_result(browser.call("browser_open", {"url": url}))
        tab = opened["tab"]["tab_id"]

        def rank(stage: str, *, exact: bool = False, intent: str | None = None) -> dict[str, Any]:
            label = f"{len(results)}-{stage}"
            before = browser.observe(tab, out, label + "-before")
            cs = candidates_for(before["semantic_snapshot"])
            bounds = browser.oracle(url).get("complete_bounds") if stage == "complete" else None
            request = {"tab_id": tab, "intent": intent or STAGES[stage]}
            if exact:
                request.update(exact_name="New Todo Input", exact_role="input")
            raw = browser.call("browser_rank", request)
            dump(out / (label + "-rank.json"), raw)
            r = grouped_structured_result(raw)
            results.append(r)
            if intent is not None:
                assert r["status"] == "unresolved" and "ref" not in r
            else:
                assert r["status"] == "selected"
                candidate = next((c for c in cs if c["ref"] == r["ref"]), None)
                assert target_correct(stage, candidate, complete_bounds=bounds), (
                    f"Wrong {stage} target"
                )
            return r

        r = rank("add", exact=True)
        assert (
            r["source"] == "deterministic"
            and r["timing_ms"]["jev"] is None
            and r["timing_ms"]["terra"] is None
        )
        r = rank("add")
        assert r["source"] == "jev", r
        browser.input(r, "type_text", ref=r["ref"], text="Pack travel bag")
        s = browser.observe(tab, out, "typed-first")
        browser.input(s, "press_key", key="Enter")
        browser.observe(tab, out, "added-first")
        r = rank("complete")
        assert r["source"] == "terra", r
        browser.input(r, "click", ref=r["ref"])
        browser.observe(tab, out, "completed-first")
        r = rank("add", exact=True)
        browser.input(r, "type_text", ref=r["ref"], text="Buy train ticket")
        s = browser.observe(tab, out, "typed-second")
        browser.input(s, "press_key", key="Enter")
        browser.observe(tab, out, "added-second")
        before_filter = browser.oracle(url)
        dump(out / "before-filter.json", before_filter)
        r = rank("filter_active")
        browser.input(r, "click", ref=r["ref"])
        browser.observe(tab, out, "final")
        final = browser.oracle(url)
        dump(out / "final-oracle.json", final)
        proof = workflow_proof(
            before_filter["tasks"], final["page"], "Pack travel bag", "Buy train ticket"
        )
        assert proof["success"]
        dump(out / "state-proof.json", proof)
        r = rank(
            "filter_active", intent="Find a control that launches a spacecraft from this page."
        )
        assert r["source"] == "terra" and "terra_abstained" in r["fallback_reason"]
        dump(
            out / "results.json",
            {
                "passed": True,
                "sources": [r["source"] for r in results],
                "selections": results,
                "state_proof": proof,
                "explicit_disable_verified": True,
                "machine_config_enabled": args.use_machine_config,
            },
        )
        print(json.dumps({"passed": True, "sources": [r["source"] for r in results]}), flush=True)
    finally:
        browser.close()


if __name__ == "__main__":
    main()
