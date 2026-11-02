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


def test_runs_start_on_the_seeds_asked_for_then_on_fresh_ones(tmp_path, monkeypatch):
    monkeypatch.setattr(record_me, "RUNS", tmp_path)
    monkeypatch.setattr(record_me, "START_DELAY", 0.0)
    fake = FakeGame(5)
    rec = record_me.Recorder(4, api=fake, files=NoMod(), game=None, say=lambda s: None, seeds=["ASKED123"])
    for _ in range(20000):
        rec.step()
        fake.person()
        if len(fake.finished) == 2 and fake.game is None:
            break
    rec.close()
    assert fake.finished[0] == "ASKED123"
    assert fake.finished[1] != "ASKED123" and not rec.seeds


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


def test_a_run_that_ends_while_the_game_is_busy_still_ends_as_it_did(tmp_path, monkeypatch):
    # The player starts the next run before the game-over (or win) screen settles: the run still ends as it did.
    import gzip

    monkeypatch.setattr(record_me, "RUNS", tmp_path)
    monkeypatch.setattr(record_me, "START_DELAY", 0.0)
    monkeypatch.setattr(record_me, "PEEK_EVERY", 0.0)
    fake = FakeGame(7)
    rec = record_me.Recorder(4, api=fake, files=NoMod(), game=None, say=lambda s: None)
    busy = {"on": False}
    real = rec.settled
    monkeypatch.setattr(rec, "settled", lambda: False if busy["on"] else real())
    for _ in range(20000):
        if fake.game is not None and fake.game.over:
            busy["on"] = True  # never settles from here on
        rec.step()
        if busy["on"] and rec.run is None:
            break
        fake.person()
    rec.close()
    files = list(tmp_path.glob("*.jsonl.gz"))
    assert len(files) == 1
    with gzip.open(files[0], "rt") as fh:
        end = json.loads(fh.read().splitlines()[-1])
    assert end["kind"] == "end" and end["why"] == "lost" and end["state"] == "GAME_OVER"


def test_a_win_whose_screen_shows_a_state_that_never_settles_still_ends_the_run_as_won(tmp_path, monkeypatch):
    # The win screen over a hand still being played out: the run is won, whatever state the game shows.
    import gzip

    monkeypatch.setattr(record_me, "RUNS", tmp_path)
    monkeypatch.setattr(record_me, "START_DELAY", 0.0)
    fake = FakeGame(8)
    rec = record_me.Recorder(4, api=fake, files=NoMod(), game=None, say=lambda s: None)
    for _ in range(40):
        rec.step()
        fake.person()
    assert rec.run is not None and fake.game is not None and not fake.game.over
    real = fake.call

    def won(method, params=None, timeout=None):
        out = real(method, params, timeout)
        if method == "gamestate" and out.get("state") != "MENU":
            out = dict(out, won=True, state="HAND_PLAYED")
        return out

    monkeypatch.setattr(fake, "call", won)
    rec.step()
    assert rec.run is None
    rec.close()
    files = list(tmp_path.glob("*.jsonl.gz"))
    with gzip.open(files[0], "rt") as fh:
        end = json.loads(fh.read().splitlines()[-1])
    assert end["kind"] == "end" and end["won"] and end["why"] == "won"


class ModThatSawTheEnd:
    """The companion mod: it reports no clock (BalatroBot's states settle as they come) and how the last run ended,
    kept until another run ends."""

    def __init__(self, folder: Path) -> None:
        self.dir = folder
        self.last_end = ""

    def write_control(self, **values) -> None:
        pass

    def read_state(self):
        return {"last_end": self.last_end, "end": "1"}


class FastPlayer(FakeGame):
    """A person who starts the next run straight from the game-over screen (its New Run button): BalatroBot never
    shows the game over settled, only the mod saw it."""

    def __init__(self, seed: int, mod: ModThatSawTheEnd) -> None:
        super().__init__(seed)
        self.mod = mod

    def person(self) -> None:
        super().person()
        if self.game is not None and self.game.over:
            v = self.game.view()
            self.mod.last_end = f"{self.game.seed}:lost:{v['ante']}:{v['round']}:GAME_OVER"
            self.finished.append(self.game.seed)
            self.game = Game(f"NEXTRUN{len(self.finished)}")
            self.fresh = True


def test_a_run_the_player_moves_on_from_fast_ends_as_the_companion_mod_saw_it(tmp_path, monkeypatch):
    import gzip

    monkeypatch.setattr(record_me, "RUNS", tmp_path)
    monkeypatch.setattr(record_me, "START_DELAY", 0.0)
    mod = ModThatSawTheEnd(tmp_path)
    fake = FastPlayer(9, mod)
    logged = []
    rec = record_me.Recorder(4, api=fake, files=mod, game=None, say=lambda s: None, log=logged.append)
    for _ in range(20000):
        rec.step()
        fake.person()
        if fake.finished and rec.finished:
            break
    rec.close()
    # The run that was lost (the next one, still being played, is saved without an end when recording stops).
    first = next(f for f in tmp_path.glob("*.jsonl.gz") if f.name.endswith(f"-{fake.finished[0]}.jsonl.gz"))
    with gzip.open(first, "rt") as fh:
        end = json.loads(fh.read().splitlines()[-1])
    assert end["kind"] == "end" and end["why"] == "lost" and not end["won"] and end["state"] == "GAME_OVER", end
    assert isinstance(end["ante"], int) and isinstance(end["round"], int)
    assert any("the mod saw it lost" in s for s in logged), logged


def test_a_win_seen_only_by_the_companion_mod_ends_the_run_as_won_even_while_the_game_is_busy(tmp_path, monkeypatch):
    import gzip

    monkeypatch.setattr(record_me, "RUNS", tmp_path)
    monkeypatch.setattr(record_me, "START_DELAY", 0.0)
    monkeypatch.setattr(record_me, "PEEK_EVERY", 0.0)
    mod = ModThatSawTheEnd(tmp_path)
    fake = FakeGame(10)
    rec = record_me.Recorder(4, api=fake, files=mod, game=None, say=lambda s: None)
    for _ in range(40):
        rec.step()
        fake.person()
    assert rec.run is not None
    seed = rec.run.seed
    mod.last_end = "SOMEOTHR:won:8:24:ROUND_EVAL"  # another run's end is not this one's
    monkeypatch.setattr(rec, "settled", lambda: False)  # the win screen keeps the game busy
    rec.step()
    assert rec.run is not None
    mod.last_end = f"{seed}:won:8:24:ROUND_EVAL"
    rec.step()
    assert rec.run is None and rec.last_seen == f"skip:{seed}"
    rec.close()
    with gzip.open(next(tmp_path.glob("*.jsonl.gz")), "rt") as fh:
        end = json.loads(fh.read().splitlines()[-1])
    assert end == {"kind": "end", "won": True, "ante": 8, "round": 24, "state": "ROUND_EVAL", "why": "won"}


def test_the_recorders_log_has_a_timestamp_per_line(tmp_path):
    log = record_me.log_to(tmp_path / "me" / "recorder.log")
    log("Recording run 1 (seed ABC).")
    log("ABC: another run began; the mod saw it won")
    lines = (tmp_path / "me" / "recorder.log").read_text().splitlines()
    assert len(lines) == 2 and lines[1].endswith("the mod saw it won") and lines[0][:4].isdigit()
