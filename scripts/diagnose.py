"""Where the bot's runs are lost, and why: the table `baseline --diagnose` writes, read (make diagnose).

    python scripts/diagnose.py results/diagnose/round10.csv [--out results/diagnose/round10-summary.json]

One row per run: where it ended and by how much it fell short (what it scored against what the lost blind needed),
its build then (jokers, x mult, main hand's level, deck, money), ante by ante its biggest hand, money and x mult
jokers, and for a share of the lost runs whether the teacher beat the lost blind when it played it again from its
start with several times the usual samples (`deep_won`: a build that couldn't have scored it against play that
fell short). The tables split the losses by blind and ante, by how short they fell, by whether deeper play beat
the blind, and set the winners' build against the late losers' ante by ante, so the ante the losers fall behind at
shows.
"""
from __future__ import annotations

import argparse
import csv
import json
import sys
from pathlib import Path

import numpy as np

# Shortfall buckets: scored / needed at the lost blind.
BUCKETS = [(0.0, 0.25, "under a quarter"), (0.25, 0.5, "a quarter to a half"), (0.5, 0.8, "a half to 80%"),
           (0.8, 1.01, "80% or more: a near miss")]
# Losses at these antes are the late losses the winners' build is set against.
LATE = (5, 6, 7, 8)


def load(path: Path) -> list[dict]:
    with path.open(newline="") as f:
        return list(csv.DictReader(f))


def num(rows: list[dict], key: str) -> np.ndarray:
    return np.array([float(r[key]) for r in rows])


def pct(k: float, n: float) -> str:
    return f"{100 * k / n:.1f}%" if n else "-"


def median(x: np.ndarray) -> float:
    return float(np.median(x)) if len(x) else float("nan")


def summarise(rows: list[dict]) -> dict:
    won = num(rows, "won") > 0.5
    lost = [r for r, w in zip(rows, won) if not w]
    n, w, l_ = len(rows), int(won.sum()), len(lost)
    out: dict = {"runs": n, "wins": w, "losses": l_, "win_rate": round(w / n, 4) if n else None}
    if not lost:
        return out
    # Where.
    kind = lambda r: "boss" if r["lost_to"] not in ("Small Blind", "Big Blind") else r["lost_to"].split()[0].lower()
    by_kind: dict[str, int] = {}
    for r in lost:
        by_kind[kind(r)] = by_kind.get(kind(r), 0) + 1
    out["by_blind"] = {k: round(v / l_, 4) for k, v in sorted(by_kind.items(), key=lambda kv: -kv[1])}
    antes = num(lost, "ante").astype(int)
    out["by_ante"] = {int(a): round(float((antes == a).mean()), 4) for a in range(1, 9)}
    bosses: dict[str, int] = {}
    for r in lost:
        if kind(r) == "boss":
            bosses[r["lost_to"]] = bosses.get(r["lost_to"], 0) + 1
    out["by_boss"] = {k: round(v / l_, 4) for k, v in sorted(bosses.items(), key=lambda kv: -kv[1])[:12]}
    # How short.
    close = num(lost, "closeness")
    out["shortfall"] = {name: round(float(((close >= lo) & (close < hi)).mean()), 4) for lo, hi, name in BUCKETS}
    out["closeness_by_boss"] = {b: round(median(np.array([float(r["closeness"]) for r in lost if r["lost_to"] == b])), 3)
                                for b in list(out["by_boss"])[:8]}
    # Deeper play.
    deep = num(lost, "deep_won")
    tried = deep >= 0
    if tried.any():
        d = {"replayed": int(tried.sum()), "beaten": round(float((deep[tried] > 0.5).mean()), 4)}
        d["by_shortfall"] = {name: round(float((deep[tried & (close >= lo) & (close < hi)] > 0.5).mean()), 4)
                             for lo, hi, name in BUCKETS if (tried & (close >= lo) & (close < hi)).any()}
        d["by_ante"] = {int(a): round(float((deep[tried & (antes == a)] > 0.5).mean()), 4)
                        for a in range(1, 9) if (tried & (antes == a)).any()}
        d["by_boss"] = {}
        for b in list(out["by_boss"])[:8]:
            m = tried & np.array([r["lost_to"] == b for r in lost])
            if m.any():
                d["by_boss"][b] = round(float((deep[m] > 0.5).mean()), 4)
        out["deeper_play"] = d
    # The build at the loss.
    money = num(lost, "money_end")
    xm = num(lost, "n_xmult")
    out["at_the_loss"] = {
        "money_unspent_median": median(money), "money_20_or_more": round(float((money >= 20).mean()), 4),
        "xmult_jokers": {str(k): round(float((xm == k).mean()), 4) for k in (0, 1, 2)} | {
            "3+": round(float((xm >= 3).mean()), 4)},
        "main_level_median": median(num(lost, "main_level")), "deck_median": median(num(lost, "deck")),
        "hands_left_median": median(num(lost, "hands_left")),
    }
    # The build ante by ante: winners against the late losers.
    winners = [r for r, ok in zip(rows, won) if ok]
    late = [r for r in lost if int(float(r["ante"])) in LATE]
    curves: dict[str, dict] = {}
    for key, label, stat in (("ante_best", "biggest_hand_median", median), ("ante_money", "money_median", median),
                             ("ante_xmult", "xmult_jokers_mean", lambda x: round(float(x.mean()), 3) if len(x) else None),
                             ("ante_jokers", "jokers_mean", lambda x: round(float(x.mean()), 3) if len(x) else None)):
        curves[label] = {}
        for who, group in (("winners", winners), ("late_losers", late)):
            vals = []
            for a in range(1, 9):
                # An ante the run never reached reads 0: left out.
                reached = [float(r[f"{key}_{a}"]) for r in group if int(float(r["ante"])) >= a or float(r["won"]) > 0.5]
                vals.append(stat(np.array(reached)) if reached else None)
            curves[label][who] = vals
    out["by_ante_curves"] = curves
    # The ante the late losers fall behind at: the first where their median biggest hand is under 60% of the winners'.
    bw, bl = curves["biggest_hand_median"]["winners"], curves["biggest_hand_median"]["late_losers"]
    behind = next((a for a in range(1, 9) if bw[a - 1] and bl[a - 1] is not None and bl[a - 1] < 0.6 * bw[a - 1]), None)
    out["late_losers_fall_behind_at_ante"] = behind
    return out


def lines(s: dict, path: Path) -> list[str]:
    o = [f"Diagnosis of {path.name}: {s['runs']:,} runs, {s['wins']:,} won ({pct(s['wins'], s['runs'])}), "
         f"{s['losses']:,} lost."]
    if not s.get("losses"):
        return o
    o.append("")
    o.append("Where the losses are:")
    o.append("  by blind: " + ", ".join(f"{k} {100 * v:.0f}%" for k, v in s["by_blind"].items()))
    o.append("  by ante:  " + ", ".join(f"{a}: {100 * v:.0f}%" for a, v in s["by_ante"].items()))
    o.append("  bosses:   " + ", ".join(f"{k} {100 * v:.0f}%" for k, v in list(s["by_boss"].items())[:8]))
    o.append("")
    o.append("How short they fell (scored against the lost blind's target):")
    o.append("  " + ", ".join(f"{k} {100 * v:.0f}%" for k, v in s["shortfall"].items()))
    o.append("  median closeness by boss: " + ", ".join(f"{k} {v:.2f}" for k, v in s["closeness_by_boss"].items()))
    d = s.get("deeper_play")
    o.append("")
    if d:
        o.append(f"Could better play have beaten the blind? ({d['replayed']:,} lost blinds played again from their start "
                 f"by the teacher with deeper search): {100 * d['beaten']:.1f}% beaten.")
        o.append("  by shortfall: " + ", ".join(f"{k} {100 * v:.0f}%" for k, v in d["by_shortfall"].items()))
        o.append("  by ante:      " + ", ".join(f"{a}: {100 * v:.0f}%" for a, v in d["by_ante"].items()))
        o.append("  by boss:      " + ", ".join(f"{k} {100 * v:.0f}%" for k, v in d["by_boss"].items()))
    else:
        o.append("No lost blind was played again (--deep-share 0).")
    a = s["at_the_loss"]
    o.append("")
    o.append(f"At the loss: money unspent median ${a['money_unspent_median']:.0f} ({100 * a['money_20_or_more']:.0f}% "
             f"with $20 or more); x mult jokers: " + ", ".join(f"{k}: {100 * v:.0f}%" for k, v in a["xmult_jokers"].items())
             + f"; main hand level median {a['main_level_median']:.0f}; deck median {a['deck_median']:.0f} cards.")
    o.append("")
    o.append("The build ante by ante, winners against the runs lost at antes 5 to 8:")
    c = s["by_ante_curves"]
    fmt = lambda v, f: "-" if v is None else f.format(v)
    for label, f in (("biggest_hand_median", "{:,.0f}"), ("money_median", "${:.0f}"), ("xmult_jokers_mean", "{:.2f}"),
                     ("jokers_mean", "{:.1f}")):
        o.append(f"  {label.replace('_', ' ')}:")
        for who in ("winners", "late_losers"):
            o.append(f"    {who:12} " + "  ".join(f"{a}: {fmt(v, f)}" for a, v in zip(range(1, 9), c[label][who])))
    behind = s.get("late_losers_fall_behind_at_ante")
    o.append("")
    o.append("The late losers' biggest hand first falls under 60% of the winners' at ante "
             f"{behind}." if behind else "The late losers' biggest hand never falls under 60% of the winners' median.")
    return o


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("csv", type=Path, help="the table baseline --diagnose wrote")
    ap.add_argument("--out", type=Path, help="where to write the summary (JSON)")
    a = ap.parse_args()
    rows = load(a.csv)
    s = summarise(rows)
    s["table"] = str(a.csv)
    print("\n".join(lines(s, a.csv)))
    if a.out:
        a.out.parent.mkdir(parents=True, exist_ok=True)
        a.out.write_text(json.dumps(s, indent=1) + "\n")
        print(f"Summary: {a.out}")


if __name__ == "__main__":
    sys.exit(main())
