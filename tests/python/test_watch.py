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
        jokers = [{"key": j["key"], "modifier": {"edition": j["edition"]},
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
            self.game = Game(p["seed"])
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
    seed, got, _ = c.next_run()
    assert seed == "CHECKME1" and [list(m) for m in got] == moves
    assert c.next_run()[0] == "TYPICAL1"
    assert c.next_run()[1] is None, "then demo runs"
    ok, msg = c.check_run("CHECKME1", [tuple(m) for m in moves], "best run")
    srv.shutdown()
    assert ok and c.summary()["moves"] == 6, msg
    assert mod.reads > 0, "the checks waited for the game through the mod's reports"
