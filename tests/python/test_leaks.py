"""The leak tally's and the forced-buy forks' summaries (scripts/leaks.py, scripts/forced_buys.py)."""
from __future__ import annotations

import gzip
import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))

import forced_buys  # noqa: E402
import leaks  # noqa: E402


def tally(blue=0, made=0, room=0, at_once=0, held=(), through=(), **extra):
    t = {"seals": {"blue_drawn": blue, "blue_planet_made": made},
         "rounds": {"begun": 3, "won_with_room": room, "won_with_room_at_once": at_once},
         "held_at_start": {k: 1 for k in held}, "held_through": {k: 1 for k in through},
         "hands": {"Pair": 6, "Two Pair": 2}, "targets": [["Talisman", [0], [0], 8], ["Talisman", [2], [1], 8]],
         "shops": {"left": 3}, "last_shop": {"money": 30, "bull_or_bootstraps": False},
         "jokers": [[2, "Joker", "buy"]], "vouchers_offered": {"Overstock": 1}, "vouchers_affordable": {"Overstock": 1},
         "vouchers_redeemed": {}}
    t.update(extra)
    return t


def test_a_leak_present_often_and_rarely_collected_is_flagged_by_the_rule(tmp_path):
    runs = [{"won": i % 2 == 0, "stage": 24, "tally": tally(blue=2 if i < 3 else 0, made=0, room=4, at_once=1,
                                                            held=["The High Priestess"], through=["The High Priestess"])}
            for i in range(10)]
    p = tmp_path / "l.jsonl.gz"
    with gzip.open(p, "wt") as fh:
        fh.write("\n".join(json.dumps(r) for r in runs) + "\n")
    out = tmp_path / "s.json"
    assert leaks.main([str(p), "--out", str(out)]) == 0
    s = json.loads(out.read_text())
    blue = s["leaks"]["blue_seal_planet"]
    assert blue["present_share"] == 0.3 and blue["collected_share"] == 0.0 and blue["fix_by_rule"]
    room = s["leaks"]["won_round_room_used"]
    assert room["collected_share"] == 0.75 and not room["fix_by_rule"]
    assert s["leaks"]["held_consumable_used"]["fix_by_rule"]
    assert s["two_pair_share"] == 0.25
    assert s["seal_spectral_targets"]["Talisman"]["share_on_most_valuable"] == 0.5
    assert s["vouchers"]["Overstock"]["redeemed_when_affordable"] == 0.0
    held = s["held_through_share"]
    assert held["kinds"]["The High Priestess"] == {"rounds": 10, "through": 10, "share": 1.0}
    assert held["over_bar"] == ["The High Priestess"]


def test_the_forced_forks_summary_pairs_each_fork_and_reads_the_rule(tmp_path):
    lines = []
    for i in range(400):
        own = [[0, 20, 0, 0], [0, 20, 0, 0]]
        forced = [[1, 24, 1, 1], [0, 21, 1, 0]] if i % 2 == 0 else [[0, 20, 1, 0], [0, 20, 0, 0]]
        lines.append({"run": i, "joker": "Campfire", "ante": 3 + i % 5, "own_ends": own, "held_ends": forced,
                      "forced_ends": own, "sold": None})
    # A rare joker: a handful of forks reads as unmeasured, not as showing nothing.
    for i in range(5):
        lines.append({"run": i, "joker": "Blueprint", "ante": 6, "own_ends": [[0, 20, 0, 0]],
                      "held_ends": [[i % 2, 22, 1, 1]], "forced_ends": [[0, 20, 0, 0]], "sold": None})
    lines.append({"run": 99, "joker": "Campfire", "ante": 7, "skipped": "can't be made"})
    p = tmp_path / "f.jsonl"
    p.write_text("\n".join(json.dumps(x) for x in lines) + "\n")
    out = tmp_path / "s.json"
    assert forced_buys.main([str(p), "--out", str(out)]) == 0
    g = json.loads(out.read_text())["jokers"]["Campfire"]
    assert g["forks"] == 400 and g["skipped"] == 1
    assert abs(g["win_rate_diff"] - 0.25) < 1e-9 and g["win_rate_diff_ci"][0] > 0.03
    assert g["arithmetic"].startswith("A:")
    assert g["forced_kept_as_shop_closed"] == 0.75
    assert set(g["by_ante"]) <= {"2-4", "5-6", "7-8"}
    assert g["as_is"]["win_rate_diff"] == 0.0, "the as-is line is summarised beside the held one"
    assert json.loads(out.read_text())["jokers"]["Blueprint"]["arithmetic"].startswith("unmeasured")


def test_tarots_used_count_the_slots_and_the_ones_used_at_once_and_packs_are_summed_by_kind():
    runs = [{"won": True, "tally": {"used": {"Death": 1, "Mercury": 2}, "used_at_once": {"The Hermit": 2, "Pluto": 1},
                                    "packs": {"offered:Arcana": 4, "affordable:Arcana": 3, "opened:Arcana": 2,
                                              "opened:Arcana:a2": 2, "bought:Tarot": 1}}},
            {"won": False, "tally": {"used": {"The Fool": 1}}}]
    t = leaks.tarots_used(runs)
    assert (t["all"], t["from_slots"], t["at_once"], t["won"], t["lost"]) == (2.0, 1.0, 1.0, 3.0, 1.0)
    p = leaks.packs(runs)
    assert p["Arcana"]["opened"] == 1.0 and p["Arcana"]["opened_by_ante"][2] == 1.0
    assert p["shop_consumables_bought"]["Tarot"] == 0.5


def test_the_pack_audit_reads_each_pack_in_the_values_dollars_and_what_beat_an_arcana_pack():
    import pack_audit
    line = {"run": 1, "step": 10, "ante": 4, "behind": 1, "money": 9, "packs": [[0, "Arcana", 4], [1, "Buffoon", 4]],
            "chosen": "Buy 0",
            "notes": [json.dumps({"quick": {"here": 1.0, "here_plus4": 1.4,
                                            "options": [["OpenBooster 0", 1.5], ["OpenBooster 1", 0.9], ["Buy 0", 2.0]]}})]}
    s = pack_audit.summarise([line])
    assert s["by_kind_and_ante"]["Arcana"][4]["median_dollars"] == 5.0
    assert s["by_kind_and_ante"]["Buffoon"][4]["median_dollars"] == -1.0
    assert s["when_arcana_beats_leaving"] == {"arcana_above_leaving": 1, "another_move_valued_higher": 1}
    # The same shop's last decision: the money spent, the Arcana pack no longer affordable.
    end = dict(line, step=11, money=2, chosen="LeaveShop 0")
    assert pack_audit.by_shop([line, end]) == {"shops": 1, "money_ran_out": 1}
    assert pack_audit.by_shop([line, dict(end, money=9)]) == {"shops": 1, "left_affordable": 1}


def test_the_pack_audit_groups_shops_by_key_when_threads_interleave_their_lines():
    import pack_audit

    def at(run, step, money, chosen, behind=0):
        return {"run": run, "step": step, "ante": 2, "behind": behind, "money": money, "chosen": chosen,
                "packs": [[0, "Arcana", 4]], "notes": []}
    lines = [at(1, 10, 9, "Buy 0"), at(2, 10, 9, "OpenBooster 0"), at(1, 11, 9, "LeaveShop 0"),
             at(2, 12, 5, "LeaveShop 0"), at(1, 30, 3, "LeaveShop 0", behind=1)]
    assert pack_audit.by_shop(lines) == {"shops": 2, "opened": 1, "left_affordable": 1}


def test_the_pack_audit_splits_an_arcana_packs_worth_into_the_formulas_part_and_the_networks():
    import pack_audit
    line = {"run": 1, "step": 3, "ante": 3, "behind": 0, "money": 9, "packs": [[0, "Arcana", 4]], "chosen": "LeaveShop 0",
            "notes": [json.dumps({"quick": {"here": 1.0, "here_formula": 0.6, "here_plus4": 1.2,
                                            "options": [["OpenBooster 0", 0.9, 0.7]]}})]}
    s = pack_audit.split([line])
    # formula +0.1 (0.7 - 0.6); networks -0.2 (the option's -0.1 overall less the formula's +0.1)
    assert s[3]["formula_median"] == 0.1 and s[3]["networks_median"] == -0.2


def test_the_bench_check_ranks_and_bands_the_antes_entered():
    import bench_check
    rows = [{"won": True, "entered": [{"ante": 5, "score": [1, 1, 0.9, 0.9], "cleared": True}],
             "final": {"5": [1, 1, 1, 1.0]}},
            {"won": False, "entered": [{"ante": 5, "score": [1, 0.5, 0.2, 0.1], "cleared": False}],
             "final": {"5": [0, 0, 0, 0.0]}}]
    s = bench_check.summarise(rows)
    assert s["all"]["auc"] == 1.0 and s["by_ante"][5]["n"] == 2
    assert s["all"]["by_band"]["0.9-1.0"]["cleared"] == 1.0 and s["all"]["by_band"]["0.1-0.3"]["cleared"] == 0.0
    assert s["final_builds"]["5"]["won_median"] == 1.0


def test_the_danger_check_counts_kills_per_appearance_by_boss():
    import danger_check
    runs = [{"won": False, "rounds": [{"ante": 4, "at_start": {"boss": "The Wall"}}]},
            {"won": True, "rounds": [{"ante": 4, "at_start": {"boss": "The Wall"}}, {"ante": 5, "at_start": {}}]}]
    b = danger_check.bosses(runs)
    assert b["The Wall"] == {"faced": 2, "killed": 1, "kill_rate": 0.5}
