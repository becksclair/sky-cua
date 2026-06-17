from __future__ import annotations

import os

LIVE_SMOKE_MODEL = "gpt-5.5"
LIVE_SMOKE_REASONING_EFFORT = "low"


def env_flag(name: str) -> bool:
    """Return True when the named environment variable is set to a truthy string."""
    return os.environ.get(name, "").strip().lower() in {"1", "true", "yes", "on"}
