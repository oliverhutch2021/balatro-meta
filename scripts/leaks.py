#!/usr/bin/env python3
"""The leak tally's summary (make leak-tally, lab #19): what the runs did with their seals, consumables, won rounds,
shops and packs, from the lines `baseline --leaks` wrote (one per run, gzipped or not).

For each leak: the share of runs where it could happen ("present") and how often the value was collected when it
could be. S's rule (stated before the tally): a leak present in a fifth of runs or more and collected less than half
the time gets a fix; the fixes go in as one bundle on one 8,000-seed arm, and this tally run again on the fixed bot is
the proof the behaviour changed, and the win rate must not fall: the lower end of that arm's 8,000-seed paired 95%
range above -1.0 point (B, #21). Each leak is also split by runs won and lost. A diagnosis, never play.

    scripts/leaks.py LINES.jsonl[.gz] --out SUMMARY.json
"""
from __future__ import annotations

import argparse
import gzip
import json
import statistics
from collections import Counter
from pathlib import Path

PRESENT_BAR = 0.20
COLLECTED_BAR = 0.50
SEAL_SPECTRALS = ("Deja Vu", "Talisman", "Medium", "Trance")


def read_lines(path: Path) -> list[dict]:
    op = gzip.open if path.suffix == ".gz" else open
    with op(path, "rt") as fh:
        return [json.loads(line) for line in fh if line.strip()]


def total(runs: list[dict], part: str) -> Counter:
    c: Counter = Counter()
    for r in runs:
        c.update(r["tally"].get(part, {}))
    return c


def ratio(a: float, b: float) -> float | None:
    return round(a / b, 4) if b else None


def leak(runs: list[dict], present, chances, collected) -> dict:
    """present(t) -> bool; chances(t), collected(t) -> counts. Rates over all runs and split won/lost."""
    def one(rs):
        n = len(rs)
        p = sum(1 for r in rs if present(r["tally"]))
        ch = sum(chances(r["tally"]) for r in rs)
        co = sum(collected(r["tally"]) for r in rs)
        return {"runs": n, "present_share": ratio(p, n), "chances": ch, "collected": co,
                "collected_share": ratio(co, ch)}
    out = one(runs)
    out["won"] = one([r for r in runs if r["won"]])
    out["lost"] = one([r for r in runs if not r["won"]])
    ps, cs = out["present_share"], out["collected_share"]
    out["fix_by_rule"] = bool(ps is not None and cs is not None and ps >= PRESENT_BAR and cs < COLLECTED_BAR)
    return out


def g(t: dict, part: str, key: str) -> int:
    return t.get(part, {}).get(key, 0)


HELD_BAR = 0.05  # the user's acceptance figure for held cards (#19)


def held_through_share(runs: list[dict]) -> dict:
    """Per Tarot or Spectral kind: rounds held at the start, still held at the end, and the share (the user's figure:
    under 5% for each kind, or a named reason). Planets apart (used at once unless Observatory)."""
    start, through = total(runs, "held_at_start"), total(runs, "held_through")
    planets = {"Mercury", "Venus", "Earth", "Mars", "Jupiter", "Saturn", "Uranus", "Neptune", "Pluto", "Planet X",
               "Ceres", "Eris"}
    kinds = {k: {"rounds": n, "through": through.get(k, 0), "share": ratio(through.get(k, 0), n)}
             for k, n in start.most_common() if k not in planets}
    all_n = sum(v["rounds"] for v in kinds.values())
    all_t = sum(v["through"] for v in kinds.values())
    return {"all": {"rounds": all_n, "through": all_t, "share": ratio(all_t, all_n)},
            "over_bar": sorted(k for k, v in kinds.items() if v["share"] is not None and v["share"] >= HELD_BAR),
            "kinds": kinds}


def summarise(runs: list[dict]) -> dict:
    seals, rounds, ways = total(runs, "seals"), total(runs, "rounds"), total(runs, "ways")
    leaks = {
        "blue_seal_planet": leak(
            runs, lambda t: g(t, "seals", "blue_drawn") > 0,
            lambda t: g(t, "seals", "blue_drawn"), lambda t: g(t, "seals", "blue_planet_made")),
        "purple_seal_tarot": leak(
            runs, lambda t: g(t, "seals", "purple_drawn") > 0,
            lambda t: g(t, "seals", "purple_drawn"), lambda t: g(t, "seals", "purple_tarot_made")),
        # A won round with room (a winning hand in hand with 2+ hands or any discard left): collected when the round
        # didn't end on that move (the bot took value first).
        "won_round_room_used": leak(
            runs, lambda t: g(t, "rounds", "won_with_room") > 0,
            lambda t: g(t, "rounds", "won_with_room"),
            lambda t: g(t, "rounds", "won_with_room") - g(t, "rounds", "won_with_room_at_once")),
        # A consumable held as a round began: collected when it was gone by the round's end (used or sold).
        "held_consumable_used": leak(
            runs, lambda t: sum(t.get("held_at_start", {}).values()) > 0,
            lambda t: sum(t.get("held_at_start", {}).values()),
            lambda t: sum(t.get("held_at_start", {}).values()) - sum(t.get("held_through", {}).values())),
        # A shop left with money at the interest cap even after the cheapest pack still on sale: collected never.
        "rich_shop_pack_opened": leak(
            runs, lambda t: g(t, "shops", "left_rich_pack_unopened") > 0,
            lambda t: g(t, "shops", "left"),
            lambda t: g(t, "shops", "left") - g(t, "shops", "left_rich_pack_unopened")),
    }
    hands = total(runs, "hands")
    played = sum(hands.values())
    main_share = []
    for r in runs:
        h = r["tally"].get("hands", {})
        if sum(h.values()):
            main_share.append(max(h.values()) / sum(h.values()))
    targets: dict = {}
    for r in runs:
        for kind, ranks, sealed, hand in r["tally"].get("targets", []):
            d = targets.setdefault(kind, {"uses": 0, "rank0": 0, "ranks": [], "already_sealed": 0, "cards": 0})
            d["uses"] += 1
            d["rank0"] += int(0 in ranks)
            d["ranks"].extend(ranks)
            d["already_sealed"] += sum(sealed)
            d["cards"] += len(ranks)
    for kind, d in targets.items():
        rs = d.pop("ranks")
        d["mean_rank"] = round(statistics.mean(rs), 2) if rs else None
        d["share_on_most_valuable"] = ratio(d["rank0"], d["uses"])
        d["share_already_sealed"] = ratio(d["already_sealed"], d["cards"])
    last = [r["tally"]["last_shop"] for r in runs if r["tally"].get("last_shop")]

    def med(xs):
        return statistics.median(xs) if xs else None
    jokers_by_ante: dict = {}
    for r in runs:
        for ante, name, how in r["tally"].get("jokers", []):
            jokers_by_ante.setdefault(how, Counter())[f"{ante}"] += 1
    joker_counts: dict = {}
    for r in runs:
        for ante, name, how in r["tally"].get("jokers", []):
            joker_counts.setdefault(how, Counter())[name] += 1
    offered, affordable, redeemed = (total(runs, k) for k in
                                     ("vouchers_offered", "vouchers_affordable", "vouchers_redeemed"))
    by4 = [r["tally"].get("blue_or_purple_by_ante4") for r in runs]
    by4 = [x for x in by4 if x is not None]
    way_names = sorted({k.split(":")[0] for k in ways})
    return {
        "runs": len(runs),
        "win_rate": ratio(sum(r["won"] for r in runs), len(runs)),
        "leaks": leaks,
        "seal_events": dict(sorted(seals.items())),
        "blue_or_purple_seal_by_ante4_share": ratio(sum(by4), len(by4)),
        "rounds": dict(sorted(rounds.items())),
        "finish_ways": {w: {o: ways.get(f"{w}:{o}", 0) for o in ("same_move", "failed", "passed", "kept", "made")}
                        for w in way_names},
        "left_at_win": dict(total(runs, "left_at_win").most_common(15)),
        "held_through_share": held_through_share(runs),
        "held_at_start": dict(total(runs, "held_at_start").most_common()),
        "held_through": dict(total(runs, "held_through").most_common()),
        "used": dict(total(runs, "used").most_common()),
        "sold": dict(total(runs, "sold").most_common()),
        "targets": dict(sorted(targets.items())),
        "seal_spectral_targets": {k: targets[k] for k in SEAL_SPECTRALS if k in targets},
        "spectral_offered": dict(total(runs, "spectral_offered").most_common()),
        "spectral_taken": dict(total(runs, "spectral_taken").most_common()),
        "shops": dict(total(runs, "shops")),
        "last_shop_money_median": {"with_bull_or_bootstraps": med([x["money"] for x in last if x["bull_or_bootstraps"]]),
                                   "without": med([x["money"] for x in last if not x["bull_or_bootstraps"]]),
                                   "runs_with": sum(1 for x in last if x["bull_or_bootstraps"]),
                                   "runs_without": sum(1 for x in last if not x["bull_or_bootstraps"])},
        "hands": dict(hands.most_common()),
        "two_pair_share": ratio(hands.get("Two Pair", 0), played),
        "two_pair_per_run": ratio(hands.get("Two Pair", 0), len(runs)),
        "main_hand_share_median": round(statistics.median(main_share), 4) if main_share else None,
        "jokers_by_ante": {how: dict(sorted(c.items())) for how, c in jokers_by_ante.items()},
        "jokers": {how: dict(c.most_common()) for how, c in joker_counts.items()},
        "vouchers": {v: {"offered": offered[v], "affordable": affordable.get(v, 0), "redeemed": redeemed.get(v, 0),
                         "redeemed_when_affordable": ratio(redeemed.get(v, 0), affordable.get(v, 0))}
                     for v in sorted(offered)},
        "vouchers_per_run": ratio(sum(redeemed.values()), len(runs)),
        "boss_rerolled_away": dict(total(runs, "boss_rerolled_away").most_common()),
        "runs_with_a_boss_reroll": sum(1 for r in runs if r["tally"].get("boss_rerolled_away")),
    }


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("lines", type=Path)
    ap.add_argument("--out", type=Path, required=True)
    a = ap.parse_args(argv)
    s = {"lines": a.lines.name, **summarise(read_lines(a.lines))}
    a.out.parent.mkdir(parents=True, exist_ok=True)
    a.out.write_text(json.dumps(s, indent=1) + "\n")
    print(f"{s['runs']} runs, {100 * (s['win_rate'] or 0):.2f}% won")
    for name, l in s["leaks"].items():
        print(f"  {name}: present in {100 * (l['present_share'] or 0):.1f}% of runs, collected "
              f"{l['collected']} of {l['chances']} ({100 * (l['collected_share'] or 0):.1f}%)"
              f"{'  -> fix (rule)' if l['fix_by_rule'] else ''}")
    h = s["held_through_share"]["all"]
    print(f"  Tarots and Spectrals still held at a round's end: {h['through']} of {h['rounds']} rounds held "
          f"({100 * (h['share'] or 0):.1f}%); kinds at 5% or more: {', '.join(s['held_through_share']['over_bar']) or 'none'}")
    print(f"Summary: {a.out}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
