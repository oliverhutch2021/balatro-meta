#!/usr/bin/env python3
"""The night's automatic gate (make night, lab #27): after the 400-run look, decide whether the long arms run or
`make leak-tally` runs instead. A diagnosis of the look, never of the bot's worth.

Hard stops (S, #27):
- White's 400 more than 8 points under the control on the same seeds (scripts/compare_runs.py's pairing);
- rounds lost after a winning hand was in hand more than twice the rate of the same 400 runs with every default off
  (B's bar);
- a look file missing (a step that crashed).
Every other bar is printed by leaks.py and read in the morning.

    scripts/night_gate.py --look WHITE.json --control CONTROL.json --on ON-SUMMARY.json --off OFF-SUMMARY.json \\
        --out GATE.json
"""
from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import compare_runs  # noqa: E402

MAX_DROP = -0.08
MAX_LOSS_RATIO = 2.0


def lost_rate(summary: dict) -> float | None:
    r = summary.get("rounds", {})
    n = r.get("with_winning_hand", 0)
    return r.get("lost_after_winning_hand", 0) / n if n else None


def decide(look: dict, control: dict, on: dict, off: dict) -> dict:
    reasons = []
    c = compare_runs.compare(control, look)
    diff = c["win_rate_diff"]
    if diff < MAX_DROP:
        reasons.append(f"White's look is {100 * diff:+.2f} points against the control on {c['seeds']} seeds")
    ron, roff = lost_rate(on), lost_rate(off)
    if ron is not None and roff is not None and roff > 0 and ron > MAX_LOSS_RATIO * roff:
        reasons.append(f"rounds lost after a winning hand: {ron:.4f} against all-off's {roff:.4f}")
    if ron is not None and roff == 0 and ron > 0.01:
        reasons.append(f"rounds lost after a winning hand: {ron:.4f} where all-off lost none")
    return {"pass": not reasons, "reasons": reasons, "white_diff": diff, "white_diff_ci": c["win_rate_diff_ci"],
            "seeds": c["seeds"], "lost_after_winning_hand": {"on": ron, "off": roff}}


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    for f in ("--look", "--control", "--on", "--off", "--out"):
        ap.add_argument(f, type=Path, required=True)
    a = ap.parse_args(argv)
    try:
        g = decide(*(json.loads(p.read_text()) for p in (a.look, a.control, a.on, a.off)))
    except (OSError, ValueError, KeyError) as e:
        g = {"pass": False, "reasons": [f"a look file is missing or unreadable: {e}"]}
    a.out.parent.mkdir(parents=True, exist_ok=True)
    a.out.write_text(json.dumps(g, indent=1) + "\n")
    print("Gate: " + ("PASS, the long arms run" if g["pass"] else "STOP, make leak-tally runs instead"))
    for r in g["reasons"]:
        print(f"  {r}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
