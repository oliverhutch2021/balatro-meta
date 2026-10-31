"""Replaying a recorded run in the engine from the states the game showed (scripts/replay_me.py).

The "game" here is the engine itself, shown in BalatroBot's format with ids of its own: a bot plays, moving hand
cards and jokers around now and then like a person, and some settled states are passed over as a recorder polling
the game can miss them. The replay has to follow every move to the same end.
"""
from __future__ import annotations

import gzip
import json
import random
import sys
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))

import replay_me as rm

engine = pytest.importorskip("balatro_meta._engine")
Game = engine.Game

OFFSET = {"card": 0, "joker": 1, "consumable": 2}


def game_id(kind: str, i: int) -> int:
    """The game's ids are never the engine's."""
    return 500_000 + 7 * i + OFFSET[kind]


def shown(g) -> dict:
    return rm.engine_gamestate(g, game_id)


def positions(arg: int, shift: int = 0) -> list[int]:
    return [i for i in range(40) if arg >> (i + shift) & 1]


def simulate(seed: str, rng: random.Random, bot: str = "greedy", skip: float = 0.0, fidget: float = 0.3,
             limit: int = 3000):
    """A person's run: the bot's moves, cards and jokers moved around at times, and the states a recorder keeps
    (each with the cards shown selected just before it)."""
    g = Game(seed)
    states = [(shown(g), [])]
    passed_over = False
    while not g.over and len(g.moves) < limit:
        v = g.view()
        if v["state"] in rm.HAND_STATES and len(v["hand"]) > 1 and rng.random() < fidget:
            g.step(8, rng.randrange(len(v["hand"])) | rng.randrange(len(v["hand"])) << 8)
            if rng.random() < 0.5:
                states.append((shown(g), []))
        if len(v["jokers"]) > 1 and rng.random() < fidget / 4:
            g.step(9, rng.randrange(len(v["jokers"])) | rng.randrange(len(v["jokers"])) << 8)
        kind, arg = g.greedy_move() if bot == "greedy" else g.teacher_move()
        before = shown(g)
        hand_ids = [c["id"] for c in before["hand"]["cards"]]
        picked = []
        if kind in (6, 7):
            picked = [hand_ids[i] for i in positions(arg)]
        elif kind in (17, 18) and arg >> 8:
            picked = [hand_ids[i] for i in positions(arg, 8)]
        assert g.step(kind, arg), f"{seed}: the bot's move {kind} {arg:#x} was refused"
        if kind in rm.REORDERS:
            continue
        # A recorder can miss the settled state between two quick clicks (buying, selling, using a card that
        # needs no target), never two in a row; a hand, a new shop or an open pack is looked at for a while.
        quick = kind in (10, 12, 13, 14, 16) or (kind == 17 and not arg >> 8)
        if skip and quick and not passed_over and rng.random() < skip:
            passed_over = True
            continue
        passed_over = False
        states.append((shown(g), picked))
    return g, states


def replay(states) -> rm.Replay:
    r = rm.Replay(states[0][0])
    for gs, picked in states[1:]:
        r.feed(gs, picked)
    return r


def same_end(r: rm.Replay, g) -> None:
    assert r.ok, r.lost
    assert not rm.compare(r.game, shown(g))
    a, b = r.game.view(), g.view()
    for k in ("ante", "round", "money", "won", "deck_cards", "hand_levels", "vouchers", "tags"):
        assert a[k] == b[k], k


@pytest.mark.parametrize("seed", ["RECORD01", "RECORD02", "RECORD03"])
def test_a_greedy_run_with_cards_moved_around_replays_to_the_same_end(seed):
    g, states = simulate(seed, random.Random(seed))
    r = replay(states)
    same_end(r, g)
    assert r.steps, "the moves were found"


def test_a_teacher_run_with_states_passed_over_replays_to_the_same_end():
    # The teacher sells, rerolls, opens packs and uses Tarots on cards; a third of the quick moves leave no state.
    g, states = simulate("RECORDT1", random.Random(7), bot="teacher", skip=0.33)
    r = replay(states)
    same_end(r, g)
    kinds = {k for k, _ in g.moves}
    assert kinds & {11, 12, 15, 18}, "the run has shop and pack moves to follow"


def test_a_played_hand_is_found_from_the_cards_selected_when_the_round_ends():
    # The hand that ends a round: the game puts the hand away, so only the cards selected before it (or the
    # score) say which cards were played.
    g, states = simulate("RECORD04", random.Random(4), fidget=0.0)
    ends = [i for i, (gs, _) in enumerate(states) if gs["state"] == "ROUND_EVAL"]
    assert ends
    r = replay(states)
    same_end(r, g)


def test_a_state_no_move_explains_stops_the_replay_with_the_reason():
    _, states = simulate("RECORD05", random.Random(5))
    bad = json.loads(json.dumps(states[6][0]))
    bad["money"] += 7
    for i in range(6, 6 + rm.Replay.MAX_SKIPPED):
        states[i] = (bad, [])
    r = replay(states)
    assert not r.ok and "money" in r.lost


def test_a_late_state_is_passed_over_when_the_next_one_explains_it():
    g, states = simulate("RECORD06", random.Random(6))
    late = json.loads(json.dumps(states[5][0]))
    late["money"] += 3  # caught while something was still landing
    states.insert(5, (late, []))
    r = replay(states)
    same_end(r, g)
    assert any("passed over" in n for n in r.notes)


def test_the_kept_state_has_no_text_from_the_game():
    gs = {"hand": {"cards": [{"id": 1, "key": "H_A", "label": "Base Card",
                              "value": {"suit": "H", "rank": "A", "effect": "+11 chips"}}]},
          "jokers": {"cards": [{"id": 2, "key": "j_joker", "label": "Joker", "value": {"effect": "+4 Mult"}}]},
          "blinds": {"boss": {"name": "The Pillar", "effect": "words", "tag_effect": "", "tag_name": ""}},
          "hands": {"Pair": {"level": 2, "played": 3, "example": [["H_A", True]]}}}
    s = rm.scrub(gs)
    assert s["hand"]["cards"][0] == {"id": 1, "key": "H_A", "value": {"suit": "H", "rank": "A",
                                                                         "effect_numbers": [11.0]}}
    assert s["jokers"]["cards"][0]["value"] == {"effect_numbers": [4.0]}
    assert s["blinds"]["boss"] == {"name": "The Pillar", "tag_name": ""}
    assert s["hands"]["Pair"] == {"level": 2, "played": 3}


def test_a_recorded_file_reads_back_even_if_its_last_line_was_cut(tmp_path):
    g, states = simulate("RECORD07", random.Random(8), limit=40)
    p = tmp_path / "run.jsonl.gz"
    with gzip.open(p, "wt") as f:
        f.write(json.dumps({"kind": "header", "seed": g.seed}) + "\n")
        for gs, picked in states:
            f.write(json.dumps({"kind": "state", "t": 0, "gs": gs, "highlighted_before": picked}) + "\n")
        f.write('{"kind": "state", "t": 0, "gs": {"sta')
    header, back = rm.read_run(p)
    assert header["seed"] == g.seed and len(back) == len(states)
    r = rm.replay_states(back)
    assert r.ok, r.lost
