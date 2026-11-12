#!/usr/bin/env python3
"""The forced-buy forks' summary (make leak-tally, lab #20 part 1): for each joker named, what taking it was worth
where the bot passed it, from the lines `baseline --force-buy ... --force-out` wrote (gzipped or not).

Each fork is one position where the bot passed the joker (on sale and affordable: it left the shop or rerolled; or it
skipped it in a Buffoon pack), one per joker and run. Two lines from there on the same fair copies, each played out
by the bot as it is: its own move, and taking the joker (selling the held joker it would miss least if the row was
full). Per joker: the forks, the win rate of each line, the paired difference (forced less own: per fork the mean
over its copies, then the mean over forks, its 95% range from the forks' spread), the blinds beaten likewise, how
often the forced joker was still held as that shop closed and at the end, and the same by ante.

S's rule (#20, stated before the forks): a joker worth 3 points of win rate or more from those states, its range
above zero, is valued wrong by the judge (class A: fixed in the value); a joker the user calls strong that shows
nothing needs a way of playing the bot lacks (class B: Plan v0). The class here is the arithmetic only; reading it is
B's.

    scripts/forced_buys.py LINES.jsonl[.gz] --out SUMMARY.json
"""
from __future__ import annotations

import argparse
import gzip
import json
import math
from pathlib import Path

CLASS_A = 0.03
BANDS = (("2-4", 2, 4), ("5-6", 5, 6), ("7-8", 7, 8))


def read_lines(path: Path) -> list[dict]:
    op = gzip.open if path.suffix == ".gz" else open
    with op(path, "rt") as fh:
        return [json.loads(line) for line in fh if line.strip()]


def mean_ci(xs: list[float]) -> tuple[float | None, list[float] | None]:
    n = len(xs)
    if n == 0:
        return None, None
    m = sum(xs) / n
    if n < 2:
        return m, None
    sd = math.sqrt(sum((x - m) ** 2 for x in xs) / (n - 1))
    h = 1.96 * sd / math.sqrt(n)
    return m, [m - h, m + h]


def group(forks: list[dict]) -> dict:
    done = [f for f in forks if "own_ends" in f]
    wins_d, stage_d, own_w, forced_w, kept_shop, kept_end = [], [], [], [], [], []
    for f in done:
        o, x = f["own_ends"], f["forced_ends"]
        n = min(len(o), len(x))
        ow = sum(e[0] for e in o[:n]) / n
        fw = sum(e[0] for e in x[:n]) / n
        own_w.append(ow)
        forced_w.append(fw)
        wins_d.append(fw - ow)
        stage_d.append(sum(b[1] - a[1] for a, b in zip(o[:n], x[:n])) / n)
        kept_shop.append(sum(e[2] for e in x[:n]) / n)
        kept_end.append(sum(e[3] for e in x[:n]) / n)
    d, ci = mean_ci(wins_d)
    sd, sci = mean_ci(stage_d)

    def r(v):
        return round(v, 4) if v is not None else None
    out = {
        "forks": len(done),
        "skipped": len(forks) - len(done),
        "own_win_rate": r(sum(own_w) / len(own_w)) if own_w else None,
        "forced_win_rate": r(sum(forced_w) / len(forced_w)) if forced_w else None,
        "win_rate_diff": r(d),
        "win_rate_diff_ci": [r(c) for c in ci] if ci else None,
        "blinds_diff": r(sd),
        "blinds_diff_ci": [r(c) for c in sci] if sci else None,
        "forced_kept_as_shop_closed": r(sum(kept_shop) / len(kept_shop)) if kept_shop else None,
        "forced_kept_at_end": r(sum(kept_end) / len(kept_end)) if kept_end else None,
        "row_full_share": r(sum(1 for f in done if f.get("sold") is not None) / len(done)) if done else None,
    }
    if ci is not None:
        out["arithmetic"] = ("A: worth 3+ points, range above zero" if d >= CLASS_A and ci[0] > 0 else
                             "above zero, under 3 points" if ci[0] > 0 else
                             "below zero" if ci[1] < 0 else "level")
    return out


def summarise(forks: list[dict]) -> dict:
    jokers = sorted({f["joker"] for f in forks})
    out = {}
    for j in jokers:
        fs = [f for f in forks if f["joker"] == j]
        g = group(fs)
        g["by_ante"] = {name: group([f for f in fs if lo <= f["ante"] <= hi]) for name, lo, hi in BANDS}
        g["by_ante"] = {k: v for k, v in g["by_ante"].items() if v["forks"]}
        out[j] = g
    return out


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("lines", type=Path)
    ap.add_argument("--out", type=Path, required=True)
    a = ap.parse_args(argv)
    forks = read_lines(a.lines)
    s = {"lines": a.lines.name, "forks": len(forks), "runs": len({f["run"] for f in forks}),
         "jokers": summarise(forks)}
    a.out.parent.mkdir(parents=True, exist_ok=True)
    a.out.write_text(json.dumps(s, indent=1) + "\n")
    for j, g in s["jokers"].items():
        ci = g["win_rate_diff_ci"]
        rng = f"{100 * ci[0]:+.1f} to {100 * ci[1]:+.1f}" if ci else "n/a"
        diff = f"{100 * g['win_rate_diff']:+.1f}" if g["win_rate_diff"] is not None else "n/a"
        print(f"  {j}: {g['forks']} forks, forced less own {diff} points ({rng}); "
              f"{g.get('arithmetic', '')}")
    print(f"Summary: {a.out}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
