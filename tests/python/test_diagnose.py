"""scripts/diagnose.py: deeper play's share of lost blinds beaten, and set against the control (make diagnose-control)."""
from __future__ import annotations

import csv
import json
import sys
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))

import diagnose

TABLE = ROOT / "results" / "diagnose" / "champion.csv"


def with_share(path: Path, share) -> Path:
    """The committed table with a deep_share column, as baseline --deep-tries writes it."""
    rows = list(csv.DictReader(TABLE.open()))
    fields = list(rows[0])
    fields.insert(fields.index("deep_won"), "deep_share")
    with path.open("w", newline="") as f:
        w = csv.DictWriter(f, fieldnames=fields)
        w.writeheader()
        for r in rows:
            won = float(r["deep_won"])
            w.writerow({**r, "deep_share": -1 if won < 0 else share(r)})
    return path


def test_a_table_of_one_try_reads_as_before_and_tries_are_averaged(tmp_path):
    before = diagnose.summarise(diagnose.load(TABLE))["deeper_play"]
    one = diagnose.summarise(diagnose.load(with_share(tmp_path / "one.csv", lambda r: r["deep_won"])))["deeper_play"]
    assert one["beaten"] == before["beaten"] and one["by_boss"] == before["by_boss"]
    half = diagnose.summarise(diagnose.load(with_share(tmp_path / "half.csv", lambda r: 0.5)))["deeper_play"]
    assert half["beaten"] == 0.5, "each blind's share of tries beaten, averaged"
    assert sum(half["by_boss_n"].values()) <= half["replayed"]


def test_deeper_play_over_the_control_is_the_difference_and_its_win_rate_at_stake(tmp_path):
    deep = diagnose.summarise(diagnose.load(TABLE))
    control = diagnose.summarise(diagnose.load(with_share(tmp_path / "c.csv", lambda r: 0.1)))
    control = json.loads(json.dumps(control))  # as the job reads it back: antes as text
    o = diagnose.over_control(deep, control)
    assert set(o["by_ante"]) == {str(a) for a in deep["deeper_play"]["by_ante"]}
    assert o["control_beaten"] == 0.1
    assert o["beaten"] == pytest.approx(deep["deeper_play"]["beaten"] - 0.1, abs=1e-4)
    loss_rate = deep["losses"] / deep["runs"]
    assert o["points_at_stake"] == pytest.approx(100 * o["beaten"] * loss_rate, abs=0.01)
    mouth = deep["by_boss"]["The Mouth"] * (deep["deeper_play"]["by_boss"]["The Mouth"] - 0.1) * loss_rate * 100
    assert o["points_at_stake_by_boss"]["The Mouth"] == pytest.approx(mouth, abs=0.01)
    g = o["play_bosses"]
    assert g["bosses"] == list(diagnose.PLAY_BOSSES) and g["control_beaten"] == 0.1
    assert g["points_at_stake"] == pytest.approx(sum(o["points_at_stake_by_boss"][b] for b in diagnose.PLAY_BOSSES),
                                                 abs=0.01)
    assert any("together" in line for line in diagnose.control_lines(o))
    other = dict(control, runs=control["runs"] - 1)
    with pytest.raises(SystemExit):
        diagnose.over_control(deep, other)
