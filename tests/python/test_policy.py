"""The network that learns to play (scripts/train_policy.py): moves as its parts and back, card picks as a proper
distribution, acting and judging agreeing, the loop guard, and imitation, PPO (with resume) and evaluation end to end
on a small recording, on the CPU.
"""
from __future__ import annotations

import itertools
import json
import math
import os
import shutil
import subprocess
import sys
from pathlib import Path

import numpy as np
import pytest

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))

import train_policy as tp
import train_value as tv
from balatro_meta._engine import Game

torch = pytest.importorskip("torch")


@pytest.fixture(scope="module")
def moves(tmp_path_factory):
    """60 greedy runs, every move recorded (the teacher's are recorded the same way)."""
    if not shutil.which("cargo"):
        pytest.skip("cargo is not installed")
    d = tmp_path_factory.mktemp("moves") / "greedy"
    r = subprocess.run(["cargo", "run", "--release", "-q", "-p", "bm-core", "--example", "baseline", "--",
                        "--runs", "60", "--threads", "2", "--seed", "9", "--record-moves", str(d)],
                       cwd=ROOT, capture_output=True, text=True, check=False, timeout=900,
                       env={k: v for k, v in os.environ.items() if k != "BM_WATCH_DIR"})
    assert r.returncode == 0, r.stdout + r.stderr
    return d


def test_recorded_moves_are_what_was_played(moves):
    man = json.loads((moves / "manifest.json").read_text())
    assert man["input"] == "screen" and man["inputs"] == tp.engine().OBS_LEN and man["runs"] == 60
    data, mov = tp.load_moves(moves, None)
    assert mov.shape == (data.rows, 3)
    assert all((m[2] >> m[0]) & 1 for m in mov), "every move's kind was allowed"
    # A run's moves replay with the engine to the end the recording gives.
    runs = json.loads(next(moves.glob("chunk-*-runs.json")).read_text())
    first = runs[0]
    rows = np.flatnonzero(data.lab[:, tv.RUN] == first["run"])
    g = Game(first["seed"])
    for k, a in mov[rows, :2]:
        assert g.step(int(k), int(a))
    assert g.over and g.view()["won"] == first["won"]


def test_moves_go_to_parts_and_back(moves):
    _data, mov = tp.load_moves(moves, None)
    parts = tp.to_parts(mov[:, 0], mov[:, 1])
    assert np.array_equal(tp.to_moves(parts["kind"], parts["slot"], parts["to"], parts["cards"]),
                          mov[:, 1].astype(np.uint64))
    # Every kind of argument.
    kinds = np.array([tp.PLAY, tp.MOVE_JOKER, tp.SELL_JOKER, tp.USE_CONSUMABLE, tp.PACK_CHOOSE, tp.LEAVE_SHOP])
    args = np.array([0b10110, 3 | 1 << 8, 2, 1 | 0b11 << 8, 4 | 0b1 << 8, 0], np.uint64)
    p = tp.to_parts(kinds, args)
    assert list(p["slot"]) == [-1, 3, 2, 1, 4, -1] and list(p["to"]) == [-1, 1, -1, -1, -1, -1]
    assert list(np.flatnonzero(p["cards"][0])) == [1, 2, 4] and list(np.flatnonzero(p["cards"][3])) == [0, 1]
    assert np.array_equal(tp.to_moves(p["kind"], p["slot"], p["to"], p["cards"]), args)


def test_card_picks_are_a_distribution():
    torch.manual_seed(3)
    for n, lo, hi in [(6, 1, 3), (5, 0, 5), (7, 2, 2)]:
        logits = torch.randn(1, n) * 2
        here = torch.ones(1, n, dtype=torch.bool)
        total = 0.0
        for bits in itertools.product([0, 1], repeat=n):
            if lo <= sum(bits) <= hi:
                cards = torch.tensor([bits], dtype=torch.bool)
                total += math.exp(float(tp.cards_logp(torch, logits, here, cards, torch.tensor([lo]),
                                                      torch.tensor([hi]))))
        assert abs(total - 1) < 1e-5, (n, lo, hi, total)
    # Sampled picks always fit their range; a forced card is always picked.
    logits = torch.randn(500, 8)
    here = torch.ones(500, 8, dtype=torch.bool)
    lo, hi = torch.full((500,), 1), torch.full((500,), 5)
    for greedy in (False, True):
        pick = tp.sample_cards(torch, logits, here, lo, hi, greedy)
        assert ((pick.sum(1) >= 1) & (pick.sum(1) <= 5)).all()


def test_the_network_acts_as_it_judges(moves):
    torch.manual_seed(0)
    data, mov = tp.load_moves(moves, None)
    model = tp.build_model(torch, 32, 1, 2, 64)
    pp = tp.Parts(torch, model)
    rows = np.arange(min(400, data.rows))
    x = torch.from_numpy(data.dense(rows))
    legal = torch.from_numpy(((mov[rows, 2:3] >> np.arange(tp.N_KINDS)[None]) & 1).astype(bool))
    with torch.no_grad():
        kind, slot, to, cards, logp, _ = pp.act(x, legal)
        lp, ent, _ = pp.evaluate(x, legal, kind, slot, to, cards)
    assert torch.allclose(logp, lp, atol=1e-4) and torch.isfinite(ent).all()
    assert legal[torch.arange(len(rows)), kind].all() and not set(kind.tolist()) & set(tp.NEVER)
    # Picked cards are cards in hand; slots point at objects there.
    hand = x[:, 320:320 + 24 * 40].reshape(len(rows), 24, 40)[..., 0] > 0
    assert not (cards & ~hand).any()
    plays = torch.isin(kind, torch.tensor([tp.PLAY, tp.DISCARD]))
    assert ((cards.sum(1) >= 1) & (cards.sum(1) <= 5))[plays].all()
    # The teacher's own moves have a probability.
    parts = {k: torch.from_numpy(v[rows]) for k, v in tp.to_parts(mov[:, 0], mov[:, 1]).items()}
    with torch.no_grad():
        lp_t, _, _ = pp.evaluate(x, legal, parts["kind"], parts["slot"], parts["to"], parts["cards"])
    assert torch.isfinite(lp_t).all()


def test_the_guard_breaks_loops():
    g = tp.Guard(2)
    legal = np.zeros((2, tp.N_KINDS), bool)
    legal[:, [tp.BUY, tp.LEAVE_SHOP, tp.MOVE_JOKER]] = True
    g.after(np.array([tp.BUY, tp.BUY]), np.array([True, False]), np.array([False, False]))
    m = g.mask(legal)
    assert not m[0, tp.BUY] and m[1, tp.BUY], "a refused kind waits for a move that goes through"
    g.after(np.array([tp.LEAVE_SHOP, tp.BUY]), np.array([False, False]), np.array([False, False]))
    assert g.mask(legal)[0, tp.BUY]
    for _ in range(tp.FREE_STREAK):
        g.after(np.array([tp.MOVE_JOKER, tp.LEAVE_SHOP]), np.array([False, False]), np.array([False, False]))
    m = g.mask(legal)
    assert not m[0, tp.MOVE_JOKER] and m[1, tp.MOVE_JOKER] and m[0, tp.LEAVE_SHOP]


def test_a_minibatch_in_pieces_gives_the_same_gradients():
    """PPO works a big minibatch out a few positions at a time to fit in the GPU's memory; the gradients must be the
    ones the whole minibatch gives at once."""
    from balatro_meta.env import VecEnv

    torch.manual_seed(1)
    model = tp.build_model(torch, 32, 1, 2, 64)
    pp = tp.Parts(torch, model)
    env, guard = VecEnv(16, seed=4), tp.Guard(16)
    rows = []
    for _ in range(3):
        x = torch.from_numpy(env.observe().copy())
        legal = torch.from_numpy(guard.mask(env.legal_kinds()))
        with torch.no_grad():
            kind, slot, to, cards, logp, _ = pp.act(x, legal)
        rows.append((x, legal, kind, slot, to, cards, logp.float()))
        k = kind.numpy()
        env.step(k, tp.to_moves(k, slot.numpy(), to.numpy(), cards.numpy()))
    cols = [torch.cat(c) for c in zip(*rows, strict=True)]
    n = cols[0].shape[0]
    batch = (*cols, torch.randn(n), torch.randn(n))
    mb = torch.randperm(n)[:40]

    def grads(micro):
        model.zero_grad(set_to_none=True)
        got = tp.ppo_grads(torch, pp, batch, mb, micro, torch.device("cpu"), 0.2, 0.5, 0.003, warm=False)
        return got, [q.grad.clone() for q in model.parameters() if q.grad is not None]

    whole, g1 = grads(40)
    parts, g2 = grads(7)
    assert len(g1) == len(g2) > 0
    assert all(torch.allclose(x, y, atol=1e-6, rtol=1e-4) for x, y in zip(g1, g2, strict=True))
    assert all(abs(whole[k] - parts[k]) < 1e-5 for k in whole)
    assert abs(whole["kl"]) < 1e-4, "the network that played and the one learning are the same so far"
    # The value head alone: everything else keeps no gradient, so the optimizer leaves it as it is.
    tp.value_only(model)
    heads = {id(q) for q in model.value_head.parameters()}
    assert all((q.grad is not None) == (id(q) in heads) for q in model.parameters())


def ending(won=False, ante=2, lost_to="Big Blind", bought=(), held=(), hands=None, **counts) -> dict:
    """A finished run as the engine's VecEnv.take_endings gives it."""
    end = {"won": won, "ante": 9 if won else ante, "lost_to": "" if won else lost_to, "money": 10, "held": list(held),
           "bought": list(bought), "moves": 100, "refused": 0, "rerolls": 0, "skips": 0, "packs": 1, "vouchers": 0,
           "blinds_beaten": 3, "hands": hands or [1, 2] + [0] * 10, "best_hand": 50.0}
    return {**end, **counts}


def test_the_run_book_counts_all_runs_the_latest_and_blocks():
    names = [f"Hand {i}" for i in range(12)]
    book = tp.RunBook(names, block_runs=2, now_runs=3)
    runs = [ending(bought=["Joker"], held=["Joker"]), ending(lost_to="The Wall", rerolls=4),
            ending(won=True, bought=["Blueprint"], held=["Blueprint", "Joker"]), ending(lost_to=tp.CUT),
            ending(won=True, held=["Blueprint"], money=40)]
    for i, r in enumerate(runs):
        book.add(r, seconds=10.0 * i)
    f = book.fields()
    assert f["lost_to"] == [["Big Blind", 1], ["The Wall", 1], [tp.CUT, 1]]
    assert f["jokers_bought"] == [["Blueprint", 1, 0.2], ["Joker", 1, 0.2]]
    assert f["jokers_in_wins"] == [["Blueprint", 2, 1.0], ["Joker", 1, 0.5]], "share of the wins holding it at the end"
    assert f["hands_played"] == [["Hand 0", 5], ["Hand 1", 10]] and f["per_run"]["rerolls"] == 0.8
    assert "runs" not in f and "win_rate" not in f, "the job's own counters say those"
    now = f["now"]
    assert (now["runs"], now["wins"], now["label"]) == (3, 2, "last 3 runs"), "the latest runs only"
    assert now["ante_hist"] == [0, 1, 0, 0, 0, 0, 0, 0, 2] and now["lost_to"] == [[tp.CUT, 1]]
    # Two blocks of two runs; the fifth run waits in the next one.
    i = {k: n for n, k in enumerate(f["block_fields"])}
    assert [b[i["end_runs"]] for b in f["blocks"]] == [2, 4] and [b[i["wins"]] for b in f["blocks"]] == [0, 1]
    assert f["blocks"][1][i["cut"]] == 1 and f["blocks"][1][i["end_seconds"]] == 30.0
    # Its state carries on in a checkpoint.
    again = tp.RunBook(names, book.state(), block_runs=99)
    again.add(ending(), seconds=50.0)
    assert again.fields()["blocks"][-1][i["end_runs"]] == 6 and again.total["runs"] == 6
    # Beyond MAX_BLOCKS, blocks merge in pairs and new blocks are twice as long.
    many = [[2 * (k + 1), float(k), 2, 1, 4, 10, 0, 1, 1, 0, 5, 0] for k in range(5)]
    merged = tp.merge_blocks(many)
    assert [b[0] for b in merged] == [4, 8, 10] and merged[0][2:4] == [4, 2] and merged[2] == many[4]


def test_imitate_ppo_and_eval_end_to_end(moves, tmp_path):
    def run(*args, env=None):
        r = subprocess.run([sys.executable, str(ROOT / "scripts" / "train_policy.py"), *args, "--cpu",
                            "--width", "32", "--layers", "1"],
                           cwd=ROOT, capture_output=True, text=True, check=False, timeout=900,
                           env={**os.environ, **(env or {})})
        assert r.returncode == 0, r.stdout + r.stderr
        return r.stdout

    run("imitate", "--data", str(moves), "--out", str(tmp_path / "imitate.pt"), "--report",
        str(tmp_path / "imitate.json"), "--epochs", "1", "--batch", "256")
    rep = json.loads((tmp_path / "imitate.json").read_text())
    assert rep["train_moves"] > 0 and 0 <= rep["history"][-1]["same_move"] <= 1
    watch = tmp_path / "watch"
    out = run("ppo", "--init", str(tmp_path / "imitate.pt"), "--dir", str(tmp_path / "ppo"), "--max-updates", "4",
              "--envs", "16", "--steps", "8", "--minibatch", "64", "--save-minutes", "0", "--value-warmup", "1",
              "--block-runs", "2", env={"BM_WATCH_DIR": str(watch)})
    assert "Done:" in out and (tmp_path / "ppo" / "latest.pt").exists()
    first = torch.load(tmp_path / "ppo" / "latest.pt", weights_only=False)["state"]
    prog = json.loads((watch / "progress.json").read_text())
    assert prog["runs"] == first["stats"]["runs"] > 0 and prog["bot"] == "network (PPO)"
    # The live numbers break the runs down as the teacher's jobs do, for all runs and the latest, block by block.
    assert first["book"]["total"]["runs"] == first["stats"]["runs"], "every finished run is counted once"
    assert sum(n for _, n in prog["lost_to"]) == prog["runs"] - prog["wins"]
    assert {"moves", "jokers_bought", "packs", "money_at_end", "refused"} <= set(prog["per_run"])
    assert prog["now"]["runs"] == min(prog["runs"], tp.NOW_RUNS) and prog["now"]["label"].startswith("last ")
    assert len(prog["blocks"]) == prog["runs"] // 2 and prog["block_fields"] == list(tp.BLOCK_FIELDS)
    out = run("ppo", "--dir", str(tmp_path / "ppo"), "--max-updates", "7", "--envs", "16", "--steps", "8",
              "--minibatch", "64", "--block-runs", "2")
    assert "Resuming" in out
    second = torch.load(tmp_path / "ppo" / "latest.pt", weights_only=False)["state"]
    assert first["updates"] == 4 and second["updates"] == 7 and second["stats"]["runs"] > first["stats"]["runs"]
    assert second["book"]["total"]["runs"] == second["stats"]["runs"], "a resumed run carries on counting"
    # Saved runs replay with the engine to their recorded end.
    for f in sorted((watch / "samples").glob("typical-*.json"))[:5]:
        s = json.loads(f.read_text())
        g = Game(s["seed"])
        for k, a in s["moves"]:
            assert g.step(k, a)
        assert g.over and (9 if g.view()["won"] else g.view()["ante"]) == s["ante"]
    # While the copied network's value warms up, how it plays doesn't change at all: only the value head learns.
    run("ppo", "--init", str(tmp_path / "imitate.pt"), "--dir", str(tmp_path / "warm"), "--max-updates", "2",
        "--envs", "16", "--steps", "8", "--minibatch", "64", "--value-warmup", "5")
    before = torch.load(tmp_path / "imitate.pt", weights_only=False)["model"]
    after = torch.load(tmp_path / "warm" / "latest.pt", weights_only=False)["model"]
    changed = {k for k in before if not torch.equal(before[k], after[k])}
    assert changed and all(k.startswith("value_head.") for k in changed), changed
    run("eval", "--model", str(tmp_path / "ppo" / "latest.pt"), "--runs", "10", "--envs", "4", "--seed", "1",
        "--out", str(tmp_path / "eval.json"))
    ev = json.loads((tmp_path / "eval.json").read_text())
    assert ev["kind"] == "baseline" and ev["done"] and ev["runs"] == 10 and len(ev["run_stages"]) == 10
    assert "?" not in ev["run_stages"] and sum(ev["ante_hist"]) == 10
    assert sum(n for _, n in ev["lost_to"]) == 10 - ev["wins"] and ev["now"]["runs"] == 10 and ev["per_run"]["moves"] > 0
