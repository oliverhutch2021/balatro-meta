#!/usr/bin/env python3
"""The shop audit's summary (make shop-audit): what the teacher saw and chose in the late shops, winners against the
runs lost at the last ante.

Reads the lines `baseline --audit-shops` wrote (gzipped or not) and the report of the same runs (its `run_stages`,
in run order, say how each run ended), and writes a summary: per group (won, lost at the win ante, lost earlier),
the shop decisions and how often the run left with money in hand while a reroll was on offer, the reroll's value
against leaving, how often its cost in value came out at zero or below (where `search` drops rerolling again,
teacher.rs), and what was chosen. A diagnosis, never play: nothing here changes the bot.

    scripts/audit_shops.py LINES.jsonl[.gz] REPORT.json --out SUMMARY.json
"""
from __future__ import annotations

import argparse
import gzip
import json
from collections import Counter, defaultdict
from pathlib import Path

WIN = "Y"
WIN_ANTE = 8


def read_lines(path: Path):
    op = gzip.open if path.suffix == ".gz" else open
    with op(path, "rt") as fh:
        for line in fh:
            line = line.strip()
            if line:
                yield json.loads(line)


def note(line: dict, kind: str) -> dict | None:
    return next((n[kind] for n in line.get("notes", []) if kind in n), None)


def group_of(stage: str) -> str:
    if stage == WIN:
        return "won"
    # Stages A.. run three to an ante: A-C ante 1, ..., V-X ante 8.
    ante = (ord(stage) - ord("A")) // 3 + 1
    return "lost at the win ante" if ante >= WIN_ANTE else "lost earlier"


def summarise(lines, stages: str) -> dict:
    groups: dict = defaultdict(lambda: {"decisions": 0, "runs": set(), "left": 0, "left_rich": 0,
                                        "left_rich_reroll_offered": 0, "reroll_offered": 0, "reroll_chosen": 0,
                                        "cost_le_zero": 0, "reroll_minus_leave": [], "money_left": [],
                                        "chosen": Counter(), "by_ante": Counter()})
    for line in lines:
        i = int(line["run"])
        if i >= len(stages):
            continue
        g = groups[group_of(stages[i])]
        g["decisions"] += 1
        g["runs"].add(i)
        g["by_ante"][int(line["ante"])] += 1
        chosen = str(line["chosen"]).split(" ")[0]
        g["chosen"][chosen] += 1
        reroll, full = note(line, "reroll"), note(line, "full")
        money = int(line["money"])
        leaving = chosen == "LeaveShop" or (full is not None and full.get("chosen") == "leave")
        if reroll is not None:
            g["reroll_offered"] += 1
            if reroll["cost"] <= 0:
                g["cost_le_zero"] += 1
            g["reroll_minus_leave"].append(reroll["q"] - reroll["here"])
        if chosen == "Reroll":
            g["reroll_chosen"] += 1
        if leaving:
            g["left"] += 1
            g["money_left"].append(money)
            if money >= 20:
                g["left_rich"] += 1
                if reroll is not None:
                    g["left_rich_reroll_offered"] += 1

    def med(xs):
        xs = sorted(xs)
        return xs[len(xs) // 2] if xs else None

    out = {}
    for name, g in groups.items():
        n = max(g["decisions"], 1)
        out[name] = {
            "runs": len(g["runs"]),
            "decisions": g["decisions"],
            "decisions_by_ante": dict(sorted(g["by_ante"].items())),
            "chosen": dict(g["chosen"].most_common()),
            "left": g["left"],
            "left_with_20_or_more": g["left_rich"],
            "left_with_20_or_more_while_a_reroll_was_offered": g["left_rich_reroll_offered"],
            "median_money_when_leaving": med(g["money_left"]),
            "reroll_offered": g["reroll_offered"],
            "reroll_chosen": g["reroll_chosen"],
            "reroll_cost_at_or_below_zero": g["cost_le_zero"],
            "reroll_quick_minus_leave_median": med(g["reroll_minus_leave"]),
            "share_of_decisions_leaving_rich": round(g["left_rich"] / n, 4),
        }
    return out


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("lines", type=Path)
    ap.add_argument("report", type=Path)
    ap.add_argument("--out", type=Path, required=True)
    a = ap.parse_args()
    report = json.loads(a.report.read_text())
    s = {"lines": str(a.lines.name), "report": str(a.report), "runs": report.get("runs"),
         "win_rate": report.get("win_rate"), "groups": summarise(read_lines(a.lines), report["run_stages"])}
    a.out.parent.mkdir(parents=True, exist_ok=True)
    a.out.write_text(json.dumps(s, indent=1) + "\n")
    for name, g in s["groups"].items():
        print(f"{name}: {g['runs']} runs, {g['decisions']} shop decisions; left {g['left']} times "
              f"({g['left_with_20_or_more']} with $20+, {g['left_with_20_or_more_while_a_reroll_was_offered']} of "
              f"them with a reroll on offer); reroll chosen {g['reroll_chosen']} of {g['reroll_offered']} offered, "
              f"its cost at or below zero {g['reroll_cost_at_or_below_zero']} times")
    print(f"Summary: {a.out}")


if __name__ == "__main__":
    main()
