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
    if "win_hand" in rows[0]:
        out["winning_hands"] = winning_hands(rows, won)
        out["jokers"] = joker_table(rows, won)
        out["swings"] = swings(rows, won)
    return out


def winning_hands(rows: list[dict], won: np.ndarray) -> dict:
    """The scoring hands: for each hand type, the share of runs it was the main scoring hand of (the type that
    cleared most blinds as the blind's biggest hand), those runs' win rate, its median final level in wins, and the
    share of all blinds it cleared; and the type of each run's biggest hand."""
    n = len(rows)
    out = {"main": {}, "biggest": {}}
    by_hand: dict[str, list[tuple[bool, int, int]]] = {}
    for r, w in zip(rows, won):
        by_hand.setdefault(r["win_hand"], []).append((bool(w), int(float(r["win_hand_blinds"])),
                                                       int(float(r["win_hand_level"]))))
    total_blinds = sum(int(float(r["win_hand_blinds"])) for r in rows)
    for h, items in sorted(by_hand.items(), key=lambda kv: -len(kv[1])):
        wins = [lvl for w, _, lvl in items if w]
        out["main"][h] = {"runs": len(items), "share": round(len(items) / n, 4),
                          "win_rate": round(sum(w for w, _, _ in items) / len(items), 4),
                          "blinds_cleared_share": round(sum(b for _, b, _ in items) / max(total_blinds, 1), 4),
                          "level_in_wins_median": float(np.median(wins)) if wins else None}
    big: dict[str, list[bool]] = {}
    for r, w in zip(rows, won):
        if r.get("best_hand_type"):
            big.setdefault(r["best_hand_type"], []).append(bool(w))
    for h, ws in sorted(big.items(), key=lambda kv: -len(kv[1])):
        out["biggest"][h] = {"runs": len(ws), "share": round(len(ws) / n, 4), "win_rate": round(sum(ws) / len(ws), 4)}
    return out


def joker_table(rows: list[dict], won: np.ndarray, least: int = 40) -> dict:
    """Each joker held at the end of a run (at least `least` runs): how often it was held, the win rate of the runs
    that held it, and the lift over the win rate of the runs that didn't. A run's end is its win or its loss, so
    this is association, not cause: a joker that keeps runs alive and one that strong runs can afford both score
    high."""
    n = len(rows)
    base = float(won.mean())
    held: dict[str, list[bool]] = {}
    for r, w in zip(rows, won):
        for j in {x for x in r.get("jokers", "").split(";") if x}:
            held.setdefault(j, []).append(bool(w))
    table = []
    for j, ws in held.items():
        if len(ws) < least:
            continue
        k, m = sum(ws), len(ws)
        others = (won.sum() - k) / max(n - m, 1)
        table.append({"joker": j, "held": m, "held_share": round(m / n, 4), "win_rate": round(k / m, 4),
                      "lift": round(k / m - others, 4)})
    table.sort(key=lambda t: -t["lift"])
    return {"base_win_rate": round(base, 4), "least_runs": least, "by_lift": table,
            "most_held": sorted(table, key=lambda t: -t["held"])[:15]}


def swings(rows: list[dict], won: np.ndarray) -> dict:
    """What moves a run's outlook most, as win rates by where a run stood at an ante: x mult jokers held as ante 4
    began, money as ante 3 began, the ante the first x mult joker arrived, the main scoring hand's final level, and
    vouchers taken."""
    out: dict = {}

    def by(key, label, groups):
        got = {}
        for name, pick in groups:
            m = np.array([pick(r) for r in rows])
            if m.sum() >= 30:
                got[name] = {"runs": int(m.sum()), "win_rate": round(float(won[m].mean()), 4)}
        out[label] = got

    xm4 = lambda r: int(float(r["ante_xmult_4"]))
    by("ante_xmult_4", "x mult jokers as ante 4 began", [("none", lambda r: xm4(r) == 0), ("one", lambda r: xm4(r) == 1),
                                                        ("two or more", lambda r: xm4(r) >= 2)])
    m3 = lambda r: int(float(r["ante_money_3"]))
    by("ante_money_3", "money as ante 3 began", [("under $8", lambda r: m3(r) < 8), ("$8 to $15", lambda r: 8 <= m3(r) <= 15),
                                                 ("$16 or more", lambda r: m3(r) >= 16)])

    def first_x(r):
        for a in range(1, 9):
            if int(float(r[f"ante_xmult_{a}"])) > 0:
                return a
        return 9
    by("first_xmult_ante", "the ante the first x mult joker was held by", [
        ("by ante 3", lambda r: first_x(r) <= 3), ("ante 4 or 5", lambda r: 4 <= first_x(r) <= 5),
        ("ante 6 to 8", lambda r: 6 <= first_x(r) <= 8), ("never", lambda r: first_x(r) == 9)])
    lvl = lambda r: int(float(r["win_hand_level"]))
    by("win_hand_level", "the main scoring hand's final level", [("1 or 2", lambda r: lvl(r) <= 2), ("3 to 5", lambda r: 3 <= lvl(r) <= 5),
                                                                 ("6 to 9", lambda r: 6 <= lvl(r) <= 9), ("10 or more", lambda r: lvl(r) >= 10)])
    v = lambda r: int(float(r["vouchers"]))
    by("vouchers", "vouchers taken", [("none", lambda r: v(r) == 0), ("one", lambda r: v(r) == 1), ("two", lambda r: v(r) == 2),
                                      ("three or more", lambda r: v(r) >= 3)])
    jk = lambda r: int(float(r["ante_jokers_3"]))
    by("ante_jokers_3", "jokers held as ante 3 began", [("0 to 2", lambda r: jk(r) <= 2), ("3", lambda r: jk(r) == 3),
                                                        ("4 or 5", lambda r: jk(r) >= 4)])
    return out


def lines(s: dict, path: Path) -> list[str]:
    head = (f"Diagnosis of {path.name}: {s['runs']:,} runs, {s['wins']:,} won ({pct(s['wins'], s['runs'])}), "
            f"{s['losses']:,} lost.")
    o = [head]
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
    if "winning_hands" in s:
        o.append("")
        o.append("The scoring hands (the hand type that cleared most of a run's blinds as the blind's biggest hand):")
        for h, t in list(s["winning_hands"]["main"].items())[:8]:
            lvl = t["level_in_wins_median"]
            o.append(f"  {h:16} main hand of {100 * t['share']:4.1f}% of runs, which won {100 * t['win_rate']:4.1f}%; "
                     f"cleared {100 * t['blinds_cleared_share']:4.1f}% of all blinds; level in wins median "
                     f"{lvl if lvl is None else int(lvl)}")
        o.append("  the run's single biggest hand: " + ", ".join(
            f"{h} {100 * t['share']:.0f}%" for h, t in list(s["winning_hands"]["biggest"].items())[:6]))
        j = s["jokers"]
        o.append("")
        o.append(f"Jokers held at the end, by lift in win rate over runs without them (base {100 * j['base_win_rate']:.1f}%, "
                 f"at least {j['least_runs']} runs; association, not cause):")
        for t in j["by_lift"][:12]:
            o.append(f"  {t['joker']:20} held in {100 * t['held_share']:4.1f}% of runs: won {100 * t['win_rate']:4.1f}% ({t['lift'] * 100:+.1f})")
        o.append("  ...")
        for t in j["by_lift"][-6:]:
            o.append(f"  {t['joker']:20} held in {100 * t['held_share']:4.1f}% of runs: won {100 * t['win_rate']:4.1f}% ({t['lift'] * 100:+.1f})")
        o.append("  most held: " + ", ".join(f"{t['joker']} {100 * t['held_share']:.0f}% ({100 * t['win_rate']:.0f}%)"
                                             for t in j["most_held"][:10]))
        o.append("")
        o.append("The biggest swings: win rate by where a run stood")
        for label, groups in s["swings"].items():
            o.append(f"  {label}: " + ", ".join(f"{g} {100 * t['win_rate']:.0f}% ({t['runs']:,})" for g, t in groups.items()))
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
