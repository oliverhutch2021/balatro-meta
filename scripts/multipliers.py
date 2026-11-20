#!/usr/bin/env python3
"""The multiplier table (S, lab #33): what one change does to a build's scoring power, from `baseline --multipliers`.

For each ante and change: the median (and quartiles) of the multiplier on power, exp(after - before) of the value's
log total (next round, and against this ante's boss with its effect); the median gain in the bench's chance of
clearing the ante; and the share of builds the change lifts to x1.8 or more of the boss's real target (B: the line
where blinds stop being lost, read with the boss's own target). Each figure for all builds and for the weakest third
(by power against the boss's target: where most losses are, B), and for the band under x1.3. Changes found by search
("best-rare:Baron") are pooled by their prefix, with the jokers they found counted. Ranked by power, the bench as the
second opinion (it reads low on weak builds).

    scripts/multipliers.py FILE.jsonl[.gz] [--out SUMMARY.json]
    scripts/multipliers.py --play [--runs 800] [--seed 300001]    (the PC job: a8e3 plays, then the table)
"""
from __future__ import annotations

import argparse
import gzip
import json
import math
import statistics
import subprocess
import sys
from collections import Counter, defaultdict
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
ROOT = Path(__file__).resolve().parent.parent
LINES = Path(".sync/multipliers")
OUT = Path("results/multipliers")

SAFE = math.log(1.8)
WEAK = math.log(1.3)


def read(path: Path) -> list[dict]:
    op = gzip.open if path.suffix == ".gz" else open
    with op(path, "rt") as fh:
        return [json.loads(x) for x in fh if x.strip()]


def family(name: str) -> str:
    """'best-rare:Baron' -> 'best-rare'; 'pair:a+b' -> 'pair'; a named joker ('joker:Mime') and others as they are."""
    return name if name.startswith("joker:") else name.split(":")[0]


def margin(rec: dict, ask: float) -> float:
    """ln(power against the boss's real target): the boss read where there is one, else the next round's."""
    p = rec.get("boss_power")
    if p is None or (isinstance(p, float) and math.isnan(p)):
        p = rec["power"]
    return p - ask


def quart(xs: list[float]) -> list[float]:
    if len(xs) < 4:
        return [round(statistics.median(xs), 3)] * 3 if xs else [None] * 3
    q = statistics.quantiles(xs, n=4)
    return [round(q[0], 3), round(statistics.median(xs), 3), round(q[2], 3)]


def table(lines: list[dict]) -> dict:
    """One ante's rows: change family -> figures for all, the weakest third, and the band under x1.3."""
    if not lines:
        return {}
    base = sorted(margin(x["before"], x["ask"][2]) for x in lines)
    third = base[len(base) // 3] if len(base) >= 3 else base[-1]
    groups = {"all": lambda m: True, "weakest_third": lambda m: m <= third, "under_x1.3": lambda m: m < WEAK}
    rows: dict = defaultdict(lambda: {g: defaultdict(list) for g in groups})
    found: dict = defaultdict(Counter)
    for x in lines:
        b = x["before"]
        m0 = margin(b, x["ask"][2])
        for name, a in x["after"].items():
            fam = family(name)
            if ":" in name and fam != name:
                found[fam][name.split(":", 1)[1]] += 1
            for g, keep in groups.items():
                if not keep(m0):
                    continue
                r = rows[fam][g]
                r["power_x"].append(math.exp(a["power"] - b["power"]))
                r["boss_x"].append(math.exp(margin(a, x["ask"][2]) - m0))
                r["bench_gain"].append(a["bench"][3] - b["bench"][3])
                r["lifted"].append(m0 < SAFE <= margin(a, x["ask"][2]))
                r["below"].append(m0 < SAFE)
    out = {}
    for fam, by in rows.items():
        out[fam] = {}
        for g, r in by.items():
            if not r["power_x"]:
                continue
            below = sum(r["below"])
            out[fam][g] = {"builds": len(r["power_x"]), "power_x": quart(r["power_x"]), "boss_x": quart(r["boss_x"]),
                           "bench_gain": round(statistics.median(r["bench_gain"]), 4),
                           "lifted_over_x1.8": round(sum(r["lifted"]) / below, 3) if below else None}
        if found[fam]:
            out[fam]["found"] = dict(found[fam].most_common(8))
    return out


def summary(lines: list[dict]) -> dict:
    s = {}
    for ante in sorted({x["ante"] for x in lines}):
        sub = [x for x in lines if x["ante"] == ante]
        m = [margin(x["before"], x["ask"][2]) for x in sub]
        s[str(ante)] = {
            "builds": len(sub),
            "cleared": round(sum(x["cleared"] for x in sub) / len(sub), 3),
            "boss_power_over_ask_median": round(math.exp(statistics.median(m)), 2),
            "at_x1.8_or_more": round(sum(v >= SAFE for v in m) / len(m), 3),
            "under_x1.3": round(sum(v < WEAK for v in m) / len(m), 3),
            "changes": table(sub),
        }
    return s


def play(runs: int, seed: int) -> Path:
    """a8e3 (generation 2's reference, the base) plays `runs` fresh Green seeds with the champion's networks, writing
    a multiplier line at the first blind choice of antes 5 and 7; returns the lines' file (gzipped)."""
    import jobs
    import league
    best = jobs.best_8k_report(ROOT)
    if best is None:
        raise SystemExit("No champion report found")
    a = list(best[1])
    i = a.index("--teacher-options")
    a[i + 1] = f"{a[i + 1]},{','.join(league.G1_A8E3)}"
    lines = LINES / f"a8e3-{runs}-from-{seed}.jsonl"
    (ROOT / lines).parent.mkdir(parents=True, exist_ok=True)
    (ROOT / OUT).mkdir(parents=True, exist_ok=True)
    cmd = [*jobs.BASELINE, "--bot", "teacher", *a, "--runs", str(runs), "--seed", str(seed), "--salt", "2",
           "--stake", "3", "--multipliers", str(lines), "--at", "5,7",
           "--out", str(OUT / f"a8e3-{runs}-from-{seed}-report.json")]
    print(f"multiplier table: a8e3 on {runs} Green seeds from {seed}", flush=True)
    if subprocess.call(cmd, cwd=ROOT) != 0:
        raise SystemExit("the run failed")
    subprocess.call(["gzip", "-f", str(lines)], cwd=ROOT)
    return ROOT / lines.with_name(lines.name + ".gz")


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("file", type=Path, nargs="?")
    ap.add_argument("--out", type=Path)
    ap.add_argument("--play", action="store_true")
    ap.add_argument("--runs", type=int, default=800)
    ap.add_argument("--seed", type=int, default=300001)
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
    for ante, t in s.items():
        print(f"Ante {ante}: {t['builds']} builds, cleared {t['cleared']:.0%}; boss power against its real target, "
              f"median x{t['boss_power_over_ask_median']}; at x1.8 or more {t['at_x1.8_or_more']:.0%}, "
              f"under x1.3 {t['under_x1.3']:.0%}")
        print(f"  {'change':<20} {'power x (all)':>22} {'power x (weakest third)':>24} {'bench +':>8} "
              f"{'lifted to x1.8':>14}")
        rows = sorted(t["changes"].items(), key=lambda kv: -(kv[1].get("weakest_third", kv[1]["all"])["power_x"][1]
                                                             or 0))
        for fam, r in rows:
            al, wk = r["all"], r.get("weakest_third", r["all"])
            lift = wk["lifted_over_x1.8"]
            print(f"  {fam:<20} {al['power_x'][1]:>8} ({al['power_x'][0]}-{al['power_x'][2]}) "
                  f"{wk['power_x'][1]:>10} ({wk['power_x'][0]}-{wk['power_x'][2]}) {wk['bench_gain']:>+8.3f} "
                  f"{'' if lift is None else f'{lift:.0%}':>14}" + (f"  {list(r['found'])[:3]}" if "found" in r else ""))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
