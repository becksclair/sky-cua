"""Unit tests for the deploy-freshness gate."""

from __future__ import annotations

import os
from pathlib import Path

import pytest

import deploy_freshness as df


def _make_repo(root: Path, *, rs: str = "fn main() {}") -> Path:
    (root / "crates" / "sky-cua-service" / "src").mkdir(parents=True)
    (root / "crates" / "sky-cua-service" / "src" / "lib.rs").write_text(rs, encoding="utf-8")
    (root / "crates" / "sky-cua-service" / "Cargo.toml").write_text("[package]\n", encoding="utf-8")
    (root / "Cargo.toml").write_text("[workspace]\n", encoding="utf-8")
    (root / "Cargo.lock").write_text("# lock\n", encoding="utf-8")
    return root


def test_fingerprint_is_deterministic_and_content_sensitive(tmp_path: Path) -> None:
    repo = _make_repo(tmp_path / "a")
    first = df.runtime_source_fingerprint(repo)
    assert first == df.runtime_source_fingerprint(repo)  # deterministic
    # An identical tree elsewhere yields the same fingerprint (path is relative).
    repo_b = _make_repo(tmp_path / "b")
    assert df.runtime_source_fingerprint(repo_b) == first
    # A content change moves it.
    (repo / "crates" / "sky-cua-service" / "src" / "lib.rs").write_text("fn changed() {}")
    assert df.runtime_source_fingerprint(repo) != first


def test_fingerprint_ignores_non_runtime_files(tmp_path: Path) -> None:
    repo = _make_repo(tmp_path / "r")
    before = df.runtime_source_fingerprint(repo)
    (repo / "docs").mkdir()
    (repo / "docs" / "x.md").write_text("docs change", encoding="utf-8")
    (repo / "scripts").mkdir()
    (repo / "scripts" / "y.py").write_text("print('hi')", encoding="utf-8")
    assert df.runtime_source_fingerprint(repo) == before


def test_stamp_round_trip_and_fresh(tmp_path: Path) -> None:
    repo = _make_repo(tmp_path / "r")
    client = tmp_path / "bin" / "sky-cua-client"
    client.parent.mkdir(parents=True)
    client.write_bytes(b"\x7fELF-fake")
    df.write_build_stamp(client, repo, deployed_at_ms=123)
    stamp = df.read_build_stamp(client)
    assert stamp is not None
    assert stamp["source_fingerprint"] == df.runtime_source_fingerprint(repo)
    assert stamp["deployed_at_ms"] == 123
    result = df.check_client_freshness(client, repo)
    assert result.fresh
    assert "current runtime source" in result.reason


def test_stamp_goes_stale_on_source_change(tmp_path: Path) -> None:
    repo = _make_repo(tmp_path / "r")
    client = tmp_path / "bin" / "sky-cua-client"
    client.parent.mkdir(parents=True)
    client.write_bytes(b"binary")
    df.write_build_stamp(client, repo)
    # Edit runtime source after the stamp was written.
    (repo / "crates" / "sky-cua-service" / "src" / "lib.rs").write_text("fn edited() {}")
    result = df.check_client_freshness(client, repo)
    assert not result.fresh
    assert "changed since" in result.reason
    assert "redeploy" in result.advice


def test_missing_binary_is_stale(tmp_path: Path) -> None:
    repo = _make_repo(tmp_path / "r")
    result = df.check_client_freshness(tmp_path / "nope" / "sky-cua-client", repo)
    assert not result.fresh
    assert "missing" in result.reason


def test_unstamped_binary_mtime_fallback(tmp_path: Path) -> None:
    repo = _make_repo(tmp_path / "r")
    client = tmp_path / "bin" / "sky-cua-client"
    client.parent.mkdir(parents=True)
    client.write_bytes(b"binary")
    source = repo / "crates" / "sky-cua-service" / "src" / "lib.rs"
    # Age every runtime source file, binary newer -> fresh (built after edits).
    for path in [
        repo / "Cargo.toml",
        repo / "Cargo.lock",
        repo / "crates" / "sky-cua-service" / "Cargo.toml",
        source,
    ]:
        os.utime(path, (1000, 1000))
    os.utime(client, (2000, 2000))
    fresh = df.check_client_freshness(client, repo)
    assert fresh.fresh
    assert "newer than all runtime source" in fresh.reason
    # Source newer than binary -> stale (edited after the build).
    os.utime(source, (3000, 3000))
    stale = df.check_client_freshness(client, repo)
    assert not stale.fresh
    assert "newer than this unstamped binary" in stale.reason


def test_allow_stale_env(monkeypatch) -> None:
    monkeypatch.delenv(df.ALLOW_STALE_ENV, raising=False)
    assert not df.allow_stale()
    monkeypatch.setenv(df.ALLOW_STALE_ENV, "1")
    assert df.allow_stale()
    monkeypatch.setenv(df.ALLOW_STALE_ENV, "off")
    assert not df.allow_stale()


def test_deployed_client_path_under_install_dir(tmp_path: Path) -> None:
    assert df.deployed_client_path(tmp_path).parts[-2:] == ("bin", "sky-cua-client")


def _stamped_client(tmp_path: Path) -> tuple[Path, Path]:
    repo = _make_repo(tmp_path / "r")
    client = tmp_path / "bin" / "sky-cua-client"
    client.parent.mkdir(parents=True)
    client.write_bytes(b"bin")
    df.write_build_stamp(client, repo)
    return repo, client


def test_assert_runtime_fresh_passes_and_caches(tmp_path: Path) -> None:
    df._GATE_CHECKED.clear()
    repo, client = _stamped_client(tmp_path)
    df.assert_runtime_fresh(client, repo_root=repo, emit=lambda _m: None)  # no raise
    assert str(client) in df._GATE_CHECKED


def test_assert_runtime_fresh_exits_when_stale(tmp_path: Path) -> None:
    df._GATE_CHECKED.clear()
    repo, client = _stamped_client(tmp_path)
    (repo / "crates" / "sky-cua-service" / "src" / "lib.rs").write_text("fn edited() {}")
    with pytest.raises(SystemExit):
        df.assert_runtime_fresh(client, repo_root=repo, emit=lambda _m: None)


def test_assert_runtime_fresh_override_does_not_exit(tmp_path: Path, monkeypatch) -> None:
    df._GATE_CHECKED.clear()
    monkeypatch.setenv(df.ALLOW_STALE_ENV, "1")
    repo, client = _stamped_client(tmp_path)
    (repo / "Cargo.lock").write_text("# changed\n", encoding="utf-8")  # now stale
    df.assert_runtime_fresh(client, repo_root=repo, emit=lambda _m: None)  # overridden, no raise


def test_gate_spawn_argv_ignores_non_sky_cua_binaries(tmp_path: Path) -> None:
    df._GATE_CHECKED.clear()
    repo = _make_repo(tmp_path / "r")
    # A python (or any non-sky-cua) spawn is never gated, even unstamped.
    df.gate_spawn_argv(["/usr/bin/python3", "-c", "pass"], repo_root=repo, emit=lambda _m: None)


def test_gate_spawn_argv_gates_stale_sky_cua_client(tmp_path: Path) -> None:
    df._GATE_CHECKED.clear()
    repo, client = _stamped_client(tmp_path)
    (repo / "Cargo.lock").write_text("# changed\n", encoding="utf-8")
    with pytest.raises(SystemExit):
        df.gate_spawn_argv([str(client), "mcp"], repo_root=repo, emit=lambda _m: None)
