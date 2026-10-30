"""The viewer (scripts/viewer.py and viewer/index.html) against stand-ins for BalatroBot and the companion mod.

The stand-in mod follows control.txt like the real one: it reports its state about fifty times a second,
reports its live stream, saves a snapshot whenever the viewer asks for a new one (two files in turn) and,
only when asked, small pictures as files. The viewer must lock the game, play a run move by move in step
with the stand-in game, keep a picture of every move, and serve all of it to the page; the page must show
the live stream, and a kept picture when you click back.
"""
from __future__ import annotations

import json
import socket
import struct
import sys
import threading
import time
import urllib.request
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))

import watch
from test_watch import FakeGame, serve

import viewer


def picture(w: int, h: int, shade: int) -> bytes:
    import numpy as np

    return viewer.png(np.full((h, w, 3), shade % 256, dtype=np.uint8))


class FakeMod:
    """Writes state.txt, snapshots and (when asked) pictures the way mods/balatro_meta_watch does."""

    def __init__(self, folder: Path, w: int = 64, h: int = 36, stream_port: int | None = None) -> None:
        self.dir = folder / "balatro_meta"
        self.dir.mkdir(parents=True, exist_ok=True)
        self.w, self.h = w, h
        self.seq = 0
        self.busy = 0
        self.stop = threading.Event()
        self.control: dict = {}
        self.stream_port = stream_port or 13346
        self.stream_works = stream_port is not None
        self.clients = 1
        self.snap_taken: str | None = None
        self.snap_done: str | None = None
        self.snap_file = ""
        self.snap_slot = 0

    def loop(self) -> None:
        t0 = time.time()
        while not self.stop.is_set():
            try:
                text = (self.dir / "control.txt").read_text()
                self.control = dict(line.split("=", 1) for line in text.splitlines() if "=" in line)
            except OSError:
                pass
            streaming = self.control.get("stream") == "1"
            status = ("listening" if self.stream_works else "error") if streaming else "off"
            lines = [f"version={viewer.MOD_VERSION}", f"time={time.time() - t0:.3f}", f"busy={self.busy}",
                     "state=SHOP", f"locked={self.control.get('lock', '0')}", f"stream_port={self.stream_port}",
                     f"stream_status={status}", f"stream_clients={self.clients if streaming else 0}",
                     "stream_fps=30", "stream_epoch=1700000000.1"]
            ask = self.control.get("snap")
            if ask and ask != self.snap_taken:
                self.snap_taken = ask
                self.snap_slot = 1 - self.snap_slot
                name = f"snap_{self.snap_slot}.png"
                (self.dir / name).write_bytes(picture(self.w, self.h, int(ask)))
                self.snap_done, self.snap_file = ask, name
            if self.snap_done:
                lines += [f"snap={self.snap_done}", f"snap_file={self.snap_file}"]
            if self.control.get("frames") == "1":
                self.seq += 1
                name = f"frame_{self.seq % 3}.png"
                (self.dir / name).write_bytes(picture(self.w, self.h, self.seq * 7))
                lines += [f"seq={self.seq}", f"frame={name}", f"width={self.w}", f"height={self.h}"]
            (self.dir / "state.txt").write_text("\n".join(lines + ["end=1"]) + "\n")
            time.sleep(0.02)


def start(tmp_path: Path, speed: str = "max", stream_port: int | None = None):
    game_srv = serve(FakeGame())
    mod = FakeMod(tmp_path, stream_port=stream_port)
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


def page_server(v, frames, drv) -> ThreadingHTTPServer:
    srv = ThreadingHTTPServer(("127.0.0.1", 0), viewer.make_handler(v, frames, drv))
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    return srv


def get(port: int, path: str):
    with urllib.request.urlopen(f"http://127.0.0.1:{port}{path}", timeout=10) as r:
        return r.status, r.headers, r.read()


def post(port: int, body: dict) -> None:
    req = urllib.request.Request(f"http://127.0.0.1:{port}/api/control", data=json.dumps(body).encode(),
                                 headers={"Content-Type": "application/json"})
    urllib.request.urlopen(req, timeout=5).read()


def wait_for(cond, seconds: float = 5.0) -> bool:
    deadline = time.time() + seconds
    while time.time() < deadline:
        if cond():
            return True
        time.sleep(0.03)
    return cond()


@pytest.fixture
def quick_normal_speed(monkeypatch):
    """1x without the long pause between moves: moves still wait for their snapshot."""
    monkeypatch.setitem(viewer.SPEEDS, "1", (1, False, 0.01, 50.0))


def test_a_run_plays_in_step_with_the_game_and_reaches_the_page(tmp_path, monkeypatch, quick_normal_speed):
    monkeypatch.setattr(watch, "WATCH", tmp_path / "watch")
    monkeypatch.setattr(viewer, "WATCH", tmp_path / "watch")
    game_srv, mod, v, frames, drv, stop = start(tmp_path, speed="1", stream_port=13999)
    try:
        assert wait_for(frames.streaming)
        drv.control()
        assert wait_for(lambda: mod.control.get("frames") == "0")
        assert mod.control.get("lock") == "1" and mod.control.get("stream") == "1"
        assert mod.control.get("fps") == "30" and mod.control.get("speed") == "1"
        assert frames.mod_status() == "ok"
        ok, msg = drv.play_run("VIEWERAA", None, "test run")
        assert ok, msg
        run = v.run
        assert run.result and len(run.moves) > 5
        assert all(m["ok"] for m in run.moves)
        last = run.moves[-1]["n"]
        assert wait_for(lambda: last in run.snaps)
        assert 0 in run.snaps, "a picture of the start"
        assert len(run.snaps) >= len(run.moves), "a picture of every move"
        assert run.snaps[last][:8] == b"\x89PNG\r\n\x1a\n"
        assert not v.snap_wanted, "every snapshot asked for arrived"
        # The page's server.
        srv = page_server(v, frames, drv)
        port = srv.server_address[1]
        code, _, body = get(port, "/api/state")
        st = json.loads(body)
        assert code == 200 and st["run"]["seed"] == "VIEWERAA" and st["run"]["n_moves"] == len(run.moves)
        assert st["link"]["mod"] == "ok" and st["link"]["stream"] == "http://127.0.0.1:13999/stream"
        assert st["link"]["stream_fps"] == 30 and st["link"]["stream_clients"] == 1
        assert st["settings"]["speed"] == "1" and st["settings"]["analysis_error"] is None
        code, _, body = get(port, f"/api/snap?run={run.id}&n={last}")
        assert code == 200 and body[:4] == b"\x89PNG"
        code, _, body = get(port, "/")
        assert code == 200 and b"BALATRO" in body
        # Controls: speed and pause reach the viewer and the mod's control file.
        post(port, {"speed": "2", "paused": True})
        assert v.speed == "2" and v.paused
        assert wait_for(lambda: mod.control.get("speed") == "2")
        # The page can't show the stream: pictures as files until it can.
        post(port, {"stream_ok": False})
        assert wait_for(lambda: mod.control.get("frames") == "1")
        post(port, {"stream_ok": True})
        assert wait_for(lambda: mod.control.get("frames") == "0")
        srv.shutdown()
    finally:
        stop.set()
        mod.stop.set()
        game_srv.shutdown()


def test_pictures_as_files_when_the_stream_cannot_run(tmp_path):
    game_srv, mod, v, frames, drv, stop = start(tmp_path, speed="1", stream_port=None)
    try:
        assert wait_for(lambda: (frames.state or {}).get("stream_status") == "error")
        drv.control()
        assert wait_for(lambda: mod.control.get("frames") == "1")
        assert wait_for(lambda: frames.latest_png()[1] is not None)
        srv = page_server(v, frames, drv)
        port = srv.server_address[1]
        st = json.loads(get(port, "/api/state")[2])
        assert "stream" not in st["link"]
        code, headers, body = get(port, "/api/frame")
        assert code == 200 and body[:4] == b"\x89PNG" and headers["X-Seq"]
        code, _, _ = get(port, f"/api/frame?have={headers['X-Seq']}")
        assert code in (200, 204)
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
    assert v.analysis_error is None


def test_a_failing_projection_is_shown_not_fatal(monkeypatch):
    """An engine module without projections (built before this viewer) is reported on the page."""
    import balatro_meta._engine as engine

    class OldGame:
        def __init__(self, seed):
            pass

    monkeypatch.setattr(engine, "Game", OldGame)
    v = viewer.Viewer("max")
    viewer.Analyst(v).loop(threading.Event())  # returns at once
    assert v.analysis_error and "make watch" in v.analysis_error


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


# ---------------------------------------------------------------------- the page in a browser
def bmp(w: int, h: int, shade: int) -> bytes:
    """A top-down 32-bit BMP like the mod's stream sends (V4 header with channel masks)."""
    size = w * h * 4
    head = struct.pack("<2sIHHI", b"BM", 122 + size, 0, 0, 122)
    head += struct.pack("<IiiHHIIiiII", 108, w, -h, 1, 32, 3, size, 2835, 2835, 0, 0)
    head += struct.pack("<IIII", 0xFF, 0xFF00, 0xFF0000, 0xFF000000) + b"BGRs" + bytes(48)
    return head + bytes((shade % 256, 90, 40, 255)) * (w * h)


class StandInStream:
    """The mod's live stream, in Python: a moving 64x36 picture, 30 times a second, to every viewer."""

    def __init__(self) -> None:
        self.connections = 0
        self.stop = threading.Event()
        outer = self

        class H(BaseHTTPRequestHandler):
            def do_GET(self):
                if not self.path.startswith("/stream"):
                    self.send_error(404)
                    return
                outer.connections += 1
                self.send_response(200)
                self.send_header("Content-Type", "multipart/x-mixed-replace; boundary=bmframe")
                self.send_header("Cache-Control", "no-store")
                self.end_headers()
                n = 0
                try:
                    while not outer.stop.is_set():
                        n += 1
                        body = bmp(64, 36, n * 9)
                        self.wfile.write(b"--bmframe\r\nContent-Type: image/bmp\r\nContent-Length: "
                                         + str(len(body)).encode() + b"\r\n\r\n" + body + b"\r\n")
                        self.wfile.flush()
                        time.sleep(1 / 30)
                except (BrokenPipeError, ConnectionResetError):
                    pass

            def log_message(self, *a):
                pass

        self.srv = ThreadingHTTPServer(("127.0.0.1", 0), H)
        self.srv.daemon_threads = True
        self.port = self.srv.server_address[1]
        threading.Thread(target=self.srv.serve_forever, daemon=True).start()

    def close(self) -> None:
        self.stop.set()
        self.srv.shutdown()


def test_the_page_shows_the_live_stream_and_kept_pictures(tmp_path, monkeypatch, quick_normal_speed):
    sync_api = pytest.importorskip("playwright.sync_api")
    monkeypatch.setattr(watch, "WATCH", tmp_path / "watch")
    monkeypatch.setattr(viewer, "WATCH", tmp_path / "watch")
    stream = StandInStream()
    game_srv, mod, v, frames, drv, stop = start(tmp_path, speed="1", stream_port=stream.port)
    srv = None
    try:
        from balatro_meta._engine import Game

        g, moves = Game("PAGETEST"), []
        for _ in range(12):
            kind, arg = g.greedy_move()
            assert g.step(kind, arg)
            moves.append((kind, arg))
        ok, msg = drv.play_run("PAGETEST", moves, "page test", source="job")
        assert ok, msg
        assert wait_for(lambda: 12 in v.run.snaps)
        v.set_link("connected", "Your Balatro, live")
        srv = page_server(v, frames, drv)
        url = f"http://127.0.0.1:{srv.server_address[1]}/"
        with sync_api.sync_playwright() as pw:
            try:
                browser = pw.chromium.launch()
            except Exception as exc:  # noqa: BLE001 - no browser on this machine
                pytest.skip(f"no browser for Playwright: {exc}")
            page = browser.new_page(viewport={"width": 1500, "height": 950})
            page.route("**/fonts.googleapis.com/**", lambda route: route.abort())  # no web needed
            page.goto(url)
            live = f"http://127.0.0.1:{stream.port}/stream"
            # The live stream plays in the middle.
            page.wait_for_function("() => typeof LV !== 'undefined' && LV.loaded", timeout=10000)
            page.wait_for_timeout(1500)  # and it stays: no picture polled meanwhile replaces it
            assert page.evaluate("() => LV.loaded && LV.cid === 1")
            assert page.eval_on_selector("#feed", "e => e.src").startswith(live)
            assert page.eval_on_selector("#feed", "e => e.naturalWidth") == 64
            page.wait_for_function("() => document.getElementById('pilltext').textContent.includes('fps')")
            assert page.locator("#msg").is_hidden()
            assert page.locator("#still").is_hidden()
            # Click back to move 5: its kept picture, and the stream rests.
            page.click("#events li[data-k='5']")
            page.wait_for_function("() => document.getElementById('still').naturalWidth === 64")
            assert "/api/snap" in page.eval_on_selector("#still", "e => e.src")
            assert page.eval_on_selector("#feed", "e => e.getAttribute('src')") is None
            assert "Move 5" in page.inner_text("#viewingtext")
            # Back to live: the stream again.
            page.click("#backlive")
            page.wait_for_function("() => LV.loaded")
            assert page.locator("#still").is_hidden()
            assert page.eval_on_selector("#feed", "e => e.src").startswith(live)
            assert stream.connections >= 2
            # The speed buttons: 1x is on, and they reach the viewer.
            assert page.eval_on_selector("[data-speed='1']", "e => e.classList.contains('on')")
            page.click("[data-speed='max']")
            assert wait_for(lambda: v.speed == "max")
            browser.close()
    finally:
        if srv is not None:
            srv.shutdown()
        stream.close()
        stop.set()
        mod.stop.set()
        game_srv.shutdown()


def test_the_stand_in_stream_is_a_valid_multipart_bmp():
    """Guards the page test's stand-in: the same framing as the mod's own stream."""
    stream = StandInStream()
    try:
        with socket.create_connection(("127.0.0.1", stream.port), timeout=5) as s:
            s.sendall(b"GET /stream HTTP/1.1\r\nHost: x\r\n\r\n")
            data = b""
            while data.count(b"--bmframe") < 3:
                data += s.recv(1 << 16)
        assert b"multipart/x-mixed-replace; boundary=bmframe" in data
        part = data.split(b"--bmframe\r\n")[1]
        head, body = part.split(b"\r\n\r\n", 1)
        assert b"image/bmp" in head and body[:2] == b"BM" and len(body) >= 122 + 64 * 36 * 4
    finally:
        stream.close()
