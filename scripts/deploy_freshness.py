"""Deploy-freshness gate for live tests.

Live smokes (and any agent-driven test) exercise a *built* `sky-cua-client`
binary — either the dev build, the staged bundle, or the locally deployed
runtime an agent reaches through its MCP config. When the Rust runtime source
changes but the binary the test uses is not rebuilt/redeployed, the test
silently runs against stale code. That is easy to do and expensive to debug.

This module makes the staleness detectable. Every build/deploy stamps a
`<client>.buildstamp.json` next to the binary it produces, recording a
deterministic fingerprint of the runtime source it was built from. Before a
live test runs, it recomputes the current fingerprint and compares it to the
stamp of the binary it will use; a mismatch (or a missing stamp) means
`cua-deploy` has not been run for the current source and the test must not
proceed.

The fingerprint is a content hash (not mtimes) so it is stable across git
checkouts and only changes when the runtime source actually changes.
"""

from __future__ import annotations

import hashlib
import json
import os
import platform
import subprocess
import sys
from collections.abc import Callable
from dataclasses import dataclass
from pathlib import Path
from typing import Any

#: Client binary basename the smoke gate guards (the sky-cua runtime).
CLIENT_BINARY_NAME = "sky-cua-client"

REPO_ROOT = Path(__file__).resolve().parents[1]

#: Default locally-deployed runtime root (mirrors `_install_shared`).
DEFAULT_LOCAL_INSTALL_DIR = Path.home() / ".local" / "share" / "sky-cua"

#: Stamp suffix written next to a client binary.
STAMP_SUFFIX = ".buildstamp.json"

#: Env escape hatch: set to a truthy value to downgrade the gate to a warning.
ALLOW_STALE_ENV = "SKY_CUA_ALLOW_STALE_DEPLOY"

#: Runtime-source roots whose content determines the compiled binary. Anything
#: outside these (docs, scripts, skills) does not require a runtime redeploy.
_SOURCE_DIR = REPO_ROOT / "crates"
_SOURCE_FILE_SUFFIXES = (".rs", ".toml")
_EXTRA_SOURCE_FILES = ("Cargo.toml", "Cargo.lock")


def runtime_source_fingerprint(repo_root: Path = REPO_ROOT) -> str:
    """Deterministic content hash of the Rust runtime source.

    Hashes every `crates/**/*.{rs,toml}` plus the workspace `Cargo.toml` and
    `Cargo.lock`, ordered by repo-relative path. Independent of mtimes, so a
    `git checkout` that restores identical bytes yields an identical
    fingerprint.
    """
    files: list[Path] = []
    source_dir = repo_root / "crates"
    if source_dir.is_dir():
        files.extend(
            path
            for path in source_dir.rglob("*")
            if path.is_file() and path.suffix in _SOURCE_FILE_SUFFIXES
        )
    for name in _EXTRA_SOURCE_FILES:
        candidate = repo_root / name
        if candidate.is_file():
            files.append(candidate)

    digest = hashlib.sha256()
    for path in sorted(files, key=lambda p: p.relative_to(repo_root).as_posix()):
        rel = path.relative_to(repo_root).as_posix()
        digest.update(rel.encode("utf-8"))
        digest.update(b"\0")
        digest.update(hashlib.sha256(path.read_bytes()).digest())
    return digest.hexdigest()


def newest_runtime_source_mtime(repo_root: Path = REPO_ROOT) -> float:
    """Most-recent mtime across the runtime source — the unstamped-binary fallback."""
    newest = 0.0
    source_dir = repo_root / "crates"
    if source_dir.is_dir():
        for path in source_dir.rglob("*"):
            if path.is_file() and path.suffix in _SOURCE_FILE_SUFFIXES:
                newest = max(newest, path.stat().st_mtime)
    for name in _EXTRA_SOURCE_FILES:
        candidate = repo_root / name
        if candidate.is_file():
            newest = max(newest, candidate.stat().st_mtime)
    return newest


def _git_revision(repo_root: Path) -> tuple[str, bool]:
    """Best-effort `(HEAD sha, dirty)`; `("unknown", False)` when git is absent."""
    try:
        sha = subprocess.run(
            ["git", "-C", str(repo_root), "rev-parse", "HEAD"],
            capture_output=True,
            text=True,
            check=True,
        ).stdout.strip()
        status = subprocess.run(
            ["git", "-C", str(repo_root), "status", "--porcelain"],
            capture_output=True,
            text=True,
            check=True,
        ).stdout
        return sha or "unknown", bool(status.strip())
    except (subprocess.CalledProcessError, OSError):
        return "unknown", False


def stamp_path_for(client_path: Path) -> Path:
    """Stamp path written next to a client binary."""
    return client_path.with_name(client_path.name + STAMP_SUFFIX)


def write_build_stamp(
    client_path: Path,
    repo_root: Path = REPO_ROOT,
    *,
    fingerprint: str | None = None,
    deployed_at_ms: int | None = None,
) -> Path:
    """Write the build/deploy stamp next to `client_path`.

    `deployed_at_ms` is recorded verbatim when provided (the caller owns the
    clock); otherwise the field is omitted so the stamp stays deterministic.
    """
    fingerprint = fingerprint or runtime_source_fingerprint(repo_root)
    git_sha, git_dirty = _git_revision(repo_root)
    stamp: dict[str, Any] = {
        "version": 1,
        "source_fingerprint": fingerprint,
        "git_sha": git_sha,
        "git_dirty": git_dirty,
        "repo_root": str(repo_root),
    }
    if deployed_at_ms is not None:
        stamp["deployed_at_ms"] = deployed_at_ms
    target = stamp_path_for(client_path)
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_text(json.dumps(stamp, indent=2) + "\n", encoding="utf-8")
    return target


def read_build_stamp(client_path: Path) -> dict[str, Any] | None:
    """Read the stamp next to `client_path`, or `None` when absent/unparseable."""
    target = stamp_path_for(client_path)
    try:
        loaded: Any = json.loads(target.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError):
        return None
    return loaded if isinstance(loaded, dict) else None


def _runtime_platform() -> str | None:
    match platform.machine().lower():
        case "x86_64" | "amd64":
            return "linux-x64"
        case "aarch64" | "arm64":
            return "linux-arm64"
        case _:
            return None


def resolve_client_path_for_freshness(client_path: Path, repo_root: Path = REPO_ROOT) -> Path:
    """Resolve the repo wrapper to the runtime binary it will actually exec."""
    try:
        wrapper_path = (repo_root / "bin" / CLIENT_BINARY_NAME).resolve(strict=False)
        candidate_path = client_path.resolve(strict=False)
    except OSError:
        return client_path
    if runtime_platform := _runtime_platform():
        bundled_runtime = client_path.parent / "runtimes" / runtime_platform / CLIENT_BINARY_NAME
        # Packaged plugin entrypoints are shell wrappers around the bundled
        # runtime. Standalone MCP installs put the ELF directly at the
        # entrypoint and may retain an older bundled tree from a prior plugin
        # install; that sibling must not override the binary hosts actually run.
        try:
            is_wrapper = client_path.read_bytes().startswith(b"#!")
        except OSError:
            is_wrapper = False
        if is_wrapper and bundled_runtime.exists():
            return bundled_runtime

    if candidate_path != wrapper_path:
        return client_path

    source_runtime = repo_root / "target" / "release" / CLIENT_BINARY_NAME
    if source_runtime.exists():
        return source_runtime
    return client_path


@dataclass(frozen=True)
class Freshness:
    """Whether a client binary was built from the current runtime source."""

    fresh: bool
    client_path: Path
    reason: str
    advice: str

    @property
    def summary(self) -> str:
        state = "fresh" if self.fresh else "STALE"
        return f"{state}: {self.client_path} — {self.reason}"


def check_client_freshness(
    client_path: Path,
    repo_root: Path = REPO_ROOT,
    *,
    deploy_command: str = "python3 scripts/deploy_plugin.py",
) -> Freshness:
    """Compare the stamp next to `client_path` against the current source."""
    client_path = resolve_client_path_for_freshness(client_path, repo_root)
    if not client_path.exists():
        return Freshness(
            fresh=False,
            client_path=client_path,
            reason="client binary is missing",
            advice=f"build/deploy it: {deploy_command}",
        )
    stamp = read_build_stamp(client_path)
    if stamp is None:
        # No deploy stamp (e.g. a plain `cargo build` dev binary). Fall back to an
        # mtime comparison: the binary must be at least as new as every runtime
        # source file, else it predates a source edit and needs rebuilding.
        if client_path.stat().st_mtime >= newest_runtime_source_mtime(repo_root):
            return Freshness(
                fresh=True,
                client_path=client_path,
                reason="unstamped binary is newer than all runtime source",
                advice="",
            )
        return Freshness(
            fresh=False,
            client_path=client_path,
            reason="runtime source is newer than this unstamped binary",
            advice="rebuild it (cargo build) or run the deploy workflow",
        )
    current = runtime_source_fingerprint(repo_root)
    stamped = stamp.get("source_fingerprint")
    if stamped == current:
        return Freshness(
            fresh=True,
            client_path=client_path,
            reason="binary matches the current runtime source",
            advice="",
        )
    return Freshness(
        fresh=False,
        client_path=client_path,
        reason="runtime source changed since this binary was deployed",
        advice=f"redeploy before live tests: {deploy_command}",
    )


def deployed_client_path(install_dir: Path = DEFAULT_LOCAL_INSTALL_DIR) -> Path:
    """Path of the locally-deployed client an agent reaches via its MCP config."""
    return install_dir / "bin" / "sky-cua-client"


def allow_stale() -> bool:
    """Whether the env escape hatch is set."""
    return os.environ.get(ALLOW_STALE_ENV, "").strip().lower() in {"1", "true", "yes", "on"}


# Paths already confirmed fresh this process, so repeated MCP spawns/agent
# launches do not recompute the fingerprint or re-print.
_GATE_CHECKED: set[str] = set()


def _stderr(message: str) -> None:
    print(message, file=sys.stderr)


def assert_runtime_fresh(
    client_path: Path,
    *,
    repo_root: Path = REPO_ROOT,
    emit: Callable[[str], None] = _stderr,
) -> None:
    """Live-test deploy gate: stop the process when `client_path` is stale.

    This is the shared choke-point gate. Every sky-cua MCP spawn (`McpClient`)
    and agent launch (`run_agent`) routes the client/runtime binary through here
    before it is used, so any live smoke that exercises the runtime refuses to
    run against a binary not built from the current source — exiting nonzero with
    a redeploy hint rather than silently testing stale code. Fresh/overridden
    binaries pass; the result is cached per path so repeated spawns are cheap.
    Set ``SKY_CUA_ALLOW_STALE_DEPLOY`` to downgrade to a warning.
    """
    key = str(client_path)
    if key in _GATE_CHECKED:
        return
    result = check_client_freshness(client_path, repo_root)
    if result.fresh:
        _GATE_CHECKED.add(key)
        return
    if allow_stale():
        emit(f"deploy-freshness: {result.summary} ({ALLOW_STALE_ENV} set; continuing)")
        _GATE_CHECKED.add(key)
        return
    emit(
        f"deploy-freshness GATE: refusing to run a live test against a stale build — "
        f"{result.reason} ({result.client_path}). {result.advice} — or set "
        f"{ALLOW_STALE_ENV}=1 to override."
    )
    raise SystemExit(1)


def gate_spawn_argv(
    argv: list[str],
    *,
    repo_root: Path = REPO_ROOT,
    emit: Callable[[str], None] = _stderr,
) -> None:
    """Gate an MCP spawn argv: enforce freshness only for the sky-cua runtime.

    Non-sky-cua spawns (other binaries, stub argv in unit tests) are ignored, so
    this is safe to call unconditionally before launching any MCP subprocess.
    """
    if argv and Path(argv[0]).name == CLIENT_BINARY_NAME:
        assert_runtime_fresh(Path(argv[0]), repo_root=repo_root, emit=emit)


def main(argv: list[str] | None = None) -> int:
    """CLI: exit nonzero when the requested client is stale.

    Usage: ``python3 scripts/deploy_freshness.py [--client PATH] [--deployed]``.
    Defaults to the locally-deployed runtime, which is the surface agent-driven
    live tests exercise.
    """
    import argparse

    parser = argparse.ArgumentParser(description="Check sky-cua deploy freshness for live tests.")
    parser.add_argument("--client", type=Path, help="Client binary to check.")
    parser.add_argument(
        "--deployed",
        action="store_true",
        help="Check the locally-deployed runtime (default when --client is omitted).",
    )
    args = parser.parse_args(argv)

    client = args.client or deployed_client_path()
    result = check_client_freshness(client)
    print(result.summary)
    if not result.fresh:
        if result.advice:
            print(f"  -> {result.advice}")
        if allow_stale():
            print(f"  ({ALLOW_STALE_ENV} set; continuing anyway)")
            return 0
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
