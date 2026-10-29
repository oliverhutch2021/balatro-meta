"""make watch's game driver, against a stand-in for BalatroBot.

The stand-in serves BalatroBot's JSON-RPC methods from a second engine run and shows the hand in a
scrambled order, so the driver must re-align it before every play. With the same rules on both
sides, a replay must finish with no mismatch; a stand-in that pays $1 extra must be caught.
"""
from __future__ import annotations

import json
import sys
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))

import watch  # noqa: E402
from balatro_meta._engine import Game  # noqa: E402


class FakeGame:
    def __init__(self, cheat: bool = False) -> None:
        self.game = None
        self.order: list[int] = []
        self.cheat = cheat
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
        return {"state": v["state"], "round_num": v["round"], "ante_num": v["ante"], "money": money,
                "round": {"hands_left": v["hands_left"], "discards_left": v["discards_left"], "chips": v["chips"]},
                "hand": {"cards": cards}, "jokers": {"cards": []}, "blinds": blinds}

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
    srv = serve(FakeGame())
    api = watch.Api(f"http://127.0.0.1:{srv.server_address[1]}")
    ok, msg = watch.replay(api, watch.Panel(tmp_path / "panel.txt"), seed, None, "test", 0.0)
    srv.shutdown()
    assert ok, msg
    assert "run over" in msg or "won" in msg


def test_a_difference_is_caught_and_saved(tmp_path, monkeypatch):
    monkeypatch.setattr(watch, "WATCH", tmp_path)
    srv = serve(FakeGame(cheat=True))
    api = watch.Api(f"http://127.0.0.1:{srv.server_address[1]}")
    ok, msg = watch.replay(api, watch.Panel(tmp_path / "panel.txt"), "WATCH002", None, "test", 0.0)
    srv.shutdown()
    assert not ok and "money" in msg
    saved = list((tmp_path / "mismatches").glob("*.json"))
    assert len(saved) == 1 and json.loads(saved[0].read_text())["differences"]
    assert "MISMATCH" in (tmp_path / "panel.txt").read_text()
