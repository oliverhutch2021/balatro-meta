"""The baseline job (crates/bm-core/examples/baseline.rs): a short run on two threads, then its reports read the
way the viewer reads them, and its best run replayed with the engine to the very end it recorded.
"""
from __future__ import annotations

import json
import os
import shutil
import subprocess
import sys
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))

from balatro_meta._engine import Game

import viewer


@pytest.fixture(scope="module")
def job(tmp_path_factory):
    if not shutil.which("cargo"):
        pytest.skip("cargo is not installed")
    tmp = tmp_path_factory.mktemp("baseline")
    watch = tmp / "watch" / "baseline"
    out = tmp / "greedy.json"
    r = subprocess.run(["cargo", "run", "--release", "-q", "-p", "bm-core", "--example", "baseline", "--",
                        "--runs", "400", "--threads", "2", "--seed", "7", "--out", str(out)],
                       cwd=ROOT, capture_output=True, text=True, check=False, timeout=900,
                       env={**os.environ, "BM_WATCH_DIR": str(watch)})
    assert r.returncode == 0, r.stdout + r.stderr
    return tmp, watch, json.loads(out.read_text()), r.stdout


def test_the_job_reports_its_numbers(job):
    _tmp, watch, final, stdout = job
    prog = json.loads((watch / "progress.json").read_text())
    assert prog["done"] and prog["runs"] == final["runs"] == 400 and prog["refused_moves"] == 0
    assert sum(prog["ante_hist"]) == 400 and len(prog["ante_hist"]) == 9
    assert prog["wins"] == prog["ante_hist"][8] and 0 <= prog["win_ci"][0] <= prog["win_rate"] <= prog["win_ci"][1]
    assert 1 <= prog["mean_ante"] <= 9 and prog["rate"] > 0
    assert 0 < sum(n for _, n in prog["lost_to"]) <= 400 - prog["wins"] and len(prog["lost_to"]) <= 12  # the top 12
    assert prog["hands_played"] and prog["jokers_bought"] and prog["per_run"]["moves"] > 10
    assert len(final["best_runs"]) == 30 and [r[0] for r in final["best_runs"]] == list(range(1, 31))
    assert "400 runs" in stdout
    # What the runs were played with, and each run's ending in seed order (scripts/compare_runs.py pairs them).
    assert final["seed_base"] == 7 and final["value_net"] is None and final["value_weight"] is None
    assert final["teacher_options"] == []
    stages = final["run_stages"]
    assert len(stages) == 400 and set(stages) <= set("ABCDEFGHIJKLMNOPQRSTUVWXY")
    assert stages.count("Y") == final["wins"]


def test_the_best_runs_are_kept_in_order(job):
    tmp, watch, final, _ = job
    best = sorted((watch / "samples").glob("best-*.json"))
    assert len(best) == 30
    jobs = viewer.JobFiles(tmp / "watch", tmp / "logs")
    ranked = jobs.best()
    assert len(ranked) == 30
    seeds = [jobs.read_sample(p)["seed"] for _, p in ranked]
    assert seeds == [r[1] for r in final["best_runs"]], "the viewer's order is the job's order"
    keys = [k for k, _ in ranked]
    assert keys == sorted(keys, reverse=True)


def replay(sample: dict) -> dict:
    """The sample's moves through the engine: how the run ends and its biggest hand."""
    g = Game(sample["seed"])
    best = 0.0
    for k, a in sample["moves"]:
        before = g.view()["chips"]
        assert g.step(k, a), (k, a)
        if k == 6:
            after = g.view()["chips"]
            best = max(best, after - before if after >= before else after)
    v = g.view()
    return {"won": v["won"], "ante": 9 if v["won"] else v["ante"], "best_hand": best, "over": g.over}


def test_a_saved_run_replays_to_the_end_the_job_recorded(job):
    tmp, watch, _final, _ = job
    jobs = viewer.JobFiles(tmp / "watch", tmp / "logs")
    for _, p in jobs.best()[:3]:
        s = jobs.read_sample(p)
        got = replay(s)
        assert got["over"] and got["won"] == s["won"] and got["ante"] == s["ante"], s["seed"]
        assert got["best_hand"] == pytest.approx(s["best_hand"], rel=1e-9), s["seed"]
    typical = sorted((watch / "samples").glob("typical-*.json"))
    for p in typical[:2]:
        s = json.loads(p.read_text())
        assert replay(s)["ante"] == s["ante"]


def test_the_teacher_bot_runs_in_the_job_and_its_runs_replay(tmp_path):
    """The teacher (sells, rerolls, Tarots with targets, vouchers, joker order) in the same job: every move
    legal, and its best runs replay with the engine to the end it recorded."""
    if not shutil.which("cargo"):
        pytest.skip("cargo is not installed")
    watch = tmp_path / "watch" / "teacher"
    out = tmp_path / "teacher.json"
    r = subprocess.run(["cargo", "run", "--release", "-q", "-p", "bm-core", "--example", "baseline", "--",
                        "--bot", "teacher", "--runs", "60", "--threads", "2", "--seed", "9", "--out", str(out)],
                       cwd=ROOT, capture_output=True, text=True, check=False, timeout=900,
                       env={**os.environ, "BM_WATCH_DIR": str(watch)})
    assert r.returncode == 0, r.stdout + r.stderr
    final = json.loads(out.read_text())
    assert final["bot"] == "teacher bot" and final["runs"] == 60 and final["refused_moves"] == 0
    assert final["per_run"]["rerolls"] > 0 and final["per_run"]["vouchers"] > 0
    jobs = viewer.JobFiles(tmp_path / "watch", tmp_path / "logs")
    best = jobs.best()
    assert best and jobs.read_sample(best[0][1])["bot"] == "teacher bot"
    for _, p in best[:3]:
        s = jobs.read_sample(p)
        got = replay(s)
        assert got["over"] and got["won"] == s["won"] and got["ante"] == s["ante"], s["seed"]


def test_the_leak_tally_and_the_forced_forks_leave_the_runs_as_they_were(tmp_path):
    """--leaks and --force-buy only watch: the runs end exactly as without them (the forks run after each run), one
    tally line per run, and each fork line pairs the run's own move with taking the named joker on the same copies."""
    if not shutil.which("cargo"):
        pytest.skip("cargo is not installed")
    base = ["cargo", "run", "--release", "-q", "-p", "bm-core", "--example", "baseline", "--", "--bot", "teacher",
            "--teacher-options", "light", "--runs", "8", "--threads", "2", "--seed", "1", "--salt", "3"]
    plain, watched = tmp_path / "plain.json", tmp_path / "watched.json"
    leaks, forced = tmp_path / "leaks.jsonl", tmp_path / "forced.jsonl"
    for out, extra in ((plain, []), (watched, ["--leaks", str(leaks), "--force-buy",
                                                "Abstract Joker@1-8,Supernova@1-8,Photograph@1-8",
                                                "--force-out", str(forced), "--force-copies", "1",
                                                "--fork-play-options", "light"])):
        r = subprocess.run([*base, *extra, "--out", str(out)], cwd=ROOT, capture_output=True, text=True,
                           check=False, timeout=900)
        assert r.returncode == 0, r.stdout + r.stderr
    assert json.loads(watched.read_text())["run_stages"] == json.loads(plain.read_text())["run_stages"]
    lines = [json.loads(x) for x in leaks.read_text().splitlines()]
    assert sorted(x["run"] for x in lines) == list(range(8))
    assert all(x["tally"]["rounds"]["begun"] >= 1 for x in lines)
    assert any(x["tally"]["ways"] for x in lines), "played live, finish_choice's ways are noted"
    for f in (json.loads(x) for x in forced.read_text().splitlines()):
        assert f["joker"] in ("Abstract Joker", "Supernova", "Photograph") and f["passes"] >= 1
        if "skipped" not in f:
            assert len(f["own_ends"]) == len(f["forced_ends"]) == 1
            assert f["forced"][-1].split()[0] in ("Buy", "PackChoose")
