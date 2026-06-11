"""Shared pytest fixtures for the scripts test suite."""

from __future__ import annotations

from pathlib import Path

import pytest


@pytest.fixture(autouse=True)
def hermetic_machine_config(monkeypatch: pytest.MonkeyPatch, tmp_path: Path) -> None:
    """Keep tests away from the developer's real machine config.

    Installer entry points seed `~/.config/sky-cua/sky-cua.toml` from
    SKY_CUA_BROWSER unconditionally; without this guard, a shell that exports
    that variable would have the test suite rewrite the real file (the Python
    twin of the Rust env_lock guard).
    """
    monkeypatch.delenv("SKY_CUA_BROWSER", raising=False)
    monkeypatch.setenv("SKY_CUA_CONFIG_PATH", str(tmp_path / "sky-cua-test-machine-config.toml"))
