#!/usr/bin/env python3
"""Emit kq agent contract for Claude SessionStart and Cursor sessionStart."""

from __future__ import annotations

import json
import shutil
import sys

CONTEXT = """kq contract: read-only KotOR/TSL query CLI. Do not extract archives.
-i/--install selects the game path (also KQ_INSTALL). It is not ignore-case.
grep case-folding is --ignore-case only. Never run `kq grep -i`.
Commands: info, ls, which, cat, grep, cache. Prefer --json for parsing.
Exit 0 ok, 1 runtime, 2 usage, 3 no match, 4 no install.
NCS is disassembly, not NSS. kq does not write, pack, or compile.
"""


def main() -> int:
    raw = sys.stdin.read()
    which = shutil.which("kq")
    extra = f"kq on PATH: {which}\n" if which else "kq is not on PATH; use ./target/release/kq after cargo build.\n"
    text = CONTEXT + extra

    payload = None
    if raw.strip():
        try:
            payload = json.loads(raw)
        except json.JSONDecodeError:
            payload = None

    # Cursor sessionStart (and similar) send JSON on stdin.
    if isinstance(payload, dict):
        json.dump({"additional_context": text}, sys.stdout)
        sys.stdout.write("\n")
        return 0

    # Claude Code SessionStart command hook.
    json.dump(
        {
            "hookSpecificOutput": {
                "hookEventName": "SessionStart",
                "additionalContext": text,
            }
        },
        sys.stdout,
    )
    sys.stdout.write("\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
