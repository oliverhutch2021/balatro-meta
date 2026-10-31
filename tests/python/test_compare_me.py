"""Setting a person's recorded decisions against the teacher's (scripts/compare_me.py, examples/judge.rs)."""
from __future__ import annotations

import json
import shutil
import sys
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))
sys.path.insert(0, str(ROOT / "tests" / "python"))

import compare_me as cm


def decision(phase="SHOP", agree=False, player_kind=3, bot_kind=11, pv=1.0, bv=1.5, measure="value", **kw):
    return {"run": "r1", "seed": "AAAA1111", "step": 3, "ante": 2, "round": 5, "phase": phase, "money": 9,
            "player": "Leave the shop", "player_kind": player_kind, "bot": "Reroll the shop", "bot_kind": bot_kind,
            "agree": agree, "measure": measure, "player_value": pv, "bot_value": bv, **kw}


def test_the_judge_reads_each_run_as_its_steps_of_moves():
    runs = [{"name": "r1", "seed": "AAAA1111", "steps": [[(8, 513), (6, 3)], [(2, 0)]]},
            {"name": "r2", "seed": "BBBB2222", "steps": [[(0, 0)]]}]
    assert cm.judge_input(runs) == "r1\tAAAA1111\t8:513,6:3|2:0\nr2\tBBBB2222\t0:0\n"


def test_the_report_counts_agreement_swaps_and_the_largest_gaps():
    ds = [decision(agree=True, pv=2.0, bv=2.0), decision(), decision(pv=1.4, bv=1.1),
          decision(phase="SELECTING_HAND", player_kind=7, bot_kind=6, measure="beat", pv=0.5, bv=0.9,
                   player="Discard H_A", bot="Play S_K")]
    runs = [{"name": "r1", "ok": True, "lost": None, "states": 40, "ante": 3, "won": False, "over": True}]
    text = cm.report(ds, runs)
    assert "1 runs (0 won), 4 decisions judged." in text
    assert "shop" in text and "1 of    3 (33%)" in text
    assert "2x in the shop: you leave shop, the bot would reroll the shop (mean gap +0.100)" in text
    assert "+0.400  ante 2 round" in text and "you Discard H_A; bot Play S_K" in text
    assert "-0.300  ante 2 shop" in text, "a choice the bot rates above its own is listed too"


def test_an_agreed_decision_has_no_gap():
    assert cm.gap(decision(agree=True, pv=1.0, bv=3.0)) == 0.0
    assert cm.gap(decision(pv=None)) is None


@pytest.mark.skipif(not shutil.which("cargo"), reason="cargo is not installed")
def test_recorded_runs_are_judged_decision_by_decision(tmp_path, monkeypatch):
    pytest.importorskip("balatro_meta._engine")
    import record_me
    import test_record_me as tr

    runs = tmp_path / "runs"
    monkeypatch.setattr(record_me, "RUNS", runs)
    monkeypatch.setattr(record_me, "START_DELAY", 0.0)
    monkeypatch.setattr(cm, "ME", tmp_path)
    monkeypatch.setattr(cm, "best_teacher", lambda root=None: [])
    fake = tr.FakeGame(21)
    rec = record_me.Recorder(4, api=fake, files=tr.NoMod(), game=None, say=lambda s: None)
    for _ in range(20000):
        rec.step()
        fake.person()
        if fake.finished and fake.game is None:
            break
    rec.close()
    assert cm.main([str(p) for p in runs.glob("*.jsonl.gz")]) == 0
    judged = [json.loads(x) for x in (tmp_path / "judged.jsonl").read_text().splitlines()]
    assert judged and all(d["phase"] != "GAME_OVER" for d in judged)
    assert {d["phase"] for d in judged} >= {"BLIND_SELECT", "SELECTING_HAND"}
    # Every decision the person made is there, once, in order.
    steps = [d["step"] for d in judged]
    assert steps == sorted(set(steps))
    # Cash-outs have one choice only, so the bot agrees with them all.
    assert all(d["agree"] for d in judged if d["phase"] == "ROUND_EVAL")
    assert all(d["measure"] == ("beat" if d["phase"] == "SELECTING_HAND" else "value") for d in judged)
    assert "decisions judged" in (tmp_path / "report.txt").read_text()
