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
        "card_sharp": card_sharp(runs),
        "tarots_used_per_run": tarots_used(runs),
        "packs_per_run": packs(runs),
        "fool_copied": dict(total(runs, "fool_copied").most_common()),
    }


TAROTS = ("The Fool", "The Magician", "The High Priestess", "The Empress", "The Emperor", "The Hierophant",
          "The Lovers", "The Chariot", "Justice", "The Hermit", "The Wheel of Fortune",
          "Strength", "The Hanged Man", "Death", "Temperance", "The Devil", "The Tower", "The Star", "The Moon",
          "The Sun", "Judgement", "The World")


def tarots_used(runs: list[dict]) -> dict:
    """Tarots used a run (S, lab #30: a bar reported in every look): from the slots (`used`) and the moment they were
    taken (`used_at_once`: picked from an Arcana pack, or bought and used). Tallies written before 13 November have no
    `used_at_once`, so they count the slots alone."""
    def n(rs, keys=("used", "used_at_once")):
        return ratio(sum(v for r in rs for key in keys for k, v in r["tally"].get(key, {}).items() if k in TAROTS),
                     len(rs))
    return {"all": n(runs), "won": n([r for r in runs if r["won"]]), "lost": n([r for r in runs if not r["won"]]),
            "from_slots": n(runs, ("used",)), "at_once": n(runs, ("used_at_once",)),
            "counted_at_once": any("used_at_once" in r["tally"] for r in runs)}


def packs(runs: list[dict]) -> dict:
    """Packs a run by kind (S, lab #30): offered in a shop's first look, affordable then, opened; and opened by ante."""
    tot: Counter = Counter()
    for r in runs:
        tot.update(r["tally"].get("packs", {}))
    kinds = sorted({k.split(":")[1] for k in tot if k.split(":")[0] in ("offered", "opened")})
    out = {kd: {w: ratio(tot.get(f"{w}:{kd}", 0), len(runs)) for w in ("offered", "affordable", "opened")}
           for kd in kinds}
    for kd in kinds:
        out[kd]["opened_by_ante"] = {a: ratio(tot.get(f"opened:{kd}:a{a}", 0), len(runs)) for a in range(1, 9)}
    out["shop_consumables_bought"] = {k.split(":")[1]: ratio(v, len(runs)) for k, v in tot.items()
                                      if k.startswith("bought:")}
    return out


def card_sharp(runs: list[dict]) -> dict:
    """Card Sharp held (S, lab #30): the rounds in which its x3 fired, and the hands it fired on against the hands
    played while it was held."""
    held = [r["tally"].get("card_sharp", {}) for r in runs if r["tally"].get("card_sharp")]
    tot: Counter = Counter()
    for c in held:
        tot.update(c)
    fired = {k[6:]: v for k, v in tot.items() if k.startswith("fired:")}
    plays = {k[6:]: v for k, v in tot.items() if k.startswith("plays:")}
    return {
        "runs_holding": len(held),
        "rounds_held": tot.get("rounds_held", 0),
        "rounds_fired": tot.get("rounds_fired", 0),
        "rounds_fired_share": ratio(tot.get("rounds_fired", 0), tot.get("rounds_held", 0)),
        "plays_held": tot.get("plays_held", 0),
        "by_hand": {h: {"played": plays[h], "fired": fired.get(h, 0), "fired_share": ratio(fired.get(h, 0), plays[h])}
                    for h in sorted(plays, key=lambda h: -plays[h])},
    }


SUIT_TAROTS = ("The Star", "The Moon", "The Sun", "The World")
# The user's named holds (#19): The Fool until the last card used is worth copying; The Hermit below $20.
HELD_EXCEPTIONS = ("The Fool", "The Hermit")


def bars(s: dict) -> list[dict]:
    """The master list's bars (lab #27) read from a summary: each {id, what, value, bar, pass} (pass None when the
    tally doesn't count it, or there were too few chances to read)."""
    e, r = s["seal_events"], s["rounds"]
    out = []

    def add(id_, what, value, bar, ok):
        out.append({"id": id_, "what": what, "value": value, "bar": bar, "pass": ok})

    held = s["held_through_share"]
    over = [k for k in held["over_bar"] if k not in HELD_EXCEPTIONS and held["kinds"][k]["rounds"] >= 10]
    add("C1", "Tarot/Spectral kinds still held at a round's end at 5% or more (10+ rounds held)",
        ", ".join(over) or "none", "none, or named", not over)
    suits = [held["kinds"][k]["share"] for k in SUIT_TAROTS if k in held["kinds"] and held["kinds"][k]["rounds"] >= 10]
    spread = round(max(suits) - min(suits), 3) if len(suits) >= 2 else None
    add("C2", "spread of the suit Tarots' held-through shares", spread, "under 0.10",
        None if spread is None else spread < 0.10)
    seal = s.get("seal_spectral_targets", {})
    uses = sum(v["uses"] for v in seal.values())
    top = sum(v["rank0"] for v in seal.values())
    add("C5", "seal Spectrals on the most valuable card", ratio(top, uses), "0.90 or more",
        None if uses < 10 else top / uses >= 0.90)
    blue = e.get("blue_drawn", 0)
    add("P1", "Blue Seals thrown while digging, per draw", ratio(e.get("blue_thrown_digging", 0), blue), "under 0.05",
        None if blue < 20 else e.get("blue_thrown_digging", 0) / blue < 0.05)
    add("P2", "a Planet per Blue Seal draw", ratio(e.get("blue_planet_made", 0), blue), "0.70 or more",
        None if blue < 20 else e.get("blue_planet_made", 0) / blue >= 0.70)
    purple = e.get("purple_drawn", 0)
    add("P3a", "a Tarot per Purple Seal draw", ratio(e.get("purple_tarot_made", 0), purple), "0.80 or more",
        None if purple < 20 else e.get("purple_tarot_made", 0) / purple >= 0.80)
    add("P3b", "Purple Seals played with discards left, per draw",
        ratio(e.get("purple_played_with_discards", 0), purple), "under 0.05",
        None if purple < 20 else e.get("purple_played_with_discards", 0) / purple < 0.05)
    missed = e.get("purple_tarot_missed", 0) + e.get("blue_planet_missed", 0)
    free = e.get("purple_played_with_discards_slot_free", 0)
    add("P3c", "Purple Seals played with discards left and a slot free, per draw", ratio(free, purple), "under 0.05",
        None if purple < 20 else free / purple < 0.05)
    wins_in_hand = r.get("with_winning_hand", 0)
    add("B1", "rounds lost after a winning hand was in hand, per such round",
        ratio(r.get("lost_after_winning_hand", 0), wins_in_hand), "not above all-off's (the night's gate: under 2x)",
        None)
    add("P4", "cards not made for want of a slot, per seal draw", ratio(missed, blue + purple), "under 0.005",
        None if blue + purple < 20 else missed / (blue + purple) < 0.005)
    add("P6a", "Two Pair's share of hands", s["two_pair_share"], "under 0.05",
        None if s["two_pair_share"] is None else s["two_pair_share"] < 0.05)
    add("P6b", "the main hand's share of a run's hands (median)", s["main_hand_share_median"], "over 0.60",
        None if s["main_hand_share_median"] is None else s["main_hand_share_median"] > 0.60)
    add("S7", "runs that rerolled a boss", s.get("runs_with_a_boss_reroll"), "counted", None)
    add("slots", "rounds ended with every consumable slot full, per round begun",
        ratio(r.get("ended_slots_full", 0), r.get("begun", 0)), "falling", None)
    return out


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("lines", type=Path)
    ap.add_argument("--out", type=Path, required=True)
    a = ap.parse_args(argv)
    s = {"lines": a.lines.name, **summarise(read_lines(a.lines))}
    s["bars"] = bars(s)
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
    cs = s["card_sharp"]
    if cs["runs_holding"]:
        print(f"  Card Sharp: held in {cs['runs_holding']} runs, its x3 fired in {cs['rounds_fired']} of "
              f"{cs['rounds_held']} rounds played with it ({100 * (cs['rounds_fired_share'] or 0):.1f}%); by hand: "
              + ", ".join(f"{h} {d['fired']}/{d['played']}" for h, d in list(cs["by_hand"].items())[:5]))
    t = s["tarots_used_per_run"]
    print(f"  Tarots used a run: {t['all']} (wins {t['won']}, losses {t['lost']}; from the slots {t['from_slots']}, "
          f"at once {t['at_once'] if t['counted_at_once'] else 'not counted in this tally'})")
    if s["fool_copied"]:
        print("  The Fool made: " + ", ".join(f"{k} {v}" for k, v in s["fool_copied"].items()))
    pk = {k: v for k, v in s["packs_per_run"].items() if k != "shop_consumables_bought"}
    if pk:
        print("  Packs a run (offered / affordable / opened): "
              + "; ".join(f"{k} {v['offered']} / {v['affordable']} / {v['opened']}" for k, v in pk.items()))
    print("Bars (lab #27):")
    for b in s["bars"]:
        mark = {True: "PASS", False: "FAIL", None: "  - "}[b["pass"]]
        print(f"  {mark}  {b['id']:5s} {b['what']}: {b['value']} (bar: {b['bar']})")
    print(f"Summary: {a.out}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
