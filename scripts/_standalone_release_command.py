"""Guarded version and Git transaction for standalone releases."""

from __future__ import annotations

import ast
import json
import os
import re
import stat
import subprocess
import tempfile
from collections.abc import Callable, Sequence
from pathlib import Path

Runner = Callable[..., subprocess.CompletedProcess[str]]
Version = tuple[int, int, int]

VERSION_PATH = Path("scripts/standalone_release.py")
MAIN_REF = "refs/heads/main"
STABLE_VERSION_PATTERN = re.compile(r"(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)")
PRODUCT_VERSION_ASSIGNMENT = re.compile(
    r'^PRODUCT_VERSION = "(?P<version>(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*))"$',
    re.MULTILINE,
)
STANDALONE_TAG_REF_PATTERN = re.compile(r"refs/tags/standalone-v(?P<version>.+)")
URL_SCHEME_PATTERN = re.compile(r"[A-Za-z][A-Za-z0-9+.-]*(?:://|::).+")
SCP_LIKE_URL_PATTERN = re.compile(r"[^/:]+(?:@[^/:]+)?:.+")


class ReleaseError(RuntimeError):
    """A guarded release precondition or operation failed."""


def parse_stable_version(value: str) -> Version:
    """Parse the canonical stable X.Y.Z form used by standalone releases."""
    if STABLE_VERSION_PATTERN.fullmatch(value) is None:
        raise ValueError(f"version must be a stable X.Y.Z value, got {value!r}")
    major, minor, patch = value.split(".")
    return int(major), int(minor), int(patch)


def calculate_release_version(
    current: str,
    *,
    bump: str = "minor",
    explicit: str | None = None,
) -> str:
    """Return an explicit increasing version or apply one stable version bump."""
    current_version = parse_stable_version(current)
    if explicit is not None:
        requested = parse_stable_version(explicit)
        if requested <= current_version:
            raise ValueError(
                f"explicit version {explicit} must be greater than current version {current}"
            )
        return explicit
    major, minor, patch = current_version
    if bump == "patch":
        patch += 1
    elif bump == "minor":
        minor += 1
        patch = 0
    elif bump == "major":
        major += 1
        minor = 0
        patch = 0
    else:
        raise ValueError(f"unsupported release bump: {bump}")
    return f"{major}.{minor}.{patch}"


def _read_product_version(path: Path) -> tuple[str, str, re.Match[str]]:
    source = path.read_text(encoding="utf-8")
    matches = list(PRODUCT_VERSION_ASSIGNMENT.finditer(source))
    if len(matches) != 1:
        raise ReleaseError(f"expected exactly one canonical PRODUCT_VERSION assignment in {path}")
    match = matches[0]
    return source, match.group("version"), match


def rewrite_product_version(path: Path, new_version: str) -> str:
    """Rewrite exactly the standalone PRODUCT_VERSION assignment and return its old value."""
    parse_stable_version(new_version)
    source, current, match = _read_product_version(path)
    replacement = f'PRODUCT_VERSION = "{new_version}"'
    updated = source[: match.start()] + replacement + source[match.end() :]
    ast.parse(updated, filename=str(path))
    descriptor, temporary_name = tempfile.mkstemp(prefix=f".{path.name}.tmp-", dir=path.parent)
    temporary = Path(temporary_name)
    try:
        os.fchmod(descriptor, stat.S_IMODE(path.stat().st_mode))
        with os.fdopen(descriptor, "w", encoding="utf-8") as stream:
            stream.write(updated)
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temporary, path)
    finally:
        temporary.unlink(missing_ok=True)
    return current


def _capture(
    runner: Runner,
    command: Sequence[str],
    *,
    repo_root: Path,
) -> subprocess.CompletedProcess[str]:
    return runner(
        list(command),
        cwd=repo_root,
        check=False,
        capture_output=True,
        text=True,
    )


def _command_detail(result: subprocess.CompletedProcess[str]) -> str:
    return (result.stderr or result.stdout or f"exit status {result.returncode}").strip()


def _checked_capture(
    runner: Runner,
    command: Sequence[str],
    *,
    repo_root: Path,
    operation: str,
) -> str:
    result = _capture(runner, command, repo_root=repo_root)
    if result.returncode != 0:
        raise ReleaseError(f"{operation} failed: {_command_detail(result)}")
    return result.stdout.rstrip("\r\n")


def _single_remote_ref(output: str, ref: str, *, operation: str) -> str:
    matches = []
    for line in output.splitlines():
        fields = line.split()
        if len(fields) == 2 and fields[1] == ref:
            matches.append(fields[0])
    if len(matches) != 1:
        raise ReleaseError(f"{operation} did not return exactly one {ref} ref")
    return matches[0]


def _remote_stable_versions(output: str) -> set[str]:
    versions: set[str] = set()
    for line in output.splitlines():
        fields = line.split()
        if len(fields) != 2:
            raise ReleaseError("remote standalone tag listing returned malformed output")
        match = STANDALONE_TAG_REF_PATTERN.fullmatch(fields[1])
        if match is None:
            continue
        version = match.group("version")
        try:
            parse_stable_version(version)
        except ValueError:
            continue
        versions.add(version)
    return versions


def _concrete_repository(destination: str, *, repo_root: Path) -> str:
    if URL_SCHEME_PATTERN.fullmatch(destination) or SCP_LIKE_URL_PATTERN.fullmatch(destination):
        return destination
    path = Path(destination).expanduser()
    if not path.is_absolute():
        path = repo_root / path
    return str(path.resolve())


def _configured_upstream(runner: Runner, *, repo_root: Path) -> tuple[str, str]:
    remote = _checked_capture(
        runner,
        ("git", "config", "--get", "branch.main.remote"),
        repo_root=repo_root,
        operation="reading main upstream remote",
    )
    merge = _checked_capture(
        runner,
        ("git", "config", "--get", "branch.main.merge"),
        repo_root=repo_root,
        operation="reading main upstream branch",
    )
    if not remote or remote == ".":
        raise ReleaseError("main must have a configured remote upstream")
    if merge != MAIN_REF:
        raise ReleaseError(f"main upstream must target {MAIN_REF}, got {merge or '<empty>'}")
    push_urls_output = _checked_capture(
        runner,
        ("git", "remote", "get-url", "--push", "--all", remote),
        repo_root=repo_root,
        operation=f"resolving {remote} push destinations",
    )
    push_urls = [line for line in push_urls_output.splitlines() if line]
    if len(push_urls) != 1:
        raise ReleaseError(
            f"main upstream remote {remote} must resolve to exactly one push destination"
        )
    return remote, _concrete_repository(push_urls[0], repo_root=repo_root)


def _require_clean_scope(
    runner: Runner,
    *,
    repo_root: Path,
    expected_status: str,
    expected_unstaged: str,
    expected_staged: str,
) -> None:
    status = _checked_capture(
        runner,
        ("git", "status", "--porcelain=v1", "--untracked-files=all"),
        repo_root=repo_root,
        operation="checking release worktree scope",
    )
    unstaged = _checked_capture(
        runner,
        ("git", "diff", "--name-only"),
        repo_root=repo_root,
        operation="checking unstaged release scope",
    )
    staged = _checked_capture(
        runner,
        ("git", "diff", "--cached", "--name-only"),
        repo_root=repo_root,
        operation="checking staged release scope",
    )
    if (status, unstaged, staged) != (expected_status, expected_unstaged, expected_staged):
        raise ReleaseError(
            "release scope contains changes other than the exact PRODUCT_VERSION assignment"
        )


def _is_checkout(root: Path) -> bool:
    return (root / ".git").exists() and (root / "scripts/build_plugin.py").is_file()


def _release_preflight(
    target_version: str,
    *,
    runner: Runner,
    repo_root: Path,
) -> tuple[str, str, str, str]:
    if not _is_checkout(repo_root):
        raise ReleaseError("release requires a source checkout")
    status = _checked_capture(
        runner,
        ("git", "status", "--porcelain=v1", "--untracked-files=all"),
        repo_root=repo_root,
        operation="checking checkout cleanliness",
    )
    if status:
        raise ReleaseError("release requires a clean checkout with no staged or untracked files")
    branch = _checked_capture(
        runner,
        ("git", "symbolic-ref", "--quiet", "--short", "HEAD"),
        repo_root=repo_root,
        operation="checking current branch",
    )
    if branch != "main":
        raise ReleaseError(f"release requires branch main, got {branch or 'detached HEAD'}")
    remote, push_url = _configured_upstream(runner, repo_root=repo_root)
    head = _checked_capture(
        runner,
        ("git", "rev-parse", "HEAD"),
        repo_root=repo_root,
        operation="reading local main",
    )
    remote_main_output = _checked_capture(
        runner,
        ("git", "ls-remote", "--exit-code", push_url, MAIN_REF),
        repo_root=repo_root,
        operation="reading remote main",
    )
    remote_head = _single_remote_ref(remote_main_output, MAIN_REF, operation="remote main read")
    if head != remote_head:
        raise ReleaseError("local HEAD must exactly equal remote main before release")

    tag = f"standalone-v{target_version}"
    local_tag = _capture(
        runner,
        ("git", "show-ref", "--verify", "--quiet", f"refs/tags/{tag}"),
        repo_root=repo_root,
    )
    if local_tag.returncode == 0:
        raise ReleaseError(f"local tag already exists: {tag}")
    if local_tag.returncode != 1:
        raise ReleaseError(f"checking local tag failed: {_command_detail(local_tag)}")
    remote_tags_result = _capture(
        runner,
        ("git", "ls-remote", "--tags", "--refs", push_url, "refs/tags/standalone-v*"),
        repo_root=repo_root,
    )
    if remote_tags_result.returncode != 0:
        raise ReleaseError(f"listing remote tags failed: {_command_detail(remote_tags_result)}")
    remote_versions = _remote_stable_versions(remote_tags_result.stdout)
    if target_version in remote_versions:
        raise ReleaseError(f"remote tag already exists: {tag}")
    if remote_versions:
        latest_remote_version = max(remote_versions, key=parse_stable_version)
        if parse_stable_version(target_version) <= parse_stable_version(latest_remote_version):
            raise ReleaseError(
                f"target version {target_version} must be greater than published standalone "
                f"version {latest_remote_version}"
            )
    return remote, push_url, head, tag


def release_command(
    *,
    product_version: str,
    repo_root: Path,
    bump: str = "minor",
    explicit_version: str | None = None,
    runner: Runner = subprocess.run,
) -> int:
    """Create and atomically push a guarded standalone release commit and tag."""
    version_path = repo_root / VERSION_PATH
    source, current_version, match = _read_product_version(version_path)
    if current_version != product_version:
        raise ReleaseError(
            "loaded PRODUCT_VERSION does not match the checkout; rerun from this checkout"
        )
    try:
        target_version = calculate_release_version(
            current_version,
            bump=bump,
            explicit=explicit_version,
        )
    except ValueError as exc:
        raise ReleaseError(str(exc)) from exc
    remote, push_url, old_head, tag = _release_preflight(
        target_version,
        runner=runner,
        repo_root=repo_root,
    )

    expected_source = (
        source[: match.start()] + f'PRODUCT_VERSION = "{target_version}"' + source[match.end() :]
    )
    rewrite_product_version(version_path, target_version)
    verify = _capture(runner, ("just", "verify-python"), repo_root=repo_root)
    if verify.returncode != 0:
        raise ReleaseError(f"just verify-python failed: {_command_detail(verify)}")

    relative = VERSION_PATH.as_posix()
    _require_clean_scope(
        runner,
        repo_root=repo_root,
        expected_status=f" M {relative}",
        expected_unstaged=relative,
        expected_staged="",
    )
    source_after, written_version, _match_after = _read_product_version(version_path)
    if written_version != target_version or source_after != expected_source:
        raise ReleaseError("version file differs from the exact PRODUCT_VERSION rewrite")

    _checked_capture(
        runner,
        ("git", "diff", "--check", "--", relative),
        repo_root=repo_root,
        operation="checking standalone version diff",
    )
    _checked_capture(
        runner,
        ("git", "add", "--", relative),
        repo_root=repo_root,
        operation="staging standalone version",
    )
    _require_clean_scope(
        runner,
        repo_root=repo_root,
        expected_status=f"M  {relative}",
        expected_unstaged="",
        expected_staged=relative,
    )
    _checked_capture(
        runner,
        ("git", "diff", "--cached", "--check", "--", relative),
        repo_root=repo_root,
        operation="checking staged standalone version diff",
    )
    _checked_capture(
        runner,
        ("git", "commit", "-m", f"Release standalone v{target_version}", "--", relative),
        repo_root=repo_root,
        operation="committing standalone version",
    )
    final_status = _checked_capture(
        runner,
        ("git", "status", "--porcelain=v1", "--untracked-files=all"),
        repo_root=repo_root,
        operation="checking post-commit worktree",
    )
    if final_status:
        raise ReleaseError("release worktree is not clean after version commit")
    commit = _checked_capture(
        runner,
        ("git", "rev-parse", "HEAD"),
        repo_root=repo_root,
        operation="reading release commit",
    )
    committed_paths = _checked_capture(
        runner,
        ("git", "diff-tree", "--no-commit-id", "--name-only", "-r", "HEAD"),
        repo_root=repo_root,
        operation="checking release commit scope",
    )
    if committed_paths != relative:
        raise ReleaseError("release commit must contain only the standalone version file")
    committed_source = _checked_capture(
        runner,
        ("git", "show", f"HEAD:{relative}"),
        repo_root=repo_root,
        operation="checking committed standalone version",
    )
    if committed_source + "\n" != expected_source:
        raise ReleaseError("release commit does not contain the exact PRODUCT_VERSION rewrite")
    parent = _checked_capture(
        runner,
        ("git", "rev-parse", "HEAD^"),
        repo_root=repo_root,
        operation="checking release commit parent",
    )
    if parent != old_head:
        raise ReleaseError("release commit is not based on the preflighted remote main commit")
    _checked_capture(
        runner,
        ("git", "tag", "-a", tag, "-m", f"sky-cua standalone v{target_version}"),
        repo_root=repo_root,
        operation="creating annotated standalone tag",
    )
    tag_ref = f"refs/tags/{tag}"
    tag_object = _checked_capture(
        runner,
        ("git", "rev-parse", tag_ref),
        repo_root=repo_root,
        operation="reading standalone tag object",
    )
    tag_type = _checked_capture(
        runner,
        ("git", "cat-file", "-t", tag_ref),
        repo_root=repo_root,
        operation="checking standalone tag type",
    )
    tagged_commit = _checked_capture(
        runner,
        ("git", "rev-list", "-n", "1", tag_ref),
        repo_root=repo_root,
        operation="checking standalone tag target",
    )
    if tag_type != "tag" or tagged_commit != commit:
        raise ReleaseError("standalone release tag is not annotated at the release commit")
    _checked_capture(
        runner,
        (
            "git",
            "push",
            "--atomic",
            "--no-follow-tags",
            push_url,
            f"{MAIN_REF}:{MAIN_REF}",
            f"{tag_ref}:{tag_ref}",
        ),
        repo_root=repo_root,
        operation="atomically pushing standalone release",
    )
    remote_output = _checked_capture(
        runner,
        ("git", "ls-remote", "--exit-code", push_url, MAIN_REF, tag_ref, f"{tag_ref}^{{}}"),
        repo_root=repo_root,
        operation="reading back standalone release refs",
    )
    remote_commit = _single_remote_ref(remote_output, MAIN_REF, operation="release readback")
    remote_tag = _single_remote_ref(remote_output, tag_ref, operation="release readback")
    remote_peeled = _single_remote_ref(
        remote_output, f"{tag_ref}^{{}}", operation="release readback"
    )
    if remote_commit != commit or remote_tag != tag_object or remote_peeled != commit:
        raise ReleaseError("remote release refs do not match the local commit and annotated tag")
    print(
        json.dumps(
            {
                "branch": "main",
                "commit": commit,
                "remote": remote,
                "status": "pushed",
                "tag": tag,
                "version": target_version,
            },
            sort_keys=True,
        )
    )
    return 0
