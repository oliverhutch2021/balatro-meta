"""The real-game checks: a hidden copy of your Balatro replays runs at full speed, and after every move the
game is compared with the engine. The viewer (make watch) starts them and stops them with itself.

The hidden game is BalatroBot's own launcher with --headless --fast on a port of its own (12350), so it
never touches a Balatro you have open; its window, which Balatro opens while it loads (full screen if
that's how you play), is hidden as soon as it appears (hide_window_script). The companion mod
(mods/balatro_meta_watch) tells the checker when the game has finished playing out a move, and readies
the run-setup deck BalatroBot's `start` needs. After every move the game's money, hands, discards, cards
in hand, round chips, blinds and tags, jokers and consumables (with their sell values), the shop's cards,
vouchers, packs and prices, and an open pack's cards are compared with the engine's. A difference is
saved to .sync/watch/mismatches/ with both states, and the checks move on to the next run. The runs
checked are the ones the running job saved for the viewer (else demo runs by the greedy bot). BalatroBot
has no call for "buy and use" or a boss reroll: a check stops (without a difference) before either.
"""
from __future__ import annotations

import json
import os
import shutil
import subprocess
import threading
import time
from pathlib import Path

import watch
from watch import (
    ROOT,
    WATCH,
    Api,
    apply_in_game,
    compare,
    describe,
    engine_view,
    fresh_seed,
    game_view,
    hand_permutation,
    settle,
    unsupported,
)

CHECK_PORT = 12350
RECORD_PORT = 12351  # make record-me's game (scripts/record_me.py): the checks wait while it is up
MOD_VERSION = "0.5.0"
# Overnight: Balatro's memory grows over a long session and can run out, so the hidden game gets a fresh
# start every hour, or sooner when it gets big, and at once when it stops answering.
RESTART_EVERY = 3600  # seconds
MEMORY_LIMIT_MB = 3000  # working set (checked every few minutes)
MEMORY_CHECK_EVERY = 300  # seconds
CALL_TIMEOUT = 30  # seconds for one BalatroBot call: longer means the game is stuck
KEEP_LOGS = 2  # log files of earlier hidden-game sessions kept (each grows about 100 MB an hour)


class ModFiles:
    """The companion mod's files for the game on one port: <save>/balatro_meta/<port>/control.txt (ours) and
    state.txt (the mod's)."""

    def __init__(self, folder: Path | None, port: int = CHECK_PORT) -> None:
        self.dir = folder / "balatro_meta" / str(port) if folder else None
        self.n = 0
        self.lock = threading.Lock()

    def write_control(self, **values) -> None:
        if not self.dir:
            return
        with self.lock:
            self.n += 1
            text = "\n".join([f"n={self.n}"] + [f"{k}={v}" for k, v in values.items()]) + "\n"
            for _ in range(3):  # Windows refuses the swap while the game is reading the file: try again
                try:
                    self.dir.mkdir(parents=True, exist_ok=True)
                    tmp = self.dir / "control.tmp"
                    tmp.write_text(text)
                    os.replace(tmp, self.dir / "control.txt")
                    return
                except OSError:
                    time.sleep(0.01)

    def read_state(self) -> dict | None:
        """The mod's newest report, or None (missing, or read while the mod was rewriting it)."""
        if not self.dir:
            return None
        try:
            text = (self.dir / "state.txt").read_text(errors="replace")
        except OSError:
            return None
        out = dict(line.split("=", 1) for line in text.splitlines() if "=" in line)
        return out if "end" in out else None


HIDE_FOR = 180  # seconds after a launch that its window is kept hidden (the game loads in well under that)


def hide_window_script(port: int, seconds: int = HIDE_FOR) -> str:
    """PowerShell that keeps the hidden game's window out of sight while it loads. Balatro opens its window
    as your own game is set up (often full screen and black until it has loaded), before BalatroBot's
    headless mode takes over. Every 50 ms this hides any window of a Balatro.exe started by the BalatroBot
    launcher on this port; a Balatro you play yourself (Steam's, or make record-me's on its own port) is
    never touched. Hiding only stops the window being drawn on screen: the game runs as before."""
    return (
        "Add-Type -Namespace BM -Name Win -MemberDefinition "
        "'[DllImport(\"user32.dll\")] public static extern bool ShowWindow(System.IntPtr h, int n);'; "
        f"$port = {int(port)}; $ours = @{{}}; $until = (Get-Date).AddSeconds({int(seconds)}); "
        "function Test-Ours($id) { for ($i = 0; $i -lt 6 -and $id; $i++) { "
        "$p = Get-CimInstance Win32_Process -Filter \"ProcessId=$id\" -ErrorAction SilentlyContinue; "
        "if (-not $p) { return $false }; "
        "if ($p.CommandLine -like '*balatrobot*' -and $p.CommandLine -match ('--port\\s+' + $port + '(\\s|$)')) "
        "{ return $true }; "
        "$id = $p.ParentProcessId }; return $false }; "
        "while ((Get-Date) -lt $until) { "
        "foreach ($g in Get-Process -Name Balatro -ErrorAction SilentlyContinue) { "
        "if (-not $ours.ContainsKey($g.Id)) { $ours[$g.Id] = Test-Ours $g.Id }; "
        "if ($ours[$g.Id] -and $g.MainWindowHandle -ne 0) { [BM.Win]::ShowWindow($g.MainWindowHandle, 0) | Out-Null } }; "
        "Start-Sleep -Milliseconds 50 }"
    )


class HiddenGame:
    """The headless Balatro the checks use, started through BalatroBot's launcher (uvx on Windows), its
    window kept out of sight while it loads (`hide_window_script`)."""

    def __init__(self, port: int = CHECK_PORT, save_folder: Path | None = None) -> None:
        self.port = port
        self.save_folder = save_folder
        self.proc: subprocess.Popen | None = None
        self.hider: subprocess.Popen | None = None
        self.launched_at = 0.0

    def command(self) -> list[str] | None:
        """The launch command, or None if uvx or the game can't be found."""
        try:
            from rng_game import find_balatro, sh, windows_profile
        except ImportError:
            return None
        prof = windows_profile()
        uvx = prof / ".local" / "bin" / "uvx.exe" if prof else None
        if not uvx or not uvx.exists():
            return None
        game = find_balatro()
        lovely = game / "winmm.dll" if (game / "winmm.dll").exists() else game / "version.dll"
        cmd = [str(uvx), "balatrobot", "serve", "--headless", "--fast", "--port", str(self.port),
               "--balatro-path", sh("wslpath", "-w", str(game)),
               "--love-path", sh("wslpath", "-w", str(game / "Balatro.exe")),
               "--lovely-path", sh("wslpath", "-w", str(lovely))]
        if self.save_folder is not None:
            logs = self.save_folder / "balatro_meta" / "logs"
            logs.mkdir(parents=True, exist_ok=True)
            cmd += ["--logs-path", sh("wslpath", "-w", str(logs))]
        return cmd

    def launch(self) -> bool:
        cmd = self.command()
        if cmd is None:
            return False
        self.prune_logs()
        self.launched_at = time.time()
        try:  # first, so it is watching before the window opens
            self.hider = subprocess.Popen(["powershell.exe", "-NoProfile", "-NonInteractive", "-WindowStyle",
                                           "Hidden", "-Command", hide_window_script(self.port)], cwd="/mnt/c",
                                          stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                                          start_new_session=True)
        except OSError:
            self.hider = None  # the game still starts; its window just shows while it loads
        self.proc = subprocess.Popen(cmd, cwd="/mnt/c", stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                                     start_new_session=True)
        return True

    def stop(self) -> None:
        """Close the hidden game and its launcher (found by the launcher's command line and the port)."""
        script = (f"Get-CimInstance Win32_Process | Where-Object {{ $_.CommandLine -like '*balatrobot*serve*' "
                  f"-and $_.CommandLine -like '*--port {self.port}*' }} | ForEach-Object {{ "
                  f"taskkill.exe /PID $_.ProcessId /T /F | Out-Null }}; "
                  f"Get-NetTCPConnection -LocalPort {self.port} -State Listen -ErrorAction SilentlyContinue | "
                  f"ForEach-Object {{ taskkill.exe /PID $_.OwningProcess /T /F | Out-Null }}")
        try:
            subprocess.run(["powershell.exe", "-NoProfile", "-NonInteractive", "-Command", script], cwd="/mnt/c",
                           capture_output=True, timeout=30, check=False)
        except (OSError, subprocess.TimeoutExpired):
            pass
        for proc in (self.proc, self.hider):
            if proc is not None and proc.poll() is None:
                proc.terminate()
        self.proc = self.hider = None

    def memory_mb(self) -> float | None:
        """How much memory the hidden game uses (its working set), or None if that can't be read."""
        script = (f"$c = Get-NetTCPConnection -LocalPort {self.port} -State Listen -ErrorAction SilentlyContinue | "
                  f"Select-Object -First 1; if ($c) {{ (Get-Process -Id $c.OwningProcess).WorkingSet64 }}")
        try:
            r = subprocess.run(["powershell.exe", "-NoProfile", "-NonInteractive", "-Command", script], cwd="/mnt/c",
                               capture_output=True, text=True, timeout=30, check=False)
            return int(r.stdout.strip().splitlines()[-1]) / 2**20
        except (OSError, subprocess.TimeoutExpired, ValueError, IndexError):
            return None

    def names_this_port(self, log: Path) -> bool:
        """A Lovely log written by the hidden game (BalatroBot names its port early on)."""
        try:
            with log.open("rb") as f:
                return f"127.0.0.1:{self.port}".encode() in f.read(1 << 20)
        except OSError:
            return False

    def prune_logs(self) -> None:
        """Logs of earlier hidden-game sessions, all but the newest few: BalatroBot's (in our own folder) and
        Lovely's (only files that name this port, so a Balatro you play yourself keeps all of its logs)."""
        if self.save_folder is None:
            return

        def newest_last(paths):
            out = []
            for p in paths:
                try:
                    out.append((p.stat().st_mtime, p))
                except OSError:
                    continue
            return [p for _, p in sorted(out)]

        for d in newest_last(d for d in (self.save_folder / "balatro_meta" / "logs").glob("*") if d.is_dir())[:-KEEP_LOGS]:
            shutil.rmtree(d, ignore_errors=True)
        lovely = newest_last((self.save_folder / "Mods" / "lovely" / "log").glob("lovely-*.log"))
        for f in [f for f in lovely if self.names_this_port(f)][:-KEEP_LOGS]:
            try:
                f.unlink()
            except OSError:
                pass  # still open: next time


class Checker:
    """Replays runs in the hidden game and in the engine together, comparing after every move."""

    def __init__(self, jobs=None, port: int = CHECK_PORT, launch: bool = True, api: Api | None = None,
                 files: ModFiles | None = None, say=print) -> None:
        self.jobs = jobs  # the viewer's JobFiles: where saved runs come from
        self.port = port
        self.launch_allowed = launch
        self.api = api
        self.say = say
        folder = watch.save_dir() if files is None else None
        self.files = files if files is not None else ModFiles(folder, port)
        self.game = HiddenGame(port, folder)
        self.lock = threading.Lock()
        self.stop_event = threading.Event()
        self.checked: set[str] = set()
        self.started_at = 0.0  # when the hidden game now running first answered
        self.memory_checked = 0.0
        self.last_error: str | None = None  # why the last try to reach the hidden game failed
        self.state = {"state": "starting", "message": "Starting the hidden game", "runs": 0, "moves": 0,
                      "differences": 0, "stopped_early": 0, "errors": 0, "restarts": 0, "last_restart": None,
                      "memory_mb": None, "up_since": None, "last_difference": None, "current": None, "last": None,
                      "port": port, "updated": time.time()}

    # -- what the page shows
    def summary(self) -> dict:
        with self.lock:
            return json.loads(json.dumps(self.state))

    def note(self, **values) -> None:
        with self.lock:
            self.state.update(values)
            self.state["updated"] = time.time()

    def add(self, key: str, n: int = 1) -> None:
        with self.lock:
            self.state[key] += n

    # -- the hidden game
    def control(self) -> None:
        self.files.write_control(lock=0)

    def mod_ok(self) -> bool:
        st = self.files.read_state()
        return bool(st) and st.get("version") == MOD_VERSION

    def connect(self) -> bool:
        """Reach the hidden game, starting it if needed. False while it isn't there yet."""
        if self.api is not None:
            try:
                self.api.call("health", timeout=3)
                return True
            except (OSError, RuntimeError, ValueError):
                self.api = None
                if self.launch_allowed and self.started_at:
                    self.restart("it stopped answering")
                    return False
        url = f"http://127.0.0.1:{self.port}"
        bridge = watch.windows_bridge()
        for api in (Api(url, timeout=CALL_TIMEOUT), Api(url, bridge, timeout=CALL_TIMEOUT) if bridge else None):
            if api is None:
                continue
            try:
                api.call("health", timeout=3)
                self.api = api
                self.last_error = None
                if not self.started_at:
                    self.started_at = time.time()
                    self.note(up_since=self.started_at)
                return True
            except (OSError, RuntimeError, ValueError) as exc:
                self.last_error = f"{'through the Windows bridge' if api.bridge else 'directly'}: {exc}"
                continue
        if self.launch_allowed and time.time() - self.game.launched_at > 90:
            if self.game.launched_at:  # the last start never answered: clear it away, and the link with it
                self.say(f"[checks] the hidden game didn't answer within 90 s (last try {self.last_error}); "
                         "starting it again")
                self.note(last_restart={"reason": f"no answer within 90 s ({self.last_error})", "time": time.time()})
                self.add("restarts")
                self.game.stop()
                watch.reset_bridge()
            if self.game.launch():
                self.note(state="starting", message="Starting a hidden copy of Balatro for the checks")
                return False
            self.note(state="off", message="The checks need BalatroBot's launcher (uvx) on Windows: RUNNING.md, step 5")
            return False
        if self.state["state"] != "off":
            self.note(state="starting", message="Waiting for the hidden copy of Balatro to start")
        return False

    def restart(self, reason: str) -> None:
        """Close the hidden game; the next connect starts a fresh one at once."""
        self.note(state="starting", message=f"Restarting the hidden game: {reason}",
                  last_restart={"reason": reason, "time": time.time()}, up_since=None, memory_mb=None)
        self.add("restarts")
        self.say(f"[checks] restarting the hidden game: {reason}")
        self.game.stop()
        watch.reset_bridge()  # a fresh link too: one left waiting on the old game can't answer for the new one
        self.api = None
        self.game.launched_at = 0.0
        self.started_at = 0.0

    def due_restart(self) -> str | None:
        """Why the hidden game should get a fresh start now, or None."""
        if not self.launch_allowed or not self.started_at:
            return None
        if time.time() - self.started_at > RESTART_EVERY:
            return "its hourly fresh start (Balatro's memory grows over a long session)"
        if time.time() - self.memory_checked > MEMORY_CHECK_EVERY:
            self.memory_checked = time.time()
            mb = self.game.memory_mb()
            if mb is not None:
                self.note(memory_mb=round(mb))
                if mb > MEMORY_LIMIT_MB:
                    return f"it was using {mb:,.0f} MB of memory"
        return None

    def wait_idle(self, cash_out: bool = False) -> None:
        """Wait until the game has played out the last move: no blocking events queued, twice in a row, in
        reports written after the wait began. Without the mod's reports, wait a fixed time instead."""
        start = time.time()
        if self.files.dir is None:  # no save folder (tests, or no game on this PC): nothing to wait for
            return
        if not self.mod_ok():
            time.sleep(1.0 if cash_out else 0.3)
            return
        seen, calm = None, 0
        while time.time() - start < 10:
            st = self.files.read_state() or {}
            clock = st.get("time")
            if clock is not None and clock != seen:
                seen = clock
                if time.time() - start > 0.1:  # a report written after the move began
                    calm = calm + 1 if st.get("busy") == "0" else 0
                    if calm >= 2:
                        return
            time.sleep(0.02)

    # -- one run
    def check_run(self, seed: str, moves, label: str) -> tuple[bool, str]:
        """Play a run in the game and the engine together. (ok, message); ok is False on a difference."""
        from balatro_meta._engine import Game

        game = Game(seed)
        api = self.api
        self.note(current={"seed": game.seed, "label": label, "move": 0})
        try:
            api.call("menu")
        except RuntimeError:
            pass
        self.wait_idle()  # a few frames in the menu: the mod readies the run-setup deck `start` needs
        gs = settle(api, api.call("start", {"deck": "RED", "stake": "WHITE", "seed": game.seed}))
        self.wait_idle()
        gs, diff = compare(api, game, gs)
        if diff:
            return False, self.mismatch(game, 0, "start", diff, gs)
        step = 0
        while not game.over and step < 5000 and not self.stop_event.is_set():
            if moves is None:
                kind, arg = game.greedy_move()
            elif step < len(moves):
                kind, arg = moves[step]
            else:
                break
            before = game.view()
            what = describe(kind, arg, game)
            why = unsupported(kind, before["state"])
            if why:
                self.add("stopped_early")
                return True, f"stopped before move {step + 1} ({what}): {why}"
            e_before = engine_view(game)
            if kind == 2:  # the cash-out total is only final once its rows have played out
                self.wait_idle(cash_out=True)
            gs = api.call("gamestate")
            if gs.get("state") in watch.HAND_STATES and e_before["hand"]:
                perm = hand_permutation(e_before["hand"], game_view(gs)["hand"])
                if perm is not None and perm != list(range(len(perm))):
                    gs = settle(api, api.call("rearrange", {"hand": perm}))
            if not game.step(kind, arg):
                return True, f"the engine refused move {step + 1} ({what})"
            gs = settle(api, apply_in_game(api, kind, arg, e_before, game))
            self.wait_idle()
            gs, diff = compare(api, game, gs)
            step += 1
            self.add("moves")
            self.note(current={"seed": game.seed, "label": label, "move": step})
            if diff:
                return False, self.mismatch(game, step, what, diff, gs)
        v = game.view()
        return True, ("won the run" if v["won"] else f"run over at ante {v['ante']}") + f" after {step} moves"

    def mismatch(self, game, step: int, what: str, diff: list[str], gs: dict) -> str:
        out = WATCH / "mismatches"
        out.mkdir(parents=True, exist_ok=True)
        f = out / f"{game.seed}-move{step}.json"
        f.write_text(json.dumps({"seed": game.seed, "move": step, "what": what, "differences": diff,
                                 "moves": game.moves, "engine": engine_view(game), "game": gs},
                                indent=1, default=str))
        shown = str(f.relative_to(ROOT) if f.is_relative_to(ROOT) else f)
        self.add("differences")
        self.note(last_difference={"seed": game.seed, "move": step, "what": what, "differences": diff[:6],
                                   "file": shown, "time": time.time()})
        return f"engine and game differ after move {step} ({what}): " + "; ".join(diff[:3]) + f". Details in {shown}"

    def next_run(self) -> tuple[str, list | None, str]:
        """A run to check: one of the job's best runs not checked yet (they go furthest, so they cover the
        most rules), else the newest typical run not checked yet, else a demo run."""
        if self.jobs is not None:
            for p in [p for _, p in self.jobs.best()] + self.jobs.samples():
                if str(p) in self.checked:
                    continue
                self.checked.add(str(p))
                s = self.jobs.read_sample(p)
                if s:
                    return s["seed"], [tuple(m) for m in s["moves"]], s.get("label") or "a job's run"
        return fresh_seed(), None, "demo run (greedy bot)"

    # -- the loop
    def loop(self, stop: threading.Event) -> None:
        self.stop_event = stop
        keeper = threading.Thread(target=self.keep_control, args=(stop,), daemon=True)
        keeper.start()
        while not stop.is_set():
            if self.someone_recording():
                if self.state["state"] != "paused":
                    self.note(state="paused", message="Paused while you record your own runs (make record-me): "
                              "its game shares your save profile")
                    if self.launch_allowed and self.started_at:
                        self.game.stop()
                        self.api = None
                        self.game.launched_at = 0.0
                        self.started_at = 0.0
                stop.wait(5)
                continue
            if not self.connect():
                stop.wait(3)
                continue
            if self.files.dir is not None and not self.mod_ok():
                st = self.files.read_state()
                self.note(state="waiting", message="Waiting for the companion mod in the hidden game"
                          if st is None else "The hidden game runs an older companion mod: it restarts with the new one")
                if st is not None and self.launch_allowed and time.time() - self.game.launched_at > 90:
                    self.say(f"[checks] the hidden game runs companion mod {st.get('version')!r}, not "
                             f"{MOD_VERSION!r}: starting it again")
                    self.game.stop()  # an old copy left running: start it again with the new mod
                    self.api = None
                stop.wait(3)
                if st is None and time.time() - self.game.launched_at > 120:
                    self.note(state="error", message="The hidden game runs without the companion mod: restart "
                              "make watch (it reinstalls the mod)")
                continue
            reason = self.due_restart()
            if reason:
                self.restart(reason)
                continue
            seed, moves, label = self.next_run()
            self.note(state="running", message="Checking runs in a hidden copy of your game")
            try:
                ok, msg = self.check_run(seed, moves, label)
            except (OSError, ValueError) as exc:  # it crashed, hung or stopped answering
                ok, msg = None, f"lost the hidden game: {exc}"
                self.add("errors")
                if self.launch_allowed:
                    self.restart("it stopped answering")
                else:
                    self.api = None
            except RuntimeError as exc:  # BalatroBot refused a call or failed in it
                ok, msg = None, f"the game refused a call: {exc}"
                self.add("errors")
            if ok is None:
                stop.wait(2)
            self.add("runs")
            self.note(last={"seed": seed, "label": label, "ok": ok, "result": msg, "time": time.time()}, current=None)
            self.say(f"[checks] {label} {seed}: {msg}")

    def someone_recording(self) -> bool:
        """`make record-me`'s game is up (its companion mod wrote its report in the last few seconds)."""
        if self.files.dir is None:
            return False
        p = self.files.dir.parent / str(RECORD_PORT) / "state.txt"
        try:
            return time.time() - p.stat().st_mtime < 8
        except OSError:
            return False

    def keep_control(self, stop: threading.Event) -> None:
        while not stop.is_set():
            self.control()
            stop.wait(1.0)

    def close(self) -> None:
        self.stop_event.set()
        self.files.write_control(lock=0, stop=1)
        if self.launch_allowed:
            self.game.stop()
