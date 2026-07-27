#!/usr/bin/env python3
"""Write a GNU-compatible ar archive (for .deb packages). Safe on macOS."""
from __future__ import annotations

import sys
import time
from pathlib import Path


def _header(name: str, size: int, mode: int = 0o100644) -> bytes:
    if len(name) > 16:
        raise ValueError(f"ar member name too long: {name!r}")
    mtime = str(int(time.time())).ljust(12)
    return (
        name.ljust(16).encode("ascii")
        + mtime.encode("ascii")
        + b"0     "
        + b"0     "
        + str(mode).ljust(8).encode("ascii")
        + str(size).ljust(10).encode("ascii")
        + b"`\n"
    )


def write_ar(output: Path, members: list[tuple[str, Path | bytes]]) -> None:
    with output.open("wb") as out:
        out.write(b"!<arch>\n")
        for name, payload in members:
            data = payload.read_bytes() if isinstance(payload, Path) else payload
            out.write(_header(name, len(data)))
            out.write(data)
            if len(data) % 2:
                out.write(b"\n")


def main() -> None:
    if len(sys.argv) < 3 or (len(sys.argv) - 2) % 2:
        print(f"usage: {sys.argv[0]} OUT.deb name path [name path ...]", file=sys.stderr)
        raise SystemExit(1)

    out = Path(sys.argv[1])
    pairs: list[tuple[str, Path]] = []
    args = sys.argv[2:]
    for i in range(0, len(args), 2):
        pairs.append((args[i], Path(args[i + 1])))
    write_ar(out, pairs)


if __name__ == "__main__":
    main()
