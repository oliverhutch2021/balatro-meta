"""The viewer (scripts/viewer.py and viewer/index.html): it replays runs a job saved, best ones first, with the
same engine, serves them and the job's numbers to the page, and the page draws the run.
"""
from __future__ import annotations

import json
import sys
import threading
import time
import urllib.error
import urllib.request
from http.server import ThreadingHTTPServer
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))

from balatro_meta._engine import Game

import viewer


def greedy_run(seed: str, limit: int = 5000) -> tuple[list[list[int]], dict]:
    """A run by the greedy bot: its moves and its last view."""
    g = Game(seed)
    moves = []
    while not g.over and len(moves) < limit:
        k, a = g.greedy_move()
        assert g.step(k, a)
        moves.append([k, a])
    return moves, g.view()


def save(folder: Path, name: str, **sample) -> Path:
    folder.mkdir(parents=True, exist_ok=True)
    p = folder / name
    p.write_text(json.dumps(sample))
    return p


def jobs_at(tmp_path: Path) -> viewer.JobFiles:
    return viewer.JobFiles(tmp_path / "watch", tmp_path / "logs")


def test_best_runs_come_first_then_start_again(tmp_path):
    samples = tmp_path / "watch" / "baseline" / "samples"
    mv = [[0, 0]]
    save(samples, "best-0000000001.json", seed="LOSS8BIG", moves=mv, pick="best", won=False, stage=22, closeness=0.9,
         best_hand=50000)
    save(samples, "best-0000000002.json", seed="WINSMALL", moves=mv, pick="best", won=True, stage=24, closeness=1,
         best_hand=20000)
    save(samples, "best-0000000003.json", seed="WINBIGGG", moves=mv, pick="best", won=True, stage=24, closeness=1,
         best_hand=90000)
    save(samples, "best-0000000004.json", seed="LOSS8BOS", moves=mv, pick="best", won=False, stage=23, closeness=0.1,
         best_hand=1000)
    save(samples, "best-0000000005.json", seed="LOSS8BOT", moves=mv, pick="best", won=False, stage=23, closeness=0.6,
         best_hand=500)
    save(samples, "typical-001.json", seed="TYPICAL1", moves=mv, pick="typical")
    jobs = jobs_at(tmp_path)
    order = [jobs.read_sample(p)["seed"] for _, p in jobs.best()]
    # Wins first, by the biggest hand; then losses by how far they got, then how close they came.
    assert order == ["WINBIGGG", "WINSMALL", "LOSS8BOT", "LOSS8BOS", "LOSS8BIG"]
    shown = [jobs.next_sample("best")["seed"] for _ in range(6)]
    assert shown == order + ["WINBIGGG"], "after the last one the list starts again"
    assert jobs.next_sample("typical")["seed"] == "TYPICAL1"
    assert jobs.next_sample("typical") is None, "typical runs are shown once, and never the best ones"


def test_only_saved_runs_can_be_asked_for(tmp_path):
    samples = tmp_path / "watch" / "baseline" / "samples"
    save(samples, "best-0000000001.json", seed="AAAAAAAA", moves=[])
    (tmp_path / "watch" / "secret.json").write_text("{}")
    jobs = jobs_at(tmp_path)
    assert jobs.sample_path("baseline/best-0000000001.json") == samples / "best-0000000001.json"
    for bad in ("../secret.json", "baseline/../../secret.json", "/etc/passwd", "baseline/missing.json",
                "baseline/samples/best-0000000001.json", ""):
        assert jobs.sample_path(bad) is None, bad


def test_a_saved_run_replays_to_its_recorded_end(tmp_path):
    moves, end = greedy_run("REPLAY01")
    samples = tmp_path / "watch" / "baseline" / "samples"
    result = "won the run" if end["won"] else f"lost at ante {end['ante']}"
    save(samples, "best-0000000001.json", seed="REPLAY01", moves=moves, pick="best", won=end["won"], stage=3,
         closeness=0.5, best_hand=100, result=result, label="one of the baseline's best runs (greedy bot)")
    v = viewer.Viewer("max")
    jobs = jobs_at(tmp_path)
    rep = viewer.Replayer(v, jobs)
    s, rank = rep.pick_run()
    assert s["seed"] == "REPLAY01" and rank == 1
    out = rep.play(threading.Event(), s["seed"], [tuple(m) for m in s["moves"]], s["label"], "job", "greedy bot",
                   s["result"], "best", rank)
    run = v.run
    assert len(run.moves) == len(moves) and out == run.result
    assert run.result.startswith("won the run" if end["won"] else f"run over at ante {end['ante']}")
    assert run.view["ante"] == end["ante"] and run.public()["rank"] == 1 and run.pick == "best"
    plays = [m for m in run.moves if m["kind"] == 6]
    assert plays and all(m["cards"] and m["hand"] and m["score"] >= 0 for m in plays)
    assert all(m["cards"] for m in run.moves if m["kind"] == 7)
    # The run at any move, for rewinding.
    k = len(moves) // 2
    assert viewer.view_at("REPLAY01", [tuple(m) for m in moves], k)["money"] == run.moves[k - 1]["money"]


def test_demo_runs_without_a_job(tmp_path):
    v = viewer.Viewer("max")
    rep = viewer.Replayer(v, jobs_at(tmp_path))
    assert rep.pick_run() == (None, None)
    rep.play(threading.Event(), "DEMORUN1", None, "demo run (greedy bot)", "demo", "greedy bot")
    assert v.run.result and v.run.moves and v.run.source == "demo"


class Checks:
    def summary(self):
        return {"state": "running", "runs": 3, "moves": 120, "differences": 0}


def get(port: int, path: str):
    with urllib.request.urlopen(f"http://127.0.0.1:{port}{path}", timeout=10) as r:
        return r.status, json.loads(r.read()) if r.headers.get("Content-Type", "").startswith("application/json") else None


def post(port: int, body: dict):
    req = urllib.request.Request(f"http://127.0.0.1:{port}/api/control", data=json.dumps(body).encode(),
                                 headers={"Content-Type": "application/json"})
    try:
        with urllib.request.urlopen(req, timeout=5) as r:
            return r.status
    except urllib.error.HTTPError as e:
        return e.code


def job_files(tmp_path: Path, seeds=("PAGEBEST", "PAGETYPI")) -> tuple[viewer.JobFiles, list]:
    """A running job's files: its note, its progress and two saved runs."""
    watch = tmp_path / "watch"
    samples = watch / "baseline" / "samples"
    runs = []
    for i, seed in enumerate(seeds):
        moves, _end = greedy_run(seed, limit=40)
        runs.append(moves)
        name = "best-0000000007.json" if i == 0 else "typical-001.json"
        save(samples, name, seed=seed, moves=moves, pick="best" if i == 0 else "typical", won=False, stage=4,
             closeness=0.4, best_hand=812, result="lost at ante 2 to Big Blind (40% of its target)",
             label="one of the baseline's best runs (greedy bot)" if i == 0 else "a typical baseline run (greedy bot)")
    import os

    (watch / "current.json").write_text(json.dumps(
        {"job": "baseline", "state": "running", "step": "greedy", "pid": os.getpid(), "time": time.time(),
         "started": "2026-10-30T00:00:00+00:00", "steps": [{"name": "build", "done": True}, {"name": "greedy", "done": False}]}))
    (watch / "baseline" / "progress.json").write_text(json.dumps(
        {"kind": "baseline", "bot": "greedy bot", "deck": "Red Deck", "stake": "White Stake", "done": False,
         "updated": time.time(), "elapsed": 10, "threads": 16, "target": 1000000, "runs": 23456, "rate": 2345.6,
         "wins": 30, "win_rate": 0.00128, "win_ci": [0.0009, 0.0018], "mean_ante": 3.17,
         "ante_hist": [100, 120, 150, 180, 90, 20, 5, 2, 30], "lost_to": [["Big Blind", 400], ["The Needle", 80]],
         "hands_played": [["Pair", 900], ["Flush", 300]], "jokers_bought": [["Joker", 900, 0.2]],
         "jokers_held_at_end": [["Joker", 800, 0.18]], "jokers_in_wins": [["Blueprint", 12, 0.4]],
         "per_run": {"moves": 86, "jokers_bought": 4.6, "rerolls": 0, "skips": 0, "packs": 6.3, "vouchers": 0,
                     "money_at_end": 19},
         "best_runs": [[1, seeds[0], "lost at ante 2 to Big Blind (40% of its target)", 812, False, 7,
                        "best-0000000007.json"]],
         "history": [[5, 10000, 3.1, 0.001], [10, 23456, 3.2, 0.0012], [15, 30000, 3.18, 0.0013]]}))
    (tmp_path / "logs").mkdir(exist_ok=True)
    (tmp_path / "logs" / "baseline-greedy.log").write_text("1024 runs (2345 a second): mean final ante 3.17\n")
    return viewer.JobFiles(watch, tmp_path / "logs"), runs


def start_server(v, jobs, checks=None):
    srv = ThreadingHTTPServer(("127.0.0.1", 0), viewer.make_handler(v, jobs, checks))
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    return srv


def test_the_page_server(tmp_path):
    jobs, runs = job_files(tmp_path)
    v = viewer.Viewer("1")
    rep = viewer.Replayer(v, jobs)
    s, rank = rep.pick_run()
    v.speed = "max"
    rep.play(threading.Event(), s["seed"], [tuple(m) for m in s["moves"]], s["label"], "job", "greedy bot",
             s["result"], s["pick"], rank)
    srv = start_server(v, jobs, Checks())
    port = srv.server_address[1]
    try:
        code, st = get(port, "/api/state")
        assert code == 200 and st["run"]["seed"] == "PAGEBEST" and st["run"]["pick"] == "best" and st["run"]["rank"] == 1
        assert st["run"]["n_moves"] == len(runs[0]) and st["run"]["view"]["hand"] is not None
        assert st["pc"]["job"]["state"] == "running" and st["pc"]["progress"]["runs"] == 23456
        assert st["pc"]["log"] and st["checks"]["moves"] == 120 and st["settings"]["pick"] == "best"
        code, view = get(port, f"/api/view?run={st['run']['id']}&n=5")
        assert code == 200 and view == viewer.view_at("PAGEBEST", [tuple(m) for m in runs[0]], 5)
        with pytest.raises(urllib.error.HTTPError) as e:
            get(port, f"/api/view?run={st['run']['id']}&n=999")
        assert e.value.code == 404
        # Controls.
        assert post(port, {"speed": "2", "paused": True}) == 200 and v.speed == "2" and v.paused
        assert post(port, {"pick": "typical"}) == 200 and v.pick == "typical" and v.next_flag
        v.next_flag = False
        assert post(port, {"play": "baseline/best-0000000007.json"}) == 200
        assert v.request == tmp_path / "watch" / "baseline" / "samples" / "best-0000000007.json" and not v.paused
        assert post(port, {"play": "../current.json"}) == 404
        # The page itself.
        with urllib.request.urlopen(f"http://127.0.0.1:{port}/", timeout=10) as r:
            assert b"BALATRO" in r.read()
    finally:
        srv.shutdown()


def test_projections_follow_the_run(tmp_path):
    v = viewer.Viewer("max")
    viewer.Replayer(v, jobs_at(tmp_path)).play(threading.Event(), "PROJECT2", None, "demo", "demo", "greedy bot")
    analyst = viewer.Analyst(v)
    stop = threading.Event()
    threading.Thread(target=analyst.loop, args=(stop,), daemon=True).start()
    deadline = time.time() + 30
    n = len(v.run.moves)
    while time.time() < deadline and len(v.run.proj) < min(n + 1, 6):
        time.sleep(0.1)
    stop.set()
    assert n in v.run.proj, "the newest position is projected first"
    p = v.run.proj[n]
    assert 1 <= p["mean_ante"] <= 9 and 0 <= p["p_clear"] <= 1 and p["n"] >= 8
    assert v.analysis_error is None


def test_a_failing_projection_is_shown_not_fatal(monkeypatch):
    """An engine module without projections (built before this viewer) is reported on the page."""
    import balatro_meta._engine as engine

    class OldGame:
        def __init__(self, seed):
            pass

    monkeypatch.setattr(engine, "Game", OldGame)
    v = viewer.Viewer("max")
    viewer.Analyst(v).loop(threading.Event())  # returns at once
    assert v.analysis_error and "make watch" in v.analysis_error


def test_the_page_draws_the_run(tmp_path):
    sync_api = pytest.importorskip("playwright.sync_api")
    jobs, _runs = job_files(tmp_path)
    v = viewer.Viewer("2")
    stop = threading.Event()
    threading.Thread(target=viewer.Replayer(v, jobs).loop, args=(stop,), daemon=True).start()
    srv = start_server(v, jobs, Checks())
    url = f"http://127.0.0.1:{srv.server_address[1]}/"
    try:
        with sync_api.sync_playwright() as pw:
            try:
                browser = pw.chromium.launch()
            except Exception as exc:  # noqa: BLE001 - no browser on this machine
                pytest.skip(f"no browser for Playwright: {exc}")
            page = browser.new_page(viewport={"width": 1600, "height": 950})
            errors = []
            page.on("pageerror", lambda e: errors.append(str(e)))
            page.route("**/fonts.googleapis.com/**", lambda route: route.abort())
            page.goto(url)
            # The best run is replayed and drawn: the blind, the money, and soon cards in hand.
            page.wait_for_function("() => S.run && S.run.seed === 'PAGEBEST'", timeout=10000)
            page.wait_for_function("() => document.querySelectorAll('#tbl .tm-hand .pc').length > 0", timeout=20000)
            assert page.locator("#tbl .ts .money").inner_text().startswith("$")
            assert "Best run #1" in page.inner_text("#about")
            assert page.locator("#runcount").inner_text().replace(",", "").isdigit()
            assert "best runs so far" in page.inner_text("#search").lower()
            assert "No differences" in page.inner_text("#checks")
            # Rewind through the play by play: the table shows that move, with a way back.
            page.wait_for_function("() => S.moves.length >= 3")
            page.click("#events li[data-k='2']")
            page.wait_for_function("() => S.sel === 2 && S.selView !== null")
            assert "After move 2" in page.inner_text("#tbl .banner")
            page.click("#backlive")
            page.wait_for_function("() => S.sel === null")
            # The choice of runs, and a best run asked for by clicking it.
            page.click("[data-pick='typical']")
            deadline = time.time() + 5
            while v.pick != "typical" and time.time() < deadline:
                time.sleep(0.05)
            assert v.pick == "typical"
            page.wait_for_function("() => S.run && S.run.seed === 'PAGETYPI'", timeout=15000)
            page.click(".tp.best")
            page.wait_for_function("() => S.run && S.run.seed === 'PAGEBEST'", timeout=15000)
            assert not errors, errors
            browser.close()
    finally:
        stop.set()
        srv.shutdown()
