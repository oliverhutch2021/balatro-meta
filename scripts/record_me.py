"""Record your own runs, move by move, to set your play against the bot's.

    make record-me               Balatro opens at 4x speed; play White Stake, Red Deck runs as you normally would
    make record-me SPEED=fast    the game at 10x (any number works too: SPEED=2)

Balatro opens through BalatroBot's launcher on a port of its own (12351), like the hidden game of the real-game checks
but in a window with sound. From the main menu each run starts by itself, with a fresh random seed, so the engine can
replay it exactly (the game counts these as seeded runs: no unlocks, achievements or streaks from them). The
recorder only reads the game: it never clicks or changes anything. Each time the game settles after a move of yours
it keeps the state (ids, cards, money, the shop: none of the game's text), in .sync/me/runs/ on this PC only. A run
ends with a win (beating the ante 8 boss: choose Main Menu, not Endless) or a loss; back at the main menu (or with New
Run) the next one starts. Ctrl+C stops recording and closes the game; whatever you played is kept.

Before starting: stop `make watch` (its hidden game shares your save profile). The jobs can keep running.
While you play, the engine replays each move to check it can follow (scripts/replay_me.py); a run it couldn't
follow to the end is still kept whole, for a fix to replay later.
"""
from __future__ import annotations

import argparse
import gzip
import json
import os
import queue
import shutil
import sys
import threading
import time
from datetime import datetime, timezone
from pathlib import Path

HERE = Path(__file__).resolve().parent
# The repo's root: this file's folder's parent, or further up for a copy kept outside scripts/ (.sync/me).
ROOT = next(p for p in (HERE, *HERE.parents) if (p / "scripts" / "watch.py").exists())
sys.path.insert(0, str(ROOT / "scripts"))
sys.path.insert(0, str(HERE))

import checker
import replay_me
import watch
from watch import STABLE, Api

PORT = 12351
CHECKS_PORT = 12350
RUNS = ROOT / ".sync" / "me" / "runs"
VERSION = 1
START_DELAY = 3.0  # seconds at the main menu before the next run starts
POLL = 0.12  # seconds between reads of the game's state
PEEK_EVERY = 0.5  # seconds between reads while the game is busy, to catch a run's end before its screen settles
SPEEDS = {"fast": 10}


def speed_value(s: str | None) -> float:
    if s is None or s == "":
        return 4.0
    if s in SPEEDS:
        return float(SPEEDS[s])
    v = float(s)
    if not 0.25 <= v <= 16:
        raise ValueError("SPEED is 4 by default, fast (10x), or a number from 0.25 to 16")
    return v


def without_highlights(gs: dict) -> dict:
    """The state with the cards' selected marks left out (selecting cards is not a move). A card's empty state
    comes as [] from the game, so one left empty here is written the same way."""
    out = json.loads(json.dumps(gs))
    for name in ("hand", "jokers", "consumables", "pack"):
        a = out.get(name)
        for c in (a.get("cards") or []) if isinstance(a, dict) else []:
            if isinstance(c.get("state"), dict):
                c["state"].pop("highlight", None)
                if not c["state"]:
                    c["state"] = []
    return out


def kept(gs: dict) -> dict:
    """What the recorder keeps of a state: no text from the game (replay_me.scrub), and the cards still to be drawn
    as a set (their order is the game's secret, which nothing here needs)."""
    s = replay_me.scrub(gs)
    deck = s.get("cards")
    if isinstance(deck, dict) and isinstance(deck.get("cards"), list):
        deck["cards"] = sorted(deck["cards"], key=lambda c: (str(c.get("key")), c.get("id") or 0))
    return s


class RunFile:
    """One run on disk: a header, then one line per kept state; gzipped when the run ends."""

    def __init__(self, gs: dict, seeded: bool, speed: float) -> None:
        RUNS.mkdir(parents=True, exist_ok=True)
        stamp = datetime.now(timezone.utc).strftime("%Y%m%d-%H%M%S")  # noqa: UP017 - Python 3.10 has no UTC
        self.seed = gs.get("seed")
        self.path = RUNS / f"{stamp}-{self.seed}.jsonl"
        self.f = self.path.open("a", encoding="utf-8")
        self.states = 0
        self.write({"kind": "header", "seed": self.seed, "deck": gs.get("deck"), "stake": gs.get("stake"),
                    "started": stamp, "seeded": seeded, "speed": speed, "recorder": VERSION})

    def write(self, rec: dict) -> None:
        self.f.write(json.dumps(rec, separators=(",", ":")) + "\n")
        self.f.flush()

    def state(self, gs: dict, highlighted_before: list) -> None:
        self.states += 1
        self.write({"kind": "state", "t": round(time.time(), 3), "gs": gs, "highlighted_before": highlighted_before})

    def close(self, end: dict | None) -> Path:
        if end:
            self.write({"kind": "end", **end})
        self.f.close()
        gz = self.path.with_suffix(".jsonl.gz")
        with self.path.open("rb") as src, gzip.open(gz, "wb") as dst:
            shutil.copyfileobj(src, dst)
        self.path.unlink()
        return gz


def engine_can_follow() -> bool:
    """The engine module is new enough to follow a run (`make build` after `make sync`)."""
    try:
        from balatro_meta._engine import Game

        return hasattr(Game, "copy") and hasattr(Game, "legal_kinds")
    except ImportError:
        return False


class LiveReplay:
    """The engine following the run as it is played, on its own thread (a hard state can take it a moment)."""

    def __init__(self, first: dict) -> None:
        self.q: queue.Queue = queue.Queue()
        self.result: dict | None = None
        self.thread = threading.Thread(target=self.work, args=(first,), daemon=True)
        self.thread.start()

    def feed(self, gs: dict, highlight: list) -> None:
        self.q.put((gs, highlight))

    def work(self, first: dict) -> None:
        try:
            r = replay_me.Replay(first)
        except Exception as e:  # noqa: BLE001 - the recording goes on whatever the replay does
            self.result = {"ok": False, "lost": f"couldn't start: {e}"}
            return
        while True:
            item = self.q.get()
            if item is None:
                break
            try:
                if not r.lost:
                    r.feed(*item)
            except Exception as e:  # noqa: BLE001
                r.lost = f"replay error: {e}"
        self.result = r.summary()

    def finish(self, path: Path) -> dict:
        self.q.put(None)
        self.thread.join(timeout=120)
        out = self.result or {"ok": False, "lost": "the replay didn't finish in time"}
        name = path.name.replace(".jsonl.gz", "").replace(".jsonl", "")
        (path.parent / f"{name}.replay.json").write_text(json.dumps(out, default=list) + "\n")
        return out


class Recorder:
    def __init__(self, speed: float, port: int = PORT, api: Api | None = None, files=None, game=None, say=print,
                 auto_start: bool = True) -> None:
        self.speed = speed
        self.port = port
        self.say = say
        self.api = api
        folder = watch.save_dir() if files is None else None
        self.files = files if files is not None else checker.ModFiles(folder, port)
        self.game = game
        if self.game is None and folder is not None:
            self.game = PlayGame(port, folder, speed)
        self.auto_start = auto_start
        self.run: RunFile | None = None
        self.replay: LiveReplay | None = None
        self.last_seen: str | None = None
        self.highlight: list = []
        self.menu_since: float | None = None
        self.finished = 0
        self.stop = threading.Event()
        self.last_ante = None
        self.started_seed: str | None = None
        self.live = engine_can_follow()
        self.peeked = 0.0

    # -- the game
    def connect(self, wait: float = 120.0) -> bool:
        deadline = time.time() + wait
        launched = False
        while time.time() < deadline and not self.stop.is_set():
            if self.api is None:
                url = f"http://127.0.0.1:{self.port}"
                bridge = watch.windows_bridge()
                for api in (Api(url, timeout=30), Api(url, bridge, timeout=30) if bridge else None):
                    if api is None:
                        continue
                    try:
                        api.call("health", timeout=3)
                        self.api = api
                        break
                    except (OSError, RuntimeError, ValueError):
                        continue
            if self.api is not None:
                return True
            if not launched and self.game is not None:
                launched = self.game.launch()
                self.say("Opening Balatro..." if launched else "BalatroBot's launcher (uvx) wasn't found: RUNNING.md, step 5")
                if not launched:
                    return False
            time.sleep(2)
        return False

    def settled(self) -> bool:
        """The game has played out the last move: the companion mod reports no blocking events, twice."""
        if getattr(self.files, "dir", None) is None:
            return True
        calm, seen, start = 0, None, time.time()
        while time.time() - start < 2.0:
            st = self.files.read_state() or {}
            clock = st.get("time")
            if clock is None:
                return True  # no mod: the state's own settling (watch.STABLE) has to do
            if clock != seen:
                seen = clock
                calm = calm + 1 if st.get("busy") == "0" and st.get("complete", "1") == "1" else 0
                if calm >= 2:
                    return True
            time.sleep(0.03)
        return False

    # -- runs
    def begin(self, gs: dict) -> None:
        deck, stake = gs.get("deck"), gs.get("stake")
        if deck != "RED" or stake != "WHITE":
            if self.last_seen != f"skip:{gs.get('seed')}":
                self.say(f"Not recording this run: it's {str(deck).title()} Deck at {str(stake).title()} Stake "
                         "(only White Stake, Red Deck runs are recorded).")
                self.last_seen = f"skip:{gs.get('seed')}"
            return
        self.run = RunFile(gs, seeded=self.started_seed == gs.get("seed"), speed=self.speed)
        self.replay = None
        self.last_ante = None
        self.say(f"Recording run {self.finished + 1} (seed {gs.get('seed')}).")

    def end(self, gs: dict | None, why: str) -> None:
        if self.run is None:
            return
        end = None
        if gs is not None:
            end = {"won": bool(gs.get("won")), "ante": gs.get("ante_num"), "round": gs.get("round_num"),
                   "state": gs.get("state"), "why": why}
        path = self.run.close(end)
        states = self.run.states
        self.run = None
        result = self.replay.finish(path) if self.replay else None
        self.replay = None
        self.finished += 1
        how = ("a win!" if end and end["won"] else f"over at ante {end['ante']}") if end and why != "stopped" \
            else "stopped part-way"
        note = "" if result is None or result.get("ok") else \
            " (the engine couldn't follow all of it: kept whole for a later fix)"
        self.say(f"Run {self.finished} saved: {how}, {states} states{note}.")

    def keep(self, gs: dict) -> None:
        """A settled state of the run being recorded: kept if anything but the selected cards changed."""
        sig = json.dumps(without_highlights(gs), sort_keys=True)
        hl = replay_me.highlighted(gs)
        if sig != self.last_seen:
            self.last_seen = sig
            k = kept(gs)
            self.run.state(k, self.highlight)
            if not self.live:
                pass
            elif self.replay is None:
                self.replay = LiveReplay(k)
            else:
                self.replay.feed(k, self.highlight)
            ante = gs.get("ante_num")
            if ante != self.last_ante and gs.get("state") == "BLIND_SELECT":
                self.last_ante = ante
                if ante and ante > 1:
                    self.say(f"  ante {ante}")
        self.highlight = hl

    def start_next(self) -> None:
        seed = watch.fresh_seed()
        self.started_seed = seed
        try:
            self.api.call("start", {"deck": "RED", "stake": "WHITE", "seed": seed})
        except RuntimeError as e:
            self.say(f"Couldn't start a run yet ({e}); trying again.")
            self.menu_since = time.time()

    def peek_end(self) -> None:
        """While the game is still busy: a run won or lost ends now, even if the next run is started before the
        win or game-over screen settles (its last hand then isn't kept, but how the run ended is)."""
        if self.run is None or time.time() - self.peeked < PEEK_EVERY:
            return
        self.peeked = time.time()
        gs = self.api.call("gamestate")
        if not isinstance(gs, dict) or gs.get("seed") != self.run.seed:
            return
        if gs.get("won") or gs.get("state") == "GAME_OVER":
            self.end(gs, "won" if gs.get("won") else "lost")
            self.last_seen = f"skip:{gs.get('seed')}"

    def step(self) -> None:
        """One look at the game."""
        if not self.settled():
            self.peek_end()
            return
        gs = self.api.call("gamestate")
        if not isinstance(gs, dict):
            return
        state = gs.get("state")
        if state == "MENU":
            if self.run is not None:
                self.end(None, "left to the menu")
            if self.menu_since is None:
                self.menu_since = time.time()
                if self.auto_start:
                    self.say(f"At the main menu: the next run starts in {START_DELAY:.0f} seconds (Ctrl+C here "
                             "to stop).")
            elif self.auto_start and time.time() - self.menu_since >= START_DELAY:
                self.menu_since = None
                self.start_next()
            return
        self.menu_since = None
        if state not in STABLE:
            return
        if self.run is not None and gs.get("seed") != self.run.seed:
            self.end(None, "another run began")
        if self.run is None:
            if self.last_seen == f"skip:{gs.get('seed')}":
                return
            self.begin(gs)
            if self.run is None:
                return
        self.keep(gs)
        if gs.get("won") or state == "GAME_OVER":
            self.end(gs, "won" if gs.get("won") else "lost")
            self.last_seen = f"skip:{gs.get('seed')}"  # what follows a win (Endless) isn't recorded

    def loop(self) -> int:
        if not self.connect():
            self.say("Couldn't reach the game. Is BalatroBot installed (RUNNING.md, step 5)?")
            return 1
        keeper = threading.Thread(target=self.keep_control, daemon=True)
        keeper.start()
        self.say("Recording. Play as you normally would; Ctrl+C here stops recording and closes the game.")
        failures = 0
        while not self.stop.is_set():
            try:
                self.step()
                failures = 0
            except (OSError, ValueError, RuntimeError) as e:
                failures += 1
                if failures >= 20:
                    self.say(f"Lost the game ({e}).")
                    break
                time.sleep(0.5)
            time.sleep(POLL)
        return 0

    def keep_control(self) -> None:
        """Keep the companion mod reporting at full rate (and readying the run-setup deck `start` needs)."""
        while not self.stop.is_set():
            self.files.write_control(lock=0)
            self.stop.wait(1.0)

    def close(self) -> None:
        self.stop.set()
        if self.run is not None:
            self.end(None, "stopped")
        self.files.write_control(lock=0, stop=1)
        if self.game is not None:
            self.game.stop()


def checks_running(folder: Path | None) -> bool:
    """The hidden game of `make watch` is up (its companion mod wrote its report in the last few seconds)."""
    if folder is None:
        return False
    p = folder / "balatro_meta" / str(CHECKS_PORT) / "state.txt"
    try:
        return time.time() - p.stat().st_mtime < 8
    except OSError:
        return False


class PlayGame(checker.HiddenGame):
    """Balatro in a window, with sound, at the chosen speed, through BalatroBot's launcher on its own port."""

    def __init__(self, port: int, save_folder: Path | None, speed: float) -> None:
        super().__init__(port, save_folder)
        self.speed = speed

    def command(self) -> list[str] | None:
        cmd = super().command()
        if cmd is None:
            return None
        cmd = [c for c in cmd if c not in ("--headless", "--fast")]
        speed = int(self.speed) if float(self.speed).is_integer() else self.speed
        return [*cmd, "--audio", "--gamespeed", str(speed)]


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--speed", default=os.environ.get("SPEED") or "4", help="4 (default), fast (10x), or a number")
    a = ap.parse_args(argv)
    try:
        speed = speed_value(a.speed)
    except ValueError as e:
        print(e)
        return 2
    folder = watch.save_dir()
    if folder is None:
        print("Balatro's save folder (%APPDATA%\\Balatro) wasn't found. Run the game once first.")
        return 1
    if checks_running(folder):
        # Often left over after make watch was closed (or its window shut before it could close the game).
        print("make watch's hidden copy of Balatro is still running (it shares your save profile): closing it...")
        checker.HiddenGame(CHECKS_PORT, folder).stop()
        for _ in range(20):
            time.sleep(1)
            if not checks_running(folder):
                break
        else:
            print("It's still running. If make watch is open, stop it (Ctrl+C in its window), then run this again.")
            return 1
        print("Closed it.")
    msg = watch.install_mod(folder)
    if msg:
        print("Installed the companion mod.")
    rec = Recorder(speed)
    try:
        return rec.loop()
    except KeyboardInterrupt:
        print()
        return 0
    finally:
        rec.close()
        n = len(list(RUNS.glob("*.jsonl.gz"))) if RUNS.exists() else 0
        print(f"Closed the game. {rec.finished} run(s) recorded this time, {n} in all (in .sync/me/runs). "
              "Thanks! Tell Claude when you've played some.")


if __name__ == "__main__":
    raise SystemExit(main())
