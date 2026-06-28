#!/usr/bin/env python3
"""Idempotently ensure Pi's OpenCode zen free-model provider exists in the VM.

The pi VM smoke drives ``opencode/deepseek-v4-flash-free`` (``DEFAULT_PI_SMOKE_MODEL``
in ``scripts/_agent_mcp_smoke.py``), but a stock ``~/.pi`` ships with no ``opencode``
provider and a default model that resolves to nothing, so pi fails before it can
list the sky-cua MCP tools. ``sync-pi-to-vm.sh`` pipes this script into the VM's
``python3`` so the provider, free model, and default are guaranteed regardless of
the host ``~/.pi`` that was rsynced in.

The API key is referenced as ``$OPENCODE_API_KEY`` (mirroring the ``firepass``
provider's env-var ``apiKey``); the runner sources that value from OpenCode's own
credential store at run time, so no secret is written into any config file.
"""

from __future__ import annotations

import json
import pathlib
from typing import Any

PROVIDER = "opencode"
MODEL_ID = "deepseek-v4-flash-free"
MODEL_REF = f"{PROVIDER}/{MODEL_ID}"
DEAD_MODEL_REF = "opencode-go/kimi-k2.7-code"

PROVIDER_BLOCK = {
    "api": "openai-completions",
    "baseUrl": "https://opencode.ai/zen/v1",
    "apiKey": "$OPENCODE_API_KEY",
    "models": [
        {
            "id": MODEL_ID,
            "name": "DeepSeek V4 Flash (free)",
            "reasoning": True,
            "input": ["text"],
            "contextWindow": 65536,
            "maxTokens": 16384,
            "cost": {"input": 0, "output": 0},
        }
    ],
}


def main() -> None:
    agent = pathlib.Path.home() / ".pi" / "agent"
    agent.mkdir(parents=True, exist_ok=True)
    models_path = agent / "models.json"
    settings_path = agent / "settings.json"

    models = _load(models_path, {})
    providers = models.setdefault("providers", {})
    providers.setdefault(PROVIDER, PROVIDER_BLOCK)
    models_path.write_text(json.dumps(models, indent=1) + "\n")

    settings = _load(settings_path, {})
    settings["defaultProvider"] = PROVIDER
    settings["defaultModel"] = MODEL_ID
    enabled = [m for m in (settings.get("enabledModels") or []) if m != DEAD_MODEL_REF]
    if MODEL_REF not in enabled:
        enabled.insert(0, MODEL_REF)
    settings["enabledModels"] = enabled
    settings_path.write_text(json.dumps(settings, indent=1) + "\n")

    print(f"ensured pi provider '{PROVIDER}' + default model {MODEL_REF}")


def _load(path: pathlib.Path, default: dict[str, Any]) -> dict[str, Any]:
    if not path.exists():
        return dict(default)
    try:
        loaded = json.loads(path.read_text())
    except (OSError, json.JSONDecodeError):
        return dict(default)
    return loaded if isinstance(loaded, dict) else dict(default)


if __name__ == "__main__":
    main()
