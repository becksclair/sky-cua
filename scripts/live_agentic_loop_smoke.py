#!/usr/bin/env python3
"""Installed-plugin agent-loop smoke entrypoint."""

from __future__ import annotations

import argparse

from _agent_mcp_smoke import DEFAULT_PI_SMOKE_MODEL
from live_agent_mcp_smoke import FIXTURES, run_fixture_smoke
from live_fallback_anchor_smoke import run_fallback_anchor_smoke

DEFAULT_AGENT = "pi"
DEFAULT_FIXTURE = "zenity"
ACCEPTANCE_AGENTS = ("opencode", "pi")
# fallback-anchor drives a distinct flow (fallback-anchor proof, not a
# dialog-dismiss task) through live_fallback_anchor_smoke.run_fallback_anchor_smoke,
# so it is offered as a --fixture choice here without joining the FIXTURES
# dict of dialog-dismissal flows in live_agent_mcp_smoke.
FIXTURE_CHOICES = (*FIXTURES.keys(), "fallback-anchor")


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Drive an installed sky-cua MCP server through an external agent CLI."
    )
    parser.add_argument(
        "--agent",
        choices=ACCEPTANCE_AGENTS,
        default=DEFAULT_AGENT,
        help=f"Agent to use for the acceptance loop. Defaults to {DEFAULT_AGENT}.",
    )
    parser.add_argument(
        "--fixture",
        choices=FIXTURE_CHOICES,
        default=DEFAULT_FIXTURE,
        help=f"Desktop fixture to launch. Defaults to {DEFAULT_FIXTURE}.",
    )
    parser.add_argument(
        "--model",
        default=None,
        help=(f"Agent model override. Pi defaults to {DEFAULT_PI_SMOKE_MODEL} when omitted."),
    )
    args = parser.parse_args()
    if args.fixture == "fallback-anchor":
        return run_fallback_anchor_smoke(agent=args.agent, model=args.model)
    return run_fixture_smoke(agent=args.agent, fixture_name=args.fixture, model=args.model)


if __name__ == "__main__":
    raise SystemExit(main())
