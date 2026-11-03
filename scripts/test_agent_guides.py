"""Keeps every tracked AGENTS.md honest about the repository paths it cites.

Agents are told to read the nearest AGENTS.md first and treat it as
authority. After file splits and removals, stale paths in those guides send
agents to code that no longer exists. This test scans backticked tokens in
every tracked AGENTS.md and fails when a token that looks like a repository
path does not exist, resolved against the repo root, the guide's own
directory, or any immediate subdirectory of the guide's directory (so a
crate-group guide may cite `src/foo.rs` meaning "inside one of these crates").
A `:line`, `::symbol` or `#anchor` suffix on a path token is ignored. Relative
Markdown link targets must resolve against the guide's own directory.
"""

from __future__ import annotations

import re
import subprocess
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[1]

# A backticked token is treated as a repository path when it starts with one
# of these prefixes, or contains a slash and ends with a source/doc extension.
PATH_PREFIXES = (
    "crates/",
    "scripts/",
    "docs/",
    "src/",
    "resources/",
    "skills/",
    "android/",
    "plans/",
    "runtime/",
    "packages/",
    ".config/",
    "bin/",
    ".github/",
    ".gitea/",
)
PATH_EXTENSIONS = (".rs", ".py", ".md", ".kt", ".ts", ".json", ".toml", ".yml", ".yaml", ".service")
TOKEN_RE = re.compile(r"`([^`\s]+)`")
SAFE_TOKEN_RE = re.compile(r"^[A-Za-z0-9_./-]+$")
LOCATION_SUFFIX_RE = re.compile(r"[:#].*$")
LINK_RE = re.compile(r"\]\(([^)\s]+)\)")


def tracked_agent_guides() -> list[Path]:
    output = subprocess.run(
        ["git", "ls-files", "--", "AGENTS.md", "*/AGENTS.md"],
        cwd=REPO_ROOT,
        check=True,
        capture_output=True,
        text=True,
    ).stdout
    return sorted({REPO_ROOT / line for line in output.splitlines() if line})


def looks_like_repo_path(token: str) -> bool:
    if "..." in token or not SAFE_TOKEN_RE.match(token):
        return False
    return token.startswith(PATH_PREFIXES) or ("/" in token and token.endswith(PATH_EXTENSIONS))


def search_roots(guide: Path) -> list[Path]:
    guide_dir = guide.parent
    return [REPO_ROOT, guide_dir, *(child for child in guide_dir.iterdir() if child.is_dir())]


def is_local_link(target: str) -> bool:
    return (
        not target.startswith(("#", "/"))
        and "://" not in target
        and not target.startswith("mailto:")
    )


def stale_path_citations() -> list[str]:
    stale: list[str] = []
    for guide in tracked_agent_guides():
        roots = search_roots(guide)
        location = guide.relative_to(REPO_ROOT)
        for line_number, line in enumerate(guide.read_text(encoding="utf-8").splitlines(), 1):
            for match in TOKEN_RE.finditer(line):
                token = LOCATION_SUFFIX_RE.sub("", match.group(1))
                if looks_like_repo_path(token) and not any(
                    (root / token).exists() for root in roots
                ):
                    stale.append(f"{location}:{line_number}: `{token}`")
            for match in LINK_RE.finditer(line):
                target = match.group(1).split("#", 1)[0]
                if target and is_local_link(target) and not (guide.parent / target).exists():
                    stale.append(f"{location}:{line_number}: link ({target})")
    return stale


def test_agent_guides_exist() -> None:
    assert len(tracked_agent_guides()) >= 10


def test_agent_guide_paths_exist() -> None:
    stale = stale_path_citations()
    assert not stale, "AGENTS.md cites paths that do not exist:\n" + "\n".join(stale)
