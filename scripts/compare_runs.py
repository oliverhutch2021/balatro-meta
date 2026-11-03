"""Two bots, or two versions of one, on the same seeds, run by run: which wins more, and how likely that is chance.

    python scripts/compare_runs.py results/value/teacher-w1.json results/value/steady-w1.json \
        [--out results/value/steady-vs-w1.json]

Each report is a baseline report (examples/baseline.rs `--out`) whose `run_stages` hold one letter per run in seed
order: A to X the blind the run ended at (A the first Small Blind, X the eighth ante's boss), Y a win. Runs are
paired by seed, the first N of both (N the shorter). Both bots see the same seeds, so the luck of a seed counts for
both, and a difference shows on fewer runs than two win rates measured apart would need.

Reported: each win rate on the common seeds, the difference with its 95% interval (paired), the runs only one of
them won, a two-sided p-value for the difference (the exact sign test on those runs), and the blinds beaten per
run with its paired difference.
"""
from __future__ import annotations

import argparse
import json
import math
import sys
from pathlib import Path

WIN = "Y"
LETTERS = "ABCDEFGHIJKLMNOPQRSTUVWXY"  # blinds beaten: A 0 ... X 23, Y 24 (a win)


def stages(report: dict) -> str:
    s = report.get("run_stages")
    if not isinstance(s, str) or not s:
        raise ValueError("the report has no run_stages (runs of more than 50,000 keep none)")
    bad = set(s) - set(LETTERS)
    if bad:
        raise ValueError(f"run_stages holds unknown letters: {''.join(sorted(bad))}")
    return s


def sign_test(k: int, n: int) -> float:
    """Two-sided exact p-value that k of n runs (each won by one bot only) went one way when each was a coin
    toss."""
    if n == 0:
        return 1.0
    k = min(k, n - k)
    # Whole numbers throughout: 2.0 ** n overflows a float past n = 1023 (8,000 seeds can have more split runs).
    # Adjacent binomial coefficients share almost all their work. Build each from the previous one,
    # keeping the sum exact, instead of recomputing k+1 large combinations from scratch.
    term = total = 1
    for i in range(1, k + 1):
        term = term * (n - i + 1) // i
        total += term
    tail = total / 2 ** n
    return min(1.0, 2.0 * tail)


def compare(a: dict, b: dict) -> dict:
    """B against A on the seeds both played."""
    sa, sb = stages(a), stages(b)
    # A report without a seed base played base 1 (baseline's default; older reports didn't name it), so it never
    # pairs with another base's runs (the exam's, a recording's) by passing unchecked.
    base_a, base_b = a.get("seed_base", 1), b.get("seed_base", 1)
    if base_a != base_b:
        raise ValueError(f"the reports differ in seed_base: {base_a!r} and {base_b!r}, so their runs don't pair")
    for key in ("deck", "stake"):
        if key in a and key in b and a[key] != b[key]:
            raise ValueError(f"the reports differ in {key}: {a[key]!r} and {b[key]!r}, so their runs don't pair")
    n = min(len(sa), len(sb))
    sa, sb = sa[:n], sb[:n]
    wa = [c == WIN for c in sa]
    wb = [c == WIN for c in sb]
    only_a = sum(x and not y for x, y in zip(wa, wb))
    only_b = sum(y and not x for x, y in zip(wa, wb))
    diff = (only_b - only_a) / n
    # Each seed's difference is -1, 0 or 1: its variance is the share of seeds that differ, less diff squared.
    se = math.sqrt(max((only_a + only_b) / n - diff * diff, 0.0) / n)
    ba = [LETTERS.index(c) for c in sa]
    bb = [LETTERS.index(c) for c in sb]
    d = [y - x for x, y in zip(ba, bb)]
    md = sum(d) / n
    sd = math.sqrt(sum((x - md) ** 2 for x in d) / (n - 1)) if n > 1 else 0.0
    return {
        "kind": "comparison",
        "a": {"bot": a.get("bot"), "win_rate": sum(wa) / n, "wins": sum(wa), "blinds_beaten": sum(ba) / n},
        "b": {"bot": b.get("bot"), "win_rate": sum(wb) / n, "wins": sum(wb), "blinds_beaten": sum(bb) / n},
        "seeds": n,
        "win_rate_diff": diff,
        "win_rate_diff_ci": [diff - 1.96 * se, diff + 1.96 * se],
        "only_a_won": only_a,
        "only_b_won": only_b,
        "p_value": sign_test(only_b, only_a + only_b),
        "blinds_beaten_diff": md,
        "blinds_beaten_diff_ci": [md - 1.96 * sd / math.sqrt(n), md + 1.96 * sd / math.sqrt(n)],
    }


def describe(c: dict, name_a: str, name_b: str) -> str:
    lo, hi = c["win_rate_diff_ci"]
    blo, bhi = c["blinds_beaten_diff_ci"]
    return "\n".join([
        f"{c['seeds']:,} seeds played by both.",
        f"  {name_a}: {100 * c['a']['win_rate']:.2f}% won, {c['a']['blinds_beaten']:.2f} blinds beaten a run",
        f"  {name_b}: {100 * c['b']['win_rate']:.2f}% won, {c['b']['blinds_beaten']:.2f} blinds beaten a run",
        (f"Difference ({name_b} less {name_a}): {100 * c['win_rate_diff']:+.2f} points of win rate "
         f"(95% interval {100 * lo:+.2f} to {100 * hi:+.2f}), {c['blinds_beaten_diff']:+.3f} blinds "
         f"({blo:+.3f} to {bhi:+.3f})."),
        (f"Seeds only one won: {name_a} {c['only_a_won']}, {name_b} {c['only_b_won']} "
         f"(p = {c['p_value']:.3g} that this split is chance)."),
    ])


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("a", type=Path, help="the report to compare against")
    ap.add_argument("b", type=Path, help="the report being tried")
    ap.add_argument("--out", type=Path, help="write the comparison here (JSON)")
    args = ap.parse_args(argv)
    try:
        c = compare(json.loads(args.a.read_text()), json.loads(args.b.read_text()))
    except (OSError, ValueError) as e:
        print(f"compare_runs: {e}", file=sys.stderr)
        return 1
    c["a"]["report"], c["b"]["report"] = str(args.a), str(args.b)
    print(describe(c, args.a.stem, args.b.stem))
    if args.out:
        args.out.parent.mkdir(parents=True, exist_ok=True)
        args.out.write_text(json.dumps(c, indent=1) + "\n")
        print(f"Comparison: {args.out}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
