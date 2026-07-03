"""Pure-logic tests for the harness screen unlock/relock guard.

No real ``loginctl`` runs: the single argv-runner choke point
(``_session_lock._run_loginctl``) is monkeypatched with canned outputs, and the
tests assert the resolution precedence, ``LockedHint`` parsing, and the context
manager's restore semantics from a recorded call list.
"""

from __future__ import annotations

import pytest

import _session_lock


class FakeLoginctl:
    """Records every argv and returns canned ``(rc, stdout)`` responses.

    ``responses`` is keyed by the full argv tuple first, then by the first
    token (the subcommand); an unmatched call returns ``(1, "")``.
    """

    def __init__(self, responses: dict[tuple[str, ...] | str, tuple[int, str]]) -> None:
        self.responses = responses
        self.calls: list[list[str]] = []

    def __call__(self, args: list[str]) -> tuple[int, str]:
        self.calls.append(list(args))
        key = tuple(args)
        if key in self.responses:
            return self.responses[key]
        if args and args[0] in self.responses:
            return self.responses[args[0]]
        return (1, "")

    def subcommands(self) -> list[str]:
        return [call[0] for call in self.calls if call]


def _install(
    monkeypatch: pytest.MonkeyPatch,
    responses: dict[tuple[str, ...] | str, tuple[int, str]],
    *,
    uid: int = 1000,
) -> FakeLoginctl:
    fake = FakeLoginctl(responses)
    monkeypatch.setattr(_session_lock, "_run_loginctl", fake)
    monkeypatch.setattr(_session_lock.os, "getuid", lambda: uid)
    return fake


# ---------------------------------------------------------------------------
# resolve_session_id precedence
# ---------------------------------------------------------------------------


def test_resolve_prefers_env_and_skips_loginctl(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv("XDG_SESSION_ID", "7")
    fake = _install(monkeypatch, {})
    assert _session_lock.resolve_session_id() == "7"
    assert fake.calls == []


def test_resolve_falls_back_to_show_user_display(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.delenv("XDG_SESSION_ID", raising=False)
    fake = _install(
        monkeypatch,
        {("show-user", "1000", "--property=Display", "--value"): (0, "5\n")},
    )
    assert _session_lock.resolve_session_id() == "5"
    assert fake.subcommands() == ["show-user"]


def test_resolve_falls_back_to_list_sessions_seat0(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.delenv("XDG_SESSION_ID", raising=False)
    listing = (
        "   1    0 root            seat0 tty1\n"
        "   3 1000 bex             seat0 tty2\n"
        "   9 1000 bex                   pts/1\n"
    )
    fake = _install(
        monkeypatch,
        {
            "show-user": (0, "\n"),
            ("list-sessions", "--no-legend"): (0, listing),
        },
    )
    # uid 0 seat0 row is skipped (not ours); the pts/1 row has no seat0.
    assert _session_lock.resolve_session_id() == "3"
    assert fake.subcommands() == ["show-user", "list-sessions"]


def test_resolve_returns_none_when_all_empty(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.delenv("XDG_SESSION_ID", raising=False)
    _install(
        monkeypatch,
        {
            "show-user": (0, "\n"),
            ("list-sessions", "--no-legend"): (0, ""),
        },
    )
    assert _session_lock.resolve_session_id() is None


# ---------------------------------------------------------------------------
# LockedHint parsing
# ---------------------------------------------------------------------------


@pytest.mark.parametrize(
    ("rc", "stdout", "expected"),
    [
        (0, "yes\n", True),
        (0, "no\n", False),
        (0, "YES\n", True),
        (0, "whatever\n", None),
        (0, "", None),
        (1, "yes\n", None),
    ],
)
def test_session_locked_parsing(
    monkeypatch: pytest.MonkeyPatch, rc: int, stdout: str, expected: bool | None
) -> None:
    _install(monkeypatch, {"show-session": (rc, stdout)})
    assert _session_lock.session_locked("7") is expected


# ---------------------------------------------------------------------------
# screen_unlocked restore semantics
# ---------------------------------------------------------------------------


def test_cm_locked_on_enter_unlocks_then_relocks(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv("XDG_SESSION_ID", "7")
    fake = _install(
        monkeypatch,
        {
            "show-session": (0, "yes\n"),
            "unlock-session": (0, ""),
            "lock-session": (0, ""),
        },
    )
    ran = False
    with _session_lock.screen_unlocked():
        ran = True
        # Unlock has happened on enter; relock has not yet fired.
        assert ["unlock-session", "7"] in fake.calls
        assert ["lock-session", "7"] not in fake.calls
    assert ran
    assert fake.subcommands() == ["show-session", "unlock-session", "lock-session"]


def test_cm_already_unlocked_touches_nothing(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv("XDG_SESSION_ID", "7")
    fake = _install(monkeypatch, {"show-session": (0, "no\n")})
    with _session_lock.screen_unlocked():
        pass
    assert fake.subcommands() == ["show-session"]


def test_cm_disabled_touches_nothing_even_if_locked(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv("XDG_SESSION_ID", "7")
    fake = _install(monkeypatch, {"show-session": (0, "yes\n")})
    with _session_lock.screen_unlocked(enabled=False):
        pass
    assert fake.calls == []


def test_cm_unresolvable_session_is_noop(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.delenv("XDG_SESSION_ID", raising=False)
    fake = _install(
        monkeypatch,
        {
            "show-user": (0, "\n"),
            ("list-sessions", "--no-legend"): (0, ""),
        },
    )
    with _session_lock.screen_unlocked():
        pass
    # Resolution tried, but no session -> no show-session/unlock/lock.
    assert "show-session" not in fake.subcommands()
    assert "unlock-session" not in fake.subcommands()
    assert "lock-session" not in fake.subcommands()


def test_cm_loginctl_failure_does_not_propagate(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv("XDG_SESSION_ID", "7")
    # unlock-session and lock-session both fail (rc=1); the body must still run
    # and the CM must exit cleanly rather than crashing the harness.
    fake = _install(
        monkeypatch,
        {
            "show-session": (0, "yes\n"),
            "unlock-session": (1, ""),
            "lock-session": (1, ""),
        },
    )
    ran = False
    with _session_lock.screen_unlocked():
        ran = True
    assert ran
    # It still attempted the unlock, and still attempted a restore relock.
    assert "unlock-session" in fake.subcommands()
    assert "lock-session" in fake.subcommands()


def test_run_loginctl_survives_a_wedged_call(monkeypatch: pytest.MonkeyPatch) -> None:
    # A wedged logind/D-Bus raises TimeoutExpired (via subprocess timeout);
    # _run_loginctl must return (127, "") like any other spawn failure so the
    # best-effort guard never blocks the harness.
    import subprocess

    def raise_timeout(*_args: object, **_kwargs: object) -> object:
        raise subprocess.TimeoutExpired(cmd=["loginctl"], timeout=5)

    monkeypatch.setattr(_session_lock.subprocess, "run", raise_timeout)
    assert _session_lock._run_loginctl(["show-session", "3", "-p", "LockedHint"]) == (127, "")
