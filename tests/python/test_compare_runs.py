"""Comparing two bots run by run on the same seeds (scripts/compare_runs.py)."""
from __future__ import annotations

import json
import math
import sys
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))

import compare_runs as cr


def report(stages: str, **extra) -> dict:
    return {"kind": "baseline", "bot": "teacher bot", "deck": "Red Deck", "stake": "White Stake",
            "run_stages": stages, **extra}


def test_runs_pair_by_seed_and_the_split_of_wins_is_tested():
    # Seeds 1-6: both win 1, only B wins 2 and 3, only A wins 4; 5 and 6 both lose (B further on 6).
    a = report("YADYAA")
    b = report("YYYCAX")
    c = cr.compare(a, b)
    assert c["seeds"] == 6
    assert (c["a"]["wins"], c["b"]["wins"]) == (2, 3)
    assert (c["only_a_won"], c["only_b_won"]) == (1, 2)
    assert c["win_rate_diff"] == pytest.approx(1 / 6)
    # Per-seed differences 0, +1, +1, -1, 0, 0: mean 1/6, variance 3/6 - 1/36.
    se = math.sqrt((3 / 6 - 1 / 36) / 6)
    assert c["win_rate_diff_ci"] == pytest.approx([1 / 6 - 1.96 * se, 1 / 6 + 1.96 * se])
    assert c["p_value"] == pytest.approx(1.0), "2 to 1 is no evidence"
    # Blinds beaten: A 24, 0, 3, 24, 0, 0; B 24, 24, 24, 2, 0, 23.
    assert c["a"]["blinds_beaten"] == pytest.approx(51 / 6)
    assert c["b"]["blinds_beaten"] == pytest.approx(97 / 6)
    assert c["blinds_beaten_diff"] == pytest.approx(46 / 6)


def test_the_shorter_report_sets_the_seeds_compared():
    c = cr.compare(report("YYYY"), report("AA"))
    assert c["seeds"] == 2 and c["a"]["wins"] == 2 and c["b"]["wins"] == 0


def test_the_sign_test_takes_many_split_runs():
    # 8,000 seeds on the PC split 724 against 855 (make teacher-big): past 1,023 a float power of two overflows.
    p = cr.sign_test(855, 1579)
    assert 0.0005 < p < 0.002
    assert cr.sign_test(5000, 10000) == pytest.approx(1.0, abs=0.02)


def test_the_sign_test_is_exact():
    assert cr.sign_test(0, 0) == 1.0
    assert cr.sign_test(10, 10) == pytest.approx(2 / 1024)
    assert cr.sign_test(0, 10) == pytest.approx(2 / 1024)
    assert cr.sign_test(5, 10) == 1.0
    # 60 of 100: P(X <= 40) for Binomial(100, 1/2), both tails.
    tail = sum(math.comb(100, i) for i in range(41)) / 2 ** 100
    assert cr.sign_test(60, 100) == pytest.approx(2 * tail)


def test_reports_that_do_not_pair_are_refused():
    with pytest.raises(ValueError, match="seed_base"):
        cr.compare(report("YA", seed_base=1), report("YA", seed_base=2))
    with pytest.raises(ValueError, match="run_stages"):
        cr.compare(report(""), report("YA"))
    with pytest.raises(ValueError, match="unknown letters"):
        cr.compare(report("YZ"), report("YA"))


def test_sign_test_matches_the_binomial_definition_including_ties_and_extreme_tails():
    for n in range(65):
        for k in range(n + 1):
            expected = min(1.0, 2 * sum(math.comb(n, i) for i in range(min(k, n - k) + 1)) / 2 ** n)
            assert cr.sign_test(k, n) == expected


def test_the_command_writes_the_comparison(tmp_path, capsys):
    pa, pb, out = tmp_path / "plain.json", tmp_path / "steady.json", tmp_path / "vs" / "c.json"
    pa.write_text(json.dumps(report("AYAY" * 50)))
    pb.write_text(json.dumps(report("YYAY" * 50)))
    assert cr.main([str(pa), str(pb), "--out", str(out)]) == 0
    c = json.loads(out.read_text())
    assert c["seeds"] == 200 and c["only_b_won"] == 50 and c["only_a_won"] == 0
    assert c["p_value"] < 1e-10
    assert c["b"]["report"] == str(pb)
    text = capsys.readouterr().out
    assert "+25.00 points of win rate" in text and "steady" in text
    assert cr.main([str(pa), str(tmp_path / "missing.json")]) == 1
