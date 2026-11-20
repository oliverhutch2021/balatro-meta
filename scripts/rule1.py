#!/usr/bin/env python3
"""Rule 1 (lab #34): a prune or a neutral change never changes play. Four styles replay seeds the league has already
played, with the same command, and every run must end in the same place (`run_stages`, seed by seed):

- the `-d5` base, a8e3 (generation 2's reference) and 4579 on generation 2's block A (200 seeds from 50,001);
- W on the wall league's first 200 seeds (from 60,001).

The styles were played on -d5's code; from -d6 each plays with -d5's defaults set back where its genes leave them
(league.as_d5), and a fifth check, "a8e3-default", plays the champion's options alone: -d6's own defaults must be a8e3.

    scripts/rule1.py [--runs 200]

Writes `results/rule1/<style>.json` and `results/rule1/summary.json`; exits 1 if any run ends elsewhere.
"""
from __future__ import annotations

import argparse
import json
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import jobs  # noqa: E402
import league  # noqa: E402

ROOT = Path(__file__).resolve().parent.parent
OUT = Path("results/rule1")
REC = Path(".sync/rule1")

# (style, its genes on the champion's options, the report that played them, the ledger that holds its options)
CHECKS = [
    ("base", league.as_d5([]), "results/league/gen-2/base-b0.json", "results/league/gen-2.json"),
    ("a8e3", league.as_d5(league.G1_A8E3), "results/league/gen-2/ref-b0.json", "results/league/gen-2.json"),
    ("4579", league.as_d5(league.G0_4579), "results/league/gen-2/4579b745fb-b0.json", "results/league/gen-2.json"),
    ("W", league.as_d5(league.W), "results/league/gen-wall/ref-b0.json", "results/league/gen-wall.json"),
    ("a8e3-default", [], "results/league/gen-2/ref-b0.json", "results/league/gen-2.json"),
]


def check(style: str, genes: list[str], before: str, ledger: str, runs: int, play) -> dict:
    led = json.loads((ROOT / ledger).read_text())
    old = json.loads((ROOT / before).read_text())
    best = jobs.best_8k_report(ROOT)
    a = list(best[1])
    i = a.index("--teacher-options")
    if a[i + 1] != led["base_options"]:
        return {"style": style, "same": False, "why": f"the champion's options changed: {a[i + 1]!r} against "
                                                      f"{led['base_options']!r}"}
    if genes:
        a[i + 1] = f"{a[i + 1]},{','.join(genes)}"
    out, rec = OUT / f"{style}.json", REC / f"{style}.jsonl"
    (ROOT / out).parent.mkdir(parents=True, exist_ok=True)
    (ROOT / rec).parent.mkdir(parents=True, exist_ok=True)
    cmd = [*jobs.BASELINE, "--bot", "teacher", *a, "--runs", str(runs), "--seed", str(led["seed_base"]),
           "--salt", str(led["salt"]), "--stake", str(led["stake"]), "--calibrate", str(rec), "--out", str(out)]
    print(f"rule 1, {style}: {runs} seeds from {led['seed_base']}", flush=True)
    if play(cmd) != 0:
        return {"style": style, "same": False, "why": "the run failed"}
    new = json.loads((ROOT / out).read_text())["run_stages"][:runs]
    was = old["run_stages"][:runs]
    diff = [k for k, (x, y) in enumerate(zip(was, new)) if x != y]
    return {"style": style, "seeds": len(new), "same": len(new) == len(was) == runs and not diff,
            "differ_at": [led["seed_base"] + k for k in diff][:20], "won_before": was.count("Y"),
            "won_after": new.count("Y")}


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--runs", type=int, default=200)
    a = ap.parse_args(argv)
    def play(cmd: list[str]) -> int:
        return subprocess.call(cmd, cwd=ROOT)
    rows = [check(*c, a.runs, play) for c in CHECKS]
    ok = all(r["same"] for r in rows)
    (ROOT / OUT / "summary.json").write_text(json.dumps({"same": ok, "checks": rows}, indent=1) + "\n")
    for r in rows:
        print(f"  {r['style']}: {'same' if r['same'] else 'DIFFERENT'} {r.get('why') or r.get('differ_at') or ''}")
    print("Rule 1: " + ("play unchanged" if ok else "PLAY CHANGED: stop and tell Claude"))
    return 0 if ok else 1


if __name__ == "__main__":
    raise SystemExit(main())
