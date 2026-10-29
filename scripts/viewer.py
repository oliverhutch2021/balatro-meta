"""The Balatro Meta viewer: watch the bots play in your own Balatro, with live stats around it.

    make watch           opens the viewer in your browser (Ctrl+C stops it; the game stays open)

The middle of the page is your game. The companion mod (mods/balatro_meta_watch) saves a small picture of
it a few times a second and this server passes it on; after every move it also keeps a snapshot, so you
can click back through the run. While the viewer runs, the game takes no mouse or keyboard input and runs
at the speed chosen on the page. Around the game: what your PC is running, how the run is going (quick
playouts by the greedy bot from fair copies of each position: hidden cards reshuffled, fresh random
streams), the moves so far, and the engine-versus-game check that every replay is. A difference is saved
to .sync/watch/mismatches/ and the viewer moves on to the next run.

The runs: the newest one a job saved for watching (.sync/watch/<job>/samples/*.json), or else demo runs by
the greedy bot. The pictures stay on your PC (in memory and .sync/, which git ignores).
"""
from __future__ import annotations

import json
import os
import struct
import subprocess
import threading
import time
import zlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import parse_qs, urlparse

import watch
from watch import (
    ROOT,
    WATCH,
    apply_in_game,
    compare,
    describe,
    engine_view,
    fresh_seed,
    game_view,
    hand_permutation,
    newest_sample,
    settle,
    unsupported,
)

PAGE = ROOT / "viewer" / "index.html"
PNG_SIGNATURE = b"\x89PNG\r\n\x1a\n"
SNAP_BUDGET = 160 << 20  # bytes of pictures a run may keep for clicking back
MOD_VERSION = "0.2.0"
DEFAULT_PORT = 8790

# Speeds on the page: (game speed, frame rate uncapped, pause between moves, factor for waits).
SPEEDS = {
    "1": (1, False, 0.8, 1.0),
    "2": (2, False, 0.4, 2.0),
    "4": (4, False, 0.2, 4.0),
    "10": (10, False, 0.05, 10.0),
    "max": (10, True, 0.0, 20.0),
}


# ---------------------------------------------------------------------- the mod's files
class GameFiles:
    """The companion mod's files next to Balatro's saves: control.txt (ours), state.txt and frames (its)."""

    def __init__(self, folder: Path | None) -> None:
        self.folder = folder
        self.dir = folder / "balatro_meta" if folder else None
        self.n = 0

    def write_control(self, **values) -> None:
        if not self.dir:
            return
        self.n += 1
        text = "\n".join([f"n={self.n}"] + [f"{k}={v}" for k, v in values.items()]) + "\n"
        try:
            self.dir.mkdir(parents=True, exist_ok=True)
            tmp = self.dir / "control.tmp"
            tmp.write_text(text)
            os.replace(tmp, self.dir / "control.txt")
        except OSError:
            pass

    def read_state(self) -> dict | None:
        if not self.dir:
            return None
        try:
            text = (self.dir / "state.txt").read_text(errors="replace")
        except OSError:
            return None
        out = {}
        for line in text.splitlines():
            if "=" in line:
                k, v = line.split("=", 1)
                out[k.strip()] = v.strip()
        return out

    def read_frame(self, name: str) -> bytes | None:
        """A PNG the mod wrote (None if it is missing or not a whole PNG yet)."""
        if not self.dir or "/" in name or "\\" in name:
            return None
        try:
            data = (self.dir / name).read_bytes()
        except OSError:
            return None
        return data if data[:8] == PNG_SIGNATURE and data[-8:-4] == b"IEND" else None


def png(rgb) -> bytes:
    """A numpy (h, w, 3) uint8 picture as PNG bytes (fast zlib level)."""
    import numpy as np

    h, w, _ = rgb.shape
    raw = np.empty((h, 1 + w * 3), dtype=np.uint8)
    raw[:, 0] = 0
    raw[:, 1:] = rgb.reshape(h, w * 3)

    def chunk(kind: bytes, data: bytes) -> bytes:
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data) & 0xFFFFFFFF)

    return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0))
            + chunk(b"IDAT", zlib.compress(raw.tobytes(), 1)) + chunk(b"IEND", b""))


class Frames:
    """Follows the mod's state and newest picture (polled about 25 times a second)."""

    def __init__(self, files: GameFiles) -> None:
        self.files = files
        self.lock = threading.Lock()
        self.state: dict | None = None
        self.state_changed = 0.0  # wall time the mod's clock last moved
        self.seq: str | None = None
        self.png: bytes | None = None
        self.got = 0.0
        self.ever_fresh = False  # the mod has been seen running since the viewer started

    def poll(self) -> None:
        st = self.files.read_state()
        if st is None:
            with self.lock:
                self.state = None
            return
        with self.lock:
            if self.state is not None and st.get("time") != self.state.get("time"):
                self.state_changed = time.time()
                self.ever_fresh = True
            elif self.state is None:
                self.state_changed = time.time()
            self.state = st
        seq = st.get("seq")
        if st.get("frame") and seq != self.seq:
            data = self.files.read_frame(st["frame"])
            if data is not None:
                with self.lock:
                    self.seq, self.png, self.got = seq, data, time.time()

    def loop(self, stop: threading.Event, viewer: Viewer | None = None) -> None:
        while not stop.is_set():
            try:
                self.poll()
                if viewer is not None:
                    self.fill_snapshots(viewer)
            except Exception:  # noqa: BLE001, S110 - a bad frame must never stop the viewer
                pass
            time.sleep(0.04)

    def fill_snapshots(self, viewer: Viewer) -> None:
        """Pictures for moves that asked for one, from the first frame taken after the move."""
        with viewer.lock:
            run = viewer.run
            pending = list(run.pending) if run else []
        if not pending:
            return
        with self.lock:
            got, data = self.got, self.png
        for n, after in pending:
            if got > after or time.time() - after > 2.0:
                with viewer.lock:
                    if data:
                        run.keep_snap(n, data)
                    if (n, after) in run.pending:
                        run.pending.remove((n, after))

    def mod_status(self) -> str:
        """ok; missing (not running in the game: it needs a restart); old (an older version is running);
        stale (it was running but the game stopped drawing, for example while minimised)."""
        with self.lock:
            st, changed, ever = self.state, self.state_changed, self.ever_fresh
        if st is None:
            return "missing"
        if st.get("version") != MOD_VERSION:
            return "old"
        if time.time() - changed > 3:
            return "stale" if ever else "missing"
        return "ok"

    def busy(self) -> tuple[str | None, str | None]:
        """(the mod's clock, blocking events queued), from its newest report."""
        with self.lock:
            st = self.state or {}
        return st.get("time"), st.get("busy")

    def latest_png(self) -> tuple[str | None, bytes | None]:
        with self.lock:
            return self.seq, self.png


# ---------------------------------------------------------------------- runs
def summary(v: dict) -> dict:
    """The engine's state as the page's charts and lists use it."""
    return {"state": v["state"], "ante": v["ante"], "round": v["round"], "money": v["money"],
            "chips": v["chips"], "target": v.get("blind_score"), "blind": v.get("blind_name"),
            "hands_left": v["hands_left"], "discards_left": v["discards_left"], "won": v["won"]}


def details(v: dict) -> dict:
    """Everything the right-hand panels show about the run now."""
    keep = ("state", "ante", "round", "money", "chips", "blind_score", "blind_name", "blind_reward",
            "blind_disabled", "hands_left", "discards_left", "hand_size", "joker_slots", "consumable_slots",
            "jokers", "consumables", "vouchers", "tags", "hand_levels", "last_hand", "deck_left", "deck_total",
            "reroll_cost", "won", "hands_played_run", "blinds", "cash_out", "pack_key", "pack_choices_left")
    out = {k: v.get(k) for k in keep}
    out["shop"] = v.get("shop")
    out["shop_vouchers"] = v.get("shop_vouchers")
    out["shop_packs"] = [p for p in (v.get("shop_packs") or []) if p]
    out["pack"] = v.get("pack")
    return out


def outcome(kind: int, before: dict, after: dict) -> str:
    """What a move did, in a few words."""
    dm = after["money"] - before["money"]
    money = f"+${dm}" if dm > 0 else (f"-${-dm}" if dm < 0 else "")
    if kind == 6:
        score = after["chips"] - before["chips"]
        text = f"{after.get('last_hand') or 'hand'} for {score:,.0f}"
        if after["state"] == "ROUND_EVAL":
            text += ", blind beaten"
        elif after["state"] == "GAME_OVER":
            text += ", run lost"
        return text
    if kind == 0:
        target = after.get("blind_score")
        return f"{after.get('blind_name') or 'blind'}" + (f", needs {target:,.0f}" if target else "")
    if kind == 1:
        gained = list(after.get("tags") or [])
        for t in before.get("tags") or []:
            if t in gained:
                gained.remove(t)
        return ", ".join(gained) if gained else "skipped"
    return money


class RunRecord:
    def __init__(self, seed: str, label: str, source: str, bot: str) -> None:
        self.id = f"{seed}-{time.time():.3f}"
        self.seed, self.label, self.source, self.bot = seed, label, source, bot
        self.started = time.time()
        self.moves: list[dict] = []
        self.move_list: list[tuple[int, int]] = []
        self.start: dict | None = None
        self.view: dict | None = None
        self.result: str | None = None
        self.mismatch: dict | None = None
        self.snaps: dict[int, bytes] = {}
        self.snap_bytes = 0
        self.pending: list[tuple[int, float]] = []  # moves waiting for their picture: (move, wall time)
        self.proj: dict[int, dict] = {}

    def keep_snap(self, n: int, data: bytes) -> None:
        """Keep a move's picture; over the budget, the oldest pictures go first."""
        self.snaps[n] = data
        self.snap_bytes += len(data)
        while self.snap_bytes > SNAP_BUDGET and len(self.snaps) > 1:
            old = min(self.snaps)
            self.snap_bytes -= len(self.snaps.pop(old))

    def public(self, since: int = 0) -> dict:
        return {"id": self.id, "seed": self.seed, "label": self.label, "source": self.source, "bot": self.bot,
                "started": self.started, "result": self.result, "mismatch": self.mismatch,
                "start": self.start, "n_moves": len(self.moves), "moves": self.moves[since:],
                "view": self.view, "snaps": sorted(self.snaps)}


class Viewer:
    """Everything the page reads and the driver writes."""

    def __init__(self, speed: str = "max") -> None:
        self.lock = threading.RLock()
        self.paused = False
        self.speed = speed if speed in SPEEDS else "max"
        self.analysis = True
        self.next_flag = False
        self.restart_flag = False
        self.link = {"game": "starting", "message": "Starting"}
        self.session = {"started": time.time(), "runs": 0, "moves": 0, "mismatches": 0}
        self.run: RunRecord | None = None
        self.last_run: RunRecord | None = None

    def new_run(self, seed: str, label: str, source: str, bot: str) -> RunRecord:
        with self.lock:
            if self.run is not None:
                self.last_run = self.run
                self.last_run.snaps = {}  # keep memory small: only the current run keeps pictures
                self.last_run.snap_bytes = 0
            self.run = RunRecord(seed, label, source, bot)
            return self.run

    def set_link(self, game: str, message: str) -> None:
        with self.lock:
            self.link = {"game": game, "message": message}

    def take_next(self) -> bool:
        with self.lock:
            flag, self.next_flag = self.next_flag, False
            return flag

    def factor(self) -> float:
        return SPEEDS[self.speed][3]


# ---------------------------------------------------------------------- the driver
class Driver:
    """Plays runs in the engine and in your game together, move by move, and records them."""

    def __init__(self, viewer: Viewer, files: GameFiles, frames: Frames | None, api=None,
                 engine_only: bool = False, launch: bool = True, seed: str = "", runs: int = 0,
                 say=print) -> None:
        self.viewer, self.files, self.frames = viewer, files, frames
        self.api = api
        self.engine_only = engine_only
        self.launch = launch
        self.seed = seed
        self.runs = runs
        self.say = say
        self.stop = threading.Event()
        self.launched_at = 0.0

    # -- the game link
    def control(self) -> None:
        """Tell the mod what the viewer wants (rewritten every second while the viewer runs)."""
        speed, uncap, _, _ = SPEEDS[self.viewer.speed]
        self.files.write_control(lock=0 if self.engine_only else 1, speed=speed, uncap=int(uncap),
                                 frames=0 if self.engine_only else 1, width=960, every=0.2)

    def release(self) -> None:
        self.files.write_control(lock=0, frames=0)

    def connect(self) -> bool:
        """Find BalatroBot, opening the game through it if needed. False while it isn't there."""
        if self.engine_only:
            return True
        if self.api is not None:
            try:
                self.api.call("health", timeout=3)
                return True
            except (OSError, RuntimeError, ValueError):
                self.api = None
        api = watch.try_urls()
        if api:
            self.api = api
            return True
        if self.launch and time.time() - self.launched_at > 120:
            self.launched_at = time.time()
            if watch.launch_game(4):
                self.viewer.set_link("launching", "Opening Balatro through BalatroBot")
                return False
        self.viewer.set_link("waiting", "Waiting for Balatro with BalatroBot (RUNNING.md, step 5)")
        return False

    def restart_game(self) -> None:
        self.viewer.set_link("launching", "Restarting Balatro so the viewer's mod loads")
        subprocess.run(["taskkill.exe", "/IM", "Balatro.exe", "/F"], capture_output=True, cwd="/mnt/c", check=False)
        self.api = None
        time.sleep(2)
        self.launched_at = 0.0

    def wait_idle(self, cash_out: bool = False) -> None:
        """Wait until the game has played out the last move (the mod reports no blocking events queued
        twice in a row). Without the mod, wait a fixed time instead."""
        if self.engine_only or self.frames is None:
            return
        f = self.viewer.factor()
        if self.frames.mod_status() != "ok":
            time.sleep((3.0 if cash_out else 0.6) / f)
            return
        deadline = time.time() + max(1.5, 10.0 / f)
        seen, calm = None, 0
        while time.time() < deadline:
            clock, busy = self.frames.busy()
            if clock != seen:
                seen = clock
                calm = calm + 1 if busy == "0" else 0
                if calm >= 2:
                    return
            time.sleep(0.03)

    def pace(self) -> None:
        pause = SPEEDS[self.viewer.speed][2]
        end = time.time() + pause
        while time.time() < end and not self.stop.is_set():
            time.sleep(0.02)
        while self.viewer.paused and not self.stop.is_set() and not self.viewer.next_flag:
            time.sleep(0.1)

    # -- one run
    def play_run(self, seed: str, moves, label: str, source: str = "demo") -> tuple[bool, str]:
        from balatro_meta._engine import Game

        game = Game(seed)
        rec = self.viewer.new_run(game.seed, label, source, "greedy bot" if moves is None else "job's bot")
        api = None if self.engine_only else self.api
        with self.viewer.lock:
            rec.start = summary(game.view())
            rec.view = details(game.view())
        if api:
            self.control()
            try:
                api.call("menu")
            except RuntimeError:
                pass
            gs = settle(api, api.call("start", {"deck": "RED", "stake": "WHITE", "seed": game.seed}))
            self.wait_idle()
            gs, diff = compare(api, game, gs)
            if diff:
                return False, self.mismatch(rec, game, 0, "start", diff, gs)
        step = 0
        while not game.over and step < 5000 and not self.stop.is_set():
            if self.viewer.restart_flag:
                with self.viewer.lock:
                    rec.result = f"stopped after {step} moves to restart the game"
                return True, rec.result
            if self.viewer.take_next():
                with self.viewer.lock:
                    rec.result = f"skipped after {step} moves"
                return True, rec.result
            if moves is None:
                kind, arg = game.greedy_move()
            elif step < len(moves):
                kind, arg = moves[step]
            else:
                break
            before = game.view()
            what = describe(kind, arg, game)
            why = unsupported(kind, before["state"]) if api else None
            if why:
                with self.viewer.lock:
                    rec.result = f"stopped before move {step + 1} ({what}): {why}"
                return True, rec.result
            e_before = engine_view(game)
            if api:
                if kind == 2:  # the cash-out total is only final once its rows have played out
                    self.wait_idle(cash_out=True)
                gs = api.call("gamestate")
                if gs.get("state") in watch.HAND_STATES and e_before["hand"]:
                    perm = hand_permutation(e_before["hand"], game_view(gs)["hand"])
                    if perm is not None and perm != list(range(len(perm))):
                        gs = settle(api, api.call("rearrange", {"hand": perm}))
            if not game.step(kind, arg):
                with self.viewer.lock:
                    rec.result = f"the engine refused move {step + 1} ({what})"
                return False, rec.result
            after = game.view()
            diff = []
            done_at = time.time()
            if api:
                gs = settle(api, apply_in_game(api, kind, arg, e_before, game))
                self.wait_idle()
                done_at = time.time()
                gs, diff = compare(api, game, gs)
            with self.viewer.lock:
                rec.move_list.append((kind, arg))
                move = {"n": step + 1, "kind": kind, "what": what, "detail": outcome(kind, before, after),
                        "ok": not diff, "t": time.time(), **summary(after)}
                if after["state"] == "SHOP" and before["state"] != "SHOP":
                    move["shows"] = [watch.card_name(i) for i in after.get("shop") or []]
                elif after["state"] == "SMODS_BOOSTER_OPENED":
                    move["shows"] = [watch.card_name(i) for i in after.get("pack") or []]
                rec.moves.append(move)
                rec.view = details(after)
                if api and self.frames:
                    rec.pending.append((step + 1, done_at))
                self.viewer.session["moves"] += 1
            if diff:
                return False, self.mismatch(rec, game, step + 1, what, diff, gs)
            step += 1
            self.pace()
        v = game.view()
        end = "won the run" if v["won"] else f"run over at ante {v['ante']}"
        with self.viewer.lock:
            rec.result = f"{end} after {step} moves"
            self.viewer.session["runs"] += 1
        return True, rec.result

    def mismatch(self, rec: RunRecord, game, step: int, what: str, diff: list[str], gs: dict) -> str:
        out = WATCH / "mismatches"
        out.mkdir(parents=True, exist_ok=True)
        f = out / f"{game.seed}-move{step}.json"
        f.write_text(json.dumps({"seed": game.seed, "move": step, "what": what, "differences": diff,
                                 "moves": game.moves, "engine": engine_view(game), "game": gs},
                                indent=1, default=str))
        shown = str(f.relative_to(ROOT) if f.is_relative_to(ROOT) else f)
        with self.viewer.lock:
            rec.mismatch = {"move": step, "what": what, "differences": diff, "file": shown}
            rec.result = f"engine and game differ after move {step}"
            self.viewer.session["mismatches"] += 1
        return f"mismatch after move {step} ({what}): " + "; ".join(diff[:3]) + f". Details in {shown}"

    # -- the loop
    def loop(self) -> None:
        n = 0
        while not self.stop.is_set() and (not self.runs or n < self.runs):
            if self.viewer.restart_flag:
                self.viewer.restart_flag = False
                self.restart_game()
            if not self.connect():
                time.sleep(3)
                continue
            if self.engine_only:
                self.viewer.set_link("engine", "Engine only: no game connected")
            else:
                self.viewer.set_link("connected", "Your Balatro, live")
            sample = None if self.seed else newest_sample()
            if sample:
                seed, moves = sample["seed"], [tuple(m) for m in sample["moves"]]
                label, source = sample["_from"], "job"
            else:
                seed, moves, label, source = self.seed or fresh_seed(), None, "demo run (greedy bot)", "demo"
            try:
                ok, msg = self.play_run(seed, moves, label, source)
            except (OSError, RuntimeError) as exc:
                self.api = None
                ok, msg = False, f"lost the game: {exc}"
                with self.viewer.lock:
                    if self.viewer.run is not None and self.viewer.run.result is None:
                        self.viewer.run.result = msg
            self.say(f"[{label}] seed {seed}: {msg}")
            n += 1
            if self.seed:
                break
            end = time.time() + (6.0 if not ok else 2.0)
            while time.time() < end and not self.stop.is_set() and not self.viewer.next_flag:
                time.sleep(0.1)
            self.viewer.next_flag = False


# ---------------------------------------------------------------------- projections
class Analyst:
    """Projections for the current run's positions, newest first (one thread, low priority)."""

    def __init__(self, viewer: Viewer) -> None:
        self.viewer = viewer

    def task(self):
        with self.viewer.lock:
            rec = self.viewer.run
            if rec is None or not self.viewer.analysis:
                return None
            n = len(rec.move_list)
            if n not in rec.proj:
                return rec, n, list(rec.move_list)
            for k in range(n - 1, -1, -1):
                if k not in rec.proj:
                    return rec, k, list(rec.move_list[:k])
        return None

    def loop(self, stop: threading.Event) -> None:
        from balatro_meta._engine import Game

        while not stop.is_set():
            t = self.task()
            if t is None:
                time.sleep(0.2)
                continue
            rec, k, moves = t
            g = Game(rec.seed)
            for m in moves[:k]:
                g.step(*m)
            p = g.projection(48, 350, (hash(rec.id) & 0xFFFF) * 1000 + k)
            with self.viewer.lock:
                rec.proj[k] = {"mean_ante": round(p["mean_ante"], 3), "p_win": round(p["p_win"], 3),
                               "p_clear": round(p["p_clear"], 3), "n": p["playouts"],
                               "final_ante": p["final_ante"]}


# ---------------------------------------------------------------------- your PC
def pid_alive(pid) -> bool:
    try:
        os.kill(int(pid), 0)
        return True
    except (OSError, TypeError, ValueError):
        return False


_JOB_CACHE: list = [0.0, None]


def job_status() -> dict:
    """The job runner's note (.sync/watch/current.json), a job's progress file if it writes one, and the
    newest speed results (read at most every two seconds)."""
    if time.time() - _JOB_CACHE[0] < 2.0 and _JOB_CACHE[1] is not None:
        return _JOB_CACHE[1]
    out: dict = {"job": None, "progress": None, "bench": None}
    try:
        cur = json.loads((WATCH / "current.json").read_text())
        running = cur.get("state") == "running" and pid_alive(cur.get("pid"))
        out["job"] = {"name": cur.get("job"), "state": "running" if running else cur.get("state"),
                      "step": cur.get("step"), "steps": cur.get("steps", []), "started": cur.get("started"),
                      "commit": cur.get("commit"), "host": cur.get("host"), "time": cur.get("time")}
        prog = WATCH / str(cur.get("job")) / "progress.json"
        if prog.exists():
            out["progress"] = json.loads(prog.read_text())
    except (OSError, ValueError):
        pass
    try:
        b = json.loads((ROOT / "results" / "bench" / "engine.json").read_text())
        out["bench"] = {"moves_per_s": b.get("moves_per_s"), "threads": b.get("threads")}
    except (OSError, ValueError):
        pass
    _JOB_CACHE[0], _JOB_CACHE[1] = time.time(), out
    return out


# ---------------------------------------------------------------------- the server
MOD_MESSAGES = {
    "missing": "The viewer's mod isn't running in your game yet. Restart Balatro once so it loads "
               "(make watch installed it).",
    "old": "Your game is running an older version of the viewer's mod. Restart Balatro once to load the new one.",
    "stale": "Balatro isn't drawing at the moment (minimised?). The picture comes back when it does.",
}


def link_status(viewer: Viewer, frames: Frames | None) -> dict:
    """The game link as the page shows it: the driver's connection plus the mod's state now."""
    link = dict(viewer.link)
    mod = frames.mod_status() if frames else "none"
    link["mod"] = mod
    if frames is not None:
        with frames.lock:
            link["mod_error"] = (frames.state or {}).get("error")
    if link.get("game") == "connected" and mod in MOD_MESSAGES:
        link["game"] = "restart" if mod in ("missing", "old") else "stale"
        link["message"] = MOD_MESSAGES[mod]
    return link


def make_handler(viewer: Viewer, frames: Frames | None, driver: Driver):
    class Handler(BaseHTTPRequestHandler):
        def log_message(self, fmt, *args):  # keep the terminal quiet
            pass

        def send(self, body, ctype="application/json", code=200, headers=None):
            data = body if isinstance(body, bytes) else (body if isinstance(body, str) else json.dumps(body)).encode()
            self.send_response(code)
            self.send_header("Content-Type", ctype if ctype.startswith("image/") else ctype + "; charset=utf-8")
            self.send_header("Cache-Control", "no-store")
            for k, v in (headers or {}).items():
                self.send_header(k, v)
            self.send_header("Content-Length", str(len(data)))
            self.end_headers()
            self.wfile.write(data)

        def do_GET(self):
            url = urlparse(self.path)
            q = parse_qs(url.query)
            try:
                if url.path in ("/", "/index.html"):
                    self.send(PAGE.read_bytes(), "text/html")
                elif url.path == "/api/state":
                    since = int(q.get("since", ["0"])[0] or 0)
                    run_id = q.get("run", [""])[0]
                    with viewer.lock:
                        run = viewer.run
                        body = {
                            "now": time.time(), "host": os.uname().nodename,
                            "settings": {"paused": viewer.paused, "speed": viewer.speed, "analysis": viewer.analysis},
                            "link": link_status(viewer, frames),
                            "session": dict(viewer.session),
                            "run": run.public(since if run and run.id == run_id else 0) if run else None,
                            "last_run": {"seed": viewer.last_run.seed, "result": viewer.last_run.result,
                                         "mismatch": viewer.last_run.mismatch} if viewer.last_run else None,
                            "proj": {str(k): v for k, v in (run.proj.items() if run else [])},
                        }
                    body["pc"] = job_status()
                    self.send(body)
                elif url.path == "/api/frame":
                    seq, data = frames.latest_png() if frames else (None, None)
                    if data is None:
                        self.send(b"", "image/png", code=204)
                    elif q.get("have", [""])[0] == seq:
                        self.send(b"", "image/png", code=204, headers={"X-Seq": seq})
                    else:
                        self.send(data, "image/png", headers={"X-Seq": seq})
                elif url.path == "/api/snap":
                    run_id, n = q.get("run", [""])[0], int(q.get("n", ["0"])[0] or 0)
                    with viewer.lock:
                        run = viewer.run if viewer.run and viewer.run.id == run_id else None
                        data = run.snaps.get(n) if run else None
                    if data is None:
                        self.send({"error": "no picture for that move"}, code=404)
                    else:
                        self.send(data, "image/png")
                else:
                    self.send({"error": "not found"}, code=404)
            except (BrokenPipeError, ConnectionResetError):
                pass
            except Exception as exc:  # noqa: BLE001 - report, keep serving
                self.send({"error": f"{type(exc).__name__}: {exc}"}, code=500)

        def do_POST(self):
            url = urlparse(self.path)
            try:
                n = int(self.headers.get("Content-Length") or 0)
                body = json.loads(self.rfile.read(min(n, 1 << 16)) or b"{}") if n else {}
                if url.path != "/api/control" or not isinstance(body, dict):
                    self.send({"error": "not found"}, code=404)
                    return
                with viewer.lock:
                    if "paused" in body:
                        viewer.paused = bool(body["paused"])
                    if body.get("speed") in SPEEDS:
                        viewer.speed = body["speed"]
                    if "analysis" in body:
                        viewer.analysis = bool(body["analysis"])
                    if body.get("next"):
                        viewer.next_flag = True
                        viewer.paused = False
                    if body.get("restart"):
                        viewer.restart_flag = True
                driver.control()
                self.send({"ok": True})
            except (BrokenPipeError, ConnectionResetError):
                pass
            except Exception as exc:  # noqa: BLE001 - report, keep serving
                self.send({"error": f"{type(exc).__name__}: {exc}"}, code=500)

    return Handler


def open_browser(url: str) -> None:
    try:
        if "microsoft" in Path("/proc/version").read_text().lower():  # WSL: use the Windows browser
            subprocess.Popen(["explorer.exe", url], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            return
    except OSError:
        pass
    import webbrowser
    webbrowser.open(url)


def serve(port: int = DEFAULT_PORT, host: str = "127.0.0.1", speed: str = "max", engine_only: bool = False,
          launch: bool = True, seed: str = "", browser: bool = True, save_folder: Path | None = None) -> None:
    try:
        os.nice(10)  # a running job comes first
    except OSError:
        pass
    viewer = Viewer(speed)
    folder = save_folder if save_folder is not None else (None if engine_only else watch.save_dir())
    files = GameFiles(folder)
    frames = None if engine_only else Frames(files)
    if folder is not None and not engine_only:
        msg = watch.install_mod(folder)
        if msg:
            print(msg)
    driver = Driver(viewer, files, frames, engine_only=engine_only, launch=launch, seed=seed)
    stop = threading.Event()
    server = None
    for p in range(port, port + 20):
        try:
            server = ThreadingHTTPServer((host, p), make_handler(viewer, frames, driver))
            break
        except OSError:
            continue
    if server is None:
        raise SystemExit(f"No free port from {port} to {port + 19}")
    threads = [threading.Thread(target=server.serve_forever, daemon=True),
               threading.Thread(target=Analyst(viewer).loop, args=(stop,), daemon=True)]
    if frames:
        threads.append(threading.Thread(target=frames.loop, args=(stop, viewer), daemon=True))

    def keep_control():
        while not stop.is_set():
            driver.control()
            time.sleep(1.0)

    if not engine_only:
        threads.append(threading.Thread(target=keep_control, daemon=True))
    for t in threads:
        t.start()
    url = f"http://localhost:{server.server_address[1]}"
    print(f"Watching on {url}  (Ctrl+C stops; the game stays open). If no browser window opens, paste that "
          "address into one.", flush=True)
    if browser:
        open_browser(url)
    try:
        driver.loop()
        while True:  # after a fixed number of runs, keep the page up until Ctrl+C
            time.sleep(1)
    except KeyboardInterrupt:
        print("\nStopped. The game stays open and takes input again in a few seconds.")
    finally:
        stop.set()
        driver.stop.set()
        driver.release()
        server.shutdown()
