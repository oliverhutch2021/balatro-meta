"""The companion mod (mods/balatro_meta_watch) against a stand-in for LOVE and the game, under LuaJIT."""
import shutil
import subprocess
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[2]


def test_the_mod_locks_input_sets_speed_and_writes_frames():
    luajit = shutil.which("luajit")
    if not luajit:
        pytest.skip("LuaJIT is not installed")
    r = subprocess.run([luajit, "tests/lua/test_watch_mod.lua"], cwd=ROOT, capture_output=True, text=True,
                       check=False)
    assert r.returncode == 0, r.stdout + r.stderr
    assert "all checks passed" in r.stdout
