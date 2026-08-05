"""Checks for x86 instructions unavailable on Saga's x86-64-v3 CPU."""

from __future__ import annotations

import re
import struct
import subprocess
from collections.abc import Callable, Sequence
from pathlib import Path

Runner = Callable[..., subprocess.CompletedProcess[str]]

LINUX_X64_RUNTIME_NAMES = (
    "sky-cua-client",
    "sky-cua-service",
    "sky-cua-overlay-host",
    "sky-cua-cosmic-helper",
    "sky-cua-chrome-host",
    "sky-cua-input-helper",
)
LINUX_X64_RUNTIME_MEMBERS = tuple(
    Path("bin/runtimes/linux-x64") / name for name in LINUX_X64_RUNTIME_NAMES
)

# GFNI is not part of x86-64-v3. The native Asgard build emitted this VEX form
# even though its ELF ISA note only advertised x86-64-v3.
UNSUPPORTED_X86_V3_INSTRUCTION = re.compile(r"\b(?:v?gf2p8[a-z0-9]*)\b", re.IGNORECASE)


def validate_x86_64_v3_elf(
    path: Path,
    *,
    label: str | None = None,
    runner: Runner = subprocess.run,
) -> None:
    """Reject an x86-64 ELF containing instructions beyond x86-64-v3."""
    data = path.read_bytes()
    if len(data) < 20 or data[:4] != b"\x7fELF":
        raise ValueError(f"portable runtime is not an ELF executable: {label or path}")
    if data[4] != 2 or data[5] != 1 or struct.unpack_from("<H", data, 18)[0] != 62:
        raise ValueError(f"portable runtime is not a little-endian x86-64 ELF: {label or path}")

    try:
        result = runner(
            ["objdump", "-d", "--no-show-raw-insn", str(path)],
            check=False,
            capture_output=True,
            text=True,
        )
    except OSError as error:
        raise RuntimeError("portable runtime validation requires objdump") from error
    if result.returncode != 0:
        detail = (result.stderr or result.stdout).strip()
        raise RuntimeError(f"objdump failed for {label or path}: {detail}")

    instructions = sorted(set(UNSUPPORTED_X86_V3_INSTRUCTION.findall(result.stdout)))
    if instructions:
        joined = ", ".join(instructions)
        raise ValueError(
            f"portable runtime {label or path} contains instructions unavailable on x86-64-v3: "
            f"{joined}"
        )


def validate_x86_64_v3_paths(paths: Sequence[Path], *, runner: Runner = subprocess.run) -> None:
    """Validate a set of built Linux x86-64 runtime executables."""
    for path in paths:
        validate_x86_64_v3_elf(path, runner=runner)
