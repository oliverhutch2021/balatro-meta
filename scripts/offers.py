#!/usr/bin/env python3
"""Offered and passed (S and B, lab #33): from `baseline --offers`, how strong the jokers a8e3's shops offered were
for the build at the time, and which it took.

Each shop row the run saw at antes 2 to 7 (its first look and after each reroll or buy) has every joker on offer read
as a multiplier on the build's power, held in place of the weakest joker (or in a free slot), on 6 deals. A joker is
"affordable" if its cost was within the money plus the weakest joker's sale; "taken" if the run left that shop holding
one it did not hold when the joker was offered. By band of multiplier and by ante: jokers offered, the share
affordable, the share of the affordable taken, and the share of shops that offered at least one at x1.3 or x1.6.
Also what Gros Michel led to: runs that held it, and Cavendish offered later.

    scripts/offers.py FILE.jsonl[.gz] [--out SUMMARY.json]
    scripts/offers.py --play [--runs 1000] [--seed 200001]    (the PC job: a8e3 plays with --offers, then this)
"""
from __future__ import annotations

import argparse
import gzip
import json
import subprocess
import sys
from collections import defaultdict
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
ROOT = Path(__file__).resolve().parent.parent
LINES = Path(".sync/offers")
OUT = Path("results/offers")
BANDS = ((0.0, 1.1), (1.1, 1.3), (1.3, 1.6), (1.6, 2.0), (2.0, 99.0))
ANTES = ((2, 3), (4, 5), (6, 7))


def read(path: Path) -> list[dict]:
    op = gzip.open if path.suffix == ".gz" else open
    with op(path, "rt") as fh:
        return [json.loads(x) for x in fh if x.strip()]


def offers_of(shop: dict) -> list[dict]:
    """Each joker kind offered in this shop, at its first appearance: multiplier, affordable, taken."""
    seen: dict = {}
    for row in shop["rows"]:
        budget = row["money"] + (row["weakest_sale"] if row["full"] else 0)
        for kind, cost, x in row["offers"]:
            if kind in seen:
                continue
            taken = shop["left_with"].count(kind) > row["jokers"].count(kind)
            seen[kind] = {"kind": kind, "x": x, "affordable": cost <= budget, "taken": taken}
    return list(seen.values())


def summary(runs: list[dict]) -> dict:
    s: dict = {}
    for lo_a, hi_a in ANTES:
        bands = {f"{lo}-{hi}": defaultdict(int) for lo, hi in BANDS}
        shops = strong13 = strong16 = 0
        for r in runs:
            for shop in r["shops"]:
                if not lo_a <= shop["ante"] <= hi_a:
                    continue
                shops += 1
                offs = offers_of(shop)
                strong13 += any(o["x"] >= 1.3 for o in offs)
                strong16 += any(o["x"] >= 1.6 for o in offs)
                for o in offs:
                    for lo, hi in BANDS:
                        if lo <= o["x"] < hi:
                            b = bands[f"{lo}-{hi}"]
                            b["offered"] += 1
                            b["affordable"] += o["affordable"]
                            b["taken"] += o["taken"] and o["affordable"]
        s[f"{lo_a}-{hi_a}"] = {
            "shops": shops,
            "shops_offering_x1.3": round(strong13 / shops, 3) if shops else None,
            "shops_offering_x1.6": round(strong16 / shops, 3) if shops else None,
            "bands": {k: {"offered": v["offered"],
                          "affordable": round(v["affordable"] / v["offered"], 3) if v["offered"] else None,
                          "taken_of_affordable": round(v["taken"] / v["affordable"], 3) if v["affordable"] else None}
                      for k, v in bands.items()},
        }
    held_gm = [r for r in runs if any("Gros Michel" in row["jokers"] for sh in r["shops"] for row in sh["rows"])]
    cav = [r for r in held_gm if any(o[0] == "Cavendish" for sh in r["shops"] for row in sh["rows"]
                                     for o in row["offers"])]
    s["gros_michel"] = {"runs": len(runs), "held_gros_michel": len(held_gm), "then_offered_cavendish": len(cav)}
    return s


def play(runs: int, seed: int) -> Path:
    """a8e3 (the -d6 defaults on the champion's networks) plays `runs` fresh Green seeds with --offers."""
    import jobs
    best = jobs.best_8k_report(ROOT)
    if best is None:
        raise SystemExit("No champion report found")
    lines = LINES / f"a8e3-{runs}-from-{seed}.jsonl"
    (ROOT / lines).parent.mkdir(parents=True, exist_ok=True)
    (ROOT / OUT).mkdir(parents=True, exist_ok=True)
    cmd = [*jobs.BASELINE, "--bot", "teacher", *best[1], "--runs", str(runs), "--seed", str(seed), "--salt", "2",
           "--stake", "3", "--offers", str(lines), "--out", str(OUT / f"a8e3-{runs}-from-{seed}-report.json")]
    print(f"offered and passed: a8e3 on {runs} Green seeds from {seed}", flush=True)
    if subprocess.call(cmd, cwd=ROOT) != 0:
        raise SystemExit("the run failed")
    subprocess.call(["gzip", "-f", str(lines)], cwd=ROOT)
    return ROOT / lines.with_name(lines.name + ".gz")


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("file", type=Path, nargs="?")
    ap.add_argument("--out", type=Path)
    ap.add_argument("--play", action="store_true")
    ap.add_argument("--runs", type=int, default=1000)
    ap.add_argument("--seed", type=int, default=200001)
    a = ap.parse_args(argv)
    if a.play:
        a.file = play(a.runs, a.seed)
        a.out = a.out or ROOT / OUT / f"a8e3-{a.runs}-from-{a.seed}.json"
    if a.file is None:
        ap.error("a lines file, or --play")
    s = summary(read(a.file))
    if a.out:
        a.out.parent.mkdir(parents=True, exist_ok=True)
        a.out.write_text(json.dumps(s, indent=1) + "\n")
    for antes, t in s.items():
        if antes == "gros_michel":
            continue
        print(f"Antes {antes}: {t['shops']} shops; offering a joker at x1.3 or more: {t['shops_offering_x1.3']}, "
              f"at x1.6: {t['shops_offering_x1.6']}")
        for band, b in t["bands"].items():
            print(f"  x{band:<8} offered {b['offered']:>6}  affordable {b['affordable']}  taken {b['taken_of_affordable']}")
    g = s["gros_michel"]
    print(f"Gros Michel: held in {g['held_gros_michel']} of {g['runs']} runs; Cavendish offered later in "
          f"{g['then_offered_cavendish']}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
