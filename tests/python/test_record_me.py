"""Recording a person's runs from the game (scripts/record_me.py), against a game played in the engine.

The game here is the engine behind BalatroBot's calls ("gamestate", "start"), and the person is the greedy bot,
who selects the cards before playing them as a person does. The recorder must start each run from the menu, keep a
state after every move, end the run when it is lost, start the next, and leave files the engine replays in full.
"""
from __future__ import annotations

import json
import random
import sys
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))

import record_me
import replay_me

engine = pytest.importorskip("balatro_meta._engine")
Game = engine.Game


def game_id(kind: str, i: int) -> int:
    return 900_000 + 5 * i + {"card": 0, "joker": 1, "consumable": 2}[kind]


class NoMod:
    """No companion mod: the recorder takes BalatroBot's settled states as they come."""

    dir = None

    def write_control(self, **values) -> None:
        pass

    def read_state(self):
        return None


class FakeGame:
    """BalatroBot's calls answered by the engine; `person()` makes the next move, cards selected first."""

    def __init__(self, seed: int) -> None:
        self.rng = random.Random(seed)
        self.game = None
        self.pending = None
        self.finished = []
        self.starts = 0
        self.fresh = False

    def call(self, method: str, params: dict | None = None, timeout=None):
        if method == "health":
            return {"status": "ok"}
        if method == "start":
            assert self.game is None, "a run was started over another"
            assert params["deck"] == "RED" and params["stake"] == "WHITE"
            self.game = Game(params["seed"])
            self.starts += 1
            self.fresh = True  # a person takes a moment to look at a new run
            return {}
        if method == "gamestate":
            if self.game is None:
                return {"state": "MENU"}
            gs = replay_me.engine_gamestate(self.game, game_id)
            if self.pending and self.pending[0] in (6, 7):
                for i, c in enumerate(gs["hand"]["cards"]):
                    if self.pending[1] >> i & 1:
                        c["state"] = dict(c["state"] or {}, highlight=True)
            return gs
        raise AssertionError(f"the recorder made a move: {method}")

    def person(self) -> None:
        if self.game is None or self.fresh:
            self.fresh = False
            return
        if self.game.over:
            self.finished.append(self.game.seed)
            self.game = None  # back to the main menu
            return
        if self.pending is None:
            kind, arg = self.game.greedy_move()
            if kind in (6, 7):
                self.pending = (kind, arg)  # selected now, played at the next look
                return
        else:
            kind, arg = self.pending
            self.pending = None
        assert self.game.step(kind, arg)


def test_runs_are_started_recorded_ended_and_replayed(tmp_path, monkeypatch):
    monkeypatch.setattr(record_me, "RUNS", tmp_path)
    monkeypatch.setattr(record_me, "START_DELAY", 0.0)
    said = []
    fake = FakeGame(3)
    rec = record_me.Recorder(4, api=fake, files=NoMod(), game=None, say=said.append)
    for _ in range(20000):
        rec.step()
        fake.person()
        if len(fake.finished) == 2 and fake.game is None:
            break
    rec.close()
    assert len(fake.finished) == 2 and fake.starts >= 2
    files = sorted(tmp_path.glob("*.jsonl.gz"))
    assert len(files) == 2, said
    for f in files:
        header, states = replay_me.read_run(f)
        assert header["seeded"] and header["deck"] == "RED" and header["stake"] == "WHITE"
        assert states[0]["gs"]["state"] == "BLIND_SELECT"
        r = replay_me.replay_states(states)
        assert r.ok, r.lost
        assert r.game.over
        live = json.loads(f.with_name(f.name.replace(".jsonl.gz", ".replay.json")).read_text())
        assert live["ok"], live.get("lost")
    assert any("saved" in s for s in said)


def test_a_run_of_another_deck_or_stake_is_not_recorded(tmp_path, monkeypatch):
    monkeypatch.setattr(record_me, "RUNS", tmp_path)
    fake = FakeGame(4)
    fake.game = Game("OTHERDK1")
    real = fake.call

    def other(method, params=None, timeout=None):
        out = real(method, params, timeout)
        if method == "gamestate" and out.get("state") != "MENU":
            out["deck"] = "BLUE"
        return out

    said = []
    rec = record_me.Recorder(4, api=type("A", (), {"call": staticmethod(other)})(), files=NoMod(), game=None,
                             say=said.append, auto_start=False)
    for _ in range(5):
        rec.step()
    rec.close()
    assert not list(tmp_path.glob("*.jsonl*"))
    assert sum("Not recording" in s for s in said) == 1


def test_the_speed_setting():
    assert record_me.speed_value(None) == 4 and record_me.speed_value("fast") == 10
    assert record_me.speed_value("2") == 2
    with pytest.raises(ValueError):
        record_me.speed_value("100")


def test_the_game_opens_in_a_window_with_sound_at_the_speed_asked(monkeypatch):
    g = record_me.PlayGame(record_me.PORT, None, 10.0)
    monkeypatch.setattr(record_me.checker.HiddenGame, "command",
                        lambda self: ["uvx", "balatrobot", "serve", "--headless", "--fast", "--port", "12351"])
    cmd = g.command()
    assert "--headless" not in cmd and "--fast" not in cmd
    assert cmd[-3:] == ["--audio", "--gamespeed", "10"]


def test_without_a_rebuilt_engine_the_runs_are_still_recorded(tmp_path, monkeypatch):
    # Right after `make sync`, before `make build`: no live replay, the same files.
    monkeypatch.setattr(record_me, "RUNS", tmp_path)
    monkeypatch.setattr(record_me, "START_DELAY", 0.0)
    monkeypatch.setattr(record_me, "engine_can_follow", lambda: False)
    fake = FakeGame(5)
    said = []
    rec = record_me.Recorder(4, api=fake, files=NoMod(), game=None, say=said.append)
    for _ in range(20000):
        rec.step()
        fake.person()
        if fake.finished and fake.game is None:
            break
    rec.close()
    files = list(tmp_path.glob("*.jsonl.gz"))
    assert len(files) == 1 and not list(tmp_path.glob("*.replay.json"))
    assert not any("couldn't follow" in s for s in said)
    r = replay_me.replay_states(replay_me.read_run(files[0])[1])
    assert r.ok, r.lost


def test_selecting_cards_is_not_a_new_state():
    plain = {"state": "SELECTING_HAND", "hand": {"cards": [{"id": 1, "key": "H_A", "state": []},
                                                           {"id": 2, "key": "S_K", "state": {"debuff": True}}]}}
    picked = json.loads(json.dumps(plain))
    picked["hand"]["cards"][0]["state"] = {"highlight": True}
    picked["hand"]["cards"][1]["state"] = {"debuff": True, "highlight": True}
    assert record_me.without_highlights(picked) == record_me.without_highlights(plain)
