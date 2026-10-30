"""The game's pictures for the viewer (scripts/art.py), read from a stand-in for Balatro.exe made here: a zip
behind some bytes, holding a small definitions file in the game's style and blank sheets. Nothing of the
real game is in this test; on your PC the viewer reads your own copy.
"""
from __future__ import annotations

import io
import json
import sys
import threading
import urllib.error
import urllib.request
import zipfile
from http.server import ThreadingHTTPServer
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))

import art

import viewer

DEFS = '''
function Game:set_render_settings()
    self.animation_atli = {
        {name = "blind_chips", path = "resources/textures/"..self.SETTINGS.GRAPHICS.texture_scaling.."x/Chips.png",px=34,py=34, frames = 3},
    }
    self.asset_atli = {
        {name = "cards_1", path = "resources/textures/"..self.SETTINGS.GRAPHICS.texture_scaling.."x/Deck.png",px=71,py=95},
        {name = "centers", path = "resources/textures/"..self.SETTINGS.GRAPHICS.texture_scaling.."x/Centers.png",px=71,py=95},
        {name = "Joker", path = "resources/textures/"..self.SETTINGS.GRAPHICS.texture_scaling.."x/Jokers.png",px=71,py=95},
        {name = "Tarot", path = "resources/textures/"..self.SETTINGS.GRAPHICS.texture_scaling.."x/Consumables.png",px=71,py=95},
        {name = 'tags', path = "resources/textures/"..self.SETTINGS.GRAPHICS.texture_scaling.."x/Tags.png",px=34,py=34},
        {name = 'stickers', path = "resources/textures/"..self.SETTINGS.GRAPHICS.texture_scaling.."x/Stickers.png",px=71,py=95},
    }
end
function Game:init_item_prototypes()
    self.P_SEALS = {
        Gold =      {order = 1,  discovered = false, set = "Seal"},
    }
    self.P_TAGS = {
        tag_economy =       {name = 'Economy Tag',      set = 'Tag', order = 1, pos = {x = 1,y = 0}},
    }
    self.P_BLINDS = {
        bl_small =           {name = 'Small Blind',  order = 1, dollars = 3, mult = 1, pos = {x=0, y=0}},
        bl_hook =            {name = 'The Hook',     order = 3, dollars = 5, mult = 2, pos = {x=0, y=1}},
    }
    self.P_CARDS = {
        H_2={name = "2 of Hearts",value = '2', suit = 'Hearts', pos = {x=0,y=0}},
        S_A={name = "Ace of Spades",value = 'Ace', suit = 'Spades', pos = {x=1,y=1}},
    }
    self.P_CENTERS = {
        c_base={max = 500, order = 1, name = "Default Base", pos = {x=1,y=0}, set = "Default", config = {}},
        m_bonus =   {max = 500, order = 2, name = "Bonus", set = "Enhanced", pos = {x=0,y=1}, config = {bonus = 30}},
        b_red=              {name = "Red Deck", stake = 1, order = 1, pos =   {x=0,y=0}, set = "Back", config = {}},
        j_joker=            {order = 1,  rarity = 1, cost = 2, name = "Joker", pos = {x=0,y=0}, set = "Joker", config = {mult = 4}},
        j_star=             {order = 2,  rarity = 4, cost = 20, name = "Star", pos = {x=1,y=0}, soul_pos = {x=1, y=1}, set = "Joker", config = {}},
        c_pluto=            {order = 1, cost = 3, name = "Pluto", pos = {x=1,y=0}, set = "Planet", config = {}},
    }
    self.shared_seals = {
        Gold = Sprite(0, 0, self.CARD_W, self.CARD_H, self.ASSET_ATLAS["centers"], {x = 1,y = 1}),
    }
    self.shared_sticker_eternal = Sprite(0, 0, self.CARD_W, self.CARD_H, self.ASSET_ATLAS["stickers"], {x = 0,y = 0})
end
'''
# Draws a position with n jokers (and no consumables) on the page and measures it, in one go so no poll redraws
# in between: the captions and counters must not overlap, and the jokers must stay inside their area.
LAYOUT_CHECK = """n => {
  const jokers = Array.from({length: n}, (_, i) => ({key: i % 2 ? 'j_star' : 'j_joker', name: 'J' + i, mult: 4 + i,
    sell: 1 + i, edition: null}));
  S.sel = 1; S.selView = {state: 'SHOP', ante: 3, round: 7, money: 12, hands_left: 4, discards_left: 3, chips: 0,
    jokers, joker_slots: 5, consumables: [], consumable_slots: 2, shop: [], shop_vouchers: [], shop_packs: [],
    reroll_cost: 5, vouchers: [], deck_left: 52, deck_total: 52};
  drawTable();
  const r = e => e.getBoundingClientRect();
  const hit = (a, b) => a.left < b.right - 0.5 && b.left < a.right - 0.5 && a.top < b.bottom - 0.5 && b.top < a.bottom - 0.5;
  const caps = [...document.querySelectorAll('#tbl .slots .item > .cap')].map(r);
  const counters = [...document.querySelectorAll('#tbl .slotcap')].map(r);
  const overlaps = [];
  const all = caps.concat(counters);
  for (let i = 0; i < all.length; i++) for (let j = i + 1; j < all.length; j++) if (hit(all[i], all[j])) overlaps.push([i, j]);
  const [jbox, cbox] = [...document.querySelectorAll('#tbl .slots')].map(r);
  const cards = [...document.querySelectorAll('#tbl .slots .jk')].map(r);
  const inside = cards.every(c => c.right <= jbox.right + 1 && c.left >= jbox.left - 1) && jbox.right <= cbox.left;
  // The note shown while looking at an earlier move covers none of the table's boxes.
  const banner = r(document.querySelector('#tbl .banner'));
  const covered = [...document.querySelectorAll('#tbl .ts .box, #tbl .slots, #tbl .tm-mid > *')].filter(e => hit(r(e), banner));
  S.sel = null; S.selView = null;
  return {caps: caps.length, overlaps, inside, covered: covered.map(e => e.className)};
}"""
PLAN_CHECK = """() => {
  const k = S.moves.length;
  S.sel = null;
  S.proj[String(k)] = {mean_ante: 5.2, p_win: 0.2, p_clear: 0.9, n: 48,
    teacher: {chances: [[300, 0.98]], main: 'Flush', total: 4200,
              plan: ['Beats the Small Blind 98% of the time.', 'Scores most with Flush (level 3).']}};
  renderChart();
  const el = document.querySelector('#plan');
  const r = el.getBoundingClientRect(), card = el.closest('.card').getBoundingClientRect();
  return {text: el.textContent, shown: r.height > 10, inside: r.left >= card.left - 0.5 && r.right <= card.right + 0.5};
}"""
SHEETS = {"Deck.png": (2, 2, 142, 190), "Centers.png": (2, 2, 142, 190), "Jokers.png": (2, 2, 142, 190),
          "Consumables.png": (2, 1, 142, 190), "Tags.png": (2, 1, 68, 68), "Stickers.png": (1, 1, 142, 190),
          "Chips.png": (3, 2, 68, 68)}


def blank_png(w: int, h: int, shade: int) -> bytes:
    """A plain grey w x h PNG."""
    import struct
    import zlib

    def chunk(kind: bytes, data: bytes) -> bytes:
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data) & 0xFFFFFFFF)

    rows = (b"\x00" + bytes([shade]) * (w * 3)) * h
    return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0))
            + chunk(b"IDAT", zlib.compress(rows, 1)) + chunk(b"IEND", b""))


@pytest.fixture
def game_exe(tmp_path) -> Path:
    """A stand-in for Balatro.exe: some program bytes, then the zip."""
    buf = io.BytesIO()
    with zipfile.ZipFile(buf, "w") as z:
        z.writestr("game.lua", DEFS)
        for i, (name, (cols, rows, w, h)) in enumerate(SHEETS.items()):
            z.writestr(f"resources/textures/2x/{name}", blank_png(cols * w, rows * h, 40 * i))
        z.writestr("resources/fonts/m6x11plus.ttf", b"\x00\x01\x00\x00 not really a font")
    exe = tmp_path / "Balatro.exe"
    exe.write_bytes(b"MZ" + b"\x00" * 1000 + buf.getvalue())
    return exe


def test_the_game_definitions_are_read_at_runtime():
    layout = art.parse(DEFS)
    assert layout["atlases"]["cards_1"] == ["Deck.png", 71, 95, 1]
    assert layout["atlases"]["blind_chips"] == ["Chips.png", 34, 34, 3]
    s = layout["sprites"]
    assert s["H_2"] == ["cards_1", 0, 0] and s["S_A"] == ["cards_1", 1, 1]
    assert s["c_base"] == ["centers", 1, 0] and s["m_bonus"] == ["centers", 0, 1] and s["b_red"] == ["centers", 0, 0]
    assert s["j_joker"] == ["Joker", 0, 0] and s["j_star"] == ["Joker", 1, 0, 1, 1], "a legendary's floating layer"
    assert s["c_pluto"] == ["Tarot", 1, 0]
    assert s["tag_economy"] == ["tags", 1, 0] and s["bl_hook"] == ["blind_chips", 0, 1]
    assert s["seal_Gold"] == ["centers", 1, 1] and s["sticker_eternal"] == ["stickers", 0, 0]
    assert layout["names"] == {"Economy Tag": "tag_economy", "Small Blind": "bl_small", "The Hook": "bl_hook"}


def test_the_sheets_and_font_come_from_the_game(game_exe):
    a = art.GameArt(game_exe)
    m = a.manifest()
    assert m["atlases"]["cards_1"] == {"url": "/art/sheet/cards_1.png", "w": 284, "h": 380, "px": 142, "py": 190,
                                       "cols": 2, "rows": 2, "frames": 1}
    assert m["atlases"]["blind_chips"]["cols"] == 3 and m["atlases"]["tags"]["px"] == 68
    assert m["sprites"]["j_star"] == ["Joker", 1, 0, 1, 1]
    assert a.sheet("Joker")[:8] == b"\x89PNG\r\n\x1a\n" and a.sheet("nope") is None
    assert a.font().startswith(b"\x00\x01")
    assert art.load(game_exe.parent) is not None
    assert art.load(game_exe.parent / "missing") is None, "no game: the page draws its own cards"


def test_the_viewer_serves_the_art_to_this_pc_only(game_exe, tmp_path):
    v = viewer.Viewer("max")
    jobs = viewer.JobFiles(tmp_path / "watch", tmp_path / "logs")

    def serve(a):
        srv = ThreadingHTTPServer(("127.0.0.1", 0), viewer.make_handler(v, jobs, None, a))
        threading.Thread(target=srv.serve_forever, daemon=True).start()
        return srv

    def get(port, path):
        with urllib.request.urlopen(f"http://127.0.0.1:{port}{path}", timeout=5) as r:
            return r.headers["Content-Type"], r.read()

    srv = serve(art.GameArt(game_exe))
    try:
        port = srv.server_address[1]
        ctype, body = get(port, "/art/manifest.json")
        assert ctype.startswith("application/json") and "j_joker" in json.loads(body)["sprites"]
        ctype, body = get(port, "/art/sheet/Joker.png")
        assert ctype == "image/png" and body[:4] == b"\x89PNG"
        assert get(port, "/art/font.ttf")[0] == "font/ttf"
        with pytest.raises(urllib.error.HTTPError):
            get(port, "/art/sheet/../game.lua")
    finally:
        srv.shutdown()
    srv = serve(None)
    try:
        with pytest.raises(urllib.error.HTTPError) as e:
            get(srv.server_address[1], "/art/manifest.json")
        assert e.value.code == 404
    finally:
        srv.shutdown()


def test_the_page_draws_with_the_game_art(game_exe, tmp_path):
    sync_api = pytest.importorskip("playwright.sync_api")
    v = viewer.Viewer("max")
    jobs = viewer.JobFiles(tmp_path / "watch", tmp_path / "logs")
    stop = threading.Event()
    rep = viewer.Replayer(v, jobs)
    threading.Thread(target=rep.loop, args=(stop,), daemon=True).start()
    srv = ThreadingHTTPServer(("127.0.0.1", 0), viewer.make_handler(v, jobs, None, art.GameArt(game_exe)))
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    try:
        with sync_api.sync_playwright() as pw:
            try:
                browser = pw.chromium.launch()
            except Exception as exc:  # noqa: BLE001 - no browser on this machine
                pytest.skip(f"no browser for Playwright: {exc}")
            page = browser.new_page(viewport={"width": 1500, "height": 900})
            errors = []
            page.on("pageerror", lambda e: errors.append(str(e)))
            page.route("**/fonts.googleapis.com/**", lambda route: route.abort())
            page.goto(f"http://127.0.0.1:{srv.server_address[1]}/")
            page.wait_for_function("() => ART !== null")
            page.wait_for_function("() => document.querySelector('#tbl .pc.art, #tbl .jk.art, #tbl .chipart') !== null",
                                   timeout=20000)
            style = page.eval_on_selector("#tbl .art", "e => e.getAttribute('style')")
            assert "/art/sheet/" in style
            # Each joker's caption sits under that joker, clear of the slot counters; a crowded row stays in its box.
            for n in (5, 11):
                got = page.evaluate(LAYOUT_CHECK, n)
                assert got["caps"] == n and got["overlaps"] == [], got
                assert got["inside"] and got["covered"] == [], got
            # The teacher's plan in words sits in the outlook card.
            got = page.evaluate(PLAN_CHECK)
            assert got["shown"] and got["inside"], got
            assert got["text"].startswith("Teacher's plan") and "Scores most with Flush" in got["text"], got
            assert not errors, errors
            browser.close()
    finally:
        stop.set()
        srv.shutdown()
