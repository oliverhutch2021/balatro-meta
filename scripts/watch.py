"""Watch runs in your own Balatro: the game replays them move by move.

    make watch                 replay runs in the game (Ctrl+C stops; the game stays open)
    make watch PAUSE=1.5       seconds between moves (default 0.8)
    make watch-dry             the same runs in the engine only, printed here (no game needed)
    make watch-mod             install the in-game panel mod into your Balatro Mods folder

Your copy of Balatro does the drawing: this script drives it through BalatroBot's API, which plays
exactly the moves the engine made. Each replay is also a check of the engine. After every move the
game's state (money, hands, discards, the cards in hand, the round's chips, the blinds and tags,
jokers, consumables, the shop's cards, vouchers, packs and prices, and an open pack's cards) is
compared with the engine's, and the first difference stops the replay and is saved to
.sync/watch/mismatches/ with both states. BalatroBot has no call for "buy and use" or for a boss
reroll; a replay stops (without a mismatch) before either.

What it replays: the newest run a job saved for watching (.sync/watch/<job>/samples/*.json, with
the job's progress shown in the panel), or else demo runs played by the greedy bot.

Before the first time (see RUNNING.md, "Watching runs in your game"): Lovely, Steamodded
1.0.0-beta-1221a or newer, BalatroBot and this project's panel mod in your Balatro Mods folder,
other gameplay mods switched off. If BalatroBot isn't running, this opens the game through it (uv on
Windows); `--no-launch` turns that off.
"""
from __future__ import annotations

import argparse
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
STABLE = {"MENU", "BLIND_SELECT", "SELECTING_HAND", "ROUND_EVAL", "SHOP", "GAME_OVER", "SMODS_BOOSTER_OPENED"}
# States where the hand can be re-ordered to match the engine's (BalatroBot's `rearrange`).
HAND_STATES = {"SELECTING_HAND", "SMODS_BOOSTER_OPENED"}


# ------------------------------------------------------------------ BalatroBot
class Api:
    def __init__(self, url: str) -> None:
        self.url = url
        self.n = 0

    def call(self, method: str, params: dict | None = None, timeout: float = 120.0):
        self.n += 1
        body = {"jsonrpc": "2.0", "method": method, "id": self.n}
        if params:
            body["params"] = params
        req = urllib.request.Request(self.url, data=json.dumps(body).encode(),
                                     headers={"Content-Type": "application/json"})
        with urllib.request.urlopen(req, timeout=timeout) as r:
            out = json.loads(r.read().decode())
        if "error" in out:
            e = out["error"]
            raise RuntimeError(f"BalatroBot refused {method}: {e.get('message')} ({e.get('code')})")
        return out.get("result")


def wsl_gateway() -> str | None:
    try:
        out = subprocess.run(["ip", "route", "show", "default"], text=True, capture_output=True).stdout
        parts = out.split()
        return parts[parts.index("via") + 1] if "via" in parts else None
    except (OSError, ValueError, IndexError):
        return None


def try_urls() -> Api | None:
    urls = [os.environ["BM_BALATROBOT_URL"]] if os.environ.get("BM_BALATROBOT_URL") else []
    urls.append("http://127.0.0.1:12346")
    gw = wsl_gateway()
    if gw:
        urls.append(f"http://{gw}:12346")
    for u in urls:
        try:
            if Api(u).call("health", timeout=3) is not None:
                return Api(u)
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
    cmd = [str(uvx), "balatrobot", "serve", "--gamespeed", f"{speed:g}",
           "--balatro-path", sh("wslpath", "-w", str(game)),
           "--love-path", sh("wslpath", "-w", str(game / "Balatro.exe")),
           "--lovely-path", sh("wslpath", "-w", str(lovely))]
    print("Opening Balatro through BalatroBot:", " ".join(cmd[1:]))
    subprocess.Popen(cmd, cwd="/mnt/c", stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, start_new_session=True)
    return True


def find_api(speed: float = 1.0, launch: bool = True) -> Api:
    api = try_urls()
    if api:
        return api
    if launch and launch_game(speed):
        for _ in range(90):
            time.sleep(1)
            api = try_urls()
            if api:
                return api
    raise SystemExit(
        "Can't reach BalatroBot. Open the game with it first, in PowerShell:\n"
        '    uvx balatrobot serve --gamespeed 1 --balatro-path "H:\\Steam\\steamapps\\common\\Balatro"\n'
        "If the game is open but Ubuntu can't reach it, turn on WSL's mirrored networking: add the two lines\n"
        "[wsl2] and networkingMode=mirrored to %UserProfile%\\.wslconfig, run `wsl --shutdown`, reopen Ubuntu.\n"
        "BM_BALATROBOT_URL=http://host:port points somewhere else. Setup: RUNNING.md, step 5.")


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
    """A shop, pack or consumable card: key, edition and (if shown) price."""
    mod = c.get("modifier") or {}
    cost = c.get("cost")
    out = {"key": c.get("key"), "edition": norm_mod(mod.get("edition"))}
    if mod.get("enhancement") or mod.get("seal"):
        out["enhancement"] = norm_mod(mod.get("enhancement"))
        out["seal"] = norm_mod(mod.get("seal"))
    buy = cost.get("buy") if isinstance(cost, dict) else cost
    if buy is not None:
        out["cost"] = buy
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
        "jokers": [{"key": j.get("key"), "edition": norm_mod((j.get("modifier") or {}).get("edition"))}
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
    v["jokers"] = [{"key": j["key"], "edition": j["edition"]} for j in v["jokers"]]
    v["blinds"] = {k: {"name": b["name"], "status": b["status"], "tag_name": b.get("tag_name")}
                   for k, b in v["blinds"].items()}
    # Opened booster slots leave the game's area; the engine keeps them as None.
    v["pack_slots"] = list(v["shop_packs"])
    v["shop_packs"] = [p for p in v["shop_packs"] if p is not None]
    return v


def pack_key(k):
    """The run's first Buffoon Pack picks its art variant unseeded: compare it without the number."""
    return "p_buffoon_normal" if isinstance(k, str) and k.startswith("p_buffoon_normal") else k


def same_items(e_items: list, g_items: list) -> bool:
    """Shop or pack cards: keys and editions; prices and modifiers when the game shows them."""
    if len(e_items) != len(g_items):
        return False
    for a, b in zip(e_items, g_items):
        if pack_key(a.get("key")) != pack_key(b.get("key")) or a.get("edition") != b.get("edition"):
            return False
        for k in ("cost", "enhancement", "seal"):
            if k in b and a.get(k) != b[k]:
                return False
    return True


def differences(e: dict, g: dict) -> list[str]:
    """What the game shows differently from the engine, for the fields that state shows."""
    out = []

    def cmp(name, a, b):
        if a != b:
            out.append(f"{name}: engine {a!r}, game {b!r}")

    def cmp_items(name, key):
        if g.get(key) is not None and not same_items(e[key], g[key]):
            out.append(f"{name}: engine {e[key]!r}, game {g[key]!r}")

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
    cmp_items("consumables", "consumables")
    st = e["state"]
    if st == "SELECTING_HAND":
        for k in ("round", "hands_left", "discards_left"):
            cmp(k, e[k], g[k])
        if g["chips"] is not None:
            cmp("round chips", float(e["chips"]), float(g["chips"]))
        cmp_hand()
        cmp("jokers", e["jokers"], g["jokers"])
    if st == "SHOP":
        cmp("jokers", e["jokers"], g["jokers"])
        cmp_items("shop cards", "shop")
        cmp_items("shop vouchers", "shop_vouchers")
        cmp_items("shop packs", "shop_packs")
        if g.get("reroll_cost") is not None:
            cmp("reroll cost", e["reroll_cost"], g["reroll_cost"])
    if st == "SMODS_BOOSTER_OPENED":
        cmp("jokers", e["jokers"], g["jokers"])
        cmp_items("pack cards", "pack")
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


# ------------------------------------------------------------------ panel
def panel_file() -> Path | None:
    """%APPDATA%/Balatro/balatro_meta_watch.txt, which the panel mod reads."""
    if os.environ.get("BM_WATCH_PANEL"):
        return Path(os.environ["BM_WATCH_PANEL"])
    try:
        from rng_game import sh
        appdata = sh("cmd.exe", "/c", "echo %APPDATA%", cwd="/mnt/c").replace("\r", "").strip()
        folder = Path(sh("wslpath", "-u", appdata)) / "Balatro"
        return folder / "balatro_meta_watch.txt" if folder.is_dir() else None
    except (RuntimeError, OSError, ImportError):
        return None


class Panel:
    def __init__(self, path: Path | None) -> None:
        self.path = path

    def show(self, lines: list[str]) -> None:
        text = "\n".join(lines[:14]) + "\n"
        print("  | " + "\n  | ".join(lines))
        if self.path:
            try:
                tmp = self.path.with_suffix(".tmp")
                tmp.write_text(text, encoding="utf-8")
                os.replace(tmp, self.path)
            except OSError:
                pass

    def clear(self) -> None:
        if self.path:
            try:
                self.path.unlink(missing_ok=True)
            except OSError:
                pass


def job_line() -> str | None:
    """The running job and its step, from the job runner's note."""
    notes = sorted(WATCH.glob("*/current.json"), key=lambda p: p.stat().st_mtime, reverse=True)
    for p in notes:
        try:
            n = json.loads(p.read_text())
        except (OSError, ValueError):
            continue
        if n.get("state") == "running" and time.time() - n.get("time", 0) < 6 * 3600:
            done = sum(1 for s in n.get("steps", []) if s.get("done"))
            return f"Job {n['job']}: step {n.get('step')} ({done}/{len(n.get('steps', []))} done)"
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
        return f"{name} {bin(arg).count('1')} card(s)"
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


def replay(api: Api | None, panel: Panel, seed: str, moves, label: str, pause: float) -> tuple[bool, str]:
    """Play one run in the engine and the game together. `moves` is a list, or None for the greedy bot."""
    from balatro_meta._engine import Game

    game = Game(seed)
    title = f"Balatro Meta - {label} - seed {game.seed}"
    if api:
        try:
            api.call("menu")
        except RuntimeError:
            pass
        gs = settle(api, api.call("start", {"deck": "RED", "stake": "WHITE", "seed": game.seed}))
        gs, diff = compare(api, game, gs)
        if diff:
            return False, report_mismatch(panel, title, game, 0, "start", diff, gs)
    step = 0
    while not game.over and step < 5000:
        if moves is None:
            kind, arg = game.greedy_move()
        elif step < len(moves):
            kind, arg = moves[step]
        else:
            break
        what = describe(kind, arg, game)
        if api and kind in UNSUPPORTED:
            end = f"stopped before move {step + 1} ({what}): {UNSUPPORTED[kind]}"
            panel.show([title, end, f"Engine and game agreed on the first {step} moves"])
            return True, end
        lines = [title]
        jl = job_line()
        if jl:
            lines.append(jl)
        v = game.view()
        lines.append(f"Ante {v['ante']}, round {v['round']}, ${v['money']}, chips {v['chips']:.0f}")
        lines.append(f"Move {step + 1}: {what}")
        if api:
            lines.append(f"Engine and game agree after {step} moves" if step else "Engine and game agree at the start")
        panel.show(lines)
        if api:
            gs = api.call("gamestate")
            e_before = engine_view(game)
            if gs.get("state") in HAND_STATES and e_before["hand"]:
                perm = hand_permutation(e_before["hand"], game_view(gs)["hand"])
                if perm is not None and perm != list(range(len(perm))):
                    gs = settle(api, api.call("rearrange", {"hand": perm}))
        if not game.step(kind, arg):
            return False, f"the engine refused move {step + 1} ({what}); the sample doesn't fit this engine"
        if api:
            gs = settle(api, apply_in_game(api, kind, arg, e_before, game))
            gs, diff = compare(api, game, gs)
            if diff:
                return False, report_mismatch(panel, title, game, step + 1, what, diff, gs)
        step += 1
        time.sleep(pause)
    v = game.view()
    end = "won the run" if v["won"] else f"run over at ante {v['ante']}"
    panel.show([title, f"{end} after {step} moves", "Engine and game agreed on every move" if api else ""])
    return True, end


def report_mismatch(panel: Panel, title: str, game, step: int, what: str, diff: list[str], gs: dict) -> str:
    out = WATCH / "mismatches"
    out.mkdir(parents=True, exist_ok=True)
    f = out / f"{game.seed}-move{step}.json"
    f.write_text(json.dumps({"seed": game.seed, "move": step, "what": what, "differences": diff,
                             "moves": game.moves, "engine": engine_view(game), "game": gs}, indent=1, default=str))
    shown = f.relative_to(ROOT) if f.is_relative_to(ROOT) else f
    panel.show([title, f"MISMATCH after move {step} ({what}):", *diff[:8], f"Saved: {shown}"])
    return f"mismatch after move {step} ({what}): " + "; ".join(diff[:3]) + f". Details in {shown}"


def install_mod() -> None:
    from rng_game import sh
    appdata = sh("cmd.exe", "/c", "echo %APPDATA%", cwd="/mnt/c").replace("\r", "").strip()
    mods = Path(sh("wslpath", "-u", appdata)) / "Balatro" / "Mods"
    if not mods.parent.is_dir():
        raise SystemExit(f"Balatro's save folder isn't at {mods.parent}. Run the game once first.")
    dest = mods / "balatro_meta_watch"
    dest.mkdir(parents=True, exist_ok=True)
    for f in (ROOT / "mods" / "balatro_meta_watch").iterdir():
        (dest / f.name).write_bytes(f.read_bytes())
    print(f"Installed the panel mod to {sh('wslpath', '-w', str(dest))}")


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--dry", action="store_true", help="engine only, no game")
    ap.add_argument("--runs", type=int, default=0, help="stop after this many runs (0 = until Ctrl+C)")
    ap.add_argument("--pause", type=float, default=0.8, help="seconds between moves")
    ap.add_argument("--seed", default="", help="replay this seed with the greedy bot")
    ap.add_argument("--speed", type=float, default=1.0, help="game speed when this script opens the game")
    ap.add_argument("--no-launch", action="store_true", help="don't open the game if BalatroBot isn't running")
    ap.add_argument("--install-mod", action="store_true")
    a = ap.parse_args()
    if a.install_mod:
        install_mod()
        return
    api = None if a.dry else find_api(a.speed, not a.no_launch)
    panel = Panel(None if a.dry else panel_file())
    if api and not panel.path:
        print("Note: Balatro's save folder wasn't found, so the in-game panel won't update.")
    n = 0
    try:
        while not a.runs or n < a.runs:
            sample = None if a.seed else newest_sample()
            if sample:
                seed, moves, label = sample["seed"], [tuple(m) for m in sample["moves"]], sample["_from"]
            else:
                seed, moves, label = a.seed or fresh_seed(), None, f"demo run {n + 1} (greedy bot)"
            ok, msg = replay(api, panel, seed, moves, label, 0.0 if a.dry else a.pause)
            print(f"[{label}] seed {seed}: {msg}")
            n += 1
            if not ok:
                raise SystemExit(1)
            if a.seed:
                break
            time.sleep(0 if a.dry else 3)
    except KeyboardInterrupt:
        print("\nStopped. The game stays open.")
    finally:
        if not a.dry:
            panel.clear()


if __name__ == "__main__":
    main()
