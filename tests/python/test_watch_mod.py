"""The companion mod (mods/balatro_meta_watch) against a stand-in for LOVE and the game, under LuaJIT."""
import os
import shutil
import subprocess
import sys
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))

import checker


def mod_test(expect: str) -> subprocess.CompletedProcess:
    luajit = shutil.which("luajit")
    if not luajit:
        pytest.skip("LuaJIT is not installed")
    return subprocess.run([luajit, "tests/lua/test_watch_mod.lua"], cwd=ROOT, capture_output=True, text=True,
                          check=False, env={**os.environ, "EXPECT_MOD_VERSION": expect})


def test_the_mod_reports_the_game_and_readies_the_next_start():
    # The version the mod reports must be the one the checks accept (checker.MOD_VERSION): a bump on one side only
    # fails here instead of leaving the checks waiting on a game they refuse.
    r = mod_test(checker.MOD_VERSION)
    assert r.returncode == 0, r.stdout + r.stderr
    assert "all checks passed" in r.stdout


def test_a_version_the_checks_dont_expect_fails():
    r = mod_test(checker.MOD_VERSION + "-other")
    assert r.returncode != 0 and "the checks expect" in r.stderr
