from __future__ import annotations

import os

from _model_profiles import model_profile, required_reasoning_effort

_CODEX_EXEC_PROFILE = model_profile("codex_exec")
LIVE_SMOKE_MODEL = _CODEX_EXEC_PROFILE.model
LIVE_SMOKE_REASONING_EFFORT = required_reasoning_effort("codex_exec")


def env_flag(name: str) -> bool:
    """Return True when the named environment variable is set to a truthy string."""
    return os.environ.get(name, "").strip().lower() in {"1", "true", "yes", "on"}
