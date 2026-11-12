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


def test_the_forced_forks_summary_pairs_each_fork_and_reads_the_rule(tmp_path):
    lines = []
    for i in range(40):
        own = [[0, 20, 0, 0], [0, 20, 0, 0]]
        forced = [[1, 24, 1, 1], [0, 21, 1, 0]] if i % 2 == 0 else [[0, 20, 1, 0], [0, 20, 0, 0]]
        lines.append({"run": i, "joker": "Campfire", "ante": 3 + i % 5, "own_ends": own, "forced_ends": forced,
                      "sold": None})
    lines.append({"run": 99, "joker": "Campfire", "ante": 7, "skipped": "can't be made"})
    p = tmp_path / "f.jsonl"
    p.write_text("\n".join(json.dumps(x) for x in lines) + "\n")
    out = tmp_path / "s.json"
    assert forced_buys.main([str(p), "--out", str(out)]) == 0
    g = json.loads(out.read_text())["jokers"]["Campfire"]
    assert g["forks"] == 40 and g["skipped"] == 1
    assert abs(g["win_rate_diff"] - 0.25) < 1e-9 and g["win_rate_diff_ci"][0] > 0.03
    assert g["arithmetic"].startswith("A:")
    assert g["forced_kept_as_shop_closed"] == 0.75
    assert set(g["by_ante"]) <= {"2-4", "5-6", "7-8"}
