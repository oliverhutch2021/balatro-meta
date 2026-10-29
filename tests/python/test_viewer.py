"""The viewer (scripts/viewer.py) against a stand-in for BalatroBot and for the companion mod.

The stand-in mod follows control.txt like the real one: it reports its state about ten times a second and
writes small raw RGBA frames, rotating over three files. The viewer must lock the game, play a run move by
move in step with the stand-in game, keep a picture after every move, and serve all of it to the page.
"""
from __future__ import annotations

import json
import sys
import threading
import time
import urllib.request
from http.server import ThreadingHTTPServer
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))

import watch
from test_watch import FakeGame, serve

import viewer


class FakeMod:
    """Writes state.txt and frames the way mods/balatro_meta_watch does."""

    def __init__(self, folder: Path, w: int = 64, h: int = 36) -> None:
        self.dir = folder / "balatro_meta"
        self.dir.mkdir(parents=True, exist_ok=True)
        self.w, self.h = w, h
        self.seq = 0
        self.busy = 0
        self.stop = threading.Event()
        self.control: dict = {}

    def loop(self) -> None:
        t0 = time.time()
        while not self.stop.is_set():
            try:
                text = (self.dir / "control.txt").read_text()
                self.control = dict(line.split("=", 1) for line in text.splitlines() if "=" in line)
            except OSError:
                pass
            lines = [f"version={viewer.MOD_VERSION}", f"time={time.time() - t0:.3f}", f"busy={self.busy}",
                     "state=SHOP", f"locked={self.control.get('lock', '0')}"]
            if self.control.get("frames") == "1":
                import numpy as np

                self.seq += 1
                name = f"frame_{self.seq % 3}.png"
                img = np.full((self.h, self.w, 3), (self.seq * 7) % 256, dtype=np.uint8)
                (self.dir / name).write_bytes(viewer.png(img))
                lines += [f"seq={self.seq}", f"frame={name}", f"width={self.w}", f"height={self.h}"]
            (self.dir / "state.txt").write_text("\n".join(lines) + "\n")
            time.sleep(0.02)


def start(tmp_path: Path, speed: str = "max"):
    game_srv = serve(FakeGame())
    mod = FakeMod(tmp_path)
    threading.Thread(target=mod.loop, daemon=True).start()
    v = viewer.Viewer(speed)
    files = viewer.GameFiles(tmp_path)
    frames = viewer.Frames(files)
    stop = threading.Event()
    threading.Thread(target=frames.loop, args=(stop, v), daemon=True).start()
    api = watch.Api(f"http://127.0.0.1:{game_srv.server_address[1]}")
    drv = viewer.Driver(v, files, frames, api=api, launch=False, say=lambda *_: None)
    drv.control()
    return game_srv, mod, v, frames, drv, stop


def get(port: int, path: str):
    with urllib.request.urlopen(f"http://127.0.0.1:{port}{path}", timeout=10) as r:
        return r.status, r.headers, r.read()


def test_a_run_plays_in_step_with_the_game_and_reaches_the_page(tmp_path, monkeypatch):
    monkeypatch.setattr(watch, "WATCH", tmp_path / "watch")
    monkeypatch.setattr(viewer, "WATCH", tmp_path / "watch")
    game_srv, mod, v, frames, drv, stop = start(tmp_path)
    try:
        time.sleep(0.4)
        assert mod.control.get("lock") == "1" and mod.control.get("frames") == "1"
        assert frames.mod_status() == "ok"
        ok, msg = drv.play_run("VIEWERAA", None, "test run")
        assert ok, msg
        run = v.run
        assert run.result and len(run.moves) > 5
        assert all(m["ok"] for m in run.moves)
        deadline = time.time() + 5
        while run.pending and time.time() < deadline:  # pictures arrive with the next frames
            time.sleep(0.05)
        assert not run.pending
        assert len(run.snaps) >= len(run.moves) - 2  # a picture after (nearly) every move
        assert run.snaps[run.moves[-1]["n"]][:8] == b"\x89PNG\r\n\x1a\n"
        # The page's server.
        srv = ThreadingHTTPServer(("127.0.0.1", 0), viewer.make_handler(v, frames, drv))
        threading.Thread(target=srv.serve_forever, daemon=True).start()
        port = srv.server_address[1]
        code, _, body = get(port, "/api/state")
        st = json.loads(body)
        assert code == 200 and st["run"]["seed"] == "VIEWERAA" and st["run"]["n_moves"] == len(run.moves)
        assert st["link"]["mod"] == "ok"
        code, headers, body = get(port, "/api/frame")
        assert code == 200 and body[:4] == b"\x89PNG" and headers["X-Seq"]
        code, _, _ = get(port, f"/api/frame?have={headers['X-Seq']}")
        assert code in (200, 204)
        code, _, body = get(port, f"/api/snap?run={run.id}&n={run.moves[-1]['n']}")
        assert code == 200 and body[:4] == b"\x89PNG"
        code, _, body = get(port, "/")
        assert code == 200 and b"BALATRO" in body
        # Controls: speed and pause reach the viewer and the mod's control file.
        req = urllib.request.Request(f"http://127.0.0.1:{port}/api/control", data=json.dumps(
            {"speed": "2", "paused": True}).encode(), headers={"Content-Type": "application/json"})
        urllib.request.urlopen(req, timeout=5).read()
        assert v.speed == "2" and v.paused
        time.sleep(0.2)
        assert mod.control.get("speed") == "2"
        srv.shutdown()
    finally:
        stop.set()
        mod.stop.set()
        game_srv.shutdown()


def test_waiting_for_the_game_to_finish_a_move(tmp_path):
    game_srv, mod, _v, _frames, drv, stop = start(tmp_path, speed="1")
    try:
        time.sleep(0.3)
        mod.busy = 3
        threading.Timer(0.6, lambda: setattr(mod, "busy", 0)).start()
        t = time.time()
        drv.wait_idle()
        waited = time.time() - t
        assert 0.5 < waited < 2.5, waited
        # Idle already: returns quickly.
        t = time.time()
        drv.wait_idle()
        assert time.time() - t < 0.6
    finally:
        stop.set()
        mod.stop.set()
        game_srv.shutdown()


def test_projections_follow_the_run(tmp_path):
    v = viewer.Viewer("max")
    drv = viewer.Driver(v, viewer.GameFiles(None), None, engine_only=True, say=lambda *_: None)
    ok, msg = drv.play_run("PROJECT2", None, "engine only")
    assert ok, msg
    analyst = viewer.Analyst(v)
    stop = threading.Event()
    threading.Thread(target=analyst.loop, args=(stop,), daemon=True).start()
    deadline = time.time() + 30
    n = len(v.run.moves)
    while time.time() < deadline and len(v.run.proj) < min(n + 1, 6):
        time.sleep(0.1)
    stop.set()
    assert n in v.run.proj, "the newest position is projected first"
    p = v.run.proj[n]
    assert 1 <= p["mean_ante"] <= 9 and 0 <= p["p_clear"] <= 1 and p["n"] >= 8


def test_the_png_encoder_round_trips():
    import zlib

    import numpy as np

    rgb = np.arange(4 * 3 * 3, dtype=np.uint8).reshape(4, 3, 3)
    data = viewer.png(rgb)
    assert data[:8] == b"\x89PNG\r\n\x1a\n"
    idat = data[data.index(b"IDAT") + 4: data.index(b"IEND") - 8]
    raw = zlib.decompress(idat)
    rows = [raw[i * 10:(i + 1) * 10] for i in range(4)]  # 4 rows of a filter byte + 3 RGB pixels
    assert len(raw) == 40 and all(r[0] == 0 for r in rows)
    assert np.frombuffer(b"".join(r[1:] for r in rows), dtype=np.uint8).reshape(4, 3, 3).tolist() == rgb.tolist()
