"""Regression contracts for workspace dependencies with runtime-sensitive features."""

from __future__ import annotations

import subprocess
import tomllib
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[1]


def test_atspi_excludes_unused_unbounded_p2p_initialization() -> None:
    workspace = tomllib.loads((REPO_ROOT / "Cargo.toml").read_text(encoding="utf-8"))
    atspi = workspace["workspace"]["dependencies"]["atspi"]
    atspi_connection = workspace["workspace"]["dependencies"]["atspi-connection"]

    assert atspi["default-features"] is False
    assert set(atspi["features"]) == {"tokio", "proxies", "wrappers"}
    assert atspi_connection["default-features"] is False
    assert set(atspi_connection["features"]) == {"wrappers"}


def test_resolved_atspi_connection_graph_excludes_p2p() -> None:
    result = subprocess.run(
        [
            "cargo",
            "tree",
            "--edges",
            "features",
            "--invert",
            "atspi-connection",
            "--depth",
            "5",
        ],
        cwd=REPO_ROOT,
        check=True,
        capture_output=True,
        text=True,
    )

    assert 'atspi-connection feature "default"' not in result.stdout
    assert 'atspi-connection feature "p2p"' not in result.stdout
