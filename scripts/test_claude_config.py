"""Claude host config merges stay local, selective, and repeatable."""

import json
import os
import stat
import threading
import time
from pathlib import Path

import pytest

import _claude_config as claude
from _claude_config import install_claude_config


def _write_json(path: Path, value: object) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value), encoding="utf-8")


def test_merges_both_hosts_preserving_unrelated_data_modes_and_idempotence(
    tmp_path: Path,
) -> None:
    home = tmp_path / "home"
    root = home / ".local/share/sky-cua"
    paths = (home / ".claude.json", home / ".config/Claude/claude_desktop_config.json")
    original = {
        "oauthAccount": {"existing": "untouched"},
        "projects": {"/workspace": {"mcpServers": {"project-server": {"command": "keep"}}}},
        "mcpServers": {
            "other": {"command": "other", "env": {"SECRET": "not-displayed"}},
            "sky-cua": {
                "type": "http",
                "url": "https://old.invalid",
                "headers": {"Authorization": "old"},
                "customMetadata": "preserved",
                "env": {"SKY_CUA_PROJECT_ROOT": "/old/checkout"},
            },
        },
    }
    for path in paths:
        _write_json(path, original)
        path.chmod(0o640)

    report = install_claude_config(root, home=home, env={"WAYLAND_DISPLAY": "old-display"})

    assert report["code"]["status"] == "updated"
    assert report["desktop"]["status"] == "updated"
    for path in paths:
        updated = json.loads(path.read_text(encoding="utf-8"))
        assert updated["oauthAccount"] == original["oauthAccount"]
        assert updated["projects"] == original["projects"]
        assert updated["mcpServers"]["other"] == original["mcpServers"]["other"]
        assert updated["mcpServers"]["sky-cua"] == {
            "type": "stdio",
            "command": str(root / "bin/sky-cua-client"),
            "args": ["mcp"],
            "env": {},
            "customMetadata": "preserved",
        }
        assert updated["mcpServers"]["node_repl"] == {
            "type": "stdio",
            "command": str(root / "bin/node_repl"),
            "args": [],
            "env": {},
        }
        assert stat.S_IMODE(path.stat().st_mode) == 0o640
    before = [path.stat() for path in paths]
    repeated = install_claude_config(root, home=home, env={})
    assert repeated["code"]["status"] == "unchanged"
    assert repeated["desktop"]["status"] == "unchanged"
    assert [path.stat().st_ino for path in paths] == [item.st_ino for item in before]


def test_absent_hosts_are_skipped_without_creating_directories(tmp_path: Path) -> None:
    home = tmp_path / "absent-home"
    report = install_claude_config(tmp_path / "root", home=home, env={})
    assert report["code"]["status"] == "skipped"
    assert report["desktop"]["status"] == "skipped"
    assert not home.exists()


def test_existing_code_directory_creates_private_user_config_only(tmp_path: Path) -> None:
    (tmp_path / ".claude").mkdir()
    (tmp_path / ".config/Claude").mkdir(parents=True)
    report = install_claude_config(tmp_path / "root", home=tmp_path, env={})
    assert report["code"]["status"] == "created"
    assert "directory exists" in report["code"].get("reason", "")
    assert stat.S_IMODE((tmp_path / ".claude.json").stat().st_mode) == 0o600
    assert report["desktop"]["status"] == "skipped"
    assert not (tmp_path / ".config/Claude/claude_desktop_config.json").exists()


@pytest.mark.parametrize(
    "invalid", ["[]", "{", '{"mcpServers": []}', '{"mcpServers": {"sky-cua": 1}}']
)
def test_both_configs_prevalidated_before_any_mutation(tmp_path: Path, invalid: str) -> None:
    code = tmp_path / ".claude.json"
    _write_json(code, {"mcpServers": {"other": {"command": "keep"}}})
    before = code.read_bytes()
    desktop = tmp_path / ".config/Claude/claude_desktop_config.json"
    desktop.parent.mkdir(parents=True)
    desktop.write_text(invalid, encoding="utf-8")
    with pytest.raises(ValueError):
        install_claude_config(tmp_path / "root", home=tmp_path, env={})
    assert code.read_bytes() == before
    assert desktop.read_text(encoding="utf-8") == invalid


def test_symlinked_config_keeps_link_and_target_mode(tmp_path: Path) -> None:
    target = tmp_path / "dotfiles/claude.json"
    _write_json(target, {"theme": "dark"})
    target.chmod(0o600)
    path = tmp_path / ".claude.json"
    path.symlink_to(target)
    install_claude_config(tmp_path / "root", home=tmp_path, env={})
    assert path.is_symlink()
    assert json.loads(target.read_text(encoding="utf-8"))["theme"] == "dark"
    assert stat.S_IMODE(target.stat().st_mode) == 0o600


def test_custom_config_home_and_explicit_overrides(tmp_path: Path) -> None:
    config_home = tmp_path / "custom-config"
    desktop = config_home / "Claude/claude_desktop_config.json"
    _write_json(desktop, {})
    report = install_claude_config(
        tmp_path / "root",
        home=tmp_path,
        env={
            "XDG_CONFIG_HOME": str(config_home),
            "SKY_CUA_BROWSER_CONTROL_MODE": " strict ",
            "SKY_CUA_PHONE_DIRECT": " ",
            "SKY_CUA_PROJECT_ROOT": "/old-checkout",
            "DISPLAY": ":42",
        },
    )
    assert report["desktop"]["config_path"] == str(desktop)
    servers = json.loads(desktop.read_text(encoding="utf-8"))["mcpServers"]
    assert servers["sky-cua"]["env"] == {"SKY_CUA_BROWSER_CONTROL_MODE": "strict"}
    assert servers["node_repl"]["env"] == servers["sky-cua"]["env"]


@pytest.mark.parametrize("initially_unchanged", [False, True])
def test_rereads_latest_after_preflight_even_for_original_noop(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, initially_unchanged: bool
) -> None:
    path = tmp_path / ".claude.json"
    root = tmp_path / "root"
    _write_json(path, {})
    if initially_unchanged:
        install_claude_config(root, home=tmp_path, env={})
    acquire = claude._ConfigLock.__enter__

    def concurrent_update(lock: claude._ConfigLock) -> claude._ConfigLock:
        # This represents a writer finishing after our snapshot, before we
        # acquire the shared lock. The installer must merge this latest file.
        current = json.loads(path.read_text())
        current["newPreference"] = "preserve"
        current.setdefault("mcpServers", {})["new-server"] = {"command": "keep"}
        current["mcpServers"]["sky-cua"] = {"command": "concurrent", "metadata": "keep"}
        _write_json(path, current)
        path.chmod(0o640)
        return acquire(lock)

    monkeypatch.setattr(claude._ConfigLock, "__enter__", concurrent_update)
    result = install_claude_config(root, home=tmp_path, env={})
    current = json.loads(path.read_text())
    assert result["code"]["status"] == "updated"
    assert current["newPreference"] == "preserve"
    assert current["mcpServers"]["new-server"] == {"command": "keep"}
    assert current["mcpServers"]["sky-cua"]["metadata"] == "keep"
    assert current["mcpServers"]["sky-cua"]["command"] == str(root / "bin/sky-cua-client")
    assert stat.S_IMODE(path.stat().st_mode) == 0o640


def test_readback_accepts_unrelated_concurrent_update(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    path = tmp_path / ".claude.json"
    _write_json(path, {})
    replace = os.replace

    def concurrent_update(
        source: str | os.PathLike[str], destination: str | os.PathLike[str]
    ) -> None:
        replace(source, destination)
        current = json.loads(path.read_text())
        current["preference"] = "new"
        current["mcpServers"]["other"] = {"command": "keep"}
        current["mcpServers"]["sky-cua"]["metadata"] = "keep"
        _write_json(path, current)

    monkeypatch.setattr(claude.os, "replace", concurrent_update)
    install_claude_config(tmp_path / "root", home=tmp_path, env={})
    assert json.loads(path.read_text())["mcpServers"]["other"] == {"command": "keep"}


def test_readback_rejects_changed_managed_field(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    path = tmp_path / ".claude.json"
    _write_json(path, {})
    replace = os.replace

    def concurrent_update(
        source: str | os.PathLike[str], destination: str | os.PathLike[str]
    ) -> None:
        replace(source, destination)
        current = json.loads(path.read_text())
        current["mcpServers"]["sky-cua"]["command"] = "changed"
        _write_json(path, current)

    monkeypatch.setattr(claude.os, "replace", concurrent_update)
    with pytest.raises(RuntimeError, match="readback differs"):
        install_claude_config(tmp_path / "root", home=tmp_path, env={})


def _fast_lock(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(claude, "_LOCK_POLL_SECONDS", 0.005)
    monkeypatch.setattr(claude, "_LOCK_HEARTBEAT_SECONDS", 0.02)
    monkeypatch.setattr(claude, "_LOCK_STALE_SECONDS", 0.1)
    monkeypatch.setattr(claude, "_LOCK_TIMEOUT_SECONDS", 0.3)


def test_fresh_lock_times_out_without_any_config_write(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    _fast_lock(monkeypatch)
    path = tmp_path / ".claude.json"
    _write_json(path, {})
    before = path.read_bytes()
    with claude._ConfigLock(path) as holder:
        with pytest.raises(RuntimeError, match="Timed out"):
            install_claude_config(tmp_path / "root", home=tmp_path, env={})
        holder.assert_owned()
        assert path.read_bytes() == before
        # More than the injected stale period elapsed; the heartbeat kept the
        # lock live throughout the contender's bounded wait.
        assert time.time_ns() - holder.path.stat().st_mtime_ns < 100_000_000
    assert not Path(f"{path}.lock").exists()


def test_waiter_merges_after_live_lock_holder_releases(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    _fast_lock(monkeypatch)
    path = tmp_path / ".claude.json"
    _write_json(path, {})
    attempted = threading.Event()
    acquire = claude._ConfigLock.__enter__
    errors: list[BaseException] = []
    holder = acquire(claude._ConfigLock(path))

    def mark_attempt(lock: claude._ConfigLock) -> claude._ConfigLock:
        attempted.set()
        return acquire(lock)

    monkeypatch.setattr(claude._ConfigLock, "__enter__", mark_attempt)

    def install() -> None:
        try:
            install_claude_config(tmp_path / "root", home=tmp_path, env={})
        except BaseException as error:
            errors.append(error)

    worker = threading.Thread(target=install)
    worker.start()
    try:
        assert attempted.wait(1)
        assert json.loads(path.read_text()) == {}
        _write_json(path, {"mcpServers": {"concurrent-server": {"command": "keep"}}})
    finally:
        holder.__exit__(None, None, None)
        worker.join(timeout=2)
    assert not worker.is_alive()
    assert not errors
    assert json.loads(path.read_text())["mcpServers"]["concurrent-server"] == {"command": "keep"}


def test_stale_lock_is_reclaimed(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    _fast_lock(monkeypatch)
    path = tmp_path / ".claude.json"
    _write_json(path, {})
    lock_path = Path(f"{path}.lock")
    lock_path.mkdir()
    os.utime(lock_path, (time.time() - 1, time.time() - 1))
    install_claude_config(tmp_path / "root", home=tmp_path, env={})
    assert "sky-cua" in json.loads(path.read_text())["mcpServers"]
    assert not lock_path.exists()


def test_stale_observation_cannot_reclaim_refreshed_lock(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    _fast_lock(monkeypatch)
    monkeypatch.setattr(claude, "_LOCK_STALE_SECONDS", 10)
    monkeypatch.setattr(claude, "_LOCK_TIMEOUT_SECONDS", 0.03)
    path = tmp_path / ".claude.json"
    lock_path = Path(f"{path}.lock")
    lock_path.mkdir()
    os.utime(lock_path, (time.time() - 20, time.time() - 20))
    original = claude._ConfigLock._current_stamp
    calls = 0

    def refresh_between_observations(lock: claude._ConfigLock) -> tuple[int, int, int]:
        nonlocal calls
        calls += 1
        if calls == 2:
            os.utime(lock_path, None)
        return original(lock)

    monkeypatch.setattr(claude._ConfigLock, "_current_stamp", refresh_between_observations)
    with pytest.raises(RuntimeError, match="Timed out"), claude._ConfigLock(path):
        pytest.fail("refreshed lock must not be reclaimed")
    assert lock_path.is_dir()


def test_ownership_loss_before_replace_aborts_and_keeps_other_lock(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    path = tmp_path / ".claude.json"
    _write_json(path, {})
    before = path.read_bytes()
    fchmod = os.fchmod
    lock_path = Path(f"{path}.lock")
    displaced = tmp_path / "displaced-lock"

    def lose_lock(descriptor: int, mode: int) -> None:
        fchmod(descriptor, mode)
        lock_path.rename(displaced)
        lock_path.mkdir()

    monkeypatch.setattr(claude.os, "fchmod", lose_lock)
    with pytest.raises(RuntimeError, match="ownership changed"):
        install_claude_config(tmp_path / "root", home=tmp_path, env={})
    assert path.read_bytes() == before
    assert lock_path.is_dir()
    assert not list(tmp_path.glob(".*.tmp-*"))


def test_symlink_locks_logical_path_and_preserves_target(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    _fast_lock(monkeypatch)
    monkeypatch.setattr(claude, "_LOCK_TIMEOUT_SECONDS", 0.03)
    target = tmp_path / "dotfiles/claude.json"
    _write_json(target, {})
    path = tmp_path / ".claude.json"
    path.symlink_to(target)
    lock_path = Path(f"{path}.lock")
    lock_path.mkdir()
    with pytest.raises(RuntimeError, match="Timed out"):
        install_claude_config(tmp_path / "root", home=tmp_path, env={})
    assert path.is_symlink()
    assert json.loads(target.read_text()) == {}
    assert not Path(f"{target}.lock").exists()


def test_second_prevalidation_under_locks_prevents_partial_write(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    code = tmp_path / ".claude.json"
    desktop = tmp_path / ".config/Claude/claude_desktop_config.json"
    _write_json(code, {})
    _write_json(desktop, {})
    before = code.read_bytes()
    acquire = claude._ConfigLock.__enter__

    def corrupt_after_preflight(lock: claude._ConfigLock) -> claude._ConfigLock:
        desktop.write_text("{")
        return acquire(lock)

    monkeypatch.setattr(claude._ConfigLock, "__enter__", corrupt_after_preflight)
    with pytest.raises(ValueError, match="not valid"):
        install_claude_config(tmp_path / "root", home=tmp_path, env={})
    assert code.read_bytes() == before
    assert not Path(f"{code}.lock").exists()
    assert not Path(f"{desktop}.lock").exists()


def test_changed_lock_mtime_is_not_released_as_ours(tmp_path: Path) -> None:
    path = tmp_path / ".claude.json"
    lock = claude._ConfigLock(path).__enter__()
    previous = lock.path.stat().st_mtime_ns
    os.utime(lock.path, ns=(previous + 1_000_000, previous + 1_000_000))
    with pytest.raises(RuntimeError, match="ownership changed"):
        lock.__exit__(None, None, None)
    assert lock.path.is_dir()


def test_heartbeat_failure_prevents_commit(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    _fast_lock(monkeypatch)
    path = tmp_path / ".claude.json"
    _write_json(path, {})
    before = path.read_bytes()
    lock = claude._ConfigLock(path).__enter__()

    def heartbeat_fails(*args: object, **kwargs: object) -> None:
        raise OSError("injected heartbeat failure")

    monkeypatch.setattr(claude.os, "utime", heartbeat_fails)
    assert lock._thread is not None
    lock._thread.join(timeout=1)
    assert not lock._thread.is_alive()
    plan = claude._plan_config(
        path, create=True, servers={"sky-cua": {"command": "new"}}, reason=""
    )
    with pytest.raises(RuntimeError, match="heartbeat failed"):
        claude._apply_config(plan, lock, {"sky-cua": {"command": "new"}})
    with pytest.raises(RuntimeError, match="heartbeat failed"):
        lock.__exit__(None, None, None)
    assert path.read_bytes() == before
    assert lock.path.is_dir()


def test_heartbeat_cannot_adopt_replacement_lock(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    _fast_lock(monkeypatch)
    path = tmp_path / ".claude.json"
    _write_json(path, {})
    before = path.read_bytes()
    lock = claude._ConfigLock(path).__enter__()
    displaced = tmp_path / "displaced-lock"
    utime = os.utime

    def replace_during_refresh(*args: object, **kwargs: object) -> None:
        # The original check passed, but a new owner creates its lock before
        # our heartbeat touches the configured lock path.
        lock.path.rename(displaced)
        lock.path.mkdir()
        utime(lock.path, None)

    monkeypatch.setattr(claude.os, "utime", replace_during_refresh)
    assert lock._thread is not None
    lock._thread.join(timeout=1)
    assert not lock._thread.is_alive()
    plan = claude._plan_config(
        path, create=True, servers={"sky-cua": {"command": "new"}}, reason=""
    )
    with pytest.raises(RuntimeError, match=r"heartbeat failed.*ownership changed"):
        claude._apply_config(plan, lock, {"sky-cua": {"command": "new"}})
    with pytest.raises(RuntimeError, match=r"heartbeat failed.*ownership changed"):
        lock.__exit__(None, None, None)
    assert path.read_bytes() == before
    assert lock.path.is_dir()
    assert lock.path.stat().st_ino != displaced.stat().st_ino
