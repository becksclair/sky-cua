# Fast headless verification: Rust + Python. Kotlin runs when a JDK is available.
verify: verify-rust verify-python verify-kotlin

verify-rust:
    cargo fmt --check
    cargo clippy --workspace --all-targets -- -D warnings
    cargo nextest run

verify-python:
    uv run ruff format --check scripts
    uv run ruff check scripts
    uv run basedpyright
    uv run pytest --ignore=scripts/test_browser_control_acceptance.py

verify-python-full:
    uv run ruff format --check scripts
    uv run ruff check scripts
    uv run basedpyright
    uv run pytest

# Skips (exit 0 with a message) when no JDK is available. Prefers mise-pinned JDK (mise.toml) when available.
verify-kotlin:
    #!/usr/bin/env bash
    set -euo pipefail
    if command -v mise >/dev/null 2>&1; then
        if ! mise x -- java -version >/dev/null 2>&1; then
            echo "verify-kotlin: no JDK found, skipping companion unit tests"; exit 0
        fi
        mise x -- ./android/phone-companion/gradlew -p android/phone-companion :app:testDebugUnitTest --console=plain
    else
        if [ -z "${JAVA_HOME:-}" ] && ! command -v java >/dev/null; then
            echo "verify-kotlin: no JDK found, skipping companion unit tests"; exit 0
        fi
        ./android/phone-companion/gradlew -p android/phone-companion :app:testDebugUnitTest --console=plain
    fi

# Runs the COSMIC desktop smoke suite. Skips cleanly (exit 0) when no live
# desktop session is available, so headless CI containers stay green.
verify-cosmic-smokes:
    python3 scripts/run_cosmic_smokes.py --skip-if-no-desktop --json
