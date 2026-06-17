#!/usr/bin/env python3
"""Installed-plugin agent-loop smoke entrypoint."""

from __future__ import annotations

import argparse

from _agent_mcp_smoke import DEFAULT_PI_SMOKE_MODEL
from live_agent_mcp_smoke import FIXTURES, run_fixture_smoke

DEFAULT_AGENT = "pi"
DEFAULT_FIXTURE = "zenity"
ACCEPTANCE_AGENTS = ("opencode", "pi")


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
        choices=tuple(FIXTURES.keys()),
        default=DEFAULT_FIXTURE,
        help=f"Desktop fixture to launch. Defaults to {DEFAULT_FIXTURE}.",
    )
    parser.add_argument(
        "--model",
        default=None,
        help=(f"Agent model override. Pi defaults to {DEFAULT_PI_SMOKE_MODEL} when omitted."),
    )
    args = parser.parse_args()
    return run_fixture_smoke(agent=args.agent, fixture_name=args.fixture, model=args.model)


if __name__ == "__main__":
    raise SystemExit(main())
