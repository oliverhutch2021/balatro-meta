"""The companion mod (mods/balatro_meta_watch) against a stand-in for LOVE and the game, under LuaJIT."""
import shutil
import socket
import subprocess
import time
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[2]


def luajit() -> str:
    exe = shutil.which("luajit")
    if not exe:
        pytest.skip("LuaJIT is not installed")
    return exe


def test_the_mod_locks_input_sets_speed_and_writes_frames():
    r = subprocess.run([luajit(), "tests/lua/test_watch_mod.lua"], cwd=ROOT, capture_output=True, text=True,
                       check=False)
    assert r.returncode == 0, r.stdout + r.stderr
    assert "all checks passed" in r.stdout


def free_port() -> int:
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def read_until(sock: socket.socket, buf: bytearray, marker: bytes) -> int:
    while marker not in buf:
        chunk = sock.recv(1 << 16)
        if not chunk:
            raise AssertionError("the stream closed early")
        buf += chunk
    return buf.index(marker)


def test_the_stream_server_sends_live_bmp_pictures():
    """The mod's own stream code (LuaSocket in a thread in the game) with a moving test picture: a browser
    gets a multipart stream of whole BMP pictures that change, and the mod hears about the viewer."""
    exe = luajit()
    if subprocess.run([exe, "-e", "require('socket')"], capture_output=True, check=False).returncode:
        pytest.skip("LuaSocket is not installed for LuaJIT")
    port = free_port()
    proc = subprocess.Popen([exe, "tests/lua/run_streamer.lua", str(port), "3"], cwd=ROOT, stdout=subprocess.PIPE,
                            stderr=subprocess.PIPE, text=True)
    try:
        deadline = time.time() + 5
        sock = None
        while sock is None and time.time() < deadline:
            try:
                sock = socket.create_connection(("127.0.0.1", port), timeout=3)
            except OSError:
                time.sleep(0.05)
        assert sock is not None, "the stream server never listened"
        sock.sendall(b"GET /stream HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n")
        buf = bytearray()
        end = read_until(sock, buf, b"\r\n\r\n")
        head = bytes(buf[:end]).decode()
        assert head.startswith("HTTP/1.1 200") and "multipart/x-mixed-replace; boundary=bmframe" in head
        del buf[:end + 4]
        pictures = []
        t0 = time.time()
        while len(pictures) < 12:
            end = read_until(sock, buf, b"\r\n\r\n")
            part_head = bytes(buf[:end]).decode()
            assert part_head.startswith("--bmframe") and "Content-Type: image/bmp" in part_head
            size = int(part_head.split("Content-Length: ")[1].split("\r\n")[0])
            del buf[:end + 4]
            while len(buf) < size + 2:
                buf += sock.recv(1 << 16)
            body = bytes(buf[:size])
            del buf[:size + 2]
            assert body[:2] == b"BM" and len(body) == 122 + 64 * 36 * 4
            pictures.append(body)
        rate = len(pictures) / (time.time() - t0)
        sock.close()
        assert len({p[122:] for p in pictures}) > 6, "the pictures move"
        assert rate > 10, f"{rate:.1f} pictures a second"
        out, _ = proc.communicate(timeout=10)
        assert f"listening={port}" in out and "clients=1" in out and "fps=" in out, out
    finally:
        proc.kill()
