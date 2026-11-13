#!/usr/bin/env python3
"""Run a step of `make night` only on the gate's side (scripts/night_gate.py): `pass` steps when the gate passed,
`stop` steps when it stopped. Otherwise print why it was skipped and succeed, so the job goes on.

    scripts/gated.py GATE.json pass|stop -- COMMAND...
"""
from __future__ import annotations

import json
import subprocess
import sys
from pathlib import Path


def main(argv: list[str]) -> int:
    if len(argv) < 4 or argv[2] != "--" or argv[1] not in ("pass", "stop"):
        print(__doc__, file=sys.stderr)
        return 2
    gate, side, cmd = Path(argv[0]), argv[1], argv[3:]
    try:
        passed = bool(json.loads(gate.read_text())["pass"])
    except (OSError, ValueError, KeyError):
        print(f"gated: no gate at {gate}: skipped")
        return 0
    if passed != (side == "pass"):
        print(f"gated: the gate {'passed' if passed else 'stopped'}, so this {side} step is skipped")
        return 0
    if cmd[0].endswith(".py"):
        cmd = [sys.executable, "-u", *cmd]
    return subprocess.run(cmd, check=False).returncode


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
