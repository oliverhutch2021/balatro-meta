import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "scripts"))
import offers  # noqa: E402


def test_offered_affordable_and_taken_are_read_per_joker_and_shop():
    row1 = {"ante": 4, "money": 6, "full": True, "weakest": "Joker", "weakest_sale": 2, "jokers": ["Joker", "Egg"],
            "offers": [["Baron", 8, 1.9], ["Popcorn", 5, 1.2]]}
    row2 = dict(row1, money=1, offers=[["Popcorn", 5, 1.2]])
    shop = {"ante": 4, "rows": [row1, row2], "left_with": ["Popcorn", "Egg"], "money_left": 1}
    o = {x["kind"]: x for x in offers.offers_of(shop)}
    assert o["Baron"] == {"kind": "Baron", "x": 1.9, "affordable": True, "taken": False}
    assert o["Popcorn"]["taken"] and len(o) == 2, "a joker offered in two rows counted once"
    s = offers.summary([{"shops": [shop]}])
    assert s["4-5"]["shops_offering_x1.6"] == 1.0
    assert s["4-5"]["bands"]["1.6-2.0"] == {"offered": 1, "affordable": 1.0, "taken_of_affordable": 0.0}
