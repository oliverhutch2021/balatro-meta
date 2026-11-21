import math
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "scripts"))
import multipliers  # noqa: E402


def line(before, after, ask=10.0, cleared=True):
    rec = lambda p: {"power": p, "boss_power": p, "bench": [1, 1, 0.5, 0.5]}  # noqa: E731
    return {"ante": 5, "cleared": cleared, "ask": [9.0, 9.5, ask], "before": rec(before),
            "after": {k: rec(v) for k, v in after.items()}}


def test_the_table_reads_multipliers_the_weakest_third_and_the_lift_over_x1_8():
    lines = [line(10.0 + 0.2 * k, {"levels+3": 10.0 + 0.2 * k + math.log(2), "best-rare:Baron": 10.5 + 0.2 * k,
                                   "joker:Mime": 10.0 + 0.2 * k}) for k in range(6)]
    t = multipliers.summary(lines)["5"]
    assert t["builds"] == 6
    lv = t["changes"]["levels+3"]
    assert abs(lv["all"]["power_x"][1] - 2.0) < 1e-6 and lv["weakest_third"]["builds"] == 3
    assert lv["all"]["lifted_over_x1.8"] > 0, "x2 lifts the builds under x1.8 that it can"
    assert t["changes"]["best-rare"]["found"] == {"Baron": 6}, "searched jokers pooled by their prefix"
    assert "joker:Mime" in t["changes"], "each named joker its own row"


def test_jokers_as_offered_read_the_best_of_k_and_leave_out_the_short_lived():
    cands = {"before": 10.0, "common": [["Popcorn", 10.0 + math.log(3)], ["Joker", 10.0 + math.log(1.2)]],
             "uncommon": [["Ramen", 10.0 + math.log(4)]], "rare": [["Baron", 10.0 + math.log(2)]]}
    one = multipliers.best_of_k(cands, 1, lasting=False, draws=2000)
    many = multipliers.best_of_k(cands, 8, lasting=False, draws=2000)
    assert 1.2 < one < many <= 4.0
    assert multipliers.best_of_k(cands, 8, lasting=True, draws=2000) <= 2.0, "Popcorn and Ramen left out"
    x = line(10.0, {"levels+3": 10.5})
    x["candidates"] = cands
    off = multipliers.offered([x, x, x])
    assert off["all"]["best_common_lasting"][1] == 1.2 and off["all"]["best_uncommon_lasting"] is None
