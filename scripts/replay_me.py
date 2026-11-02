"""Your runs, move by move: the engine's moves between the states the real game showed.

`make record-me` (scripts/record_me.py) keeps the real game's state each time it settles after a move of yours. This
file lines the engine up with those states: the run starts from its seed, and between one state and the next the
replay finds the move that takes the engine there (or two moves, when the game settled only once for both). Before
each move the engine's hand and jokers are put in the game's order, so a hand plays in the order you played it, and
cards are followed by the game's ids from one state to the next.

The comparison is the real-game checks' (scripts/watch.py) with three allowances: the hand, jokers and consumables
are compared in any order (and then lined up), a card's debuff mark is left out, and the round's chips may differ by
a point or so (the engine's rounding of some scores is still being looked into). Anything else that differs stops
the replay of that run there and says why; the recording itself always goes on.

    python scripts/replay_me.py .sync/me/runs/<run>.jsonl.gz     replay one recorded run and say how far it got
"""
from __future__ import annotations

import gzip
import itertools
import json
import re
import sys
from pathlib import Path

from watch import ROOT, differences, engine_view, game_view, hand_permutation

# Text the game writes about cards and blinds is never kept (nothing from the game's text goes to disk): only ids,
# keys, names, numbers and flags. A card's effect line keeps just its numbers (a joker's current +Mult, say).
DROP_KEYS = frozenset({"label", "tag_effect", "example", "description"})
NUMBER = re.compile(r"-?\d+(?:\.\d+)?")

HAND_STATES = ("SELECTING_HAND", "SMODS_BOOSTER_OPENED")
MOVE_CARD, MOVE_JOKER = 8, 9
REORDERS = (4, 5, MOVE_CARD, MOVE_JOKER)
CHIPS_ALLOWANCE = 1.0  # points, plus a small share of the score (see the module's notes)
MAX_TARGETS = 3  # cards a consumable can be used on (Tarots that change cards take up to 3)


def scrub(x, in_blinds: bool = False):
    """The game's state as the recorder keeps it: without the game's text (see DROP_KEYS)."""
    if isinstance(x, dict):
        out = {}
        for k, v in x.items():
            if k in DROP_KEYS:
                continue
            if k == "effect":
                if not in_blinds and isinstance(v, str):
                    out["effect_numbers"] = [float(n) for n in NUMBER.findall(v)]
                continue
            out[k] = scrub(v, in_blinds or k == "blinds")
        return out
    if isinstance(x, list):
        return [scrub(v, in_blinds) for v in x]
    return x


def area(gs: dict, name: str) -> list[dict]:
    a = gs.get(name)
    if isinstance(a, dict):
        return a.get("cards") or []
    return a if isinstance(a, list) else []


def ids(gs: dict, name: str) -> list:
    return [c.get("id") for c in area(gs, name)]


def highlighted(gs: dict) -> list:
    """Ids of the hand cards the game shows as selected."""
    return [c.get("id") for c in area(gs, "hand") if isinstance(c.get("state"), dict) and c["state"].get("highlight")]


def lenient(card: dict) -> dict:
    return {k: card.get(k) for k in ("key", "enhancement", "edition", "seal")}


def joker_match_key(j: dict) -> tuple:
    """A joker as matched between the engine and the game, in any order (sortable: a face-down joker has no key,
    a plain one no edition)."""
    return (j.get("key") or "", j.get("edition") or "", bool(j.get("face_down")))


def held_key(c: dict) -> tuple:
    return (c.get("key"), c.get("edition"), c.get("sell"))


def compare(game, gs: dict) -> list[str]:
    """What the game shows differently from the engine, with the module's allowances."""
    e, g = engine_view(game), game_view(gs)
    eh, gh = [lenient(c) for c in e["hand"]], [lenient(c) for c in g["hand"]]
    if len(eh) == len(gh) and hand_permutation(eh, gh) is not None:
        e["hand"] = g["hand"]  # the same cards (in some order): lined up separately
    else:
        e["hand"], g["hand"] = eh, gh
    if sorted(map(joker_match_key, e["jokers"])) == sorted(map(joker_match_key, g["jokers"])):
        e["jokers"] = sorted(e["jokers"], key=lambda j: (*joker_match_key(j), j.get("sell") or 0))
        g["jokers"] = sorted(g["jokers"], key=lambda j: (*joker_match_key(j), j.get("sell") or 0))
    if g.get("consumables") is not None:
        e["consumables"] = sorted(e["consumables"], key=lambda c: repr(held_key(c)))
        g["consumables"] = sorted(g["consumables"], key=lambda c: repr(held_key(c)))
    ec, gc = e.get("chips"), g.get("chips")
    if ec is not None and gc is not None and abs(float(ec) - float(gc)) <= CHIPS_ALLOWANCE + 1e-4 * abs(float(gc)):
        e["chips"] = gc
    out = differences(e, g)
    # The jokers in every state (the checks compare them only where a move of theirs can change them).
    # (Without face-down marks: the game turns jokers face up again a moment after the cash-out screen opens.)
    held = (sorted(repr((j.get("key"), j.get("edition"))) for j in e["jokers"]),
            sorted(repr((j.get("key"), j.get("edition"))) for j in g["jokers"]))
    if not out and e["state"] not in ("SELECTING_HAND", "SHOP", "SMODS_BOOSTER_OPENED") and held[0] != held[1]:
        out.append(f"jokers: engine {[j['key'] for j in e['jokers']]}, game {[j['key'] for j in g['jokers']]}")
    return out


def reorder(game, kind: int, order: list[int]) -> list[tuple[int, int]]:
    """Put the engine's hand (kind 8) or jokers (kind 9) in `order` (order[t]: today's index of what goes to t)."""
    cur = list(range(len(order)))
    moves = []
    for t, want in enumerate(order):
        p = cur.index(want)
        if p != t:
            arg = p | t << 8
            if not game.step(kind, arg):
                raise RuntimeError(f"the engine refused to move a card from {p} to {t}")
            moves.append((kind, arg))
            cur.insert(t, cur.pop(p))
    return moves


def line_up(game, gs: dict, follow: dict) -> list[tuple[int, int]]:
    """Moves that put the engine's hand and jokers in the game's order, applied to `game`. `follow` maps the game's
    ids to the engine's (cards and jokers), and is brought up to date."""
    moves = []
    v = game.view()
    # The hand: each game card to an engine card, the one it was before if it is still there.
    g_cards, e_cards = area(gs, "hand"), v["hand"]
    if v["state"] in HAND_STATES and len(e_cards) == len(g_cards) and e_cards:
        gl = [lenient(c) for c in game_view(gs)["hand"]]
        el = [lenient(c) for c in e_cards]
        taken = [False] * len(el)
        order = []
        for gi, gc in enumerate(g_cards):
            want = follow.get(("card", gc.get("id")))
            pick = next((i for i, c in enumerate(e_cards) if not taken[i] and c["id"] == want
                         and (gl[gi]["key"] is None or el[i] == gl[gi])), None)
            if pick is None:
                pick = next((i for i in range(len(el)) if not taken[i] and el[i] == gl[gi]), None)
            if pick is None:  # a face-down card the game doesn't name
                pick = next((i for i in range(len(el)) if not taken[i] and gl[gi]["key"] is None), None)
            if pick is None:
                order = None
                break
            taken[pick] = True
            order.append(pick)
        if order is not None:
            for gc, i in zip(g_cards, order):
                follow[("card", gc.get("id"))] = e_cards[i]["id"]
            moves += reorder(game, MOVE_CARD, order)
    # The jokers.
    g_jokers, e_jokers = area(gs, "jokers"), v["jokers"]
    gj = game_view(gs)["jokers"]
    if len(e_jokers) == len(gj) and e_jokers:
        taken = [False] * len(e_jokers)
        order = []
        for gi, j in enumerate(gj):
            want = follow.get(("joker", g_jokers[gi].get("id")))
            pick = next((i for i, x in enumerate(e_jokers) if not taken[i] and x["id"] == want
                         and joker_match_key(x) == joker_match_key(j)), None)
            if pick is None:
                pick = next((i for i, x in enumerate(e_jokers) if not taken[i]
                             and joker_match_key(x) == joker_match_key(j) and x.get("sell") == j.get("sell")), None)
            if pick is None:
                pick = next((i for i, x in enumerate(e_jokers) if not taken[i]
                             and joker_match_key(x) == joker_match_key(j)), None)
            if pick is None:
                order = None
                break
            taken[pick] = True
            order.append(pick)
        if order is not None:
            for gi, i in enumerate(order):
                follow[("joker", g_jokers[gi].get("id"))] = e_jokers[i]["id"]
            moves += reorder(game, MOVE_JOKER, order)
    return moves


def masks_up_to(n: int, k: int):
    for size in range(1, min(k, n) + 1):
        for combo in itertools.combinations(range(n), size):
            yield sum(1 << i for i in combo)


def submasks(mask: int, k: int):
    """Masks of 1 to k of the positions in `mask`."""
    pos = [i for i in range(64) if mask >> i & 1]
    for size in range(1, min(k, len(pos)) + 1):
        for combo in itertools.combinations(pos, size):
            yield sum(1 << i for i in combo)


def card_moves(v: dict, before: dict, after: dict, picked: int, gone: int, wide: bool = True):
    """Plays and discards to try. The cards that left the hand are the ones played or discarded (with The Hook, two
    more); the cards shown as selected just before are the likeliest. When the round ended with the hand put away,
    any 1 to 5 cards may have been played: the round's score and the poker hands played tell them apart."""
    n = len(v["hand"])
    shown_after = after.get("state") in HAND_STATES or bool(area(after, "hand"))
    masks = []
    if picked:
        masks.append(picked)
    if gone and shown_after:
        if gone.bit_count() <= 5:
            masks.append(gone)
        if wide:
            masks += list(submasks(gone, 5))
    plays = [(6, m) for m in masks] + [(7, m) for m in masks]
    if not shown_after and wide:
        plays += [(6, m) for m in masks_up_to(n, 5)]
    return plays


def candidates(game, before: dict, after: dict, highlight: list | None, follow: dict, wide: bool = True):
    """Moves to try from the engine's state (lined up with `before`) towards `after`, likeliest first."""
    v = game.view()
    st = v["state"]
    legal = game.legal_kinds()
    n_hand = len(v["hand"])
    engine_ids = [c["id"] for c in v["hand"]]

    def pos(game_id):
        e = follow.get(("card", game_id))
        return engine_ids.index(e) if e in engine_ids else None

    after_ids = set(ids(after, "hand"))
    gone = sum(1 << p for x in ids(before, "hand") if x not in after_ids and (p := pos(x)) is not None)
    picked = sum(1 << p for x in (highlight or []) if (p := pos(x)) is not None)
    out = []
    if st == "BLIND_SELECT":
        out += [(0, 0), (1, 0), (20, 0)]
    elif st == "ROUND_EVAL":
        out.append((2, 0))
    elif st == "SELECTING_HAND":
        out += card_moves(v, before, after, picked, gone, wide)
    elif st == "SHOP":
        out += [(3, 0), (11, 0)]
        out += [(12, i) for i in range(len(v["shop"]))]
        out += [(14, i) for i in range(len(v["shop_vouchers"]))]
        out += [(15, i) for i, p in enumerate(v["shop_packs"]) if p is not None]
        out += [(13, i) for i, s in enumerate(v["shop"]) if s.get("set") in ("Tarot", "Planet", "Spectral")]
    elif st == "SMODS_BOOSTER_OPENED":
        # Skipping first: a card taken to no visible effect (The World on Spades) looks the same, and is rarer.
        out.append((19, 0))
        for i in range(len(v["pack"])):
            out.append((18, i))
            if n_hand:
                if picked:
                    out.append((18, i | picked << 8))
                if wide:
                    out += [(18, i | m << 8) for m in masks_up_to(n_hand, MAX_TARGETS) if m != picked]
    if st != "GAME_OVER":
        out += [(10, i) for i in range(len(v["jokers"]))]
        out += [(16, i) for i in range(len(v["consumables"]))]
        for i in range(len(v["consumables"])):
            out.append((17, i))
            if n_hand and st in HAND_STATES:
                if picked:
                    out.append((17, i | picked << 8))
                if wide:
                    out += [(17, i | m << 8) for m in masks_up_to(n_hand, MAX_TARGETS) if m != picked]
    seen = set()
    for kind, arg in out:
        if (kind, arg) not in seen and 0 <= kind < len(legal) and legal[kind]:
            seen.add((kind, arg))
            yield kind, arg


def picked_mask(game, highlight: list | None, follow: dict) -> int:
    """The engine's hand positions of the cards the player had selected (by the game's ids)."""
    engine_ids = [c["id"] for c in game.view()["hand"]]
    out = 0
    for x in highlight or []:
        e = follow.get(("card", x))
        if e in engine_ids:
            out |= 1 << engine_ids.index(e)
    return out


def try_move(game, kind: int, arg: int, after: dict):
    """(the engine after the move, its differences from `after`), or None if the engine refuses the move."""
    c = game.copy()
    if not c.step(kind, arg):
        return None
    return c, compare(c, after)


def weight(diff: list[str]) -> int:
    """How far off a move's result is, for saying which move came closest: a wrong state counts most."""
    return sum(10 if d.startswith("state") else 1 for d in diff)


def soft_differences(game, gs: dict) -> list[str]:
    """Checks that choose between moves the comparison can't tell apart (which cards a round's last hand was): the
    round's score, and each poker hand's level and how often it has been played."""
    out = []
    v = game.view()
    gc = (gs.get("round") or {}).get("chips")
    ec = v.get("chips")
    if gc is not None and ec is not None and abs(float(ec) - float(gc)) > CHIPS_ALLOWANCE + 1e-4 * abs(float(gc)):
        out.append(f"chips: engine {ec}, game {gc}")
    levels = {h["name"]: (h["level"], h["played"]) for h in v["hand_levels"]}
    hands = gs.get("hands")
    if isinstance(hands, dict):
        for name, h in hands.items():
            shown = isinstance(h, dict) and h.get("level") is not None and h.get("played") is not None
            if shown and levels.get(name, (1, 0)) != (h["level"], h["played"]):
                out.append(f"{name}: engine (level, played) {levels.get(name, (1, 0))}, game "
                           f"{(h['level'], h['played'])}")
    # The cards still to be drawn, as a set (a card taken from a Standard pack goes there unseen).
    deck = gs.get("cards")
    if isinstance(deck, dict) and isinstance(deck.get("cards"), list) and deck["cards"]:
        g_deck = sorted(deck_entry(c) for c in deck["cards"])
        e_deck = sorted(deck_entry_engine(c) for c in v["deck_cards"] if c["left"])
        if g_deck != e_deck:
            out.append(f"deck: {len(e_deck)} cards in the engine's, {len(g_deck)} in the game's, not the same")
    return out


def deck_entry(c: dict) -> tuple:
    m = c.get("modifier") if isinstance(c.get("modifier"), dict) else {}
    return (str(c.get("key")), *(str(watch_norm(m.get(k))) for k in ("enhancement", "edition", "seal")))


def deck_entry_engine(c: dict) -> tuple:
    return (str(c["key"]), *(str(c.get(k)) for k in ("enhancement", "edition", "seal")))


def watch_norm(x):
    from watch import norm_mod

    return norm_mod(x)


def played_orders(game, arg: int, kind: int, after: dict):
    """A move on cards (a play, or a Tarot's targets) put in another order just before it, too quickly for the game
    to settle in between: each order of those cards, tried until the result matches."""
    mask = arg if kind in (6, 7) else arg >> 8
    pos = [i for i in range(64) if mask >> i & 1]
    if not 1 < len(pos) <= 5:
        return None
    for perm in itertools.permutations(pos):
        if list(perm) == pos:
            continue
        c = game.copy()
        order = list(range(len(c.view()["hand"])))
        for slot, p in zip(pos, perm):
            order[slot] = p
        pre = reorder(c, MOVE_CARD, order)
        if c.step(kind, arg) and not compare(c, after):
            return c, [*pre, (kind, arg)]
    return None


def align_hand_by_ids(game, gs: dict, follow: dict) -> list[tuple[int, int]] | None:
    """Put the engine's hand in the order of the game's state `gs` by the ids alone (cards a Tarot is about to
    change still look like their old selves in the engine). None if some card isn't known."""
    engine_ids = [c["id"] for c in game.view()["hand"]]
    want = [follow.get(("card", x)) for x in ids(gs, "hand")]
    if len(want) != len(engine_ids) or None in want or sorted(want) != sorted(engine_ids):
        return None
    return reorder(game, MOVE_CARD, [engine_ids.index(e) for e in want])


class Line:
    """One way the run may have gone: the engine after it, the moves, and the game's ids it follows."""

    def __init__(self, game, follow: dict, before: dict, moves: list, steps: list) -> None:
        self.game, self.follow, self.before, self.moves, self.steps = game, follow, before, moves, steps

    def after(self, game, moves: list, gs: dict, index: int) -> Line:
        follow = dict(self.follow)
        moves = list(moves) + line_up(game, gs, follow)
        steps = self.steps + ([{"state": index, "moves": moves}] if moves else [])
        return Line(game, follow, gs, self.moves + moves, steps)


def bases(line: Line, gs: dict) -> list[tuple]:
    """The engine as it stands, and lined up with the game's state after the move: jokers moved just before the
    move (too quickly for the game to settle in between) score in their new order, and cards moved just before a
    Tarot was used on them show in that order afterwards. Moved after the move instead (jokers rearranged on the
    cash-out screen), the engine as it stands is the right one: both are tried, as it stands first."""
    game, before = line.game, line.before
    out = [(game, [])]
    lined, pre = game, []
    gj = game_view(gs)["jokers"]
    ej = game.view()["jokers"]
    if sorted(map(repr, map(joker_match_key, gj))) == sorted(map(repr, map(joker_match_key, ej))):
        trial = game.copy()
        moved = line_up(trial, {"state": None, "jokers": gs.get("jokers"), "hand": {"cards": []}}, dict(line.follow))
        if moved:
            lined, pre = trial, moved
    if set(ids(before, "hand")) == set(ids(gs, "hand")) and ids(before, "hand") != ids(gs, "hand"):
        trial = lined.copy()
        moved = align_hand_by_ids(trial, gs, line.follow)
        if moved:
            lined, pre = trial, [*pre, *moved]
    if pre:
        out.append((lined, pre))
    return out


def pack_evidence(line: Line, moves: list, passed: list[dict]) -> int:
    """How far a move list goes against what states passed over since the last one showed of an open pack: a
    card already gone from it there was taken (a Wheel of Fortune that missed leaves no other trace), so a skip,
    or taking another card, fits less."""
    shown = [p for p in passed if p.get("state") == "SMODS_BOOSTER_OPENED" and isinstance(p.get("pack"), dict)]
    if not shown or line.game.view()["state"] != "SMODS_BOOSTER_OPENED":
        return 0
    before = [c["key"] for c in line.game.view()["pack"]]
    left = [c.get("key") for c in shown[-1]["pack"].get("cards") or []]
    gone = list(before)
    for k in left:
        if k in gone:
            gone.remove(k)
    if not gone or len(left) >= len(before):
        return 0
    taken = [before[a & 0xFF] for k, a in moves if k == 18 and (a & 0xFF) < len(before)]
    if not taken:
        return 1
    return 0 if taken[0] in gone else 1


def explain(line: Line, gs: dict, highlight: list | None, passed: list[dict] | None = None) -> tuple[list, list[str]]:
    """The moves that take this line's engine to the game's state `gs`: [(checks failed, rank, engine, moves)],
    best first (empty if none), and what came closest otherwise. "No move" fits only when nothing that a move
    would change differs (a planet taken from a pack shows only in its hand's level)."""
    game = line.game
    before = line.before
    closest = compare(game, gs)  # no move at all, unless a move comes closer (a wrong state counts most)
    matches = []
    if not closest:
        soft = len(soft_differences(game, gs))
        if soft == 0:
            return [(0, 0, game.copy(), [])], []
        matches.append((soft, -1, game.copy(), []))
    tried = []
    rank = 0
    for base, pre in bases(line, gs):
        found = False
        picked = picked_mask(base, highlight, line.follow)
        for kind, arg in candidates(base, before, gs, highlight, line.follow):
            rank += 1
            r = try_move(base, kind, arg, gs)
            if r is None:
                continue
            c, diff = r
            if not diff:
                soft = len(soft_differences(c, gs))
                matches.append((soft, rank, c, [*pre, (kind, arg)]))
                # A Tarot taken from a pack to no visible effect looks like any other: look on for every take
                # that fits, so the others are kept in reserve. So with two packs of a kind in the shop: which was
                # opened shows only later, in the one left (its art differs).
                if soft == 0 and kind not in (15, 18, 19):
                    found = True
                    break
                continue
            tried.append((kind, arg, c, pre, picked))
            if weight(diff) < weight(closest):
                closest = diff
            if (kind in (6, 7) or (kind in (17, 18) and arg >> 8)) and all(
                    d.startswith(("round chips", "state", "hand")) for d in diff):
                r2 = played_orders(base, arg, kind, gs)
                if r2 is not None:
                    matches.append((len(soft_differences(r2[0], gs)), rank, r2[0], [*pre, *r2[1]]))
                    found = True
                    break
        if found or any(m[0] == 0 and m[3] for m in matches):
            break
    if not any(m[0] == 0 and m[3] for m in matches):
        # Two moves the game played out with no settled state in between (a planet taken from a pack, then the
        # rest of the pack skipped): when no single move fits every check.
        for n, (kind, arg, c, pre, picked) in enumerate(tried):
            # A card used on cards: only on the ones the player had selected (Death from a pack, say, then The
            # Fool making another Death before the game settled).
            if kind in (17, 18) and arg >> 8 and arg >> 8 != picked:
                continue
            # A hand that ends the round, then Cash Out clicked before the cash-out screen settled; or a hand or
            # discard that made a card (Vagabond, 8 Ball, a Purple Seal...), used or sold before the game settled.
            made = len(c.view()["consumables"]) > len(game.view()["consumables"])
            if kind in (6, 7) and not made and (kind == 7 or c.view()["state"] != "ROUND_EVAL"):
                continue
            for k2, a2 in candidates(c, before, gs, highlight, line.follow, wide=False):
                r = try_move(c, k2, a2, gs)
                if r is not None and not r[1]:
                    soft = len(soft_differences(r[0], gs))
                    matches.append((soft, 100000 + n, r[0], [*pre, (kind, arg), (k2, a2)]))
                    if soft == 0:
                        break
            if matches and matches[-1][0] == 0 and matches[-1][3]:
                break
    if passed:
        matches = [(m[0] + pack_evidence(line, m[3], passed), *m[1:]) for m in matches]
    matches.sort(key=lambda m: (m[0], m[1]))
    return matches, closest


class Replay:
    """One recorded run in the engine, state by state. When two moves fit a state equally well (taking a Tarot
    whose effect doesn't show, or skipping the pack), the replay goes on with the likelier one and keeps the other
    in reserve: if a later state shows the first was wrong, it goes back and takes the other."""

    # States in a row that no move explains before the replay counts as lost: a state can be caught while an
    # effect is still landing (a Tarot from Hallucination, say), and the next one then explains both.
    MAX_SKIPPED = 3
    MAX_RESERVES = 48

    def __init__(self, first: dict) -> None:
        from balatro_meta._engine import Game

        seed = first.get("seed")
        if not seed:
            raise ValueError("the game's state has no seed")
        game = Game(seed)
        self.lost: str | None = None
        self.why = ""
        self.skipped = 0
        self.notes: list[str] = []
        self.index = 0
        self.history: list[tuple[dict, list | None]] = [(first, None)]
        self.reserves: list[tuple[int, Line]] = []  # (state index, the line had it gone the other way there)
        diff = compare(game, first)
        if diff:
            self.lost = "the run's start differs: " + "; ".join(diff[:4])
        follow: dict = {}
        moves = line_up(game, first, follow) if not diff else []
        self.line = Line(game, follow, first, moves, [])

    @property
    def ok(self) -> bool:
        return self.lost is None

    @property
    def game(self):
        return self.line.game

    @property
    def moves(self) -> list:
        return self.line.moves

    @property
    def steps(self) -> list:
        return self.line.steps

    @property
    def before(self) -> dict:
        return self.line.before

    def advance(self, line: Line, gs: dict, highlight, index: int, reserves: list | None,
                passed: list[dict] | None = None) -> Line | None:
        matches, closest = explain(line, gs, highlight, passed)
        if not matches:
            where = f"{line.before.get('state')} -> {gs.get('state')}, ante {gs.get('ante_num')}"
            self.why = f"state {index} ({where}): no move explains it ({'; '.join(closest[:3])})"
            return None
        best = matches[0]
        if reserves is not None:
            # Other moves that fit as well: one of each kind first (a take for a skip), then a couple more.
            same = [m for m in matches[1:] if m[0] == best[0] and m[3] and best[3] and m[3][-1] != best[3][-1]]
            kinds = {best[3][-1][0]} if best[3] else set()
            keep = []
            for m in same:
                if m[3][-1][0] not in kinds:
                    kinds.add(m[3][-1][0])
                    keep.append(m)
            keep += [m for m in same if m not in keep][:2]
            for m in reversed(keep):  # the likeliest is tried first (taken from the end)
                reserves.append((index, line.after(m[2], m[3], gs, index)))
        return line.after(best[2], best[3], gs, index)

    def feed(self, gs: dict, highlight: list | None = None,
             ahead: list[tuple[dict, list | None]] | None = None) -> list[tuple[int, int]] | None:
        """The next settled state the game showed. Returns the engine's moves that lead there (an empty list
        when only the order of cards or jokers changed), or None if no move explains it (yet; see `lost`).
        `ahead`: the states that come next, when the whole run is known (a recorded run, not a live one)."""
        self.index += 1
        self.history.append((gs, highlight))
        if self.lost:
            return None
        n_before = len(self.line.moves)
        alternatives: list[tuple[int, Line]] = []
        passed = [self.history[j][0] for j in range(self.index - self.skipped, self.index)]
        new = self.advance(self.line, gs, highlight, self.index, alternatives, passed)
        if new is not None and len(new.moves) > n_before:
            # Kept back, tried last: this state as one caught while a move was still landing (a Tarot used,
            # its cards not yet changed), with the next state explained from the one before it.
            self.reserves.append((self.index, self.line))
            self.reserves.extend(alternatives)
        if new is None:
            # A state caught while a move was still landing (a pack's card taken while the pack still shows, a
            # pack opened before its price is paid) is passed over when this line explains one of the next states,
            # unless a kept-back line explains them too (an earlier choice was then wrong).
            soon = self.explains_soon(self.line, [*passed, gs], ahead)
            new = self.from_reserve(ahead if soon else None)
        if new is None:
            self.skipped += 1
            if self.skipped >= self.MAX_SKIPPED:
                self.lost = self.why
            return None
        if self.skipped:
            self.notes.append(f"states {self.index - self.skipped} to {self.index - 1} were passed over")
        self.skipped = 0
        self.line = new
        del self.reserves[:-self.MAX_RESERVES]
        return new.moves[n_before:] if len(new.moves) >= n_before else list(new.moves)

    def explains_soon(self, line: Line, seen: list[dict], ahead: list[tuple[dict, list | None]] | None) -> bool:
        """Whether `line` (at this state) explains one of the next states, as many as may still be passed over."""
        seen = list(seen)
        why = self.why
        try:
            for j, (g2, h2) in enumerate((ahead or [])[: self.MAX_SKIPPED - 1 - self.skipped]):
                if self.advance(line, g2, h2, self.index + 1 + j, None, seen) is not None:
                    return True
                seen.append(g2)
            return False
        finally:
            self.why = why

    def from_reserve(self, ahead: list[tuple[dict, list | None]] | None = None) -> Line | None:
        """The newest kept-back line that explains every state since it branched, up to this one (and, with
        `ahead`, one of the next states too: else it stays kept back)."""
        later = []
        found = None
        while self.reserves:
            at, start = self.reserves.pop()
            line, ok = start, True
            for j in range(at + 1, self.index + 1):
                gs, hl = self.history[j]
                nxt = self.advance(line, gs, hl, j, None)
                if nxt is None:
                    ok = False
                    break
                line = nxt
            if not ok:
                continue
            if ahead is not None and not self.explains_soon(line, [], ahead):
                later.append((at, start))
                continue
            found = (at, line)
            break
        self.reserves.extend(reversed(later))
        if found is None:
            return None
        self.notes.append(f"state {found[0]}: went back and took the other move that fitted")
        return found[1]

    def summary(self) -> dict:
        v = self.game.view()
        return {"seed": self.game.seed, "ok": self.ok, "lost": self.lost, "moves": self.moves,
                "steps": self.steps, "states": self.index + 1, "ante": v["ante"], "won": v["won"],
                "over": self.game.over, "notes": self.notes}


# ------------------------------------------------------------------ recorded runs
def read_run(path: Path) -> tuple[dict, list[dict]]:
    """A recorded run: its header and its states (each {"t", "gs", "highlighted_before"})."""
    opener = gzip.open if path.suffix == ".gz" else open
    header, states = {}, []
    with opener(path, "rt") as f:
        for line in f:
            line = line.strip()
            if not line:
                continue
            try:
                rec = json.loads(line)
            except json.JSONDecodeError:
                break  # the last line of a run the recorder didn't get to finish
            if rec.get("kind") == "header":
                header = rec
            elif rec.get("kind") == "state":
                states.append(rec)
    return header, states


def replay_states(states: list[dict]) -> Replay | None:
    if not states:
        return None
    r = Replay(states[0]["gs"])
    for i, s in enumerate(states[1:], 1):
        ahead = [(t["gs"], t.get("highlighted_before")) for t in states[i + 1:i + Replay.MAX_SKIPPED]]
        r.feed(s["gs"], s.get("highlighted_before"), ahead)
        if r.lost:
            break
    return r


# ------------------------------------------------------------------ the engine as BalatroBot reports a game
def engine_gamestate(game, id_of=lambda kind, i: i) -> dict:
    """The engine's state in BalatroBot's format (for tests and the recorder's self-check). `id_of` turns the
    engine's ids into the game's (they are never the same numbers in a real game)."""
    v = game.view()

    def mods(**kw):
        m = {k: x for k, x in kw.items() if x}
        return m or []

    def flags(**kw):
        s = {k: True for k, x in kw.items() if x}
        return s or []

    hand = [{"id": id_of("card", c["id"]), "key": None if c["face_down"] else c["key"],
             "set": "ENHANCED" if c["enhancement"] else "DEFAULT",
             "modifier": mods(enhancement=c["enhancement"], edition=c["edition"], seal=c["seal"]),
             "state": flags(debuff=c["debuff"], hidden=c["face_down"])} for c in v["hand"]]
    jokers = [{"id": id_of("joker", j["id"]), "key": j["key"], "set": "JOKER", "modifier": mods(edition=j["edition"]),
               "state": flags(hidden=j["face_down"], debuff=j["debuff"]), "cost": {"sell": j["sell"], "buy": 0}}
              for j in v["jokers"]]

    def item(x, buy=True):
        out = {"key": x["key"], "set": (x.get("set") or "JOKER").upper(), "modifier": mods(edition=x.get("edition"))}
        if "enhancement" in x:
            out["set"] = "ENHANCED" if x.get("enhancement") else "DEFAULT"
            out["modifier"] = mods(enhancement=x.get("enhancement"), edition=x.get("edition"), seal=x.get("seal"))
        cost = {}
        if buy and x.get("cost") is not None:
            cost["buy"] = x["cost"]
        if x.get("sell") is not None:
            cost["sell"] = x["sell"]
        if cost:
            out["cost"] = cost
        return out

    consumables = [dict(item(c), id=id_of("consumable", c["id"])) for c in v["consumables"]]
    blinds = {slot: {"type": slot.upper(), "name": b["name"], "status": b["status"],
                     "tag_name": b.get("tag_name") or ""} for slot, b in v["blinds"].items()}
    hands = {h["name"]: {"level": h["level"], "chips": h["chips"], "mult": h["mult"], "played": h["played"]}
             for h in v["hand_levels"]}
    deck = [{"key": c["key"], "modifier": mods(enhancement=c["enhancement"], edition=c["edition"], seal=c["seal"])}
            for c in v["deck_cards"] if c["left"]]
    return {
        "hands": hands,
        "cards": {"cards": deck},
        "state": v["state"], "ante_num": v["ante"], "round_num": v["round"], "money": v["money"],
        "seed": game.seed, "deck": "RED", "stake": "WHITE", "won": v["won"],
        "round": {"hands_left": v["hands_left"], "discards_left": v["discards_left"], "chips": v["chips"],
                  "reroll_cost": v["reroll_cost"]},
        "hand": {"cards": hand}, "jokers": {"cards": jokers}, "consumables": {"cards": consumables},
        "shop": {"cards": [item(x) for x in v["shop"]]},
        "vouchers": {"cards": [item(x) for x in v["shop_vouchers"]]},
        "packs": {"cards": [item(x) for x in v["shop_packs"] if x is not None]},
        "pack": {"cards": [item(x, buy=False) for x in v["pack"]]},
        "blinds": blinds,
    }


def main(argv: list[str] | None = None) -> int:
    import argparse

    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("runs", nargs="*", type=Path, help="recorded runs (default: all in .sync/me/runs)")
    a = ap.parse_args(argv)
    paths = a.runs or sorted((ROOT / ".sync" / "me" / "runs").glob("*.jsonl*"))
    if not paths:
        print("No recorded runs yet: make record-me")
        return 1
    for p in paths:
        _header, states = read_run(p)
        r = replay_states(states)
        if r is None:
            print(f"{p.name}: no states")
            continue
        s = r.summary()
        how = "won" if s["won"] else (f"over at ante {s['ante']}" if s["over"] else f"ante {s['ante']}, not finished")
        print(f"{p.name}: {len(states)} states, {len(s['moves'])} engine moves, {how}; "
              + ("replayed to the end" if r.ok else f"replay stopped: {r.lost}"))
    return 0


if __name__ == "__main__":
    sys.path.insert(0, str(ROOT / "scripts"))
    raise SystemExit(main())
