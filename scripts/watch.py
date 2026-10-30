"""Watch the bots play in your own Balatro, with live stats around the game.

    make watch                 open the viewer in your browser (Ctrl+C stops; the game stays open)
    make watch SPEED=4         start at another speed (1, 2, 4, 10 or max; the page can change it)
    make watch-dry             replay runs in the engine only, printed here (no game, no browser)
    make watch-mod             (re)install the companion mod into your Balatro Mods folder

Your copy of Balatro does the drawing: BalatroBot plays exactly the moves the engine made, and the
companion mod (mods/balatro_meta_watch) passes a picture of the game to the viewer page
(scripts/viewer.py), blocks your mouse and keyboard while the bots play, and runs the game at the page's
speed. Each replay is also a check of the engine: after every move the game's state (money, hands,
discards, the cards in hand, the round's chips, the blinds and tags, jokers and consumables, the shop's
cards, vouchers, packs and prices, and an open pack's cards) is compared with the engine's. A difference
is saved to .sync/watch/mismatches/ with both states and the viewer moves on. BalatroBot has no call for
"buy and use" or for a boss reroll; a replay stops (without a mismatch) before either.

Before the first time: RUNNING.md, step 5 (Lovely, Steamodded, BalatroBot; other gameplay mods off).
make watch installs or updates the companion mod itself; restart Balatro once afterwards.
"""
from __future__ import annotations

import json
import os
import subprocess
import sys
import time
import urllib.error
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
WATCH = ROOT / ".sync" / "watch"
sys.path.insert(0, str(ROOT / "scripts"))

MOVE_NAMES = ("select blind", "skip blind", "cash out", "leave shop", "sort by rank", "sort by suit", "play",
              "discard", "move a card", "move a joker", "sell a joker", "reroll the shop", "buy", "buy and use",
              "redeem a voucher", "open a pack", "sell a consumable", "use a consumable", "take a pack card",
              "skip the pack", "reroll the boss")
# Moves BalatroBot has no call for: a replay stops before them (not a mismatch).
UNSUPPORTED = {13: "BalatroBot has no 'buy and use'", 20: "BalatroBot has no boss reroll"}
# Moves BalatroBot only takes in some states (engine state before the move).
ALLOWED_STATES = {
    9: {"SELECTING_HAND", "SHOP", "SMODS_BOOSTER_OPENED"},  # move a joker
    10: {"SELECTING_HAND", "SHOP"},  # sell a joker
    16: {"SELECTING_HAND", "SHOP"},  # sell a consumable
    17: {"SELECTING_HAND", "SHOP"},  # use a consumable
}


def unsupported(kind: int, state: str) -> str | None:
    """Why BalatroBot can't make this move now, or None."""
    if kind in UNSUPPORTED:
        return UNSUPPORTED[kind]
    if kind in ALLOWED_STATES and state not in ALLOWED_STATES[kind]:
        return f"BalatroBot can't do that in {state}"
    return None
STABLE = {"MENU", "BLIND_SELECT", "SELECTING_HAND", "ROUND_EVAL", "SHOP", "GAME_OVER", "SMODS_BOOSTER_OPENED"}
# States where the hand can be re-ordered to match the engine's (BalatroBot's `rearrange`).
HAND_STATES = {"SELECTING_HAND", "SMODS_BOOSTER_OPENED"}


# ------------------------------------------------------------------ BalatroBot
BALATROBOT_URL = "http://127.0.0.1:12346"
NO_PROXY = urllib.request.build_opener(urllib.request.ProxyHandler({}))


class WinBridge:
    """BalatroBot's Windows localhost, reached through scripts/win_bridge.py under Windows Python.

    WSL's default (NAT) networking can't reach a server on Windows' 127.0.0.1, which is where
    BalatroBot listens. The bridge runs on Windows through WSL's interop and relays each call over
    stdin/stdout, so no networking or firewall change is needed."""

    def __init__(self, python: str, script: str, cwd: str | None = "/mnt/c") -> None:
        self.proc = subprocess.Popen([python, "-u", script], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                     stderr=subprocess.DEVNULL, text=True, cwd=cwd)

    def post(self, url: str, body: dict, timeout: float):
        if self.proc.poll() is not None:
            raise OSError("the Windows bridge has stopped")
        self.proc.stdin.write(json.dumps({"url": url, "body": body, "timeout": timeout}) + "\n")
        self.proc.stdin.flush()
        line = self.proc.stdout.readline()
        if not line:
            raise OSError("the Windows bridge has stopped")
        out = json.loads(line)
        if not out.get("ok"):
            raise OSError(out.get("error") or "the Windows bridge failed")
        return out["result"]

    def close(self) -> None:
        if self.proc.poll() is None:
            self.proc.terminate()


_bridge: list = []  # the one bridge of this process, once started (or [None] if it can't be)


def windows_bridge() -> WinBridge | None:
    """Start the bridge once, under WSL only (needs a Windows Python; BM_WIN_PYTHON overrides)."""
    if not _bridge:
        bridge = None
        try:
            is_wsl = "microsoft" in Path("/proc/version").read_text().lower()
        except OSError:
            is_wsl = False
        if is_wsl:
            try:
                from rng_game import find_windows_python, win_path
                bridge = WinBridge(str(find_windows_python()), win_path(ROOT / "scripts" / "win_bridge.py"))
            except (SystemExit, RuntimeError, OSError, ImportError):
                bridge = None
        _bridge.append(bridge)
    return _bridge[0]


class Api:
    def __init__(self, url: str, bridge: WinBridge | None = None) -> None:
        self.url = url
        self.bridge = bridge
        self.n = 0

    def call(self, method: str, params: dict | None = None, timeout: float = 120.0):
        self.n += 1
        body = {"jsonrpc": "2.0", "method": method, "id": self.n}
        if params:
            body["params"] = params
        if self.bridge:
            out = self.bridge.post(self.url, body, timeout)
        else:
            req = urllib.request.Request(self.url, data=json.dumps(body).encode(),
                                         headers={"Content-Type": "application/json"})
            with NO_PROXY.open(req, timeout=timeout) as r:
                out = json.loads(r.read().decode())
        if "error" in out:
            e = out["error"]
            raise RuntimeError(f"BalatroBot refused {method}: {e.get('message')} ({e.get('code')})")
        return out.get("result")


def try_urls() -> Api | None:
    """BalatroBot directly (mirrored networking, or BM_BALATROBOT_URL), else through the bridge."""
    url = os.environ.get("BM_BALATROBOT_URL") or BALATROBOT_URL
    candidates = [Api(url)]
    bridge = windows_bridge()
    if bridge and not os.environ.get("BM_BALATROBOT_URL"):
        candidates.append(Api(url, bridge))
    for api in candidates:
        try:
            if api.call("health", timeout=3) is not None:
                return api
        except (urllib.error.URLError, OSError, RuntimeError, ValueError):
            continue
    return None


def launch_game(speed: float) -> bool:
    """Open Balatro through BalatroBot's launcher (uvx on Windows). False if uvx isn't installed."""
    try:
        from rng_game import find_balatro, sh, windows_profile
    except ImportError:
        return False
    prof = windows_profile()
    uvx = prof / ".local" / "bin" / "uvx.exe" if prof else None
    if not uvx or not uvx.exists():
        return False
    game = find_balatro()
    lovely = game / "winmm.dll" if (game / "winmm.dll").exists() else game / "version.dll"
    cmd = [str(uvx), "balatrobot", "serve", "--gamespeed", str(max(1, round(speed))),
           "--balatro-path", sh("wslpath", "-w", str(game)),
           "--love-path", sh("wslpath", "-w", str(game / "Balatro.exe")),
           "--lovely-path", sh("wslpath", "-w", str(lovely))]
    print("Opening Balatro through BalatroBot:", " ".join(cmd[1:]))
    subprocess.Popen(cmd, cwd="/mnt/c", stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, start_new_session=True)
    return True


# ------------------------------------------------------------------ states
def norm_mod(v) -> str | None:
    if v in (None, "", False):
        return None
    return str(v).upper()


def card_entry(c: dict) -> dict:
    """A card as BalatroBot reports it. A face-down card it doesn't name becomes a wildcard (key None)."""
    mod = c.get("modifier") or {}
    st = c.get("state") or {}
    return {"key": c.get("key"), "enhancement": norm_mod(mod.get("enhancement")),
            "edition": norm_mod(mod.get("edition")), "seal": norm_mod(mod.get("seal")),
            "debuff": bool(st.get("debuff"))}


def same_card(engine_card: dict, game_card: dict) -> bool:
    return game_card["key"] is None or engine_card == game_card


def area_cards(x) -> list | None:
    """The cards of an area as BalatroBot reports it (an object with `cards`, or a list); None if absent."""
    if isinstance(x, dict):
        return x.get("cards") or []
    if isinstance(x, list):
        return x
    return None


def item_entry(c: dict) -> dict:
    """A shop, pack or consumable card: key, edition, and the price and sell value the game shows."""
    mod = c.get("modifier") or {}
    cost = c.get("cost")
    out = {"key": c.get("key"), "edition": norm_mod(mod.get("edition"))}
    if c.get("set") in ("DEFAULT", "ENHANCED"):  # playing cards: their enhancement and seal count too
        out["enhancement"] = norm_mod(mod.get("enhancement"))
        out["seal"] = norm_mod(mod.get("seal"))
    if isinstance(cost, dict):
        if cost.get("buy") is not None:
            out["cost"] = cost["buy"]
        if cost.get("sell") is not None:
            out["sell"] = cost["sell"]
    elif cost is not None:
        out["cost"] = cost
    return out


def item_list(x) -> list | None:
    cards = area_cards(x)
    return None if cards is None else [item_entry(c) for c in cards]


def game_view(gs: dict) -> dict:
    """BalatroBot's game state in the engine's terms."""
    rnd = gs.get("round") or {}
    hand = (gs.get("hand") or {}).get("cards") or []
    jokers = (gs.get("jokers") or {}).get("cards") or []
    blinds_raw = gs.get("blinds") or {}
    blinds = {}
    items = blinds_raw.items() if isinstance(blinds_raw, dict) else ((b.get("type", ""), b) for b in blinds_raw)
    for k, b in items:
        if isinstance(b, dict):
            blinds[str(b.get("type") or k).lower()] = {"name": b.get("name"), "status": b.get("status"),
                                                        "tag_name": b.get("tag_name")}
    return {
        "state": gs.get("state"),
        "ante": gs.get("ante_num"),
        "round": gs.get("round_num"),
        "money": gs.get("money"),
        "hands_left": rnd.get("hands_left"),
        "discards_left": rnd.get("discards_left"),
        "chips": rnd.get("chips"),
        "hand": [card_entry(c) for c in hand],
        "jokers": [dict({"key": j.get("key"), "edition": norm_mod((j.get("modifier") or {}).get("edition"))},
                        **({"sell": j["cost"]["sell"]} if isinstance(j.get("cost"), dict)
                           and j["cost"].get("sell") is not None else {}))
                   for j in jokers],
        "blinds": blinds,
        "reroll_cost": rnd.get("reroll_cost"),
        "consumables": item_list(gs.get("consumables")),
        "shop": item_list(gs.get("shop")),
        "shop_vouchers": item_list(gs.get("vouchers")),
        "shop_packs": item_list(gs.get("packs")),
        "pack": item_list(gs.get("pack")),
    }


def engine_view(game) -> dict:
    v = dict(game.view())
    v["hand"] = [{k: c[k] for k in ("key", "enhancement", "edition", "seal", "debuff")} for c in v["hand"]]
    v["jokers"] = [{"key": j["key"], "edition": j["edition"], "sell": j["sell"]} for j in v["jokers"]]
    v["blinds"] = {k: {"name": b["name"], "status": b["status"], "tag_name": b.get("tag_name")}
                   for k, b in v["blinds"].items()}
    # Opened booster slots leave the game's area; the engine keeps them as None.
    v["pack_slots"] = list(v["shop_packs"])
    v["shop_packs"] = [p for p in v["shop_packs"] if p is not None]
    return v


def pack_key(k):
    """The run's first Buffoon Pack picks its art variant unseeded: compare it without the number."""
    return "p_buffoon_normal" if isinstance(k, str) and k.startswith("p_buffoon_normal") else k


# What is compared, area by area, beyond key and edition. Prices matter where they are paid (the shop)
# and sell values where they are held; a pack's cards are taken for free.
SHOP_FIELDS = ("cost", "enhancement", "seal")
HELD_FIELDS = ("sell",)
PACK_FIELDS = ("enhancement", "seal")


def same_items(e_items: list, g_items: list, fields=SHOP_FIELDS) -> bool:
    """Cards in one area: keys and editions, plus `fields` when the game shows them."""
    if len(e_items) != len(g_items):
        return False
    for a, b in zip(e_items, g_items):
        if pack_key(a.get("key")) != pack_key(b.get("key")) or a.get("edition") != b.get("edition"):
            return False
        for k in fields:
            if k in b and a.get(k) != b[k]:
                return False
    return True


def differences(e: dict, g: dict) -> list[str]:
    """What the game shows differently from the engine, for the fields that state shows."""
    out = []

    def cmp(name, a, b):
        if a != b:
            out.append(f"{name}: engine {a!r}, game {b!r}")

    def cmp_items(name, key, fields=SHOP_FIELDS):
        if g.get(key) is not None and not same_items(e[key], g[key], fields):
            shown = ("key", "edition", *fields)
            ev = [{k: i.get(k) for k in shown if k in i} for i in e[key]]
            gv = [{k: i.get(k) for k in shown if k in i} for i in g[key]]
            out.append(f"{name}: engine {ev!r}, game {gv!r}")

    def cmp_jokers():
        if not same_items(e["jokers"], g["jokers"], HELD_FIELDS):
            shown = ("key", "edition", "sell")
            ev = [{k: j.get(k) for k in shown} for j in e["jokers"]]
            gv = [{k: j.get(k) for k in shown} for j in g["jokers"]]
            out.append(f"jokers: engine {ev!r}, game {gv!r}")

    def cmp_hand():
        named = [(c, d) for c, d in zip(e["hand"], g["hand"]) if d["key"] is not None]
        if len(e["hand"]) != len(g["hand"]) or any(c["key"] != d["key"] for c, d in named):
            cmp("hand", [c["key"] for c in e["hand"]], [c["key"] for c in g["hand"]])
        elif any(c != d for c, d in named):
            cmp("hand modifiers", e["hand"], g["hand"])

    cmp("state", e["state"], g["state"])
    if out:
        return out
    for k in ("ante", "money"):
        cmp(k, e[k], g[k])
    cmp_items("consumables", "consumables", HELD_FIELDS)
    st = e["state"]
    if st == "SELECTING_HAND":
        for k in ("round", "hands_left", "discards_left"):
            cmp(k, e[k], g[k])
        if g["chips"] is not None:
            cmp("round chips", float(e["chips"]), float(g["chips"]))
        cmp_hand()
        cmp_jokers()
    if st == "SHOP":
        cmp_jokers()
        cmp_items("shop cards", "shop")
        cmp_items("shop vouchers", "shop_vouchers")
        cmp_items("shop packs", "shop_packs")
        if g.get("reroll_cost") is not None:
            cmp("reroll cost", e["reroll_cost"], g["reroll_cost"])
    if st == "SMODS_BOOSTER_OPENED":
        cmp_jokers()
        cmp_items("pack cards", "pack", PACK_FIELDS)
        if e["hand"] or g["hand"]:
            cmp_hand()
    if st == "BLIND_SELECT":
        for slot in ("small", "big", "boss"):
            eb, gb = e["blinds"].get(slot), g["blinds"].get(slot)
            if gb is None:
                continue
            cmp(f"{slot} blind", eb["name"], gb["name"])
            cmp(f"{slot} blind status", eb["status"], gb["status"])
            if slot != "boss" and gb.get("tag_name"):
                cmp(f"{slot} blind tag", eb["tag_name"], gb["tag_name"])
    return out


def hand_permutation(e_hand: list[dict], g_hand: list[dict]) -> list[int] | None:
    """Indices into the game's hand that put it in the engine's order (None if the cards differ)."""
    used = [False] * len(g_hand)
    perm = []
    for c in e_hand:
        match = next((i for i, d in enumerate(g_hand) if not used[i] and d == c), None)
        if match is None:  # a face-down card the game doesn't name stands in for any card
            match = next((i for i, d in enumerate(g_hand) if not used[i] and same_card(c, d)), None)
        if match is None:
            return None
        used[match] = True
        perm.append(match)
    return perm if len(perm) == len(g_hand) else None


# ------------------------------------------------------------------ your PC
def save_dir() -> Path | None:
    """Balatro's save folder (%APPDATA%/Balatro), where the companion mod's files live."""
    if os.environ.get("BM_BALATRO_SAVE"):
        return Path(os.environ["BM_BALATRO_SAVE"])
    try:
        from rng_game import sh
        appdata = sh("cmd.exe", "/c", "echo %APPDATA%", cwd="/mnt/c").replace("\r", "").strip()
        folder = Path(sh("wslpath", "-u", appdata)) / "Balatro"
        return folder if folder.is_dir() else None
    except (RuntimeError, OSError, ImportError):
        return None


def newest_sample() -> dict | None:
    files = sorted(WATCH.glob("*/samples/*.json"), key=lambda p: p.stat().st_mtime, reverse=True)
    for p in files:
        try:
            s = json.loads(p.read_text())
            if s.get("seed") and isinstance(s.get("moves"), list):
                s["_from"] = f"{p.parent.parent.name} job"
                return s
        except (OSError, ValueError):
            continue
    return None


# ------------------------------------------------------------------ replay
def fresh_seed() -> str:
    """A new random seed (never one a bot trained on: training seeds come from the jobs' samplers)."""
    import secrets

    alphabet = "123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ"
    return "".join(secrets.choice(alphabet) for _ in range(8))


def describe(kind: int, arg: int, game) -> str:
    name = MOVE_NAMES[kind] if 0 <= kind < len(MOVE_NAMES) else f"move {kind}"
    if kind in (6, 7):
        return f"{name} {arg.bit_count()} card(s)"
    v = game.view()
    i = arg & 0xFF
    pick = {12: "shop", 13: "shop", 14: "shop_vouchers", 15: "shop_packs", 16: "consumables",
            17: "consumables", 18: "pack"}.get(kind)
    if pick:
        lst = v[pick]
        if i < len(lst) and lst[i]:
            return f"{name}: {card_name(lst[i])}"
    return name


SUITS = {"S": "Spades", "H": "Hearts", "D": "Diamonds", "C": "Clubs"}
RANKS = {"T": "10", "J": "Jack", "Q": "Queen", "K": "King", "A": "Ace"}


def card_name(item: dict) -> str:
    """A readable name: the engine's name, or 'Queen of Hearts' for a playing card key like H_Q."""
    key = item.get("key") or ""
    if len(key) >= 3 and key[1] == "_" and key[0] in SUITS:
        r = key[2:]
        return f"{RANKS.get(r, r)} of {SUITS[key[0]]}"
    return item.get("name") or key


def settle(api: Api, gs: dict) -> dict:
    """Wait until the game reaches a state where a move can be made."""
    for _ in range(600):
        if gs and gs.get("state") in STABLE:
            return gs
        time.sleep(0.1)
        gs = api.call("gamestate")
    return gs


def apply_in_game(api: Api, kind: int, arg: int, game_before: dict, game) -> dict:
    positions = [i for i in range(64) if arg >> i & 1]
    idx, targets = arg & 0xFF, [i for i in range(56) if arg >> (8 + i) & 1]
    if kind == 11:
        return api.call("reroll")
    if kind == 12:
        return api.call("buy", {"card": idx})
    if kind == 14:
        return api.call("buy", {"voucher": idx})
    if kind == 15:
        # The game's booster area loses opened packs; the engine keeps their slots.
        slots = game_before["pack_slots"]
        return api.call("buy", {"pack": sum(1 for p in slots[:idx] if p is not None)})
    if kind == 16:
        return api.call("sell", {"consumable": idx})
    if kind == 17:
        return api.call("use", {"consumable": idx, "cards": targets} if targets else {"consumable": idx})
    if kind == 18:
        return api.call("pack", {"card": idx, "targets": targets} if targets else {"card": idx})
    if kind == 19:
        return api.call("pack", {"skip": True})
    if kind == 0:
        return api.call("select")
    if kind == 1:
        return api.call("skip")
    if kind == 2:
        return api.call("cash_out")
    if kind == 3:
        return api.call("next_round")
    if kind == 6:
        return api.call("play", {"cards": positions})
    if kind == 7:
        return api.call("discard", {"cards": positions})
    if kind == 10:
        return api.call("sell", {"joker": arg})
    if kind == 9:
        n = len(game_before["jokers"])
        order = list(range(n))
        j = order.pop(arg & 0xFF)
        order.insert(arg >> 8 & 0xFF, j)
        return api.call("rearrange", {"jokers": order})
    # sorting and moving hand cards: the hand is re-aligned after the step
    return api.call("gamestate")


def compare(api: Api, game, gs: dict, wait: float = 2.0) -> tuple[dict, list[str]]:
    """Line the game's hand up with the engine's, then compare. Some effects land in the game a moment
    after its state settles (a Tarot from Hallucination, a Perkeo copy), so a difference is only kept if
    it is still there after `wait` seconds of re-reading the game."""
    deadline = time.time() + wait
    while True:
        e = engine_view(game)
        if e["state"] in HAND_STATES and gs.get("state") == e["state"] and e["hand"]:
            perm = hand_permutation(e["hand"], game_view(gs)["hand"])
            if perm is not None and perm != list(range(len(perm))):
                gs = settle(api, api.call("rearrange", {"hand": perm}))
        diff = differences(e, game_view(gs))
        if not diff or time.time() >= deadline:
            return gs, diff
        time.sleep(0.25)
        gs = settle(api, api.call("gamestate"))


def install_mod(folder: Path | None = None) -> str | None:
    """Copy the companion mod into Balatro's Mods folder if it is missing or out of date. Returns what it
    did, or None if nothing was needed."""
    folder = folder or save_dir()
    if folder is None:
        raise SystemExit("Balatro's save folder wasn't found (%APPDATA%\\Balatro). Run the game once first.")
    src = ROOT / "mods" / "balatro_meta_watch"
    dest = folder / "Mods" / "balatro_meta_watch"
    if dest.is_dir() and all((dest / f.name).exists() and (dest / f.name).read_bytes() == f.read_bytes()
                             for f in src.iterdir()):
        return None
    dest.mkdir(parents=True, exist_ok=True)
    for f in src.iterdir():
        (dest / f.name).write_bytes(f.read_bytes())
    return (f"Installed the viewer's companion mod in {dest}. If Balatro is open, restart it once so it "
            "loads (the viewer has a button for that).")


def main() -> None:
    import argparse

    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--dry", action="store_true", help="engine only, printed here: no game, no browser")
    ap.add_argument("--engine-only", action="store_true", help="the viewer page without a game")
    ap.add_argument("--runs", type=int, default=0, help="--dry: stop after this many runs (0 = until Ctrl+C)")
    ap.add_argument("--seed", default="", help="replay this seed with the greedy bot")
    ap.add_argument("--speed", default="1", help="the visible game's speed: 1 (normal), 2, 4 or max")
    ap.add_argument("--port", type=int, default=8790)
    ap.add_argument("--host", default="127.0.0.1")
    ap.add_argument("--no-browser", action="store_true")
    ap.add_argument("--no-launch", action="store_true", help="don't open the game if BalatroBot isn't running")
    ap.add_argument("--install-mod", action="store_true")
    a = ap.parse_args()
    import viewer

    if a.install_mod:
        print(install_mod() or "The companion mod is already up to date.")
        return
    if a.dry:
        v = viewer.Viewer("max")
        drv = viewer.Driver(v, viewer.GameFiles(None), None, engine_only=True, seed=a.seed, runs=a.runs or 0)
        try:
            drv.loop()
        except KeyboardInterrupt:
            print("\nStopped.")
        return
    speed = a.speed if a.speed in viewer.SPEEDS else viewer.DEFAULT_SPEED
    viewer.serve(port=a.port, host=a.host, speed=speed, engine_only=a.engine_only, launch=not a.no_launch,
                 seed=a.seed, browser=not a.no_browser)


if __name__ == "__main__":
    main()
