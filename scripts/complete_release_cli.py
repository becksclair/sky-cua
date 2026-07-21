#!/usr/bin/env python3
"""Checkout-free controller for one immutable sky-cua complete release."""

from __future__ import annotations

import sys
from pathlib import Path

sys.dont_write_bytecode = True


def _release_root() -> Path:
    root = Path(__file__).resolve().parent
    if not (root / "RELEASE.json").is_file():
        raise SystemExit("install.py must run from an extracted or installed sky-cua release root")
    return root


def main(argv: list[str] | None = None) -> int:
    arguments = list(sys.argv[1:] if argv is None else argv)
    if not arguments or arguments[0] in {"-h", "--help"}:
        print(
            "usage: install.py {verify|install|ensure|verify-activation|resolve-active|recover|rollback} [options]\n"
            "  verify    verify this extracted immutable release\n"
            "  install   install this release and optionally configure MCP hosts\n"
            "  ensure    verify activation and repair it only when required\n"
            "  verify-activation  verify producer-owned machine activation without mutation\n"
            "  resolve-active  print verified active runtime paths without selector environment variables\n"
            "  recover   finish an interrupted generation transaction\n"
            "  rollback  atomically activate the retained prior generation"
        )
        return 0

    root = _release_root()
    module_root = root / "components" / "installer"
    if not module_root.is_dir() or module_root.is_symlink():
        raise SystemExit("release is missing its verified installer component")
    sys.path.insert(0, str(module_root))

    command, *rest = arguments
    if command in {"install", "ensure", "verify-activation", "resolve-active", "rollback"}:
        from install_complete_release import main as install_main

        if command == "install":
            return install_main([str(root), *rest])
        if command in {"ensure", "verify-activation", "resolve-active"}:
            return install_main([str(root), *rest], operation=command)
        return install_main(["--rollback", *rest])
    if command in {"verify", "recover"}:
        from release_generation import main as generation_main

        if command == "verify":
            return generation_main([command, str(root), *rest])
        return generation_main([command, *rest])
    raise SystemExit(f"unknown command: {command}")


if __name__ == "__main__":
    raise SystemExit(main())
