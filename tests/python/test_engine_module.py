"""The compiled engine module loads and agrees with the LuaJIT fixture."""

import math
from pathlib import Path

import balatro_meta as bm

FIXTURE = Path(__file__).resolve().parents[1] / "fixtures" / "rng" / "reference.tsv"


def same(a, b):
    """Equal as the fixture means it: NaN matches NaN (some strings hash to NaN, as in the game)."""
    return (math.isnan(a) and math.isnan(b)) or a == b


def _lines(kind):
    for line in FIXTURE.read_text().splitlines():
        parts = line.split("\t")
        if parts[0] == kind:
            yield parts


def test_version():
    assert bm.__version__


def test_hashes_match_luajit():
    for n, (_, s, want) in enumerate(_lines("HASH")):
        assert same(bm.pseudohash(s), float(want)), s
        if n > 500:
            break


def test_generator_matches_luajit():
    for n, (_, d, want) in enumerate(_lines("RAND")):
        got = bm.lua_random(float(d), 12)
        assert got == [float(x) for x in want.split(",")], d
        if n > 100:
            break


def test_streams_match_luajit():
    keys = ["shuffle", "nr1", "nr2", "cdt1", "rarity1sho", "Joker11sho", "edisho1", "lucky_mult",
            "lucky_money", "wheel_of_fortune", "boss", "Tag1", "stdset1", "front", "erratic",
            "misprint", "glass", "hook", "cavendish", "gros_michel", "Voucher1", "soul_Tarot1", "", "x"]
    for _, seed, want in list(_lines("SEQ"))[:20]:
        st = bm.Streams(seed)
        got = [st.pseudoseed(k) for _ in range(3) for k in keys]
        assert all(same(a, float(b)) for a, b in zip(got, want.split(","))), seed
