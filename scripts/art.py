"""The game's own pictures for the viewer page, read from your installed Balatro at runtime.

The ownership rule: nothing from the game goes into this repo (no code, text, art or sound). So nothing
here holds any of it. Each time the viewer starts, this reads your copy of Balatro.exe (a LOVE game: a zip
archive behind the program), finds which sheet and which cell every card, joker, consumable, voucher,
pack, tag, seal, sticker and blind chip is drawn from (from the game's own definitions, read then), and
serves the sheets and the game's font to the page, from memory, to your own browser only. If the game
isn't found or can't be read, the page draws its own cards instead.
"""
from __future__ import annotations

import re
import threading
import zipfile
from pathlib import Path

SCALE = 2  # the game ships 1x and 2x sheets; the page uses the 2x ones
# Which sheet each kind of definition is drawn from (by its `set`).
SET_ATLAS = {"Joker": "Joker", "Tarot": "Tarot", "Planet": "Tarot", "Spectral": "Tarot", "Voucher": "Voucher",
             "Booster": "Booster", "Back": "centers", "Enhanced": "centers", "Default": "centers",
             "Edition": "centers", "Tag": "tags", "Stake": "chips"}

_ATLAS = re.compile(r"\{\s*name\s*=\s*[\"'](\w+)[\"']\s*,\s*path\s*=\s*\"resources/textures/\"\s*\.\.[^\"]*\"x/([\w.]+)\"\s*,"
                    r"\s*px\s*=\s*(\d+)\s*,\s*py\s*=\s*(\d+)\s*(?:,\s*frames\s*=\s*(\d+))?")
_ENTRY = re.compile(r"^\s*(\w+)\s*=\s*\{(.*)$")
_POS = re.compile(r"(?<![\w])pos\s*=\s*\{\s*x\s*=\s*(\d+)\s*,\s*y\s*=\s*(\d+)\s*\}")
_SOUL = re.compile(r"soul_pos\s*=\s*\{\s*x\s*=\s*(\d+)\s*,\s*y\s*=\s*(\d+)\s*\}")
_SET = re.compile(r"(?<![\w])set\s*=\s*[\"'](\w+)[\"']")
_NAME = re.compile(r"(?<![\w])name\s*=\s*[\"']([^\"']+)[\"']")
_OWN_ATLAS = re.compile(r"(?<![\w])atlas\s*=\s*[\"'](\w+)[\"']")
_SPRITE = re.compile(r"(\w+)\s*=\s*Sprite\(\s*0\s*,\s*0\s*,\s*self\.CARD_W\s*,\s*self\.CARD_H\s*,\s*"
                     r"self\.ASSET_ATLAS\[[\"'](\w+)[\"']\]\s*,\s*\{\s*x\s*=\s*(\d+)\s*,\s*y\s*=\s*(\d+)\s*\}\s*\)")


def block(text: str, start: str) -> str:
    """The text of the `{ ... }` block that follows `start` (braces balanced), or ''."""
    i = text.find(start)
    if i < 0:
        return ""
    i = text.find("{", i)
    depth = 0
    for j in range(i, len(text)):
        if text[j] == "{":
            depth += 1
        elif text[j] == "}":
            depth -= 1
            if depth == 0:
                return text[i:j + 1]
    return ""


def parse(game_lua: str) -> dict:
    """Sheets and cells from the game's definitions: {"atlases": {name: [file, px, py, frames]},
    "sprites": {key: [atlas, x, y] (+ soul x, y)}, "names": {display name: key}}."""
    atlases = {m[1]: [m[2], int(m[3]), int(m[4]), int(m[5] or 1)] for m in _ATLAS.finditer(game_lua)}
    sprites: dict[str, list] = {}
    names: dict[str, str] = {}
    cards = block(game_lua, "self.P_CARDS")
    blinds = block(game_lua, "self.P_BLINDS")
    for line in game_lua.splitlines():
        m = _ENTRY.match(line)
        if not m:
            continue
        key, rest = m[1], m[2]
        pos = _POS.search(rest)
        if not pos:
            continue
        kind = _SET.search(rest)
        own = _OWN_ATLAS.search(rest)
        if own and own[1] in atlases:
            atlas = own[1]
        elif kind and kind[1] in SET_ATLAS:
            atlas = SET_ATLAS[kind[1]]
        elif line.strip() and line in cards:
            atlas = "cards_1"
        elif key.startswith("bl_") and line in blinds:
            atlas = "blind_chips"
        else:
            continue
        cell = [atlas, int(pos[1]), int(pos[2])]
        soul = _SOUL.search(rest)
        if soul:
            cell += [int(soul[1]), int(soul[2])]
        sprites[key] = cell
        name = _NAME.search(rest)
        if name and key.startswith(("bl_", "tag_")):
            names[name[1]] = key
    for prefix, start in (("seal_", "shared_seals"), ("sticker_", "shared_stickers")):
        for m in _SPRITE.finditer(block(game_lua, start)):
            sprites[prefix + m[1]] = [m[2], int(m[3]), int(m[4])]
    for m in re.finditer(r"shared_sticker_(\w+)\s*=\s*Sprite\([^)]*ASSET_ATLAS\[[\"'](\w+)[\"']\]\s*,\s*"
                         r"\{\s*x\s*=\s*(\d+)\s*,\s*y\s*=\s*(\d+)\s*\}\s*\)", game_lua):
        sprites["sticker_" + m[1]] = [m[2], int(m[3]), int(m[4])]
    return {"atlases": atlases, "sprites": sprites, "names": names}


class GameArt:
    """Your Balatro's sheets and font, read once from the game and kept in memory."""

    def __init__(self, exe: Path) -> None:
        self.exe = exe
        self.lock = threading.Lock()
        self.zip = zipfile.ZipFile(exe)
        layout = parse(self.zip.read("game.lua").decode("utf-8", "replace"))
        files = set(self.zip.namelist())
        self.atlases = {name: a for name, a in layout["atlases"].items()
                        if f"resources/textures/{SCALE}x/{a[0]}" in files}
        used = {s[0] for s in layout["sprites"].values()}
        self.atlases = {k: v for k, v in self.atlases.items() if k in used}
        self.sprites = {k: s for k, s in layout["sprites"].items() if s[0] in self.atlases}
        self.names = layout["names"]
        self.cache: dict[str, bytes] = {}
        if not self.sprites:
            raise ValueError("no sprites found in the game's definitions")

    def manifest(self) -> dict:
        """What the page needs: each sheet's cell size and grid, and each sprite's cell."""
        out = {}
        for name, (file, px, py, frames) in self.atlases.items():
            w, h = self.size(name)
            out[name] = {"url": f"/art/sheet/{name}.png", "w": w, "h": h, "px": px * SCALE, "py": py * SCALE,
                         "cols": max(1, w // (px * SCALE)), "rows": max(1, h // (py * SCALE)), "frames": frames}
        return {"atlases": out, "sprites": self.sprites, "names": self.names, "font": "/art/font.ttf"}

    def sheet(self, name: str) -> bytes | None:
        if name not in self.atlases:
            return None
        return self.read(f"resources/textures/{SCALE}x/{self.atlases[name][0]}")

    def font(self) -> bytes | None:
        try:
            return self.read("resources/fonts/m6x11plus.ttf")
        except KeyError:
            return None

    def read(self, member: str) -> bytes:
        with self.lock:
            if member not in self.cache:
                self.cache[member] = self.zip.read(member)
            return self.cache[member]

    def size(self, name: str) -> tuple[int, int]:
        png = self.sheet(name) or b""
        if png[:8] != b"\x89PNG\r\n\x1a\n":
            return 0, 0
        return int.from_bytes(png[16:20], "big"), int.from_bytes(png[20:24], "big")


def load(folder: Path | None = None) -> GameArt | None:
    """Your Balatro's art, or None (no game found, or it can't be read)."""
    try:
        if folder is None:
            from rng_game import find_balatro

            folder = find_balatro()
        return GameArt(folder / "Balatro.exe")
    except (SystemExit, OSError, KeyError, ValueError, zipfile.BadZipFile, ImportError):
        return None
