#!/usr/bin/env python3
"""The build bench's validation (S and B, lab #33; `baseline --bench FILE`): does the bench's score of the build a run
held as an ante began predict whether the run cleared that ante?

Per ante (5 to 8) and overall: the ranking's accuracy (AUC: the chance a run that cleared scored above one that
didn't; 0.5 is no skill) and the share cleared by score band. Then the final builds: their score by ante (the build
frozen as the run ended, each boss the ante can hold averaged), for the won and the lost.

    scripts/bench_check.py BENCH.jsonl[.gz] --out SUMMARY.json
"""
from __future__ import annotations

import argparse
import gzip
import json
import statistics
from pathlib import Path

BANDS = ((0.0, 0.1), (0.1, 0.3), (0.3, 0.5), (0.5, 0.7), (0.7, 0.9), (0.9, 1.01))


def read(path: Path) -> list[dict]:
    op = gzip.open if path.suffix == ".gz" else open
    with op(path, "rt") as fh:
        return [json.loads(x) for x in fh if x.strip()]


def auc(pos: list[float], neg: list[float]) -> float | None:
    if not pos or not neg:
        return None
    # Rank-sum form (ties count half).
    allv = sorted([(v, 1) for v in pos] + [(v, 0) for v in neg])
    rank, i, rsum = 0.0, 0, 0.0
    while i < len(allv):
        j = i
        while j < len(allv) and allv[j][0] == allv[i][0]:
            j += 1
        r = (i + 1 + j) / 2
        rsum += r * sum(1 for k in range(i, j) if allv[k][1])
        i = j
    n1, n0 = len(pos), len(neg)
    return round((rsum - n1 * (n1 + 1) / 2) / (n1 * n0), 4)


def check(entries: list[tuple[int, float, bool]]) -> dict:
    pos = [s for _, s, c in entries if c]
    neg = [s for _, s, c in entries if not c]
    bands = {}
    for lo, hi in BANDS:
        sub = [c for _, s, c in entries if lo <= s < hi]
        bands[f"{lo:.1f}-{min(hi, 1.0):.1f}"] = {"n": len(sub), "cleared": round(sum(sub) / len(sub), 3) if sub else None,
                                                "mean_score": round(statistics.mean(s for _, s, _ in entries if lo <= s < hi), 3)
                                                if sub else None}
    return {"n": len(entries), "cleared": round(len(pos) / len(entries), 3) if entries else None, "auc": auc(pos, neg),
            "by_band": bands}


def summarise(rows: list[dict]) -> dict:
    entries = [(e["ante"], e["score"][3], bool(e["cleared"])) for r in rows for e in r.get("entered", [])]
    out = {"runs": len(rows), "all": check(entries),
           "by_ante": {a: check([x for x in entries if x[0] == a]) for a in range(5, 9)}}
    finals = {}
    for a in ("5", "6", "7", "8"):
        won = [r["final"][a][3] for r in rows if a in r.get("final", {}) and r["won"]]
        lost = [r["final"][a][3] for r in rows if a in r.get("final", {}) and not r["won"]]
        finals[a] = {"won_median": round(statistics.median(won), 3) if won else None,
                     "lost_median": round(statistics.median(lost), 3) if lost else None,
                     "lost_above_half": round(sum(x > 0.5 for x in lost) / len(lost), 3) if lost else None}
    out["final_builds"] = finals
    return out


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("bench", type=Path)
    ap.add_argument("--out", type=Path, required=True)
    a = ap.parse_args(argv)
    s = {"bench": a.bench.name, **summarise(read(a.bench))}
    a.out.parent.mkdir(parents=True, exist_ok=True)
    a.out.write_text(json.dumps(s, indent=1) + "\n")
    al = s["all"]
    print(f"{s['runs']} runs; {al['n']} antes entered from 5, {al['cleared']} cleared; AUC {al['auc']}")
    for k, v in al["by_band"].items():
        print(f"  score {k}: {v['n']} antes, cleared {v['cleared']} (mean score {v['mean_score']})")
    for an, v in s["by_ante"].items():
        print(f"  ante {an}: {v['n']} entered, cleared {v['cleared']}, AUC {v['auc']}")
    for an, v in s["final_builds"].items():
        print(f"  final build at ante {an}: median score won {v['won_median']}, lost {v['lost_median']}; "
              f"lost runs whose final build scores over 0.5: {v['lost_above_half']}")
    print(f"Summary: {a.out}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
