"""Screen unlock/relock guard for the desktop capture harnesses.

The visual harnesses (``overlay_motion_animations.py`` stills/portal recording
and ``live_agent_cursor_kde_smoke.py --mode layer-shell-motion-glide``) record
the real desktop. When the KDE Plasma 6 Wayland session is locked, the
compositor renders the lock-screen greeter over everything, so the recording
captures the greeter instead of the overlay. This module unlocks the session
for the duration of a harness run and restores the original lock state on the
way out (it relocks only if it did the unlocking).

Mechanism (verified on KDE Plasma 6 Wayland; no passwords or PAM):

- Unlock without a prompt: ``loginctl unlock-session <SESSION_ID>`` (singular,
  the caller's own session — logind authorizes the session owner with no polkit
  prompt). Never ``unlock-sessions`` (plural), which hits polkit ``auth_admin``.
- Detect: ``loginctl show-session <SESSION_ID> -p LockedHint --value`` -> the
  literal ``yes`` / ``no``.
- Relock: ``loginctl lock-session <SESSION_ID>``.

Session-id resolution is the fragile part: the harness shell often lacks
``XDG_SESSION_ID`` (we have seen ``XDG_SESSION_TYPE=tty`` there). It is resolved
in order: the ``XDG_SESSION_ID`` env var, then the user's primary graphical
session via ``loginctl show-user <uid> --property=Display --value``, then a
parse of ``loginctl list-sessions --no-legend`` for the current user's seat0
session.

Everything here is best-effort: a ``loginctl`` failure prints a warning to
stderr and continues. Lock handling must never crash the harness — a locked
screen degrades the recording, it does not justify aborting the run.
"""

from __future__ import annotations

import contextlib
import os
import subprocess
import sys
from collections.abc import Iterator


def _warn(message: str) -> None:
    """Emits a best-effort warning to stderr; never raises."""
    with contextlib.suppress(Exception):
        print(f"[session-lock] {message}", file=sys.stderr)


def _run_loginctl(args: list[str]) -> tuple[int, str]:
    """Runs ``loginctl <args>`` and returns ``(returncode, stdout)``.

    This is the single choke point for every ``loginctl`` invocation so tests
    can monkeypatch it with canned outputs. It never raises and never blocks the
    harness for long: a missing binary, spawn failure, or a wedged logind/D-Bus
    (each call bounded by the timeout) surfaces as a non-zero return code with
    empty stdout, honouring the best-effort contract in this module's docstring.
    """
    try:
        completed = subprocess.run(
            ["loginctl", *args],
            capture_output=True,
            text=True,
            check=False,
            timeout=5,
        )
    except (OSError, subprocess.SubprocessError):
        return (127, "")
    return (completed.returncode, completed.stdout)


def _resolve_from_show_user() -> str | None:
    """The current user's primary graphical session id (``Display`` property)."""
    rc, stdout = _run_loginctl(["show-user", str(os.getuid()), "--property=Display", "--value"])
    if rc != 0:
        return None
    session_id = stdout.strip()
    return session_id or None


def _resolve_from_list_sessions() -> str | None:
    """Parses ``list-sessions --no-legend`` for the current user's seat0 session.

    Columns are ``SESSION UID USER SEAT TTY``; a graphical session is the row
    whose UID matches ours and that is attached to ``seat0``.
    """
    rc, stdout = _run_loginctl(["list-sessions", "--no-legend"])
    if rc != 0:
        return None
    uid = str(os.getuid())
    for line in stdout.splitlines():
        fields = line.split()
        if len(fields) < 4:
            continue
        session_id, session_uid = fields[0], fields[1]
        if session_uid != uid:
            continue
        if "seat0" not in fields[2:]:
            continue
        return session_id or None
    return None


def resolve_session_id() -> str | None:
    """Resolves the caller's graphical logind session id, or ``None``.

    Precedence: ``XDG_SESSION_ID`` env, then ``show-user <uid> Display``, then a
    ``list-sessions`` seat0 parse. Every step is best-effort; ``None`` means no
    session could be resolved and the caller should no-op.
    """
    try:
        env_session = os.environ.get("XDG_SESSION_ID", "").strip()
        if env_session:
            return env_session
        display = _resolve_from_show_user()
        if display:
            return display
        return _resolve_from_list_sessions()
    except Exception as error:  # resolution must never raise
        _warn(f"session-id resolution failed: {error}")
        return None


def session_locked(session_id: str) -> bool | None:
    """Returns the session's ``LockedHint`` as a bool, or ``None`` if unknown."""
    rc, stdout = _run_loginctl(["show-session", session_id, "-p", "LockedHint", "--value"])
    if rc != 0:
        return None
    value = stdout.strip().lower()
    if value == "yes":
        return True
    if value == "no":
        return False
    return None


def unlock_session(session_id: str) -> None:
    """Unlocks the session (own-session, no polkit prompt); best-effort."""
    rc, _stdout = _run_loginctl(["unlock-session", session_id])
    if rc != 0:
        _warn(f"unlock-session {session_id} failed (rc={rc})")


def lock_session(session_id: str) -> None:
    """Relocks the session; best-effort."""
    rc, _stdout = _run_loginctl(["lock-session", session_id])
    if rc != 0:
        _warn(f"lock-session {session_id} failed (rc={rc})")


@contextlib.contextmanager
def screen_unlocked(*, enabled: bool = True) -> Iterator[None]:
    """Unlocks the session for the ``with`` body and restores the lock state.

    On enter: if ``enabled`` and a session resolves and it is currently locked,
    unlock it and remember that we did. On exit: relock only if we unlocked
    (state restore). When disabled, unresolvable, or already unlocked, this is a
    pure passthrough. A ``loginctl`` failure warns and continues — the body
    always runs, even if that means running against a locked screen.
    """
    did_unlock = False
    session_id: str | None = None
    if enabled:
        try:
            session_id = resolve_session_id()
            if session_id is not None and session_locked(session_id) is True:
                unlock_session(session_id)
                did_unlock = True
        except Exception as error:  # lock handling must not crash the harness
            _warn(f"unlock-on-enter failed: {error}")
    try:
        yield
    finally:
        if did_unlock and session_id is not None:
            try:
                lock_session(session_id)
            except Exception as error:  # relock must not crash the harness
                _warn(f"relock-on-exit failed: {error}")
