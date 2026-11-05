"""The real-game checks (scripts/checker.py) against a stand-in for BalatroBot.

The stand-in serves BalatroBot's JSON-RPC methods from a second engine run, shows the hand in a
scrambled order (so the checker must re-align it before every play) and, like the real game, shows a
price and a sell value on every card. With the same rules on both sides, a check must finish with no
difference; a stand-in that pays $1 extra, prices a card wrongly or values a joker wrongly must be caught.
"""
from __future__ import annotations

import json
import sys
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))

import checker
import watch
from balatro_meta._engine import Game


def replay(api, seed: str, tmp_path, moves=None):
    """One run checked against `api` (no companion mod: nothing to wait for)."""
    c = checker.Checker(api=api, files=checker.ModFiles(None), launch=False, say=lambda *_: None)
    ok, msg = c.check_run(seed, moves, "test")
    return ok, msg, c


def area(items: list, cheat_price: bool = False) -> dict:
    """Cards in BalatroBot's area format. Like the game, every card shows a price and a sell value, also
    where no price is paid (a pack's cards show their shop price, a Planet $3)."""
    cards = []
    for i, it in enumerate(items):
        if it is None:
            continue
        buy = it.get("cost", 3)
        c = {"key": it["key"], "modifier": {"edition": it.get("edition"), "enhancement": it.get("enhancement"),
                                            "seal": it.get("seal")},
             "cost": {"buy": buy + (1 if cheat_price and i == 0 else 0), "sell": it.get("sell", max(1, buy // 2))}}
        cards.append(c)
    return {"count": len(cards), "cards": cards}


class FakeGame:
    def __init__(self, cheat: bool = False, cheat_price: bool = False, cheat_sell: bool = False) -> None:
        self.game = None
        self.stake = None  # BalatroBot's name of the stake the last start asked for
        self.order: list[int] = []
        self.cheat = cheat
        self.cheat_price = cheat_price
        self.cheat_sell = cheat_sell
        self.state = "MENU"

    def view(self) -> dict:
        if self.game is None:
            return {"state": "MENU"}
        v = self.game.view()
        hand = v["hand"]
        if len(self.order) != len(hand):
            self.order = list(range(len(hand)))[::-1]  # a scrambled display order
        cards = [{"key": hand[i]["key"],
                  "modifier": {"enhancement": hand[i]["enhancement"], "edition": hand[i]["edition"],
                               "seal": hand[i]["seal"]},
                  "state": {"debuff": hand[i]["debuff"]}} for i in self.order]
        blinds = {k: {"type": k.upper(), "name": b["name"], "status": b["status"], "tag_name": b.get("tag_name")}
                  for k, b in v["blinds"].items()}
        money = v["money"] + (1 if self.cheat and v["state"] == "SHOP" else 0)
        jokers = [{"key": j["key"], "modifier": {"edition": j["edition"]}, "state": {"hidden": j["face_down"]},
                   "cost": {"buy": 2 * j["sell"], "sell": j["sell"] + (1 if self.cheat_sell and i == 0 else 0)}}
                  for i, j in enumerate(v["jokers"])]
        return {"state": v["state"], "round_num": v["round"], "ante_num": v["ante"], "money": money,
                "round": {"hands_left": v["hands_left"], "discards_left": v["discards_left"], "chips": v["chips"],
                          "reroll_cost": v["reroll_cost"]},
                "hand": {"cards": cards}, "jokers": {"cards": jokers}, "blinds": blinds,
                "consumables": area(v["consumables"]), "shop": area(v["shop"], self.cheat_price),
                "vouchers": area(v["shop_vouchers"]), "packs": area(v["shop_packs"]), "pack": area(v["pack"])}

    def mask(self, positions) -> int:
        m = 0
        for pos in positions or []:
            m |= 1 << self.order[pos]
        return m

    def rpc(self, method: str, p: dict):
        kinds = {"select": 0, "skip": 1, "cash_out": 2, "next_round": 3}
        if method == "health":
            return {"status": "ok"}
        if method == "menu":
            self.game = None
        elif method == "start":
            self.stake = p.get("stake", "WHITE")
            self.game = Game(p["seed"], watch.BALATROBOT_STAKES.index(self.stake) + 1)
            self.order = []
        elif method in kinds:
            assert self.game.step(kinds[method], 0), method
            self.order = []
        elif method in ("play", "discard"):
            mask = 0
            for pos in p["cards"]:
                mask |= 1 << self.order[pos]
            assert self.game.step(6 if method == "play" else 7, mask), method
            self.order = []
        elif method == "rearrange":
            self.order = [self.order[i] for i in p["hand"]]
        elif method == "reroll":
            assert self.game.step(11, 0), method
        elif method == "buy":
            if "card" in p:
                assert self.game.step(12, p["card"]), method
            elif "voucher" in p:
                assert self.game.step(14, p["voucher"]), method
            else:
                slots = [i for i, b in enumerate(self.game.view()["shop_packs"]) if b is not None]
                assert self.game.step(15, slots[p["pack"]]), method
            self.order = []
        elif method == "sell":
            assert self.game.step(10, p["joker"]) if "joker" in p else self.game.step(16, p["consumable"]), method
        elif method == "use":
            assert self.game.step(17, p["consumable"] | self.mask(p.get("cards")) << 8), method
            self.order = []
        elif method == "pack":
            if p.get("skip"):
                assert self.game.step(19, 0), method
            else:
                assert self.game.step(18, p["card"] | self.mask(p.get("targets")) << 8), method
            self.order = []
        elif method != "gamestate":
            raise AssertionError(f"the stand-in doesn't know {method}")
        return self.view()


def serve(fake: FakeGame):
    class H(BaseHTTPRequestHandler):
        def do_POST(self):
            body = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
            try:
                out = {"jsonrpc": "2.0", "id": body["id"], "result": fake.rpc(body["method"], body.get("params") or {})}
            except AssertionError as e:
                out = {"jsonrpc": "2.0", "id": body["id"], "error": {"code": -32003, "message": str(e)}}
            data = json.dumps(out).encode()
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(data)))
            self.end_headers()
            self.wfile.write(data)

        def log_message(self, *a):
            pass

    srv = ThreadingHTTPServer(("127.0.0.1", 0), H)
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    return srv


@pytest.mark.parametrize("seed", ["WATCH001", "K4QJ2B7X", "ZZ9ZZ9ZZ"])
def test_replay_agrees_with_an_identical_game(seed, tmp_path, monkeypatch):
    monkeypatch.setattr(watch, "WATCH", tmp_path)
    monkeypatch.setattr(checker, "WATCH", tmp_path)
    srv = serve(FakeGame())
    api = watch.Api(f"http://127.0.0.1:{srv.server_address[1]}")
    ok, msg = replay(api, seed, tmp_path)[:2]
    srv.shutdown()
    assert ok, msg
    assert "run over" in msg or "won" in msg


def test_shop_and_pack_moves_reach_the_game(tmp_path, monkeypatch):
    """The greedy bot buys, opens packs and uses Planets: those moves must be sent and agree."""
    monkeypatch.setattr(watch, "WATCH", tmp_path)
    monkeypatch.setattr(checker, "WATCH", tmp_path)
    fake = FakeGame()
    sent: list[str] = []
    rpc = fake.rpc
    fake.rpc = lambda m, p: (sent.append(m), rpc(m, p))[1]
    srv = serve(fake)
    api = watch.Api(f"http://127.0.0.1:{srv.server_address[1]}")
    for seed in ("SHOPPER1", "SHOPPER2", "SHOPPER3"):
        ok, msg = replay(api, seed, tmp_path)[:2]
        assert ok, msg
    srv.shutdown()
    assert {"buy", "pack", "use"} <= set(sent), sorted(set(sent))


def test_a_wrong_shop_price_is_caught(tmp_path, monkeypatch):
    monkeypatch.setattr(watch, "WATCH", tmp_path)
    monkeypatch.setattr(checker, "WATCH", tmp_path)
    srv = serve(FakeGame(cheat_price=True))
    api = watch.Api(f"http://127.0.0.1:{srv.server_address[1]}")
    ok, msg = replay(api, "WATCH003", tmp_path)[:2]
    srv.shutdown()
    assert not ok and "shop cards" in msg, msg


def test_a_wrong_sell_value_is_caught(tmp_path, monkeypatch):
    monkeypatch.setattr(watch, "WATCH", tmp_path)
    monkeypatch.setattr(checker, "WATCH", tmp_path)
    srv = serve(FakeGame(cheat_sell=True))
    api = watch.Api(f"http://127.0.0.1:{srv.server_address[1]}")
    for seed in ("SHOPPER1", "SHOPPER2", "SHOPPER3"):  # runs where the bot buys a joker
        ok, msg = replay(api, seed, tmp_path)[:2]
        if not ok:
            break
    srv.shutdown()
    assert not ok and "jokers" in msg and "sell" in msg, msg


def test_a_difference_is_caught_and_saved(tmp_path, monkeypatch):
    monkeypatch.setattr(watch, "WATCH", tmp_path)
    monkeypatch.setattr(checker, "WATCH", tmp_path)
    srv = serve(FakeGame(cheat=True))
    api = watch.Api(f"http://127.0.0.1:{srv.server_address[1]}")
    ok, msg, c = replay(api, "WATCH002", tmp_path)
    srv.shutdown()
    assert not ok and "money" in msg
    st = c.summary()  # shown on the page
    assert st["differences"] == 1 and st["last_difference"]["differences"]
    saved = list((tmp_path / "mismatches").glob("*.json"))
    assert len(saved) == 1 and json.loads(saved[0].read_text())["differences"]



def test_the_windows_bridge_relays_calls(tmp_path, monkeypatch):
    """The relay that reaches BalatroBot on Windows' localhost, run here with this Python."""
    monkeypatch.setattr(watch, "WATCH", tmp_path)
    monkeypatch.setattr(checker, "WATCH", tmp_path)
    srv = serve(FakeGame())
    url = f"http://127.0.0.1:{srv.server_address[1]}"
    bridge = watch.WinBridge(sys.executable, str(ROOT / "scripts" / "win_bridge.py"), cwd=None)
    try:
        api = watch.Api(url, bridge)
        assert api.call("health") == {"status": "ok"}
        ok, msg = replay(api, "BRIDGE01", tmp_path)[:2]
        assert ok, msg
        # A refused call comes back as BalatroBot's refusal, not a crash.
        with pytest.raises(RuntimeError, match="refused"):
            api.call("no_such_method")
        # A dead server is an OSError the caller can retry.
        srv.shutdown()
        srv.server_close()
        with pytest.raises(OSError):
            api.call("health", timeout=2)
    finally:
        bridge.close()


def test_a_refused_start_is_reported_and_the_checks_carry_on(tmp_path, monkeypatch):
    """BalatroBot failing inside `start` (as it did after a lost run, before the companion mod readied the
    run-setup deck) is counted as an interrupted check, and the link to the game is kept."""
    monkeypatch.setattr(watch, "WATCH", tmp_path)
    monkeypatch.setattr(checker, "WATCH", tmp_path)
    fake = FakeGame()
    rpc = fake.rpc

    def refuse_start(method, p):
        if method == "start":
            raise AssertionError("attempt to index field 'viewed_back' (a nil value)")
        return rpc(method, p)

    fake.rpc = refuse_start
    srv = serve(fake)
    api = watch.Api(f"http://127.0.0.1:{srv.server_address[1]}")
    lines = []
    c = checker.Checker(api=api, files=checker.ModFiles(None), launch=False, say=lines.append)
    stop = threading.Event()
    t = threading.Thread(target=c.loop, args=(stop,), daemon=True)
    t.start()
    deadline = time.time() + 10
    while time.time() < deadline and c.summary()["errors"] < 1:
        time.sleep(0.05)
    stop.set()
    t.join(5)
    srv.shutdown()
    st = c.summary()
    assert st["errors"] >= 1 and st["differences"] == 0
    assert "refused" in st["last"]["result"] and "viewed_back" in st["last"]["result"]
    assert c.api is api, "a refusal is not a lost connection"


class AlwaysIdle(checker.ModFiles):
    """A companion mod that reports an idle game, freshly, every time it is read."""

    def __init__(self) -> None:
        super().__init__(Path("/nonexistent"), 12350)
        self.reads = 0

    def write_control(self, **values) -> None:
        self.last_control = values

    def read_state(self):
        self.reads += 1
        return {"version": checker.MOD_VERSION, "time": str(self.reads), "busy": "0", "end": "1"}


def test_the_checks_take_the_jobs_best_runs_first(tmp_path, monkeypatch):
    import viewer

    monkeypatch.setattr(watch, "WATCH", tmp_path)
    monkeypatch.setattr(checker, "WATCH", tmp_path)
    samples = tmp_path / "watch" / "job" / "samples"
    samples.mkdir(parents=True)
    g = Game("CHECKME1")
    moves = []
    for _ in range(6):
        k, a = g.greedy_move()
        g.step(k, a)
        moves.append([k, a])
    (samples / "typical-1.json").write_text(json.dumps({"seed": "TYPICAL1", "moves": moves, "pick": "typical"}))
    (samples / "best-0000000001.json").write_text(json.dumps(
        {"seed": "CHECKME1", "moves": moves, "pick": "best", "won": False, "stage": 3, "closeness": 0.5}))
    jobs = viewer.JobFiles(tmp_path / "watch", tmp_path / "logs")
    srv = serve(FakeGame())
    api = watch.Api(f"http://127.0.0.1:{srv.server_address[1]}")
    mod = AlwaysIdle()
    c = checker.Checker(jobs=jobs, api=api, files=mod, launch=False, say=lambda *_: None)
    seed, got, *_ = c.next_run()
    assert seed == "CHECKME1" and [list(m) for m in got] == moves
    assert c.next_run()[0] == "TYPICAL1"
    assert c.next_run()[1] is None, "then demo runs"
    ok, msg = c.check_run("CHECKME1", [tuple(m) for m in moves], "best run")
    srv.shutdown()
    assert ok and c.summary()["moves"] == 6, msg
    assert mod.reads > 0, "the checks waited for the game through the mod's reports"


def test_a_run_saved_at_a_stake_is_checked_at_that_stake(tmp_path, monkeypatch):
    """A job's run at Gold Stake: the checks take its stake from the sample and start the game there, and the
    engine plays it there too (at White Stake its moves would not even fit)."""
    import viewer

    monkeypatch.setattr(watch, "WATCH", tmp_path)
    monkeypatch.setattr(checker, "WATCH", tmp_path)
    samples = tmp_path / "watch" / "job" / "samples"
    samples.mkdir(parents=True)
    g = Game("GOLDRUN1", 8)
    moves = []
    for _ in range(40):
        if g.over:
            break
        k, a = g.greedy_move()
        g.step(k, a)
        moves.append([k, a])
    (samples / "best-0000000001.json").write_text(json.dumps(
        {"seed": "GOLDRUN1", "stake": 8, "moves": moves, "pick": "best", "won": False, "stage": 2}))
    jobs = viewer.JobFiles(tmp_path / "watch", tmp_path / "logs")
    fake = FakeGame()
    srv = serve(fake)
    api = watch.Api(f"http://127.0.0.1:{srv.server_address[1]}")
    c = checker.Checker(jobs=jobs, api=api, files=AlwaysIdle(), launch=False, say=lambda *_: None)
    seed, got, label, stake = c.next_run()
    assert (seed, stake) == ("GOLDRUN1", 8) and "Gold Stake" in label
    ok, msg = c.check_run(seed, got, label, stake)
    srv.shutdown()
    assert ok, msg
    assert fake.stake == "GOLD" and c.summary()["moves"] == len(moves)
    assert watch.stake_of({"seed": "X"}) == 1, "runs saved before stakes were named are White Stake's"
    with pytest.raises(ValueError):
        watch.stake_of({"stake": 9})


def test_face_down_jokers_are_compared_by_their_true_values():
    """Under Amber Acorn the jokers are face down: the bot's screen hides their sell values, but the check
    compares the true ones (as BalatroBot reports them), and also which jokers are face down."""
    base = {"state": "SHOP", "ante": 8, "money": 112, "consumables": [], "shop": [], "shop_vouchers": [],
            "shop_packs": [], "reroll_cost": 5, "hand": [], "blinds": {}}
    e = dict(base, jokers=[{"key": "j_egg", "edition": "FOIL", "sell": 63, "face_down": True}])
    g = dict(base, jokers=[{"key": "j_egg", "edition": "FOIL", "sell": 63, "face_down": True}])
    assert not [d for d in watch.differences(e, g) if d.startswith("jokers")]
    g_up = dict(base, jokers=[{"key": "j_egg", "edition": "FOIL", "sell": 63, "face_down": False}])
    assert [d for d in watch.differences(e, g_up) if d.startswith("jokers")], "a joker the wrong way up is caught"
    view = watch.game_view({"state": "SHOP", "jokers": {"cards": [
        {"key": "j_egg", "modifier": {"edition": "FOIL"}, "state": {"hidden": True}, "cost": {"buy": 6, "sell": 63}}]}})
    assert view["jokers"] == [{"key": "j_egg", "edition": "FOIL", "face_down": True, "sell": 63}]


class FakeHiddenGame:
    """Stands in for the hidden Balatro's process control: counts stops and launches, reports a memory size."""

    def __init__(self, mb=500.0):
        self.mb, self.stops, self.launches, self.launched_at = mb, 0, 0, 0.0

    def stop(self):
        self.stops += 1

    def launch(self):
        self.launches += 1
        self.launched_at = time.time()
        return True

    def memory_mb(self):
        return self.mb


def test_the_hidden_game_gets_fresh_starts(tmp_path, monkeypatch):
    monkeypatch.setattr(checker, "WATCH", tmp_path)
    c = checker.Checker(files=checker.ModFiles(None), launch=True, say=lambda *_: None)
    c.game = FakeHiddenGame()
    assert c.due_restart() is None, "nothing running yet"
    c.started_at = time.time()
    assert c.due_restart() is None and c.summary()["memory_mb"] == 500
    c.started_at = time.time() - checker.RESTART_EVERY - 1
    assert "hourly" in c.due_restart()
    c.started_at, c.memory_checked, c.game.mb = time.time(), 0.0, 3500.0
    assert "3,500 MB" in c.due_restart()
    c.restart("a test")
    st = c.summary()
    assert c.game.stops == 1 and c.api is None and c.started_at == 0 and c.game.launched_at == 0
    assert st["restarts"] == 1 and st["last_restart"]["reason"] == "a test" and st["state"] == "starting"
    # The next connect starts a new one at once (no game answers here).
    monkeypatch.setattr(watch, "windows_bridge", lambda: None)
    assert not c.connect() and c.game.launches == 1


def test_a_game_that_stops_answering_is_restarted(tmp_path, monkeypatch):
    monkeypatch.setattr(watch, "WATCH", tmp_path)
    monkeypatch.setattr(checker, "WATCH", tmp_path)
    monkeypatch.setattr(watch, "windows_bridge", lambda: None)
    fake = FakeGame()
    srv = serve(fake)
    api = watch.Api(f"http://127.0.0.1:{srv.server_address[1]}")
    c = checker.Checker(api=api, files=checker.ModFiles(None), launch=True, say=lambda *_: None)
    c.game = FakeHiddenGame()
    assert c.connect()
    c.started_at = time.time()
    srv.shutdown()
    srv.server_close()  # the game is gone
    assert not c.connect()
    assert c.game.stops == 1 and c.summary()["restarts"] == 1 and c.api is None
    assert not c.connect() and c.game.launches == 1, "and started again at once"


def test_old_logs_of_the_hidden_game_are_pruned(tmp_path):
    save = tmp_path / "Balatro"
    ours = save / "balatro_meta" / "logs"
    lovely = save / "Mods" / "lovely" / "log"
    lovely.mkdir(parents=True)
    import os

    for i in range(4):
        d = ours / f"2026-10-30T0{i}-00-00"
        d.mkdir(parents=True)
        (d / "12350.log").write_text("x")
        os.utime(d, (1000 + i, 1000 + i))
    for i in range(4):
        f = lovely / f"lovely-2026.10.30-0{i}.00.00.log"
        f.write_text("INFO - BB.SERVER :: HTTP server listening on http://127.0.0.1:12350\n")
        os.utime(f, (1000 + i, 1000 + i))
    mine = lovely / "lovely-2026.10.29-23.00.00.log"
    mine.write_text("INFO - BB.SERVER :: HTTP server listening on http://127.0.0.1:12346\n")
    os.utime(mine, (900, 900))
    checker.HiddenGame(12350, save).prune_logs()
    assert sorted(d.name for d in ours.iterdir()) == ["2026-10-30T02-00-00", "2026-10-30T03-00-00"]
    left = sorted(f.name for f in lovely.iterdir())
    assert left == ["lovely-2026.10.29-23.00.00.log", "lovely-2026.10.30-02.00.00.log", "lovely-2026.10.30-03.00.00.log"], \
        "the newest two of the hidden game's logs stay, and a Balatro you play keeps all of its own"


def test_a_stopped_bridge_is_replaced(monkeypatch):
    class Dead:
        class proc:
            @staticmethod
            def poll():
                return 1

    monkeypatch.setattr(watch, "_bridge", [Dead()])
    monkeypatch.setattr(watch, "Path", Path)
    started = []

    def fake_bridge(python, script, cwd="/mnt/c"):
        started.append(1)
        return "new bridge"

    monkeypatch.setattr(watch, "WinBridge", fake_bridge)
    monkeypatch.setitem(sys.modules, "rng_game", type(sys)("rng_game"))
    sys.modules["rng_game"].find_windows_python = lambda: "python.exe"
    sys.modules["rng_game"].win_path = lambda p: str(p)
    real_read = Path.read_text
    monkeypatch.setattr(Path, "read_text", lambda self, *a, **k: "Linux microsoft-standard-WSL2"
                        if str(self) == "/proc/version" else real_read(self, *a, **k))
    assert watch.windows_bridge() == "new bridge" and started == [1]


def test_the_hidden_games_window_is_kept_out_of_sight_while_it_loads(monkeypatch):
    started = []

    class FakePopen:
        def __init__(self, cmd, **kw):
            self.cmd, self.alive = cmd, True
            started.append(self)

        def poll(self):
            return None if self.alive else 0

        def terminate(self):
            self.alive = False

    game = checker.HiddenGame(port=12350)
    monkeypatch.setattr(game, "command", lambda: ["uvx.exe", "balatrobot", "serve", "--port", "12350"])
    monkeypatch.setattr(game, "prune_logs", lambda: None)
    monkeypatch.setattr(checker.subprocess, "Popen", FakePopen)
    monkeypatch.setattr(checker.subprocess, "run", lambda *a, **k: None)
    assert game.launch()
    hider, launcher = started
    assert hider.cmd[0] == "powershell.exe" and launcher.cmd[0] == "uvx.exe", "the hider is watching first"
    script = hider.cmd[-1]
    assert "$port = 12350;" in script and "ShowWindow" in script and "Get-Process -Name Balatro" in script
    assert "balatrobot" in script, "only a Balatro started by the launcher on this port is hidden"
    game.stop()
    assert not hider.alive and not launcher.alive


def test_the_bridge_takes_only_the_reply_to_its_own_call(tmp_path, monkeypatch):
    """A reply left over from an earlier call, or a stray line, is never taken as the answer; a bridge that
    goes quiet is given up on instead of waited on for ever."""
    script = tmp_path / "bridge.py"
    script.write_text(
        "import json, sys\n"
        "for line in sys.stdin:\n"
        "    req = json.loads(line)\n"
        "    if req['body'].get('method') == 'quiet':\n"
        "        continue\n"
        "    print('some banner a Windows Python printed', flush=True)\n"
        "    print(json.dumps({'id': req['id'] - 1, 'ok': True, 'result': {'result': 'stale'}}), flush=True)\n"
        "    print(json.dumps({'id': req['id'], 'ok': True, 'result': {'result': req['body']['method']}}), flush=True)\n")
    bridge = watch.WinBridge(sys.executable, str(script), cwd=None)
    monkeypatch.setattr(watch.WinBridge, "SLACK", 0.5)
    try:
        api = watch.Api("http://127.0.0.1:1", bridge)
        assert api.call("health", timeout=1) == "health"
        assert api.call("gamestate", timeout=1) == "gamestate"
        with pytest.raises(OSError, match="didn't answer"):
            api.call("quiet", timeout=0.5)
        with pytest.raises(OSError, match="stopped"):
            api.call("health", timeout=1)
    finally:
        bridge.close()


def test_a_hidden_game_that_never_answers_is_started_again_and_says_why(tmp_path, monkeypatch):
    monkeypatch.setattr(checker, "WATCH", tmp_path)
    lines = []
    c = checker.Checker(files=checker.ModFiles(None), launch=True, say=lines.append)
    c.game = FakeHiddenGame()
    monkeypatch.setattr(watch, "windows_bridge", lambda: None)
    resets = []
    monkeypatch.setattr(watch, "reset_bridge", lambda: resets.append(1))
    assert not c.connect() and c.game.launches == 1 and not lines, "the first start says nothing"
    c.game.launched_at -= 91
    assert not c.connect()
    assert c.game.stops == 1 and c.game.launches == 2 and resets == [1]
    assert len(lines) == 1 and "didn't answer within 90 s" in lines[0] and "directly" in lines[0]
    assert c.summary()["restarts"] == 1


def test_a_saved_difference_replays_on_todays_engine_and_the_checks_keep_a_tally(tmp_path, monkeypatch):
    """`make recheck`: a saved difference whose game state the engine now agrees with reads as fixed; one it
    still differs from stays open. The checks' tally of runs and moves compared persists across sessions."""
    import recheck
    monkeypatch.setattr(watch, "WATCH", tmp_path)
    monkeypatch.setattr(checker, "WATCH", tmp_path)
    tally = tmp_path / "checks.json"
    # A game that lies about its money in the shop: the checks save the difference (with the stake), and the tally
    # counts the run and its moves.
    srv = serve(FakeGame(cheat=True))
    api = watch.Api(f"http://127.0.0.1:{srv.server_address[1]}")
    c = checker.Checker(api=api, files=checker.ModFiles(None), launch=False, say=lambda *_: None, tally_path=tally)
    ok, msg = c.check_run("WATCH003", None, "test")
    srv.shutdown()
    assert not ok and "money" in msg
    c.write_tally(force=True)
    t = json.loads(tally.read_text())
    assert t["sessions"] == 1 and t["moves"] > 0 and t["differences"] == 1 and t["runs"] == 0, "runs count in the loop"
    assert checker.Checker(files=checker.ModFiles(None), launch=False, say=lambda *_: None, tally_path=tally).tally["sessions"] == 2
    files = sorted((tmp_path / "mismatches").glob("*.json"))
    assert len(files) == 1
    saved = json.loads(files[0].read_text())
    assert saved["stake"] == 1 and saved["moves"]
    # Replayed: the lie is still in the saved state, so the difference is open; the stake is read from the file.
    r = recheck.recheck_file(files[0])
    assert r["now"] and r["now"][0].startswith("money") and r["stake"] == 1
    # The same file with the game's money put right: fixed. And one from before files named their stake.
    fixed = dict(saved)
    fixed["game"] = dict(saved["game"], money=saved["engine"]["money"])
    (tmp_path / "mismatches" / "fixed-case.json").write_text(json.dumps(fixed, default=str))
    old = dict(fixed)
    old.pop("stake")
    (tmp_path / "mismatches" / "old-case.json").write_text(json.dumps(old, default=str))
    results = [recheck.recheck_file(p) for p in sorted((tmp_path / "mismatches").glob("*.json"))]
    s = recheck.summarise(results)
    assert (s["files"], s["fixed"], s["open"], s["errors"]) == (3, 2, 1, 0)
    assert s["open_by_field"] == {"money": 1}
