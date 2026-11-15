"""The league (scripts/league.py): members, blocks, B's statistics and a whole generation on a stand-in player."""
from __future__ import annotations

import json
import random
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))

import league  # noqa: E402


def test_members_are_the_same_each_time_and_distinct():
    a, b = league.members_for(0), league.members_for(0)
    assert a == b and len(a) == league.MEMBERS
    assert len({league.member_id(m) for m in a}) == len(a)
    names = {n for n, *_ in league.GENES}
    assert all(league.gene_name(g) in names for m in a for g in m)


def test_breeding_keeps_genes_from_the_parents_and_known_names():
    r = random.Random(1)
    p = [["spend-down=20", "long-power-x=1.5"], ["no-open-packs", "skips"]]
    names = {n for n, *_ in league.GENES}
    for _ in range(50):
        c = league.breed(p, r)
        assert c and all(league.gene_name(g) in names for g in c)


def test_blocks_follow_each_other_and_generations_never_share_seeds():
    assert league.block_seeds(0, 0) == (league.SEED_BASE, 200)
    assert league.block_seeds(0, 1) == (league.SEED_BASE + 200, 800)
    assert league.block_seeds(0, 2) == (league.SEED_BASE + 1000, 3000)
    assert league.block_seeds(1, 0)[0] >= league.SEED_BASE + sum(league.BLOCKS)


def test_the_paired_estimate_and_its_shrinkage():
    assert league.paired("YYAB", "YAYB") == (4, 1, 1)
    e = league.estimate([(100, 10, 20), (100, 5, 5)])
    assert abs(e["diff"] - 0.05) < 1e-12 and e["lo"] < e["diff"] < e["hi"]
    ests = {"a": {"seeds": 200, "diff": 0.10, "se": 0.04}, "b": {"seeds": 200, "diff": -0.10, "se": 0.04},
            "c": {"seeds": 200, "diff": 0.0, "se": 0.04}}
    sh = league.shrink(ests)
    assert 0 < sh["a"] < 0.10 and -0.10 < sh["b"] < 0, "pulled toward the mean"


def test_a_generation_plays_cuts_promotes_and_names_parents(tmp_path, monkeypatch):
    monkeypatch.setattr(league, "ROOT", tmp_path)
    monkeypatch.setattr(league.jobs, "best_8k_report",
                        lambda root: (0.75, ["--value-weight", "2.5", "--teacher-options", "precise"], "x.json"))
    monkeypatch.setattr(league, "MEMBERS", 6)
    monkeypatch.setattr(league, "BLOCKS", (20, 40, 60))
    monkeypatch.setattr(league, "PROMOTE", (3, 2))
    played = []

    def play(cmd):
        played.append(cmd)
        if cmd[0] == "gzip":
            return 0
        out = Path(tmp_path / cmd[cmd.index("--out") + 1])
        runs = int(cmd[cmd.index("--runs") + 1])
        opts = cmd[cmd.index("--teacher-options") + 1]
        # A member with "skips" wins every seed; everyone else wins every other one, like the reference.
        stages = "Y" * runs if "skips" in opts else "".join("Y" if i % 2 else "A" for i in range(runs))
        out.parent.mkdir(parents=True, exist_ok=True)
        out.write_text(json.dumps({"run_stages": stages}))
        return 0
    led = league.run_generation(0, play=play)
    ms = led["members"]
    refs = [c for c in played if c[0] != "gzip" and "--out" in c and "/ref-" in c[c.index("--out") + 1]]
    assert len(refs) == 3, "the reference plays every block"
    winners = [k for k, m in ms.items() if "skips" in m["genes"]]
    for k in winners:
        assert ms[k]["estimate"]["diff"] > 0
    assert (tmp_path / league.ledger_path(0)).exists()
    # Resuming plays nothing new.
    n = len(played)
    league.run_generation(0, play=play)
    assert len([c for c in played[n:] if c[0] != "gzip"]) == 0


def test_selection_uses_the_latest_block_alone_and_a_stop_keeps_the_rest_for_later(tmp_path, monkeypatch):
    monkeypatch.setattr(league, "ROOT", tmp_path)
    monkeypatch.setattr(league.jobs, "best_8k_report",
                        lambda root: (0.75, ["--value-weight", "2.5", "--teacher-options", "precise"], "x.json"))
    members = [["skips"], ["spend-down=20"], ["long-run=0.2"]]
    played = []

    def play(cmd):
        played.append(cmd)
        if cmd[0] == "gzip":
            return 0
        out = Path(tmp_path / cmd[cmd.index("--out") + 1])
        runs, first = int(cmd[cmd.index("--runs") + 1]), int(cmd[cmd.index("--seed") + 1])
        opts = cmd[cmd.index("--teacher-options") + 1]
        # "skips" wins every seed of the first block only, "spend-down" every seed of the later ones.
        later = first > 70001
        good = ("skips" in opts and not later) or ("spend-down" in opts and later)
        out.parent.mkdir(parents=True, exist_ok=True)
        out.write_text(json.dumps({"run_stages": "Y" * runs if good else "".join("Y" if i % 2 else "M" for i in
                                                                                  range(runs))}))
        return 0
    led = league.run_generation("t", play=play, members_given=members, blocks=(20, 40, 60), promote=(2, 1),
                                seed_base=70001, cut_first=False, stop_after=0)
    assert not any(m.get("block_1") for m in led["members"].values()), "stopped after block A"
    led = league.run_generation("t", play=play, members_given=members, blocks=(20, 40, 60), promote=(2, 1),
                                seed_base=70001, cut_first=False)
    ms = {",".join(m["genes"]): m for m in led["members"].values()}
    assert ms["spend-down=20"].get("block_2") and not ms["skips"].get("block_2"), "block C chosen on block B alone"
    assert ms["spend-down=20"]["parent"] and not ms["skips"]["parent"]


def test_the_wall_league_has_the_designed_members_and_its_own_seeds():
    ms = league.wall_members()
    flat = [",".join(m) for m in ms]
    assert "danger-x" in flat and "long-power-x=0.5" in flat and "danger-spend=0.75,danger-x" in flat
    assert league.block_seeds("wall", 0, league.WALL_BLOCKS, league.WALL_SEEDS) == (league.WALL_SEEDS, 1000)
    assert league.WALL_SEEDS >= league.SEED_BASE + 3 * league.GEN_SEEDS, "never a generation's seeds"
