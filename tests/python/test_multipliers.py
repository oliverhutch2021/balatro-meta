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
