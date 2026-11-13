release: verify
  python3 install.py release --patch
  python3 install.py install

# Fast headless verification: Rust + Python + JS. Kotlin runs when a JDK is available.
verify: verify-rust verify-python verify-js verify-kotlin

# A host with no enabled `verify-rust` leaves `verify` with an unknown
# dependency, which stops every recipe in this file from loading, so Windows
# keeps the whole-workspace gate it had before the macOS split.
# Rust gate for the host OS: every crate except the Linux-only ones on macOS, the whole workspace elsewhere.
[linux]
[windows]
verify-rust: verify-rust-linux

# Rust gate for the host OS: every crate except the Linux-only ones on macOS, the whole workspace elsewhere.
[macos]
verify-rust: verify-rust-macos

# sky-cua-linux and sky-cua-cosmic-helper need GStreamer and Wayland system
# libraries, so they build only on Linux.
macos_rust_packages := "--workspace --exclude sky-cua-linux --exclude sky-cua-cosmic-helper"

verify-rust-linux:
    cargo fmt --check
    cargo clippy --workspace --all-targets -- -D warnings
    cargo nextest run --workspace

# rustfmt needs no system libraries, so formatting still covers every crate.
verify-rust-macos:
    cargo fmt --check
    cargo clippy {{ macos_rust_packages }} --all-targets -- -D warnings
    cargo nextest run {{ macos_rust_packages }}

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

# Typechecks and tests the three bun projects. All three are installed first
# because packages/browser-use's tests import runtime/cua-node source; then
# browser-use is built before its tests (runtime/cua-node's tests read its
# build output) and cua-node is built before its lock-truth tests. Uses
# `bun run test` (the package scripts), never bare `bun test`, because
# sky-cua-js's script builds dist/ before testing.
verify-js:
    #!/usr/bin/env bash
    set -euo pipefail
    for dir in packages/browser-use packages/sky-cua-js runtime/cua-node; do
        (cd "$dir" && bun install --frozen-lockfile)
    done
    (cd packages/browser-use && bun run build && bun run typecheck && bun run test)
    (cd packages/sky-cua-js && bun run typecheck && bun run test)
    (cd runtime/cua-node && bun run build && bun run typecheck && bun run test)

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
