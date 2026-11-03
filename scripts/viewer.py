"""The Balatro Meta viewer: thousands of runs a second in the background, one of them up close.

    make watch            opens the viewer in your browser (Ctrl+C stops it; a running job carries on)
    make watch SPEED=2    start the replay at another speed (1, 2, 4 or max; the page can change it)

Like Monopoly Meta's viewer. A job (make baseline, later make train) plays runs on every thread and,
every few seconds, saves one it has just finished (its seed and every move) in .sync/watch/<job>/samples.
The viewer replays that very run, move by move, with the same engine, and the page draws it: the blind
and its target, the score, the hand, the jokers, the shop and the packs. Around it: the job's live
numbers (runs played, runs a second, win rate, how runs end, the jokers and hands it favours), the run's
outlook (quick playouts from fair copies of each position: hidden cards reshuffled, fresh random
streams), the moves so far, and the real-game checks, in which a hidden copy of your Balatro replays runs
at full speed and compares every move with the engine (scripts/checker.py). With no job running it shows
demo runs by the greedy bot.

The page draws the run with the art from your own Balatro: the viewer reads the sheets and the font from
your installed Balatro.exe at start-up (scripts/art.py), keeps them in memory and serves them to a browser on
this PC only. Nothing of the game is stored anywhere or enters the repo. Without the game, the page draws its
own cards.

The viewer never touches a job: it only reads the job's files, at the lowest priority.

Its Progress panel charts every bot measured on this PC (results/progress.jsonl, which the job runner adds to as
each measuring step finishes) against the 90% goal, with the running job's win rate so far as a live point.
"""
from __future__ import annotations

import json
import os
import subprocess
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import parse_qs, urlparse

from watch import ROOT, STAKE_NAMES, WATCH, card_name, describe, fresh_seed, stake_of

PAGE = ROOT / "viewer" / "index.html"
LOGS = ROOT / ".sync" / "logs"
RESULTS = ROOT / "results"
PROGRESS = RESULTS / "progress.jsonl"  # every bot a job measured (scripts/jobs.py appends to it)
GOAL = 0.9  # the win rate the project is after (White Stake)
DEFAULT_PORT = 8790
# Seconds between replayed moves at each of the page's speeds (a played hand stays up a little longer).
SPEEDS = {"1": 1.1, "2": 0.55, "4": 0.28, "max": 0.04}
DEFAULT_SPEED = "1"
SAMPLE_FRESH = 900  # a job's typical run is replayed if it is at most this many seconds old
PICKS = ("best", "typical")  # which of the job's runs the replay shows first


# ---------------------------------------------------------------------- the run on screen
def summary(v: dict) -> dict:
    """The engine's state as the page's charts and lists use it."""
    return {"state": v["state"], "ante": v["ante"], "round": v["round"], "money": v["money"],
            "chips": v["chips"], "target": v.get("blind_score"), "blind": v.get("blind_name"),
            "hands_left": v["hands_left"], "discards_left": v["discards_left"], "won": v["won"]}


def details(v: dict) -> dict:
    """Everything the page draws about the run now."""
    keep = ("state", "ante", "round", "money", "chips", "blind_score", "blind_name", "blind_reward",
            "blind_disabled", "hands_left", "discards_left", "hand_size", "joker_slots", "consumable_slots",
            "hand", "jokers", "consumables", "vouchers", "tags", "hand_levels", "last_hand", "deck_left",
            "deck_total", "reroll_cost", "won", "hands_played_run", "blinds", "cash_out", "pack_key",
            "pack_choices_left", "shop", "shop_vouchers", "pack", "deck_cards", "voucher_keys")
    out = {k: v.get(k) for k in keep}
    out["shop_packs"] = [p for p in (v.get("shop_packs") or []) if p]
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


def chosen_cards(hand: list[dict], mask: int) -> list[dict]:
    return [c for i, c in enumerate(hand) if mask >> i & 1]


class RunRecord:
    def __init__(self, seed: str, label: str, source: str, bot: str, expected: str | None = None,
                 stake: int = 1) -> None:
        self.id = f"{seed}-{time.time():.3f}"
        self.seed, self.label, self.source, self.bot = seed, label, source, bot
        self.stake = stake
        self.expected = expected  # what the job recorded for this run
        self.started = time.time()
        self.pick: str | None = None  # "best" or "typical" (from a job), None for a demo
        self.rank: int | None = None  # place among the job's best runs when the replay began
        self.moves: list[dict] = []
        self.move_list: list[tuple[int, int]] = []
        self.start: dict | None = None
        self.view: dict | None = None
        self.result: str | None = None
        self.proj: dict[int, dict] = {}

    def public(self, since: int = 0) -> dict:
        return {"id": self.id, "seed": self.seed, "stake": self.stake, "label": self.label, "source": self.source,
                "bot": self.bot,
                "expected": self.expected, "pick": self.pick, "rank": self.rank, "started": self.started,
                "result": self.result,
                "start": self.start, "n_moves": len(self.moves), "moves": self.moves[since:], "view": self.view}


class Viewer:
    """Everything the page reads and the replay writes."""

    def __init__(self, speed: str = DEFAULT_SPEED) -> None:
        self.lock = threading.RLock()
        self.paused = False
        self.speed = speed if speed in SPEEDS else DEFAULT_SPEED
        self.analysis = True
        self.analysis_error: str | None = None
        self.pick = "best"
        self.request: Path | None = None  # a run the page asked to see next
        self.next_flag = False
        self.session = {"started": time.time(), "runs": 0, "moves": 0}
        self.run: RunRecord | None = None
        self.last_run: RunRecord | None = None

    def new_run(self, seed: str, label: str, source: str, bot: str, expected: str | None = None,
                stake: int = 1) -> RunRecord:
        with self.lock:
            if self.run is not None:
                self.last_run = self.run
            self.run = RunRecord(seed, label, source, bot, expected, stake)
            return self.run

    def take_next(self) -> bool:
        with self.lock:
            flag, self.next_flag = self.next_flag, False
            return flag


def view_at(seed: str, moves: list[tuple[int, int]], n: int, stake: int = 1) -> dict:
    """The run as it was after its first n moves (replayed with the engine, at the run's stake)."""
    from balatro_meta._engine import Game

    g = Game(seed, stake)
    for k, a in moves[:n]:
        g.step(k, a)
    return details(g.view())


# ---------------------------------------------------------------------- the jobs' files
def pid_alive(pid) -> bool:
    try:
        os.kill(int(pid), 0)
        return True
    except (OSError, TypeError, ValueError):
        return False


def tail(path: Path, n: int = 6) -> list[str]:
    try:
        with path.open("rb") as f:
            f.seek(0, 2)
            f.seek(max(0, f.tell() - 8192))
            lines = f.read().decode(errors="replace").splitlines()
    except OSError:
        return []
    return [line.rstrip() for line in lines if line.strip() and not line.startswith("===")][-n:]


class JobFiles:
    """What the jobs leave for the viewer: .sync/watch/current.json (which job and step is running, from
    scripts/jobs.py), <job>/progress.json (its live numbers) and <job>/samples/*.json (runs to replay)."""

    def __init__(self, watch: Path = WATCH, logs: Path = LOGS) -> None:
        self.watch, self.logs = watch, logs
        self.lock = threading.Lock()
        self.cache: tuple[float, dict] = (0.0, {})
        self.shown: set[str] = set()

    def status(self) -> dict:
        """The job, its progress and the last lines of its log (read at most once a second)."""
        with self.lock:
            if time.time() - self.cache[0] < 1.0:
                return self.cache[1]
        out: dict = {"job": None, "progress": None, "log": [], "bench": None}
        try:
            cur = json.loads((self.watch / "current.json").read_text())
            running = cur.get("state") == "running" and pid_alive(cur.get("pid"))
            state = "running" if running else ("stopped" if cur.get("state") == "running" else cur.get("state"))
            out["job"] = {"name": cur.get("job"), "state": state, "step": cur.get("step"),
                          "steps": cur.get("steps", []), "started": cur.get("started"), "commit": cur.get("commit"),
                          "host": cur.get("host"), "time": cur.get("time")}
            prog = self.watch / str(cur.get("job")) / "progress.json"
            if prog.exists():
                out["progress"] = json.loads(prog.read_text())
            if cur.get("step"):
                out["log"] = tail(self.logs / f"{cur.get('job')}-{cur.get('step')}.log")
        except (OSError, ValueError):
            pass
        try:
            b = json.loads((ROOT / "results" / "bench" / "engine.json").read_text())
            out["bench"] = {"moves_per_s": b.get("moves_per_s"), "threads": b.get("threads")}
        except (OSError, ValueError):
            pass
        with self.lock:
            self.cache = (time.time(), out)
        return out

    def samples(self, fresh: float | None = SAMPLE_FRESH, pattern: str = "*.json") -> list[Path]:
        """The jobs' saved runs, newest first (only those newer than `fresh` seconds, if given)."""
        now = time.time()
        files = []
        for p in self.watch.glob(f"*/samples/{pattern}"):
            try:
                m = p.stat().st_mtime
            except OSError:
                continue
            if fresh is None or now - m <= fresh:
                files.append((m, p))
        return [p for _, p in sorted(files, reverse=True)]

    def running_job(self) -> str | None:
        st = self.status().get("job") or {}
        return st.get("name") if st.get("state") == "running" else None

    def best(self) -> list[tuple[tuple, Path]]:
        """The best runs the jobs kept (best-*.json), best first: the running job's if one runs, else the
        newest job's. Won first; among wins the biggest hand; among losses the furthest blind, then the
        closest to its target."""
        files = self.samples(fresh=None, pattern="best-*.json")
        job = self.running_job() or (files[0].parent.parent.name if files else None)
        out = []
        for p in files:
            if p.parent.parent.name != job:
                continue
            s = self.read_sample(p)
            if not s:
                continue
            won = bool(s.get("won"))
            hand = float(s.get("best_hand") or 0)
            key = (won, int(s.get("stage") or 0), hand if won else float(s.get("closeness") or 0), hand)
            out.append((key, p))
        out.sort(key=lambda kp: kp[0], reverse=True)
        return out

    def sample_path(self, name: str) -> Path | None:
        """A saved run named as '<job>/<file>' (only files in a job's samples folder)."""
        import re

        if not re.fullmatch(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+\.json", name or "") or ".." in name:
            return None
        job, file = name.split("/")
        p = self.watch / job / "samples" / file
        return p if p.is_file() else None

    def read_sample(self, p: Path) -> dict | None:
        try:
            s = json.loads(p.read_text())
        except (OSError, ValueError):
            return None
        if not s.get("seed") or not isinstance(s.get("moves"), list):
            return None
        s.setdefault("label", f"run from {p.parent.parent.name}")
        s["_job"] = p.parent.parent.name
        s["_file"] = str(p)
        return s

    def next_sample(self, pick: str = "best") -> dict | None:
        """The next run to replay. "best": the best run kept that hasn't been shown yet (when every one
        has, the list starts again from the top). "typical": the newest typical run not shown yet."""
        if pick == "best":
            best = self.best()
            for _, p in best:
                if str(p) not in self.shown:
                    self.shown.add(str(p))
                    return self.read_sample(p)
            if best:
                self.shown -= {str(p) for _, p in best}
                _, p = best[0]
                self.shown.add(str(p))
                return self.read_sample(p)
        for p in self.samples():
            if str(p) in self.shown or p.name.startswith("best-"):
                continue
            s = self.read_sample(p)
            self.shown.add(str(p))
            if s:
                return s
        return None


# ---------------------------------------------------------------------- progress
def read_progress(path: Path = PROGRESS, results: Path = RESULTS) -> list[dict]:
    """Every bot measured on this PC, oldest first: the lines of results/progress.jsonl (scripts/jobs.py adds one
    as each measuring step finishes), plus any finished report in results/ that it doesn't list (a job that ran
    on code from before the log existed)."""
    entries: list[dict] = []
    try:
        for line in path.read_text().splitlines():
            try:
                e = json.loads(line)
            except ValueError:
                continue
            if isinstance(e, dict) and e.get("runs"):
                entries.append(e)
    except OSError:
        pass
    listed = {(e.get("out"), e.get("runs"), e.get("wins")) for e in entries}
    for p in sorted(results.rglob("*.json")):
        try:
            r = json.loads(p.read_text())
        except (OSError, ValueError):
            continue
        if not isinstance(r, dict) or r.get("kind") != "baseline" or not r.get("done") or not r.get("runs"):
            continue
        out = str(p.relative_to(results.parent)) if p.is_relative_to(results.parent) else str(p)
        if (out, r.get("runs"), r.get("wins")) in listed:
            continue
        when = r.get("updated") or p.stat().st_mtime
        entries.append({"time": time.strftime("%Y-%m-%dT%H:%M:%S+00:00", time.gmtime(float(when))), "job": None,
                        "step": None, "code": None, "title": None, "out": out, "bot": r.get("bot"), "seed": None,
                        "value_net": None, "value_weight": None, "runs": r.get("runs"), "wins": r.get("wins"),
                        "win_rate": r.get("win_rate"), "win_ci": r.get("win_ci"), "mean_ante": r.get("mean_ante")})
    entries.sort(key=lambda e: str(e.get("time") or ""))
    return entries


class Progress:
    """read_progress(), read again at most every few seconds."""

    def __init__(self, path: Path = PROGRESS, results: Path = RESULTS, every: float = 5.0) -> None:
        self.path, self.results, self.every = path, results, every
        self.lock = threading.Lock()
        self.cache: tuple[float, list[dict]] = (0.0, [])

    def get(self) -> dict:
        with self.lock:
            if time.time() - self.cache[0] >= self.every:
                self.cache = (time.time(), read_progress(self.path, self.results))
            return {"goal": GOAL, "entries": self.cache[1]}


# ---------------------------------------------------------------------- the replay
class Replayer:
    """Replays runs move by move at the page's speed: the newest run a job saved, or a demo run."""

    def __init__(self, viewer: Viewer, jobs: JobFiles) -> None:
        self.viewer, self.jobs = viewer, jobs

    def wait(self, seconds: float, stop: threading.Event) -> None:
        end = time.time() + seconds
        while time.time() < end and not stop.is_set() and not self.viewer.next_flag:
            time.sleep(0.02)

    def play(self, stop: threading.Event, seed: str, moves, label: str, source: str, bot: str,
             expected: str | None = None, pick: str | None = None, rank: int | None = None,
             stake: int = 1) -> str:
        from balatro_meta._engine import Game

        game = Game(seed, stake)  # a run plays again only at the stake it was played at
        v = self.viewer
        if stake != 1:
            label = f"{label}, {STAKE_NAMES[stake - 1]}"
        rec = v.new_run(game.seed, label, source, bot, expected, stake)
        with v.lock:
            rec.pick, rec.rank = pick, rank
            rec.start = summary(game.view())
            rec.view = details(game.view())
        step = 0
        while not game.over and step < 5000 and not stop.is_set():
            while v.paused and not stop.is_set() and not v.next_flag:
                time.sleep(0.05)
            if v.take_next():
                with v.lock:
                    rec.result = f"skipped after {step} moves"
                return rec.result
            if moves is None:
                kind, arg = game.greedy_move()
            elif step < len(moves):
                kind, arg = moves[step]
            else:
                break
            before = game.view()
            what = describe(kind, arg, game)
            if not game.step(kind, arg):
                with v.lock:
                    rec.result = f"the engine refused move {step + 1} ({what})"
                return rec.result
            after = game.view()
            move = {"n": step + 1, "kind": kind, "what": what, "detail": outcome(kind, before, after),
                    "t": time.time(), **summary(after)}
            if kind in (6, 7):
                move["cards"] = chosen_cards(before["hand"], arg)
            if kind == 6:
                move["hand"] = after.get("last_hand")
                move["score"] = after["chips"] - before["chips"]
            if after["state"] == "SHOP" and before["state"] != "SHOP":
                move["shows"] = [card_name(i) for i in after.get("shop") or []]
            elif after["state"] == "SMODS_BOOSTER_OPENED":
                move["shows"] = [card_name(i) for i in after.get("pack") or []]
            with v.lock:
                rec.move_list.append((kind, arg))
                rec.moves.append(move)
                rec.view = details(after)
                v.session["moves"] += 1
            step += 1
            self.wait(SPEEDS[v.speed] * (1.6 if kind == 6 else 1.0), stop)
        end = game.view()
        result = "won the run" if end["won"] else f"run over at ante {end['ante']}"
        with v.lock:
            rec.result = f"{result} after {step} moves"
            v.session["runs"] += 1
        return rec.result

    def pick_run(self) -> tuple[dict | None, int | None]:
        """The run to replay next (asked for by the page, else by the page's choice of runs) and its place
        among the job's best runs."""
        with self.viewer.lock:
            asked, self.viewer.request = self.viewer.request, None
            pick = self.viewer.pick
        s = self.jobs.read_sample(asked) if asked else self.jobs.next_sample(pick)
        rank = None
        if s and s.get("pick") == "best":
            rank = next((i + 1 for i, (_, p) in enumerate(self.jobs.best()) if str(p) == s.get("_file")), None)
        return s, rank

    def loop(self, stop: threading.Event) -> None:
        while not stop.is_set():
            s, rank = self.pick_run()
            try:
                if s:
                    self.play(stop, s["seed"], [tuple(m) for m in s["moves"]], s.get("label") or "a job's run",
                              "job", s.get("bot") or "the job's bot", s.get("result"), s.get("pick") or "typical",
                              rank, stake_of(s))
                else:
                    self.play(stop, fresh_seed(), None, "demo run (greedy bot)", "demo", "greedy bot")
            except Exception as exc:  # noqa: BLE001 - a bad sample must never stop the viewer
                with self.viewer.lock:
                    if self.viewer.run is not None and self.viewer.run.result is None:
                        self.viewer.run.result = f"replay failed: {type(exc).__name__}: {exc}"
            self.viewer.next_flag = False
            self.wait(2.5 if self.viewer.speed != "max" else 0.5, stop)


# ---------------------------------------------------------------------- projections
class Analyst:
    """Projections for the run on screen, newest position first (one thread, low priority)."""

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

    def fail(self, message: str) -> None:
        with self.viewer.lock:
            self.viewer.analysis_error = message

    def loop(self, stop: threading.Event) -> None:
        try:
            from balatro_meta._engine import Game
        except ImportError as exc:
            self.fail(f"The engine module isn't built ({exc}). Run make build.")
            return
        if not hasattr(Game, "projection"):
            self.fail("The engine module is older than this viewer. Stop the viewer and run make watch again "
                      "(it rebuilds the engine first).")
            return
        while not stop.is_set():
            t = self.task()
            if t is None:
                time.sleep(0.2)
                continue
            rec, k, moves = t
            try:
                g = Game(rec.seed, rec.stake)
                for m in moves[:k]:
                    g.step(*m)
                salt = (hash(rec.id) & 0xFFFF) * 1000 + k
                p = g.projection(48, 350, salt)
                out = {"mean_ante": round(p["mean_ante"], 3), "p_win": round(p["p_win"], 3),
                       "p_clear": round(p["p_clear"], 3), "n": p["playouts"], "final_ante": p["final_ante"]}
                # The teacher's own view of the position (its value model), when the engine has it.
                v = g.value_view(salt) if hasattr(g, "value_view") else None
                if v:
                    out["teacher"] = {"chances": [[round(t), round(c, 3)] for t, c in v["chances"]],
                                      "main": v["main"], "total": round(v["total"]),
                                      "plan": list(v.get("plan", []))}
            except Exception as exc:  # noqa: BLE001 - a failed projection is shown, never fatal
                self.fail(f"Projection failed at move {k}: {type(exc).__name__}: {exc}")
                time.sleep(5)
                continue
            with self.viewer.lock:
                rec.proj[k] = out
                self.viewer.analysis_error = None


# ---------------------------------------------------------------------- the server
LOOPBACK = {"127.0.0.1", "::1", "::ffff:127.0.0.1"}


def make_handler(viewer: Viewer, jobs: JobFiles, checks=None, art=None, progress: Progress | None = None):
    """`checks` is anything with a summary() (scripts/checker.py's Checker), or None; `art` is your game's
    pictures (scripts/art.py's GameArt), or None."""
    progress = progress or Progress()

    class Handler(BaseHTTPRequestHandler):
        def log_message(self, fmt, *args):  # keep the terminal quiet
            pass

        def send(self, body, ctype="application/json", code=200):
            data = body if isinstance(body, bytes) else (body if isinstance(body, str) else json.dumps(body)).encode()
            self.send_response(code)
            text = ctype.startswith(("text/", "application/json"))
            self.send_header("Content-Type", ctype + ("; charset=utf-8" if text else ""))
            self.send_header("Cache-Control", "no-store")
            self.send_header("Content-Length", str(len(data)))
            self.end_headers()
            self.wfile.write(data)

        def send_art(self, path: str) -> None:
            """Your game's pictures and font, to a browser on this PC only (never stored anywhere)."""
            if art is None or self.client_address[0] not in LOOPBACK:
                self.send({"error": "no game art here"}, code=404)
            elif path == "/art/manifest.json":
                self.send(art.manifest())
            elif path == "/art/font.ttf" and art.font():  # (no-store: the browser keeps no copy on disk)
                self.send(art.font(), "font/ttf")
            elif path.startswith("/art/sheet/") and path.endswith(".png") and art.sheet(path[11:-4]):
                self.send(art.sheet(path[11:-4]), "image/png")
            else:
                self.send({"error": "not found"}, code=404)

        def do_GET(self):
            url = urlparse(self.path)
            q = parse_qs(url.query)
            try:
                if url.path in ("/", "/index.html"):
                    self.send(PAGE.read_bytes(), "text/html")
                elif url.path.startswith("/art/"):
                    self.send_art(url.path)
                elif url.path == "/api/state":
                    since = int(q.get("since", ["0"])[0] or 0)
                    run_id = q.get("run", [""])[0]
                    with viewer.lock:
                        run = viewer.run
                        body = {
                            "now": time.time(), "host": os.uname().nodename,
                            "settings": {"paused": viewer.paused, "speed": viewer.speed, "analysis": viewer.analysis,
                                         "analysis_error": viewer.analysis_error, "pick": viewer.pick},
                            "session": dict(viewer.session),
                            "run": run.public(since if run and run.id == run_id else 0) if run else None,
                            "last_run": {"seed": viewer.last_run.seed, "result": viewer.last_run.result}
                            if viewer.last_run else None,
                            "proj": {str(k): v for k, v in (run.proj.items() if run else [])},
                        }
                    body["pc"] = jobs.status()
                    body["checks"] = checks.summary() if checks is not None else None
                    self.send(body)
                elif url.path == "/api/progress":
                    self.send(progress.get())
                elif url.path == "/api/view":
                    run_id, n = q.get("run", [""])[0], int(q.get("n", ["0"])[0] or 0)
                    with viewer.lock:
                        run = viewer.run if viewer.run and viewer.run.id == run_id else None
                        seed, moves, stake = (run.seed, list(run.move_list), run.stake) if run else (None, [], 1)
                    if seed is None or not 0 <= n <= len(moves):
                        self.send({"error": "no such move in the run on screen"}, code=404)
                    else:
                        self.send(view_at(seed, moves, n, stake))
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
                    if body.get("pick") in PICKS and body["pick"] != viewer.pick:
                        viewer.pick = body["pick"]
                        viewer.next_flag = True  # show that kind of run now
                    if body.get("play"):
                        path = jobs.sample_path(str(body["play"]))
                        if path is None:
                            self.send({"error": "no such saved run"}, code=404)
                            return
                        viewer.request = path
                        viewer.next_flag = True
                        viewer.paused = False
                    if body.get("next"):
                        viewer.next_flag = True
                        viewer.paused = False
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


def serve(port: int = DEFAULT_PORT, host: str = "127.0.0.1", speed: str = DEFAULT_SPEED, checks: bool = True,
          launch: bool = True, browser: bool = True) -> None:
    try:
        os.nice(10)  # a running job comes first
    except OSError:
        pass
    viewer = Viewer(speed)
    jobs = JobFiles()
    import art as art_mod

    art = art_mod.load()
    print("Drawing the run with the art from your Balatro." if art else
          "Your Balatro wasn't found, so the page draws its own cards.", flush=True)
    checker = None
    if checks:
        import checker as checker_mod
        import watch

        folder = watch.save_dir()
        if folder is not None:
            msg = watch.install_mod(folder)
            if msg:
                print(msg, flush=True)
        checker = checker_mod.Checker(jobs=jobs, launch=launch)
    server = None
    for p in range(port, port + 20):
        try:
            server = ThreadingHTTPServer((host, p), make_handler(viewer, jobs, checker, art))
            break
        except OSError:
            continue
    if server is None:
        raise SystemExit(f"No free port from {port} to {port + 19}")
    stop = threading.Event()
    threads = [threading.Thread(target=server.serve_forever, daemon=True),
               threading.Thread(target=Replayer(viewer, jobs).loop, args=(stop,), daemon=True),
               threading.Thread(target=Analyst(viewer).loop, args=(stop,), daemon=True)]
    if checker is not None:
        threads.append(threading.Thread(target=checker.loop, args=(stop,), daemon=True))
    for t in threads:
        t.start()
    url = f"http://localhost:{server.server_address[1]}"
    print(f"Watching on {url}  (Ctrl+C stops the viewer; a running job carries on). If no browser window "
          "opens, paste that address into one.", flush=True)
    if browser:
        open_browser(url)
    try:
        while True:
            time.sleep(1)
    except KeyboardInterrupt:
        print("\nStopping the viewer" + (" and the hidden game" if checker is not None else "") + ".", flush=True)
    finally:
        stop.set()
        if checker is not None:
            checker.close()
        server.shutdown()
