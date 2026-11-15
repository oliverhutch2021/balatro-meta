#!/usr/bin/env python3
"""Did the bot see the danger, and what would spending have bought? (S, lab #33, A1 and A2.)

A1, from `--calibrate` lines: at every shop before a blind of antes 4 to 6, the value's own chance of living through
the ante's blinds left as the shop closed (`shop_before.seen.survive`), against whether the run did. By band of that
chance: shops, the chance it gave on average, the share that lived. A value that sees the danger has the two close; a
value that thought it was safe when it wasn't shows deaths in its high bands. Also the money left in each band.

A2, from `--danger-bench` lines: for runs lost at the boss of antes 4 to 6 with $10 or more as the shop before it
closed, the boss's chance with the build as it stood and after spending that money in fair copies of the shop.

    scripts/danger_check.py --calib CAL.jsonl[.gz] [--danger DB.jsonl[.gz]] --out SUMMARY.json
"""
from __future__ import annotations

import argparse
import gzip
import json
import statistics
from pathlib import Path

BANDS = ((0.0, 0.5), (0.5, 0.75), (0.75, 0.9), (0.9, 0.97), (0.97, 1.01))
LETTERS = "ABCDEFGHIJKLMNOPQRSTUVWXY"


def read(path: Path) -> list[dict]:
    op = gzip.open if path.suffix == ".gz" else open
    with op(path, "rt") as fh:
        return [json.loads(x) for x in fh if x.strip()]


def shops(runs: list[dict]) -> list[tuple[int, float, bool, int]]:
    """(ante, the value's survival chance at the shop's close, lived through the ante, money left) for each shop before a
    blind of antes 4 to 6."""
    out = []
    for r in runs:
        rounds = r.get("rounds", [])
        last_ante = rounds[-1]["ante"] if rounds else 0
        died_at = None if r.get("won") else last_ante
        for x in rounds:
            st = x.get("at_start") or {}
            sh = st.get("shop_before") or {}
            seen = sh.get("seen") or {}
            if 4 <= x["ante"] <= 6 and "survive" in seen:
                out.append((x["ante"], seen["survive"], died_at != x["ante"], sh.get("money_left", 0)))
    return out


def perception(rows: list[tuple[int, float, bool, int]]) -> dict:
    table = {}
    for lo, hi in BANDS:
        sub = [r for r in rows if lo <= r[1] < hi]
        table[f"{lo:.2f}-{min(hi, 1.0):.2f}"] = {
            "shops": len(sub),
            "chance_given": round(statistics.mean(r[1] for r in sub), 3) if sub else None,
            "lived": round(sum(r[2] for r in sub) / len(sub), 3) if sub else None,
            "money_left_median": statistics.median(r[3] for r in sub) if sub else None,
        }
    return table


def ceiling(rows: list[dict]) -> dict:
    if not rows:
        return {}
    before = [r["before"] for r in rows]
    after = [r["after_mean"] for r in rows]
    by_ante = {}
    for a in (4, 5, 6):
        sub = [r for r in rows if r["ante"] == a]
        if sub:
            by_ante[a] = {"deaths": len(sub), "before": round(statistics.mean(r["before"] for r in sub), 3),
                          "after": round(statistics.mean(r["after_mean"] for r in sub), 3)}
    return {"deaths": len(rows), "money_median": statistics.median(r["money"] for r in rows),
            "boss_chance_before": round(statistics.mean(before), 3), "boss_chance_after": round(statistics.mean(after), 3),
            "expected_saved_share": round(statistics.mean(after) - statistics.mean(before), 3),
            "after_over_half": round(sum(a > 0.5 for a in after) / len(after), 3), "by_ante": by_ante}


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--calib", type=Path, required=True)
    ap.add_argument("--danger", type=Path)
    ap.add_argument("--out", type=Path, required=True)
    a = ap.parse_args(argv)
    rows = shops(read(a.calib))
    s = {"calib": a.calib.name, "shops": len(rows), "perception": perception(rows),
         "perception_by_ante": {an: perception([r for r in rows if r[0] == an]) for an in (4, 5, 6)}}
    if a.danger:
        s["ceiling"] = ceiling(read(a.danger))
    a.out.parent.mkdir(parents=True, exist_ok=True)
    a.out.write_text(json.dumps(s, indent=1) + "\n")
    print(f"{s['shops']} shops before a blind of antes 4 to 6: the value's chance of living through the ante, "
          "against what happened")
    for k, v in s["perception"].items():
        print(f"  {k}: {v['shops']} shops, chance given {v['chance_given']}, lived {v['lived']}, "
              f"money left {v['money_left_median']}")
    c = s.get("ceiling")
    if c:
        print(f"Spending the money of {c['deaths']} runs lost at an ante 4-6 boss (median ${c['money_median']}): "
              f"the boss's chance {c['boss_chance_before']} -> {c['boss_chance_after']}; "
              f"over half after in {c['after_over_half']}")
    print(f"Summary: {a.out}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
