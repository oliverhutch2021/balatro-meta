"""The league (scripts/league.py): members, blocks, B's statistics and a whole generation on a stand-in player."""
from __future__ import annotations

import json
import random
import sys
from pathlib import Path

import pytest

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
    assert league.paired({"run_stages": "YYAB"}, {"run_stages": "YAYB"}) == (4, 1, 1)
    with pytest.raises(ValueError, match="seed_base"):
        league.paired({"run_stages": "YY", "seed_base": 1}, {"run_stages": "YY", "seed_base": 30001})
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


def test_the_wall_league_has_s_six_members_on_w_and_its_own_seeds(tmp_path, monkeypatch):
    ms = [",".join(m) for m in league.wall_members()]
    assert len(ms) == 6 and "commit-main" in ms and "card-tiers" in ms and "long-run=0.2" in ms
    assert not any("danger-spend" in m for m in ms), "out until rebuilt (B)"
    assert league.block_seeds("wall", 0, league.WALL_BLOCKS, league.WALL_SEEDS) == (league.WALL_SEEDS, 2000)
    assert league.WALL_SEEDS >= league.SEED_BASE + 3 * league.GEN_SEEDS, "never a generation's seeds"
    # The reference plays W, each member W and its gene.
    monkeypatch.setattr(league, "ROOT", tmp_path)
    monkeypatch.setattr(league.jobs, "best_8k_report",
                        lambda root: (0.75, ["--value-weight", "2.5", "--teacher-options", "precise"], "x.json"))
    played = []

    def play(cmd):
        played.append(cmd)
        if cmd[0] != "gzip":
            out = Path(tmp_path / cmd[cmd.index("--out") + 1])
            out.parent.mkdir(parents=True, exist_ok=True)
            out.write_text(json.dumps({"run_stages": "Y" * int(cmd[cmd.index("--runs") + 1])}))
        return 0
    league.run_generation("wall-t", play=play, members_given=league.wall_members(), blocks=(10, 20), promote=(6,),
                          seed_base=90001, stop_after=0, cut_first=False, background=league.W)
    opts = [c[c.index("--teacher-options") + 1] for c in played if c[0] != "gzip"]
    assert opts[0] == "precise," + ",".join(league.W), "the reference is W"
    assert all(o.startswith("precise," + ",".join(league.W)) for o in opts) and len(opts) == 7


def test_generation_1_has_s_founders_crosses_and_the_base_and_plays_4579_as_reference(tmp_path, monkeypatch):
    ms = league.members_for(1)
    keys = {",".join(m) for m in ms}
    assert len(ms) == league.MEMBERS and [] in ms, "the -d5 base is a member"
    for m in league.GEN1_FOUNDERS + league.GEN1_CROSSES:
        assert ",".join(sorted(m)) in keys
    monkeypatch.setattr(league, "ROOT", tmp_path)
    monkeypatch.setattr(league, "MEMBERS", 40)
    monkeypatch.setattr(league.jobs, "best_8k_report",
                        lambda root: (0.75, ["--value-weight", "2.5", "--teacher-options", "precise"], "x.json"))
    played = []

    def play(cmd):
        played.append(cmd)
        if cmd[0] != "gzip":
            out = Path(tmp_path / cmd[cmd.index("--out") + 1])
            out.parent.mkdir(parents=True, exist_ok=True)
            out.write_text(json.dumps({"run_stages": "Y" * int(cmd[cmd.index("--runs") + 1])}))
        return 0
    led = league.run_generation(1, play=play, stop_after=0)
    ref = next(c for c in played if c[0] != "gzip" and "/ref-b0" in c[c.index("--out") + 1])
    assert ref[ref.index("--teacher-options") + 1] == "precise," + ",".join(league.GEN1_REFERENCE)
    base = next(c for c in played if c[0] != "gzip" and "/base-b0" in c[c.index("--out") + 1])
    assert base[base.index("--teacher-options") + 1] == "precise"
    assert all("lead_ante4" in m for m in led["members"].values())


def test_generation_2_is_s_list_and_plays_a8e3_as_reference(tmp_path, monkeypatch):
    ms = league.members_for(2)
    keys = {",".join(m) for m in ms}
    assert len(ms) == 40 and len(keys) == 40 and [] in ms, "the -d5 base is a member"
    designed = league.dedupe(league.GEN2_DESIGNED)
    assert len(designed) == 18 and ms[:18] == designed
    assert ",".join(sorted(g for g in league.G1_A8E3 if g != "no-card-tiers")) in keys, "the tiers pair has the tiers on"
    assert all("fix-dollars" not in league.gene_names(m) or "deck-fix" in league.gene_names(m) for m in ms)
    monkeypatch.setattr(league, "ROOT", tmp_path)
    monkeypatch.setattr(league.jobs, "best_8k_report",
                        lambda root: (0.75, ["--value-weight", "2.5", "--teacher-options", "precise"], "x.json"))
    played = []

    def play(cmd):
        played.append(cmd)
        if cmd[0] != "gzip":
            out = Path(tmp_path / cmd[cmd.index("--out") + 1])
            out.parent.mkdir(parents=True, exist_ok=True)
            out.write_text(json.dumps({"run_stages": "Y" * int(cmd[cmd.index("--runs") + 1])}))
        return 0
    led = league.run_generation(2, play=play, stop_after=0)
    assert led["reference"] == league.G1_A8E3
    ref = next(c for c in played if c[0] != "gzip" and "/ref-b0" in c[c.index("--out") + 1])
    assert ref[ref.index("--teacher-options") + 1] == "precise," + ",".join(league.G1_A8E3)
    assert int(ref[ref.index("--seed") + 1]) == league.SEED_BASE + 2 * league.GEN_SEEDS, "fresh seeds"


def test_a_partnered_gene_is_dropped_without_its_partner():
    assert league.tidy(["fix-dollars=1", "skips"]) == ["skips"]
    assert league.tidy(["deck-fix", "fix-dollars=1"]) == ["deck-fix", "fix-dollars=1"]


def test_a_generation_begun_keeps_its_ledgers_members_when_the_genes_change(tmp_path, monkeypatch):
    monkeypatch.setattr(league, "ROOT", tmp_path)
    led = {"members": {"aaaa": {"genes": ["skips"]}}}
    p = tmp_path / league.ledger_path(5)
    p.parent.mkdir(parents=True, exist_ok=True)
    p.write_text(json.dumps(led))
    monkeypatch.setattr(league.jobs, "best_8k_report",
                        lambda root: (0.75, ["--value-weight", "2.5", "--teacher-options", "precise"], "x.json"))
    out = league.run_generation(5, plan_only=True)
    assert list(out["members"]) == ["aaaa"], "no member is added from a regenerated list"
