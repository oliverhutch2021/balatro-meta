"""The network that learns to play Balatro (make train): it copies the teacher, then improves by PPO on the GPU.

    python scripts/train_policy.py imitate --data ~/.cache/balatro-meta/data/moves-1 --out models/policy/imitate.pt
    python scripts/train_policy.py ppo --init models/policy/imitate.pt --dir models/policy --hours 8
    python scripts/train_policy.py eval --model models/policy/latest.pt --runs 5000 --seed 1 --out results/train/eval.json

It reads only the screen (observe.rs: `encode`), as a set of objects: the cards in hand, the jokers in order, the
consumables, the shop's cards, vouchers and packs, an open pack's cards, and one token for everything else (money,
hands, discards, the blind, the ante, hand levels, the deck). A transformer reads them together, so joker order and
card positions are part of what it sees. It picks a kind of move (only kinds the game allows now), then the slot
(which joker to sell, which shop card to buy...), where to move it, and which cards to play, throw or target, each
over the objects present. A second head estimates what the run will still earn.

- imitate: behaviour cloning on the teacher's moves (`baseline --record-moves`, `make teacher-moves`), runs split
  whole between training and judging. The value head learns what each position went on to earn.
- ppo: thousands of runs at once on all threads (the engine), the network on the GPU. Rewarded for every blind beaten
  (1/24 each) and for winning (+1); a refused move costs a little. Checkpoints every few minutes; rerunning resumes.
  Live numbers and runs to replay go to $BM_WATCH_DIR for make watch.
- eval: plays given seeds (the baseline job's: `run_seed(base, i)`, so the teacher's runs on the same seeds compare
  run by run) with its most likely moves, into a report like the baseline job's.

Moves the real game's bridge can't make (buy and use at once, boss reroll) and display-only sorting are never chosen.
"""
from __future__ import annotations

import argparse
import collections
import json
import math
import os
import random
import signal
import sys
import time
from pathlib import Path

import numpy as np

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))

N_KINDS = 21
# Move kinds (env.rs).
SELECT_BLIND, SKIP_BLIND, CASH_OUT, LEAVE_SHOP, SORT_RANK, SORT_SUIT, PLAY, DISCARD, MOVE_CARD, MOVE_JOKER, \
    SELL_JOKER, REROLL, BUY, BUY_AND_USE, REDEEM_VOUCHER, OPEN_BOOSTER, SELL_CONSUMABLE, USE_CONSUMABLE, \
    PACK_CHOOSE, PACK_SKIP, REROLL_BOSS = range(N_KINDS)
NEVER = (SORT_RANK, SORT_SUIT, BUY_AND_USE, REROLL_BOSS)  # not chosen by the network
FREE = (MOVE_CARD, MOVE_JOKER)  # moves that change nothing but order
MAX_HAND = 24
N_JOKER_TOKENS = 152  # joker kinds + empty + face down (observe.rs: joker_token)
GROUPS = ("hand", "jokers", "consumables", "shop_row", "shop_vouchers", "boosters", "pack", "cands", "dcands")
GLOBAL_PARTS = ("global", "hand_levels", "deck", "vouchers", "pack_booster", "tail", "learner")
# Which group a kind's slot points into; kinds with a target "to" (moves) and with cards to pick.
SLOT_GROUP = {MOVE_CARD: "hand", MOVE_JOKER: "jokers", SELL_JOKER: "jokers", BUY: "shop_row",
              REDEEM_VOUCHER: "shop_vouchers", OPEN_BOOSTER: "boosters", SELL_CONSUMABLE: "consumables",
              USE_CONSUMABLE: "consumables", PACK_CHOOSE: "pack", PLAY: "cands", DISCARD: "dcands"}
# Play candidates (lab #38, after the exchange evaluation: the teacher's play took ante-1 deaths from 17% to 2%): in a
# round the engine offers its PLAY_CANDS best plays (learner::play_candidates, best first) as tokens, then a free pick.
# A play points at one: a candidate plays its cards (no card term); the free pick (slot FREE_SLOT), or no candidates
# shown at all (a screen-only recording), picks cards with the card head as before. Each candidate token's card
# columns (from CAND_CARDS) are the hand positions it plays.
PLAY_CANDS = 8
FREE_SLOT = PLAY_CANDS
CAND_CARDS = 24
# Discard candidates (lab #38, smoke-3): the planner's plans as throws (learner::discard_candidates), then a free
# pick (slot D_FREE), the same way: a candidate throws its cards, the free pick uses the card head.
DISCARD_CANDS = 8
D_FREE = DISCARD_CANDS
DCAND_CARDS = 16
HAS_TO = (MOVE_CARD, MOVE_JOKER)
# Cards picked: (fewest, most) for each kind that picks cards.
CARD_RANGE = {PLAY: (1, 5), DISCARD: (1, 5), USE_CONSUMABLE: (0, 5), PACK_CHOOSE: (0, 5)}
# Rewards (lab #38, Stage 0): the win, plus a potential of the run's position kept at a loss (antezero's lesson: zeroed,
# every loss returns the same and nothing is learned) - W_BLIND a blind beaten, on the high-water mark of position, so
# an ante sent back (Hieroglyph, Petroglyph) never pays twice, and W_SCORE for the share of the blind's target scored.
# Time is counted by blinds (B's form): a transition that ends a blind or the run is discounted by --gamma-blind, every
# other by 1, so a long shop costs nothing by itself (guarded by the shop cap and the free-move rules).
BLIND_REWARD = 1 / 24
W_BLIND = 0.04
GAMMA_BLIND = 0.98  # --gamma-blind's default
W_SCORE = 0.02
WIN_REWARD = 1.0
SHOP_CAP = 60  # moves in one shop (its packs included) after which only leaving is offered
# Two values (lab #38, S's ruling 1, B's form): column 0 the win-only return, column 1 the depth return (the
# potential and the small costs). The advantage is A_win + beta x A_depth, normalised; beta starts at 1 and steps
# down as the evaluator's win rate rises (`next_beta`).
N_VALUES = 2
WIN, DEPTH = 0, 1
REFUSED_COST = 0.01
FREE_COST = 0.0002
MAX_MOVES = 3000  # a run still going after this many moves is cut short
FREE_STREAK = 16  # reorder moves in a row before reordering is put aside until another move
# The job's numbers for make watch (RunBook): "now" is the latest NOW_RUNS runs; the training's progress is kept a
# block of BLOCK_RUNS runs at a time, and beyond MAX_BLOCKS blocks the oldest merge in pairs.
NOW_RUNS = 1000
BLOCK_RUNS = 1000
MAX_BLOCKS = 400
BLOCK_FIELDS = ("end_runs", "end_seconds", "runs", "wins", "ante", "moves", "refused", "jokers", "packs", "rerolls",
                "money", "cut")
CUT = "Too many moves"  # what ends a run cut short (the engine's tally::CUT)


def shown(path) -> str:
    """A path as reports show it: the home folder as ~ (reports are committed and the repo can be public)."""
    s, home = str(path), str(Path.home())
    return "~" + s[len(home):] if s == home or s.startswith(home + "/") else s


def engine():
    from balatro_meta import _engine

    return _engine


def layout() -> dict[str, tuple[int, int, int]]:
    """The screen encoding's parts: name -> (start, slots, width)."""
    return {name: (start, slots, width) for name, start, slots, width in engine().OBS_LAYOUT}


# ---------------------------------------------------------------------------------------------- moves <-> parts
def to_parts(kinds: np.ndarray, args: np.ndarray) -> dict[str, np.ndarray]:
    """Moves (kind, argument) as the network's parts: the slot in its group, where to move it, and the cards."""
    kinds = kinds.astype(np.int64)
    args = args.astype(np.uint64)
    n = len(kinds)
    slot = np.full(n, -1, np.int64)
    to = np.full(n, -1, np.int64)
    low = (args & np.uint64(0xFF)).astype(np.int64)
    high = ((args >> np.uint64(8)) & np.uint64(0xFF)).astype(np.int64)
    has_slot = np.isin(kinds, list(SLOT_GROUP)) & (kinds != PLAY) & (kinds != DISCARD)
    slot[has_slot] = low[has_slot]
    # A play's or a discard's argument is its cards: as parts it is the free pick (a recorded move names no
    # candidate).
    slot[kinds == PLAY] = FREE_SLOT
    slot[kinds == DISCARD] = D_FREE
    has_to = np.isin(kinds, HAS_TO)
    to[has_to] = high[has_to]
    bits = np.where(np.isin(kinds, (PLAY, DISCARD)), args,
                    np.where(np.isin(kinds, (USE_CONSUMABLE, PACK_CHOOSE)), args >> np.uint64(8), np.uint64(0)))
    cards = ((bits[:, None] >> np.arange(MAX_HAND, dtype=np.uint64)[None, :]) & np.uint64(1)).astype(bool)
    return {"kind": kinds, "slot": slot, "to": to, "cards": cards}


def to_moves(kind: np.ndarray, slot: np.ndarray, to: np.ndarray, cards: np.ndarray) -> np.ndarray:
    """The engine's argument for each move from its parts."""
    kind = kind.astype(np.int64)
    bits = (cards.astype(np.uint64) << np.arange(MAX_HAND, dtype=np.uint64)[None, :]).sum(1).astype(np.uint64)
    s = np.maximum(slot, 0).astype(np.uint64)
    t = np.maximum(to, 0).astype(np.uint64)
    arg = np.zeros(len(kind), np.uint64)
    arg = np.where(np.isin(kind, (PLAY, DISCARD)), bits, arg)
    arg = np.where(np.isin(kind, HAS_TO), s | (t << np.uint64(8)), arg)
    arg = np.where(np.isin(kind, (SELL_JOKER, BUY, REDEEM_VOUCHER, OPEN_BOOSTER, SELL_CONSUMABLE)), s, arg)
    arg = np.where(np.isin(kind, (USE_CONSUMABLE, PACK_CHOOSE)), s | (bits << np.uint64(8)), arg)
    return arg


# ---------------------------------------------------------------------------------------------- the network
def need_torch():
    try:
        import torch
    except ImportError:
        raise SystemExit("PyTorch is missing: run `make setup-gpu` first.") from None
    return torch


HAND_EXTRA = 3


def hand_features(torch, raw):
    """What each card in hand makes with the others, from the cards on screen: how many share its rank, how many its
    suit, and the most ranks of a five-rank stretch (a straight's) it sits in (ace high or low). (B, slots, 3)"""
    ranks = raw[..., 1:14].float()  # 2..A
    suits = raw[..., 14:18].float()
    per_rank = ranks.sum(1)
    per_suit = suits.sum(1)
    same_rank = (ranks * per_rank[:, None, :]).sum(-1)
    same_suit = (suits * per_suit[:, None, :]).sum(-1)
    present = (per_rank > 0).float()
    ext = torch.cat([present[:, 12:13], present], 1)  # the ace low, then 2..A
    windows = ext.unfold(1, 5, 1).sum(-1)  # (B, 10): ext[w..w+4]
    best = torch.stack([windows[:, max(0, q - 4):min(9, q) + 1].max(1).values for q in range(14)], 1)
    at = torch.cat([best[:, 1:14]], 1)  # rank i (2..A) sits at ext index i + 1
    stretch = (ranks * at[:, None, :]).sum(-1)
    stretch = torch.maximum(stretch, ranks[..., 12] * best[:, 0:1])  # an ace in A-2-3-4-5
    return torch.stack([same_rank / 4, same_suit / 8, stretch / 5], -1)


def build_model(torch, d: int = 128, layers: int = 4, heads: int = 4, ff: int = 384):
    nn = torch.nn
    lay = layout()
    e = engine()
    if (e.PLAY_CANDS, e.CAND_CARDS, getattr(e, "DISCARD_CANDS", None), getattr(e, "DCAND_CARDS", None)) != (
            PLAY_CANDS, CAND_CARDS, DISCARD_CANDS, DCAND_CARDS):
        raise SystemExit("the engine's play or discard candidates differ from this script's: rebuild (make build)")

    class Policy(nn.Module):
        def __init__(self):
            super().__init__()
            self.cfg = {"d": d, "layers": layers, "heads": heads, "ff": ff}
            self.lay = lay
            self.proj = nn.ModuleDict({g: nn.Linear(lay[g][2] + (HAND_EXTRA if g == "hand" else 0), d)
                                       for g in GROUPS})
            self.glob = nn.Linear(sum(lay[p][1] * lay[p][2] for p in GLOBAL_PARTS), d)
            self.group_emb = nn.Embedding(len(GROUPS) + 1, d)
            self.slot_emb = nn.Embedding(MAX_HAND, d)
            self.joker_emb = nn.Embedding(N_JOKER_TOKENS + 1, d)
            self.norm_in = nn.LayerNorm(d)
            layer = nn.TransformerEncoderLayer(d, heads, ff, dropout=0.0, batch_first=True, norm_first=True,
                                               activation="gelu")
            self.enc = nn.TransformerEncoder(layer, layers, enable_nested_tensor=False)
            self.norm_out = nn.LayerNorm(d)
            self.kind_head = nn.Sequential(nn.Linear(d, d), nn.GELU(), nn.Linear(d, N_KINDS))
            # Two values (lab #38, S's ruling 1): the win-only return and the depth (the potential's) return, each in
            # units of its own running spread (`Spread`).
            self.value_head = nn.Sequential(nn.Linear(d, d), nn.GELU(), nn.Linear(d, N_VALUES))
            # Its last layer starts at zero (B, lab #38): an untrained head's random output was about 60 times the
            # real signal in the advantage, and the spread's rescaling kept it there.
            nn.init.zeros_(self.value_head[-1].weight)
            nn.init.zeros_(self.value_head[-1].bias)
            self.kind_emb = nn.Embedding(N_KINDS, d)
            self.q_slot, self.k_slot = nn.Linear(d, d), nn.Linear(d, d)
            self.q_to, self.k_to = nn.Linear(d, d), nn.Linear(d, d)
            self.card_head = nn.Sequential(nn.Linear(2 * d, d), nn.GELU(), nn.Linear(d, 1))
            # Token positions of each group in the sequence (the global token is 0).
            spans, at = {}, 1
            for g in GROUPS:
                spans[g] = (at, at + lay[g][1])
                at += lay[g][1]
            self.spans, self.n_tokens = spans, at
            # The input's length: a screen-only input (the teacher's recorded moves, written before the learner's
            # block existed) is padded with zeros for that block (lab #38).
            self.n_in = max(s + k * w for s, k, w in lay.values())
            group_of = torch.zeros(N_KINDS, at, dtype=torch.bool)
            for k, g in SLOT_GROUP.items():
                a, b = spans[g]
                group_of[k, a:b] = True
            self.register_buffer("group_of", group_of, persistent=False)
            allowed = torch.ones(N_KINDS, dtype=torch.bool)
            allowed[list(NEVER)] = False
            self.register_buffer("allowed", allowed, persistent=False)

        def encode(self, x):
            """Tokens (B, T, d), their presence (B, T) and the global token's output (B, d)."""
            B = x.shape[0]
            if x.shape[1] < self.n_in:
                x = torch.nn.functional.pad(x, (0, self.n_in - x.shape[1]))
            sq = torch.sign(x) * torch.log1p(torch.abs(x))  # big numbers (targets, chips) squashed
            toks, present = [], []
            gl = torch.cat([sq[:, s:s + n * w] for s, n, w in (self.lay[p] for p in GLOBAL_PARTS)], 1)
            toks.append((self.glob(gl) + self.group_emb.weight[len(GROUPS)])[:, None, :])
            present.append(torch.ones(B, 1, dtype=torch.bool, device=x.device))
            for gi, g in enumerate(GROUPS):
                s, n, w = self.lay[g]
                raw = x[:, s:s + n * w].reshape(B, n, w)
                feats = sq[:, s:s + n * w].reshape(B, n, w)
                if g == "hand":
                    # The computed hand features first, so the screen's own columns are last and can grow at the end
                    # (an input switched on later: `load_widened`).
                    feats = torch.cat([hand_features(torch, raw), feats], -1)
                t = self.proj[g](feats) + self.group_emb.weight[gi] + self.slot_emb.weight[:n][None]
                if g == "jokers":
                    t = t + self.joker_emb(self._joker_ids(raw[..., 1], raw[..., 0] > 0))
                    p = raw[..., 0] > 0
                elif g in ("shop_row", "pack"):
                    is_joker = raw[..., 0] > 0
                    t = t + self.joker_emb(self._joker_ids(raw[..., 4], is_joker))
                    p = raw[..., 0:3].sum(-1) > 0
                else:
                    p = raw[..., 0] > 0
                toks.append(t)
                present.append(p)
            t = self.norm_in(torch.cat(toks, 1))
            present = torch.cat(present, 1)
            h = self.norm_out(self.enc(t, src_key_padding_mask=~present))
            return h, present, h[:, 0]

        @staticmethod
        def _joker_ids(scalar, has):
            ids = torch.round(scalar.float() * N_JOKER_TOKENS).long().clamp(0, N_JOKER_TOKENS - 1)
            return torch.where(has, ids, torch.full_like(ids, N_JOKER_TOKENS))

        def kind_logits(self, cls, legal):
            logits = self.kind_head(cls).float()
            ok = legal & self.allowed[None]
            # A run with no kind left (it shouldn't happen) may still pick from what the game allows.
            ok = torch.where(ok.any(1, keepdim=True), ok, legal)
            return logits.masked_fill(~ok, -1e9)

        def slot_logits(self, h, present, cls, kind):
            q = self.q_slot(cls + self.kind_emb(kind))
            logits = torch.einsum("bd,btd->bt", q, self.k_slot(h)).float() / math.sqrt(q.shape[-1])
            ok = self.group_of[kind] & present
            return logits.masked_fill(~ok, -1e9), ok

        def to_logits(self, h, present, cls, kind, slot_tok):
            src = h[torch.arange(h.shape[0], device=h.device), slot_tok]
            q = self.q_to(cls + self.kind_emb(kind) + src)
            logits = torch.einsum("bd,btd->bt", q, self.k_to(h)).float() / math.sqrt(q.shape[-1])
            ok = self.group_of[kind] & present
            ok[torch.arange(h.shape[0], device=h.device), slot_tok] = False
            return logits.masked_fill(~ok, -1e9), ok

        def card_logits(self, h, present, cls, kind, slot_tok, forced):
            a, b = self.spans["hand"]
            with_slot = torch.isin(kind, torch.tensor([USE_CONSUMABLE, PACK_CHOOSE], device=h.device))
            src = h[torch.arange(h.shape[0], device=h.device), slot_tok] * with_slot[:, None]
            q = (cls + self.kind_emb(kind) + src)[:, None, :].expand(-1, b - a, -1)
            logits = self.card_head(torch.cat([h[:, a:b], q], -1)).squeeze(-1).float()
            here = present[:, a:b]
            logits = torch.where(forced & here, torch.full_like(logits, 30.0), logits)
            return logits.masked_fill(~here, -1e9), here

        def value(self, cls):
            """(B, N_VALUES): the win and depth values, each in units of its running spread."""
            return self.value_head(cls).float()

    return Policy()


# ---------------------------------------------------------------------------------------------- distributions
def cat_logp(torch, logits, idx):
    lp = torch.log_softmax(logits, -1)
    return lp.gather(1, idx.clamp(min=0)[:, None]).squeeze(1)


def cat_entropy(torch, logits):
    lp = torch.log_softmax(logits, -1)
    return -(lp.exp() * lp).sum(-1)


def count_dist(torch, p, top: int):
    """P(number picked = 0..top, and more) for independent picks with probabilities p (B, n)."""
    B, n = p.shape
    c = torch.zeros(B, top + 2, device=p.device, dtype=p.dtype)
    c[:, 0] = 1.0
    for i in range(n):
        pi = p[:, i:i + 1]
        shifted = torch.cat([torch.zeros_like(c[:, :1]), c[:, :-1]], 1)
        new = c * (1 - pi) + shifted * pi
        new[:, -1] = new[:, -1] + c[:, -1] * pi[:, 0]
        c = new
    return c


def cards_logp(torch, logits, here, cards, lo, hi):
    """log P(these cards) for independent picks conditioned on picking between lo and hi of them (per row)."""
    l = logits.float()
    lp1 = -torch.nn.functional.softplus(-l)
    lp0 = -torch.nn.functional.softplus(l)
    picked = cards & here
    base = torch.where(picked, lp1, torch.where(here, lp0, torch.zeros_like(l))).sum(1)
    p = torch.sigmoid(l) * here
    dist = count_dist(torch, p, 5)
    ks = torch.arange(7, device=l.device)[None, :]
    valid = ((ks >= lo[:, None]) & (ks <= hi[:, None])).float()
    z = (dist * valid).sum(1).clamp(min=1e-12)
    return base - torch.log(z)


def cards_entropy(torch, logits, here):
    p = torch.sigmoid(logits.float())
    h = -(p * torch.log(p.clamp(min=1e-9)) + (1 - p) * torch.log((1 - p).clamp(min=1e-9)))
    return (h * here).sum(1)


def conditioned_picks(torch, p, lo, hi, gen=None):
    """Independent picks with probabilities p (B, n), conditioned on picking between lo and hi of them, drawn exactly:
    card by card, each taken with the chance that taking it (or not) still leaves the rest able to bring the count into
    range. So a draw has exactly the probability cards_logp gives it. Returns the picks and which rows could fit at
    all (a row that can't, fewer cards present than lo, picks nothing)."""
    B, n = p.shape
    q = p.double()
    K = int(hi.max()) + 2 if B else 2  # counts 0 .. the largest hi, and "more" (which never fits)
    # suffix[:, i, k]: P(k of the cards from i on are picked), as count_dist counts all of them.
    suffix = torch.zeros(B, n + 1, K, dtype=torch.float64, device=p.device)
    suffix[:, n, 0] = 1.0
    for i in range(n - 1, -1, -1):
        nxt, pi = suffix[:, i + 1], q[:, i:i + 1]
        shifted = torch.cat([torch.zeros_like(nxt[:, :1]), nxt[:, :-1]], 1)
        cur = nxt * (1 - pi) + shifted * pi
        cur[:, -1] = cur[:, -1] + nxt[:, -1] * pi[:, 0]
        suffix[:, i] = cur
    ks = torch.arange(K - 1, device=p.device)[None, :]

    def fits(rest, count):
        total = count[:, None] + ks
        return (rest[:, :K - 1] * ((total >= lo[:, None]) & (total <= hi[:, None]))).sum(1)

    count = torch.zeros(B, dtype=torch.long, device=p.device)
    pick = torch.zeros(B, n, dtype=torch.bool, device=p.device)
    u = torch.rand(B, n, device=p.device, generator=gen, dtype=torch.float64)
    for i in range(n):
        rest = suffix[:, i + 1]
        yes = q[:, i] * fits(rest, count + 1)
        no = (1 - q[:, i]) * fits(rest, count)
        take = u[:, i] * (yes + no) < yes
        pick[:, i] = take
        count = count + take.long()
    return pick, fits(suffix[:, 0], torch.zeros_like(count)) > 0


def sample_cards(torch, logits, here, lo, hi, greedy: bool, gen=None):
    """Picks between lo and hi of the present cards: sampled from the picks' distribution given that the count fits
    (conditioned_picks: exactly what cards_logp scores), or the likeliest."""
    p = torch.sigmoid(logits.float()) * here
    if greedy:
        pick = (p > 0.5) & here
    else:
        # Only the columns up to the last card present in any row: the rest pick nothing.
        cols = int(here.any(0).nonzero().max()) + 1 if here.any() else 0
        pick = torch.zeros_like(here)
        if cols:
            got, _ = conditioned_picks(torch, p[:, :cols], lo, hi, gen)
            pick[:, :cols] = got & here[:, :cols]
    # Rows outside the range (greedy, or a row no pick could fit): the likeliest cards fill or trim them.
    n = pick.sum(1)
    order = torch.argsort(torch.where(here, p, torch.full_like(p, -1.0)), 1, descending=True)
    rank = torch.empty_like(order)
    rank.scatter_(1, order, torch.arange(p.shape[1], device=p.device)[None].expand_as(order))
    need_more = n < lo
    pick = torch.where(need_more[:, None], (rank < lo[:, None]) & here, pick)
    too_many = pick.sum(1) > hi
    if too_many.any():
        sel_rank = torch.argsort(torch.argsort(torch.where(pick, p, torch.full_like(p, -1.0)), 1, descending=True), 1)
        pick = torch.where(too_many[:, None], pick & (sel_rank < hi[:, None]), pick)
    return pick


class Parts:
    """What one batch of positions and moves need from the network: log-probabilities, entropy, value."""

    def __init__(self, torch, model, value_detached: bool = False):
        # value_detached: the value head learns on the network's features without changing them, so learning the
        # value can't change how it plays (PPO; with them shared, a value going wrong drags the play down with it).
        self.t, self.m, self.value_detached = torch, model, value_detached

    def lo_hi(self, kind):
        torch = self.t
        lo = torch.zeros_like(kind)
        hi = torch.zeros_like(kind)
        for k, (a, b) in CARD_RANGE.items():
            lo = torch.where(kind == k, torch.full_like(kind, a), lo)
            hi = torch.where(kind == k, torch.full_like(kind, b), hi)
        return lo, hi

    def slot_token(self, kind, slot):
        torch = self.t
        start = torch.zeros_like(kind)
        for k, g in SLOT_GROUP.items():
            start = torch.where(kind == k, torch.full_like(kind, self.m.spans[g][0]), start)
        return torch.where(slot >= 0, start + slot.clamp(min=0), torch.zeros_like(kind))

    def forced(self, x):
        s, n, w = self.m.lay["hand"]
        return x[:, s:s + n * w].reshape(x.shape[0], n, w)[..., 39] > 0

    def is_candidate(self, kind, slot):
        """Plays and discards that name a candidate (slot below the free pick's): their cards are the candidate's,
        never picked."""
        return (((kind == PLAY) & (slot < FREE_SLOT)) | ((kind == DISCARD) & (slot < D_FREE))) & (slot >= 0)

    def _group_cards(self, x, slot, group, col):
        torch = self.t
        s, n, w = self.m.lay[group]
        if x.shape[1] < s + n * w:
            return torch.zeros(x.shape[0], MAX_HAND, dtype=torch.bool, device=x.device)
        rows = x[:, s:s + n * w].reshape(x.shape[0], n, w)
        tok = rows[torch.arange(x.shape[0], device=x.device), slot.clamp(0, n - 1)]
        return tok[:, col:col + MAX_HAND] > 0.5

    def candidate_cards(self, x, kind, slot):
        """The hand positions candidate `slot` plays or throws (B, MAX_HAND), read from its token's card columns."""
        play = self._group_cards(x, slot, "cands", CAND_CARDS)
        throw = self._group_cards(x, slot, "dcands", DCAND_CARDS)
        return self.t.where((kind == DISCARD)[:, None], throw, play)

    def evaluate(self, x, legal, kind, slot, to, cards):
        """log pi(move), entropy and value for given moves (as parts)."""
        torch, m = self.t, self.m
        h, present, cls = m.encode(x)
        kl = m.kind_logits(cls, legal)
        logp = cat_logp(torch, kl, kind)
        ent = cat_entropy(torch, kl)
        sl, sok = m.slot_logits(h, present, cls, kind)
        # A slot (and a target) only when there is one to pick, exactly as act() decides: otherwise the move was
        # judged with a term it was never played with.
        has_slot = torch.isin(kind, torch.tensor(list(SLOT_GROUP), device=x.device)) & sok.any(1)
        slot_tok = self.slot_token(kind, slot)
        logp = logp + torch.where(has_slot, cat_logp(torch, sl, slot_tok), torch.zeros_like(logp))
        ent = ent + torch.where(has_slot, cat_entropy(torch, sl), torch.zeros_like(ent))
        tl, tok = m.to_logits(h, present, cls, kind, slot_tok)
        has_to = torch.isin(kind, torch.tensor(HAS_TO, device=x.device)) & tok.any(1)
        to_tok = self.slot_token(kind, to)
        logp = logp + torch.where(has_to, cat_logp(torch, tl, to_tok), torch.zeros_like(logp))
        ent = ent + torch.where(has_to, cat_entropy(torch, tl), torch.zeros_like(ent))
        has_cards = torch.isin(kind, torch.tensor(list(CARD_RANGE), device=x.device))
        has_cards = has_cards & ~(self.is_candidate(kind, slot) & has_slot)
        cl, here = m.card_logits(h, present, cls, kind, slot_tok, self.forced(x))
        lo, hi = self.lo_hi(kind)
        logp = logp + torch.where(has_cards, cards_logp(torch, cl, here, cards, lo, hi), torch.zeros_like(logp))
        ent = ent + torch.where(has_cards, cards_entropy(torch, cl, here), torch.zeros_like(ent))
        return logp, ent, m.value(cls.detach() if self.value_detached else cls)

    def act(self, x, legal, greedy: bool = False):
        """Moves for a batch of positions (as parts), with their log-probabilities and the value."""
        torch, m = self.t, self.m
        h, present, cls = m.encode(x)
        kl = m.kind_logits(cls, legal)
        kind = kl.argmax(1) if greedy else torch.distributions.Categorical(logits=kl).sample()
        sl, sok = m.slot_logits(h, present, cls, kind)
        has_slot = torch.isin(kind, torch.tensor(list(SLOT_GROUP), device=x.device)) & sok.any(1)
        st = sl.argmax(1) if greedy else torch.distributions.Categorical(logits=sl).sample()
        st = torch.where(has_slot, st, torch.zeros_like(st))
        slot = torch.zeros_like(kind)
        for k, g in SLOT_GROUP.items():
            slot = torch.where(kind == k, st - m.spans[g][0], slot)
        slot = torch.where(has_slot, slot, torch.full_like(slot, -1))
        slot_tok = self.slot_token(kind, slot)
        tl, tok = m.to_logits(h, present, cls, kind, slot_tok)
        has_to = torch.isin(kind, torch.tensor(HAS_TO, device=x.device)) & tok.any(1)
        tt = tl.argmax(1) if greedy else torch.distributions.Categorical(logits=tl).sample()
        to = torch.zeros_like(kind)
        for k in HAS_TO:
            to = torch.where(kind == k, tt - m.spans[SLOT_GROUP[k]][0], to)
        to = torch.where(has_to, to, torch.full_like(to, -1))
        cl, here = m.card_logits(h, present, cls, kind, slot_tok, self.forced(x))
        lo, hi = self.lo_hi(kind)
        cand = self.is_candidate(kind, slot)
        has_cards = torch.isin(kind, torch.tensor(list(CARD_RANGE), device=x.device)) & ~cand
        cards = sample_cards(torch, cl, here, lo, hi, greedy) & has_cards[:, None]
        cards = torch.where(cand[:, None], self.candidate_cards(x, kind, slot), cards)
        # The log-probability of exactly this move (the same sums as evaluate()).
        logp = cat_logp(torch, kl, kind)
        logp = logp + torch.where(has_slot, cat_logp(torch, sl, slot_tok), torch.zeros_like(logp))
        to_tok = self.slot_token(kind, to)
        logp = logp + torch.where(has_to, cat_logp(torch, tl, to_tok), torch.zeros_like(logp))
        logp = logp + torch.where(has_cards, cards_logp(torch, cl, here, cards, lo, hi), torch.zeros_like(logp))
        return kind, slot, to, cards, logp, m.value(cls)


# ---------------------------------------------------------------------------------------------- saving
# The network's input layout's version (B, lab #38): 2 from 46680c3, the hand's computed features first in each hand
# token. A checkpoint of an older version would load with its hand inputs scrambled, so it is refused.
LAYOUT_VERSION = 4  # 3: lab #38's play candidates ("cands"); 4: the discard candidates ("dcands"; each moved the
# global token's group embedding)


def save(torch, path: Path, model, extra: dict | None = None) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    tmp = path.with_suffix(".tmp")
    torch.save({"cfg": model.cfg, "model": model.state_dict(), "layout": dict(model.lay),
                "layout_version": LAYOUT_VERSION, **(extra or {})}, tmp)
    os.replace(tmp, path)


def load_model(torch, path: Path, device):
    ck = torch.load(path, map_location=device, weights_only=False)
    if ck.get("layout_version") != LAYOUT_VERSION:
        raise SystemExit(f"{path}: input layout version {ck.get('layout_version')}, this code reads {LAYOUT_VERSION} "
                         "(the inputs moved: v2 the hand's features, v3 the play candidates, v4 the discard "
                         "candidates): it would load with its "
                         "inputs scrambled; train afresh")
    model = build_model(torch, **ck["cfg"]).to(device)
    check_growth(ck.get("layout", {}), dict(model.lay))
    load_widened(torch, model, ck["model"])
    return model, ck


def check_growth(saved: dict, now: dict) -> None:
    """The layouts a widened load may join (B, lab #38): every saved part still there with as many slots; a part's
    width may grow (at its end), but in the global token only its last part may (the parts are joined end to end
    there, so a growth anywhere else lands mid-input); no part may be new unless it comes last."""
    for name, (_, n, w) in saved.items():
        if name not in now:
            raise ValueError(f"layout: part {name!r} is gone")
        _, n2, w2 = now[name]
        if n2 != n or w2 < w:
            raise ValueError(f"layout: part {name!r} changed from {n} x {w} to {n2} x {w2}")
        if w2 > w and name in GLOBAL_PARTS and name != GLOBAL_PARTS[-1]:
            raise ValueError(f"layout: {name!r} grew inside the global token (only {GLOBAL_PARTS[-1]!r} may)")
    new = [p for p in now if p not in saved]
    if saved and any(p in GLOBAL_PARTS[:-1] or p in GROUPS for p in new):
        raise ValueError(f"layout: new parts {new} not at the end")


def load_widened(torch, model, saved: dict) -> list[str]:
    """Load saved weights into a model whose inputs may since have grown (B, lab #38: an input switched on later
    loads into a checkpoint without a restart). A weight whose shape grew keeps its saved values in the leading
    rows and columns, the rest zero: a new input column then changes nothing until its weights move, and a new
    output row starts at 0. This is right only for growth at the END of each projection's input: in the global token
    only the last part (the learner block) may grow; a token may grow at the end of its own columns (its computed
    features come first). `check_growth` refuses any other change. Returns the names widened."""
    own = model.state_dict()
    widened = []
    for k, w in saved.items():
        if k in own and own[k].shape != w.shape:
            if own[k].dim() != w.dim() or any(n < o for n, o in zip(own[k].shape, w.shape)):
                raise ValueError(f"{k}: saved {tuple(w.shape)} can't widen to {tuple(own[k].shape)}")
            new = torch.zeros_like(own[k])
            new[tuple(slice(0, n) for n in w.shape)] = w
            saved = {**saved, k: new}
            widened.append(k)
    model.load_state_dict(saved)
    return widened


def device_of(torch, cpu: bool):
    return torch.device("cuda" if torch.cuda.is_available() and not cpu else "cpu")


# bf16 on the GPU only when asked (--bf16): antezero found bf16 rounding flipping near-tied choices between acting and
# learning (PPO ratios to e^15), so from Stage 0 (lab #38) the network runs in float32 with TF32 matrix products.
BF16 = {"on": False}


def autocast(torch, device):
    if device.type == "cuda" and BF16["on"]:
        return torch.autocast("cuda", dtype=torch.bfloat16)
    import contextlib

    return contextlib.nullcontext()


def fast_float32(torch, a) -> None:
    """float32 with TF32 matrix products (the default), or bf16 autocast with --bf16."""
    BF16["on"] = bool(getattr(a, "bf16", False))
    if torch.cuda.is_available():
        torch.backends.cuda.matmul.allow_tf32 = True
        torch.backends.cudnn.allow_tf32 = True


def wilson(k: int, n: int) -> tuple[float, float]:
    if n == 0:
        return 0.0, 0.0
    z, p = 1.96, k / n
    d = 1 + z * z / n
    c = (p + z * z / (2 * n)) / d
    h = z * math.sqrt(p * (1 - p) / n + z * z / (4 * n * n)) / d
    return max(0.0, c - h), min(1.0, c + h)


def write_json(path: Path, obj) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    tmp = path.with_suffix(path.suffix + ".tmp")
    tmp.write_text(json.dumps(obj, indent=1) + "\n")
    os.replace(tmp, path)


# ---------------------------------------------------------------------------------------------- imitation
def load_moves(folder: Path, max_runs: int | None):
    import train_value as tv

    if not (folder / "manifest.json").exists():
        raise SystemExit(f"{folder} has no finished recording of the teacher's moves: run make teacher-moves first.")
    data = tv.load_data(folder, max_runs)
    if data.manifest.get("input") != "screen":
        raise SystemExit(f"{folder} is not a recording of moves (baseline --record-moves).")
    chunks = sorted(p.name[: -len("-ptr.npy")] for p in folder.glob("chunk-*-ptr.npy"))
    mov = np.concatenate([np.load(folder / f"{c}-mov.npy") for c in chunks])[: data.rows]
    return data, mov


def returns(lab: np.ndarray, gamma: float) -> np.ndarray:
    """What each recorded position went on to earn in its run (the PPO rewards: blinds beaten, the win)."""
    from train_value import END_STAGE, RUN, STAGE, WON

    g = np.zeros(len(lab), np.float32)
    run, stage = lab[:, RUN], lab[:, STAGE]
    nxt = 0.0
    for i in range(len(lab) - 1, -1, -1):
        last = i == len(lab) - 1 or run[i + 1] != run[i]
        if last:
            r = (lab[i, END_STAGE] - stage[i]) * BLIND_REWARD + lab[i, WON] * WIN_REWARD
            nxt = r
        else:
            nxt = (stage[i + 1] - stage[i]) * BLIND_REWARD + gamma * nxt
        g[i] = nxt
    return g


def imitate(a) -> dict:
    torch = need_torch()
    fast_float32(torch, a)
    import train_value as tv

    t0 = time.time()
    torch.manual_seed(a.seed)
    device = device_of(torch, a.cpu)
    data, mov = load_moves(Path(a.data).expanduser(), a.max_runs)
    keep = ~np.isin(mov[:, 0], NEVER)
    part = tv.split(data.lab[:, tv.RUN])
    train_rows = np.flatnonzero(keep & (part == 0))
    judge_rows = np.flatnonzero(keep & (part != 0))
    if not len(train_rows) or not len(judge_rows):
        raise SystemExit("Too few runs to keep some apart.")
    parts = to_parts(mov[:, 0], mov[:, 1])
    legal = ((mov[:, 2:3] >> np.arange(N_KINDS)[None, :]) & 1).astype(bool)
    g = returns(data.lab, a.gamma)
    b = tv.Batches(torch, data, device)
    T = lambda v: torch.from_numpy(v).to(device)
    kind_t, slot_t, to_t = T(parts["kind"]), T(parts["slot"]), T(parts["to"])
    cards_t, legal_t, g_t = T(parts["cards"]), T(legal), T(g)
    model = build_model(torch, a.width, a.layers, a.heads, a.width * 3).to(device)
    pp = Parts(torch, model)
    opt = torch.optim.AdamW(model.parameters(), lr=a.lr, weight_decay=1e-4)
    per_epoch = max(1, math.ceil(len(train_rows) / a.batch))
    sched = torch.optim.lr_scheduler.OneCycleLR(opt, max_lr=a.lr, total_steps=a.epochs * per_epoch, pct_start=0.05)
    gen = torch.Generator().manual_seed(a.seed)
    tr = torch.from_numpy(train_rows).to(device)
    ju = torch.from_numpy(judge_rows[:a.judge_rows]).to(device)
    print(f"Imitating {len(train_rows):,} teacher moves ({len(judge_rows):,} kept apart) on {device}; "
          f"{sum(p.numel() for p in model.parameters()):,} weights.", flush=True)

    def judge():
        model.eval()
        tot = n = right = right_kind = 0.0
        with torch.no_grad():
            for k in range(0, len(ju), a.batch):
                rows = ju[k:k + a.batch]
                x = b.x(rows)
                with autocast(torch, device):
                    lp, _, _ = pp.evaluate(x, legal_t[rows], kind_t[rows], slot_t[rows], to_t[rows], cards_t[rows])
                    kind, slot, to, cards, _, _ = pp.act(x, legal_t[rows], greedy=True)
                same = (kind == kind_t[rows]) & (slot == slot_t[rows]) & (to == to_t[rows]) \
                    & (cards == cards_t[rows]).all(1)
                tot += float(-lp.sum())
                n += len(rows)
                right += float(same.sum())
                right_kind += float((kind == kind_t[rows]).sum())
        model.train()
        return tot / max(n, 1), right / max(n, 1), right_kind / max(n, 1)

    history = []
    best = (math.inf, None)
    step = 0
    for epoch in range(a.epochs):
        perm = tr[torch.randperm(len(tr), generator=gen).to(device)]
        run_loss = seen = 0.0
        for k in range(0, len(perm), a.batch):
            rows = perm[k:k + a.batch]
            x = b.x(rows)
            with autocast(torch, device):
                lp, _, v = pp.evaluate(x, legal_t[rows], kind_t[rows], slot_t[rows], to_t[rows], cards_t[rows])
            # (A teacher move the network can't express, say a slot it doesn't see, would swamp the batch: capped.)
            loss = -lp.clamp(min=-30).mean() + a.value_coef * ((v.sum(-1) - g_t[rows]) ** 2).mean()
            opt.zero_grad(set_to_none=True)
            loss.backward()
            torch.nn.utils.clip_grad_norm_(model.parameters(), 1.0)
            opt.step()
            sched.step()
            step += 1
            run_loss += float(loss.detach()) * len(rows)
            seen += len(rows)
            if step % a.report_every == 0:
                print(f"  step {step}: loss {run_loss / seen:.4f}", flush=True)
        jl, acc, kacc = judge()
        history.append({"epoch": epoch + 1, "train_loss": round(run_loss / max(seen, 1), 5),
                        "judge_nll": round(jl, 5), "same_move": round(acc, 4), "same_kind": round(kacc, 4)})
        print(f"epoch {epoch + 1}: judged -log p {jl:.4f}, same move as the teacher {acc:.1%} "
              f"(kind {kacc:.1%})", flush=True)
        if jl < best[0]:
            best = (jl, {k2: v2.detach().clone() for k2, v2 in model.state_dict().items()})
    model.load_state_dict(best[1])
    save(torch, Path(a.out), model, {"kind": "imitate", "data": str(a.data)})
    report = {"kind": "imitate", "data": shown(a.data), "moves": int(keep.sum()), "train_moves": len(train_rows),
              "judge_moves": len(judge_rows), "weights": sum(p.numel() for p in model.parameters()),
              "cfg": model.cfg, "history": history, "device": str(device), "seconds": round(time.time() - t0, 1),
              "model": str(a.out)}
    if a.report:
        write_json(Path(a.report), report)
    return report


# ---------------------------------------------------------------------------------------------- playing
class Watch:
    """Live numbers and runs to replay for make watch (the baseline job's files and names)."""

    def __init__(self, folder: Path | None, bot: str, target: int | None = None, threads: int = 0):
        self.dir = folder
        self.bot, self.target, self.threads = bot, target, threads
        self.history: list = []
        self.last = 0.0
        if folder:
            (folder / "samples").mkdir(parents=True, exist_ok=True)
            for p in (folder / "samples").glob("typical-*.json"):
                p.unlink()

    def sample(self, seed: str, moves: list, won: bool, ante: int, stage: int) -> None:
        if not self.dir:
            return
        result = "won the run" if won else f"lost at ante {ante}"
        body = {"seed": seed, "bot": self.bot, "source": "train", "pick": "typical", "run": 0,
                "label": f"a run by the {self.bot}", "result": result, "won": won, "ante": 9 if won else ante,
                "stage": stage, "closeness": 1.0 if won else 0.0, "best_hand": 0, "moves": moves}
        ms = int(time.time() * 1000)
        write_json(self.dir / "samples" / f"typical-{ms:015d}.json", body)
        files = sorted((self.dir / "samples").glob("typical-*.json"))
        for p in files[:-40]:
            p.unlink(missing_ok=True)

    def report(self, stats: dict, done: bool = False, extra=None) -> None:
        """Write progress.json (at most once a second unless done); `extra` is a dict or a function making one."""
        if not self.dir or (time.time() - self.last < 1.0 and not done):
            return
        self.last = time.time()
        if callable(extra):
            extra = extra()
        write_json(self.dir / "progress.json", progress_body(stats, self.bot, self.target, self.threads, done,
                                                             self.history, extra))


def progress_body(stats: dict, bot: str, target, threads: int, done: bool, history: list, extra=None) -> dict:
    runs, wins = stats["runs"], stats["wins"]
    ante_hist = stats["ante_hist"]
    lo, hi = wilson(wins, runs)
    mean = sum((i + 1) * n for i, n in enumerate(ante_hist)) / max(runs, 1)
    body = {"kind": "baseline", "bot": bot, "deck": "Red Deck", "stake": "White Stake", "done": done,
            "updated": time.time(), "elapsed": stats["elapsed"], "threads": threads, "target": target or runs,
            "runs": runs, "rate": stats["rate"], "moves": stats["moves"], "wins": wins,
            "win_rate": wins / max(runs, 1), "win_ci": [lo, hi], "mean_ante": mean, "ante_hist": ante_hist,
            "per_run": {"moves": stats["moves"] / max(runs, 1)}, "history": history}
    return {**body, **(extra or {})}


def ended_stage(blinds: int, ante: int) -> int:
    """The blind a lost run ended at (0 to 23): blinds beaten, or the ante's first blind if more were skipped."""
    a = min(max(ante, 1), 8)
    return int(min(max(blinds, (a - 1) * 3), a * 3 - 1))


class Guard:
    """Keeps a run from going round in circles: a kind of move the game just refused is put aside until a move
    goes through, and so is reordering after FREE_STREAK reorders in a row. (The network sees the narrower list
    of kinds, so what it learns matches what it could choose.)"""

    def __init__(self, n: int, skips: bool = True, shop_cap: int = 0):
        self.free = np.zeros(n, np.int64)
        self.blocked = np.zeros((n, N_KINDS), bool)
        self.allowed = np.ones(N_KINDS, bool)
        self.allowed[list(NEVER)] = False
        # Stage 0 (lab #38): blind skips off (a skip's tag is a reward the learner could farm; antezero's first
        # cheat), and a cap on moves in one shop, its packs included, after which only leaving is offered (with time
        # counted by blinds, a shop's moves are otherwise free).
        self.skips = skips
        self.shop_cap = shop_cap
        self.shop_moves = np.zeros(n, np.int64)

    @staticmethod
    def in_shop(legal: np.ndarray) -> np.ndarray:
        """Shop and pack states (where leaving the shop or skipping a pack is a move)."""
        return legal[:, LEAVE_SHOP] | legal[:, PACK_SKIP]

    def mask(self, legal: np.ndarray) -> np.ndarray:
        out = legal & ~self.blocked
        out[self.free >= FREE_STREAK] &= ~np.isin(np.arange(N_KINDS), FREE)
        if not self.skips:
            out[:, SKIP_BLIND] &= ~legal[:, SELECT_BLIND]
        if self.shop_cap:
            capped = (self.shop_moves >= self.shop_cap) & self.in_shop(legal)
            if capped.any():
                leave = np.zeros(N_KINDS, bool)
                leave[[LEAVE_SHOP, PACK_SKIP]] = True
                out[capped] &= leave
        none_left = ~(out & self.allowed).any(1)
        out[none_left] = legal[none_left]
        return out

    def after(self, kinds: np.ndarray, refused: np.ndarray, ended: np.ndarray) -> None:
        rows = np.flatnonzero(refused)
        self.blocked[rows, kinds[rows]] = True
        went = ~refused
        self.blocked[went] = False
        self.free = np.where(went, np.where(np.isin(kinds, FREE), self.free + 1, 0), self.free)
        self.free[ended] = 0
        self.blocked[ended] = False

    def count_shop(self, legal: np.ndarray, ended: np.ndarray) -> None:
        """After a move from a state with these legal kinds: count it if it was made in a shop or pack."""
        self.shop_moves = np.where(self.in_shop(legal) & ~ended, self.shop_moves + 1, 0)


def shaped(high, phi, position, progress, cleared, done, won, gamma_blind: float):
    """One step's reward under the potential (lab #38): the win, plus g * Phi(after) - Phi(before), where Phi is
    W_BLIND x the high-water mark of position plus W_SCORE x the round's progress, kept at the run's end, and g is
    `gamma_blind` on a transition that ends a blind or the run, 1 on every other. Returns (reward, g, the high-water
    mark after, Phi after); the caller zeroes the last two for runs that ended."""
    high_now = np.maximum(high, position)
    phi_now = W_BLIND * high_now + W_SCORE * progress
    g = np.where(cleared | done, gamma_blind, 1.0)
    return won * WIN_REWARD + g * phi_now - phi, g, high_now, phi_now


class Spread:
    """Each value head's running spread (the root mean square of its returns, an average that forgets slowly), so
    each head learns targets of about unit size however small the returns are (S, lab #38: at the start the value
    loss read 0.0000). When a spread changes, the head's last layer is rescaled so its outputs in returns stay as
    they were (PopArt's step, without a mean)."""

    def __init__(self, n: int = N_VALUES, decay: float = 0.99, floor: tuple = (0.05, 0.005),
                 state: dict | None = None):
        # Each head's floor near its reward's own scale (B, lab #38): with the win head at a tiny floor until the
        # first win, a won run's normalised target was about 300 and its loss took the policy's step.
        self.decay, self.floors = decay, tuple(floor)
        st = state if isinstance(state, dict) else {}
        self.ms = list(st.get("ms", [0.0] * n))
        self.batches = int(st.get("batches", 0))

    def sigma(self) -> list[float]:
        # The average corrected for its start at zero (as Adam's), so the first batches are read at their own size.
        corr = 1.0 - self.decay ** self.batches if self.batches else 1.0
        return [max(math.sqrt(m / corr), self.floors[h]) if self.batches else 1.0 for h, m in enumerate(self.ms)]

    def update(self, torch, model, returns) -> None:
        """Fold this batch's returns (T, N, N_VALUES) in, and keep the heads' outputs in returns unchanged."""
        old = self.sigma()
        for h in range(len(self.ms)):
            self.ms[h] = self.decay * self.ms[h] + (1 - self.decay) * float((returns[..., h] ** 2).mean())
        self.batches += 1
        new = self.sigma()
        last = model.value_head[-1]
        with torch.no_grad():
            for h in range(len(new)):
                last.weight[h] *= old[h] / new[h]
                last.bias[h] *= old[h] / new[h]

    def state(self) -> dict:
        return {"ms": list(self.ms), "batches": self.batches}


def next_beta(beta: float, prev: dict | None, ev: dict) -> tuple[float, str]:
    """The depth head's weight after an evaluation (B's rule, lab #38): 1 while the win rate is under 20%, falling to
    0 at 100%; it only steps down, and a step is undone when the win rate fell from the previous evaluation by
    more than 2 paired standard errors (same seeds, run by run)."""
    win = ev["win_rate"]
    target = 1.0 if win < 0.2 else max(0.0, (1.0 - win) / 0.8)
    # An undone step is held until the win rate passes its level at that step by 2 SE (B: no oscillation).
    hold = (prev or {}).get("hold")
    if hold and win < hold["win"] + 2 * hold["se"]:
        ev["hold"] = hold
        return beta, ""
    if prev and prev.get("run_stages") and ev.get("run_stages"):
        a, b = prev["run_stages"], ev["run_stages"]
        n = min(len(a), len(b))
        lost = sum(1 for i in range(n) if a[i] == "Y" and b[i] != "Y")
        gained = sum(1 for i in range(n) if b[i] == "Y" and a[i] != "Y")
        d = (gained - lost) / max(n, 1)
        se = math.sqrt(max((gained + lost) / max(n, 1) - d * d, 0.0) / max(n, 1))
        if se > 0 and d < -2 * se and prev.get("beta_before") is not None:
            ev["hold"] = {"win": prev["win_rate"], "se": se}
            return prev["beta_before"], f"win rate fell {d:+.2%} (paired SE {se:.2%}): beta back to {prev['beta_before']}"
    return min(beta, target), ""


def new_stats() -> dict:
    return {"runs": 0, "wins": 0, "moves": 0, "ante_hist": [0] * 9, "elapsed": 0.0, "rate": 0.0}


def new_totals() -> dict:
    return {"runs": 0, "wins": 0, "ante_hist": [0] * 9, "moves": 0, "refused": 0, "rerolls": 0, "skips": 0,
            "packs": 0, "vouchers": 0, "blinds_beaten": 0, "money": 0, "jokers": 0, "lost_to": {}, "bought": {},
            "held": {}, "held_won": {}, "hands": [0] * 12}


END_KEYS = ("won", "ante", "lost_to", "money", "held", "bought", "moves", "refused", "rerolls", "skips", "packs",
            "vouchers", "blinds_beaten", "hands")


def add_end(t: dict, end: dict) -> None:
    """Count one finished run (the engine's VecEnv.take_endings / cut_run) into totals."""
    won = bool(end["won"])
    t["runs"] += 1
    t["wins"] += int(won)
    t["ante_hist"][min(max(int(end["ante"]), 1), 9) - 1] += 1
    for k in ("moves", "refused", "rerolls", "skips", "packs", "vouchers", "blinds_beaten", "money"):
        t[k] += int(end[k])
    t["jokers"] += len(end["bought"])
    if not won:
        t["lost_to"][end["lost_to"]] = t["lost_to"].get(end["lost_to"], 0) + 1
    for j in end["bought"]:
        t["bought"][j] = t["bought"].get(j, 0) + 1
    for j in end["held"]:
        t["held"][j] = t["held"].get(j, 0) + 1
        if won:
            t["held_won"][j] = t["held_won"].get(j, 0) + 1
    t["hands"] = [a + int(b) for a, b in zip(t["hands"], end["hands"], strict=False)]


def breakdown(t: dict, hand_names) -> dict:
    """What make watch shows of a job's runs, with examples/baseline.rs's field names."""
    runs, wins = t["runs"], t["wins"]
    per = max(runs, 1)
    lo, hi = wilson(wins, runs)

    def table(counts: dict, of: int) -> list:
        rows = sorted(counts.items(), key=lambda kv: (-kv[1], kv[0]))[:20]
        return [[k, n, round(n / max(of, 1), 6)] for k, n in rows]

    return {"runs": runs, "wins": wins, "win_rate": wins / per, "win_ci": [lo, hi],
            "mean_ante": sum((i + 1) * n for i, n in enumerate(t["ante_hist"])) / per, "ante_hist": list(t["ante_hist"]),
            "blinds_beaten_per_run": t["blinds_beaten"] / per,
            "lost_to": [[k, n] for k, n in sorted(t["lost_to"].items(), key=lambda kv: (-kv[1], kv[0]))[:12]],
            "hands_played": [[hand_names[i], n] for i, n in enumerate(t["hands"]) if n],
            "jokers_bought": table(t["bought"], runs), "jokers_held_at_end": table(t["held"], runs),
            "jokers_in_wins": table(t["held_won"], wins),
            "per_run": {"moves": t["moves"] / per, "jokers_bought": t["jokers"] / per, "rerolls": t["rerolls"] / per,
                        "skips": t["skips"] / per, "packs": t["packs"] / per, "vouchers": t["vouchers"] / per,
                        "money_at_end": t["money"] / per, "refused": t["refused"] / per}}


class RunBook:
    """The job's numbers from every finished run, for make watch: over all runs, over the latest NOW_RUNS ("now"),
    and block by block (BLOCK_FIELDS), so the viewer can show the network changing as it trains. Its state goes in
    the PPO checkpoint, so a resumed run carries on counting."""

    def __init__(self, hand_names, state: dict | None = None, block_runs: int = BLOCK_RUNS,
                 now_runs: int = NOW_RUNS) -> None:
        s = state or {}
        self.hand_names = list(hand_names)
        self.total = s.get("total") or new_totals()
        self.recent = collections.deque(s.get("recent") or [], maxlen=now_runs)
        self.block_runs = int(s.get("block_runs") or block_runs)
        self.blocks = [list(b) for b in s.get("blocks") or []]
        self.block = s.get("block") or dict.fromkeys(BLOCK_FIELDS[2:], 0)

    def add(self, end: dict, seconds: float) -> None:
        end = {k: end[k] for k in END_KEYS}
        add_end(self.total, end)
        self.recent.append(end)
        b = self.block
        for k, x in (("runs", 1), ("wins", int(end["won"])), ("ante", int(end["ante"])), ("moves", end["moves"]),
                     ("refused", end["refused"]), ("jokers", len(end["bought"])), ("packs", end["packs"]),
                     ("rerolls", end["rerolls"]), ("money", end["money"]), ("cut", int(end["lost_to"] == CUT))):
            b[k] += int(x)
        if b["runs"] >= self.block_runs:
            self.blocks.append([self.total["runs"], round(seconds, 1), *(b[k] for k in BLOCK_FIELDS[2:])])
            self.block = dict.fromkeys(BLOCK_FIELDS[2:], 0)
            if len(self.blocks) > MAX_BLOCKS:
                self.blocks = merge_blocks(self.blocks)
                self.block_runs *= 2

    def fields(self) -> dict:
        """progress.json's breakdown of the runs (what ends them, jokers, hands, an average run), the same for the
        latest runs under "now", and the blocks."""
        now = new_totals()
        for e in self.recent:
            add_end(now, e)
        out = breakdown(self.total, self.hand_names)
        for k in ("runs", "wins", "win_rate", "win_ci", "mean_ante", "ante_hist"):
            del out[k]  # the job's own counters say these
        return {**out, "now": {**breakdown(now, self.hand_names), "label": f"last {len(self.recent):,} runs"},
                "block_fields": list(BLOCK_FIELDS), "block_runs": self.block_runs, "blocks": self.blocks}

    def state(self) -> dict:
        return {"total": self.total, "recent": list(self.recent), "block_runs": self.block_runs,
                "blocks": self.blocks, "block": self.block}


def merge_blocks(blocks: list) -> list:
    """Pairs of blocks as one (ending where the second ends); an odd last block stays as it is."""
    out = []
    for i in range(0, len(blocks) - 1, 2):
        a, b = blocks[i], blocks[i + 1]
        out.append([b[0], b[1], *(x + y for x, y in zip(a[2:], b[2:], strict=True))])
    if len(blocks) % 2:
        out.append(blocks[-1])
    return out


# The evaluator (lab #38, B's rules): EVAL_RUNS fresh seeds from EVAL_BASE, played with the likeliest moves at every
# --eval-every updates; the exam's seeds (jobs.EXAM_SEED) report a gate once and choose nothing. Training never plays
# either set: its engine is told to pass over them (`VecEnv.reject_seeds`).
EVAL_BASE = 9001
EVAL_RUNS = 10_000
EXAM_BASE = 5003


def eval_seeds(e, base: int, runs: int) -> list[str]:
    return [e.run_seed(base, i) for i in range(runs)]


def survival(ante_hist: list[int]) -> list[float]:
    """The share of runs still alive after each ante 1 to 8 (a win is alive after all of them)."""
    n = max(sum(ante_hist), 1)
    return [round(sum(ante_hist[k:]) / n, 4) for k in range(1, 9)]


def play_seeds(torch, pp, device, e, seeds: list[str], n_envs: int, threads: int, skips: bool, shop_cap: int,
               greedy: bool = True, watch=None, block_runs: int = 0,
               engine_rounds: bool = False) -> tuple[dict, list[str], "RunBook"]:
    """Play each of `seeds` once with the network (likeliest moves unless `greedy` is False), under training's own
    rules; returns (stats, each seed's ending stage letter, the RunBook). With `engine_rounds` the teacher plays the
    rounds and the network everything else (S, lab #38: the exchange evaluation, how much of the wall is play)."""
    from balatro_meta.env import VecEnv

    runs = len(seeds)
    n = min(n_envs, runs)
    env = VecEnv(n, seed=0, threads=threads, auto=True, skips=skips)
    queue = list(range(runs))
    slot_run = [-1] * n
    for i in range(n):
        r = queue.pop(0)
        env.set_seed(i, seeds[r])
        slot_run[i] = r
    stages = ["?"] * runs
    moves = [0] * n
    blinds = [0] * n
    stats = new_stats()
    book = RunBook(e.HAND_NAMES, block_runs=block_runs or BLOCK_RUNS)
    guard = Guard(n, skips=skips, shop_cap=shop_cap)
    t0 = time.time()
    finished = 0
    while finished < runs:
        x = torch.from_numpy(env.observe()).to(device)
        legal_np = guard.mask(env.legal_kinds())
        legal = torch.from_numpy(legal_np).to(device)
        with torch.no_grad(), autocast(torch, device):
            kind, slot, to, cards, _, _ = pp.act(x, legal, greedy=greedy)
        kind_np = kind.cpu().numpy()
        args = to_moves(kind_np, slot.cpu().numpy(), to.cpu().numpy(), cards.cpu().numpy())
        if engine_rounds:
            ek, ea, in_round = env.round_moves()
            kind_np = np.where(in_round, ek, kind_np)
            args = np.where(in_round, ea, args).astype(np.uint64)
        _, done, won, refused, ante = env.step(kind_np, args)
        cleared = env.cleared
        endings = dict(env.take_endings()) if done.any() else {}
        ended = done.copy()
        for i in range(n):
            if slot_run[i] < 0:
                continue
            moves[i] += 1
            blinds[i] += int(cleared[i])
            cut = moves[i] >= MAX_MOVES
            if done[i] or cut:
                book.add(endings[i] if done[i] else env.cut_run(i), time.time() - t0)
                r = slot_run[i]
                stage = 24 if won[i] else ended_stage(blinds[i], int(ante[i]))
                stages[r] = chr(65 + stage)
                stats["runs"] += 1
                stats["wins"] += int(won[i])
                stats["moves"] += moves[i]
                stats["ante_hist"][8 if won[i] else min(max(int(ante[i]), 1), 8) - 1] += 1
                finished += 1
                moves[i], blinds[i] = 0, 0
                ended[i] = True
                if queue:
                    r2 = queue.pop(0)
                    env.set_seed(i, seeds[r2])
                    slot_run[i] = r2
                else:
                    slot_run[i] = -1
        guard.after(kind_np, refused, ended)
        guard.count_shop(legal_np, ended)
        stats["elapsed"] = time.time() - t0
        stats["rate"] = stats["runs"] / max(stats["elapsed"], 1e-9)
        if watch is not None:
            watch.report(stats, extra=book.fields)
    return stats, stages, book


def eval_policy(a) -> dict:
    """Play seeds with the network's likeliest moves: the baseline job's (--seed), the evaluator's (--evaluator,
    EVAL_RUNS from EVAL_BASE) or the exam's (--exam, from EXAM_BASE: the gate, reported once)."""
    torch = need_torch()
    fast_float32(torch, a)
    e = engine()
    device = device_of(torch, a.cpu)
    model, _ = load_model(torch, Path(a.model), device)
    model.eval()
    pp = Parts(torch, model)
    base = EXAM_BASE if a.exam else EVAL_BASE if a.evaluator else a.seed
    seeds = eval_seeds(e, base, a.runs)
    watch = Watch(Path(os.environ["BM_WATCH_DIR"]) if os.environ.get("BM_WATCH_DIR") else None,
                  "network", a.runs, a.threads)
    stats, stages, book = play_seeds(torch, pp, device, e, seeds, a.envs, a.threads, getattr(a, "skips", False),
                                     getattr(a, "shop_cap", SHOP_CAP), greedy=not a.sample, watch=watch,
                                     block_runs=a.block_runs, engine_rounds=a.engine_rounds)
    runs, wins = stats["runs"], stats["wins"]
    lo, hi = wilson(wins, runs)
    mean = sum((i + 1) * k for i, k in enumerate(stats["ante_hist"])) / max(runs, 1)
    fields = book.fields()
    report = progress_body(stats, "network", a.runs, a.threads, True, [], {
        "model": str(a.model), "seed_base": base, "greedy": not a.sample, "run_stages": "".join(stages),
        "refused_moves": book.total["refused"], "exam": bool(a.exam), "engine_rounds": bool(a.engine_rounds),
        **fields})
    report.update({"win_ci": [lo, hi], "mean_ante": mean, "survival": survival(stats["ante_hist"])})
    watch.report(stats, done=True, extra=fields)
    if a.out:
        write_json(Path(a.out), report)
    print(f"{runs:,} runs: {wins:,} won ({wins / max(runs, 1):.2%}, 95% range {lo:.2%} to {hi:.2%}), "
          f"mean final ante {mean:.2f}; alive after each ante {survival(stats['ante_hist'])}", flush=True)
    return report


def explain(a) -> dict:
    """Each value head's explained variance for a checkpoint (S, lab #38), on fresh play under training's rules:
    `--envs` runs played `--steps` moves by the network (sampled, as training plays), each head's lambda-return
    worked out as training does, and 1 - Var(return - value) / Var(return)."""
    torch = need_torch()
    fast_float32(torch, a)
    from balatro_meta.env import VecEnv

    e = engine()
    device = device_of(torch, a.cpu)
    model, ck = load_model(torch, Path(a.model), device)
    model.eval()
    pp = Parts(torch, model)
    st = ck.get("state", {})
    known = bool(st.get("spread"))
    sig = np.array(Spread(state=st["spread"]).sigma() if known else [1.0] * N_VALUES)
    N, T = a.envs, a.steps
    env = VecEnv(N, seed=a.seed * 7_919 + 13, threads=a.threads, auto=True, skips=False)
    env.reject_seeds(eval_seeds(e, EVAL_BASE, EVAL_RUNS) + eval_seeds(e, EXAM_BASE, EVAL_RUNS))
    guard = Guard(N, skips=False, shop_cap=SHOP_CAP)
    high, phi = np.zeros(N, np.int64), np.zeros(N)
    vals = np.zeros((T + 1, N, N_VALUES))
    rews = np.zeros((T, N, N_VALUES))
    gam = np.ones((T, N))
    dones = np.zeros((T, N))
    for t in range(T + 1):
        x = torch.from_numpy(env.observe().copy()).to(device)
        legal_np = guard.mask(env.legal_kinds())
        with torch.no_grad(), autocast(torch, device):
            kind, slot, to, cards, _, v = pp.act(x, torch.from_numpy(legal_np).to(device))
        vals[t] = v.float().cpu().numpy() * sig
        if t == T:
            break
        k = kind.cpu().numpy()
        _, done, won, refused, _ = env.step(k, to_moves(k, slot.cpu().numpy(), to.cpu().numpy(), cards.cpu().numpy()))
        r, g, high_now, phi_now = shaped(high, phi, env.position, env.progress, env.cleared, done, won, GAMMA_BLIND)
        rews[t, :, WIN] = won * WIN_REWARD
        rews[t, :, DEPTH] = r - won * WIN_REWARD - refused * REFUSED_COST
        gam[t], dones[t] = g, done
        high, phi = np.where(done, 0, high_now), np.where(done, 0.0, phi_now)
        guard.after(k, refused, done)
        guard.count_shop(legal_np, done)
    adv = np.zeros((N, N_VALUES))
    rets = np.zeros((T, N, N_VALUES))
    for t in range(T - 1, -1, -1):
        nonterm = (1.0 - dones[t])[:, None]
        delta = rews[t] + gam[t][:, None] * vals[t + 1] * nonterm - vals[t]
        adv = delta + gam[t][:, None] * a.lam * nonterm * adv
        rets[t] = adv + vals[t]
    out = {"model": str(a.model), "envs": N, "steps": T, "lam": a.lam, "spread_known": known}
    for h, name in ((WIN, "win"), (DEPTH, "depth")):
        R, V = rets[..., h].ravel(), vals[:T, :, h].ravel()
        var = R.var()
        # With the spread unknown (a checkpoint kept without it) the values' units are lost: the explained variance
        # at the best scale, the square of their correlation, is the most the head could explain.
        out[f"explained_{name}"] = float(1 - (R - V).var() / var) if var > 1e-12 and known else None
        out[f"explained_{name}_best_scale"] = (float(np.corrcoef(R, V)[0, 1] ** 2)
                                              if var > 1e-12 and V.var() > 1e-12 else None)
        out[f"return_sd_{name}"] = float(np.sqrt(var))
    print(json.dumps(out), flush=True)
    if a.out:
        write_json(Path(a.out), out)
    return out


def gae(xp, r, vals, last_v, done, gamma, lam: float):
    """Each head's GAE (numpy or torch, `xp`): r and vals (T, N, H), last_v (N, H) the value where the runs are now,
    done and gamma (T, N) each transition's end and its own discount (--gamma-blind on one that ends a blind, else
    1). A_t = delta_t + g_t * lam * (1 - d_t) * A_{t+1}, delta_t = r_t + g_t * (1 - d_t) * V_{t+1} - V_t; so A + V
    is the lambda-return (`scripts/preflight.py` checks it against the definition)."""
    T = r.shape[0]
    adv = xp.zeros_like(r)
    g = xp.zeros_like(last_v)
    for t in range(T - 1, -1, -1):
        nv = last_v if t == T - 1 else vals[t + 1]
        nonterm = (1.0 - done[t])[:, None]
        gm = gamma[t][:, None]
        delta = r[t] + gm * nv * nonterm - vals[t]
        g = delta + gm * lam * nonterm * g
        adv[t] = g
    return adv


def ppo_grads(torch, pp, batch, mb, micro: int, device, clip: float, value_coef: float, ent_coef: float,
              warm: bool, shop_ent_mult: float = 1.0) -> dict:
    """Adds PPO's loss gradients on minibatch `mb` (indices into `batch`) to the network's, worked out `micro`
    positions at a time so a big minibatch fits in the GPU's memory: the same gradients as the whole minibatch at
    once. Returns the minibatch's mean policy, value and entropy losses, clipped share and approximate KL divergence
    from the policy that played (before this step). (Past the GPU's memory, Windows spills into the PC's RAM and
    training crawls rather than failing.)"""
    fx, fl, fk, fs, ft, fc, flp, fadv, fret, *rest = batch
    fsh = rest[0] if rest else None
    out = dict.fromkeys(("policy", "value", "entropy", "clip", "kl", "entropy_shop", "entropy_round"), 0.0)
    n = len(mb)
    for c in range(0, n, micro):
        sub = mb[c:c + micro]
        w = len(sub) / n
        with autocast(torch, device):
            lp, ent, v = pp.evaluate(fx[sub].float(), fl[sub], fk[sub], fs[sub], ft[sub], fc[sub])
        log_ratio = (lp - flp[sub]).clamp(-20.0, 20.0)
        ratio = torch.exp(log_ratio)
        pg = -torch.min(ratio * fadv[sub], ratio.clamp(1 - clip, 1 + clip) * fadv[sub]).mean()
        vl = ((v - fret[sub]) ** 2).sum(-1).mean()
        el = ent.mean()
        if fsh is not None:
            # Entropy by phase (S, lab #38): shops and packs, and rounds.
            sh = fsh[sub]
            if sh.any():
                out["entropy_shop"] += float(ent[sh].mean().detach()) * w
            if (~sh).any():
                out["entropy_round"] += float(ent[~sh].mean().detach()) * w
        # More entropy on shop and pack decisions than in rounds (lab #38: --shop-ent-mult).
        ew = ent if fsh is None else ent * torch.where(fsh[sub], shop_ent_mult, 1.0)
        loss = value_coef * vl if warm else pg + value_coef * vl - ent_coef * ew.mean()
        (loss * w).backward()
        out["policy"] += float(pg.detach()) * w
        out["value"] += float(vl.detach()) * w
        out["entropy"] += float(el.detach()) * w
        out["clip"] += float(((ratio - 1).abs() > clip).float().mean().detach()) * w
        # (ratio - 1) - log(ratio): never below zero, so a big move towards the moves it already made shows too.
        out["kl"] += float(((ratio - 1) - log_ratio).mean().detach()) * w
    return out


def judge_progress(best: dict | None, win: float, ante: float, runs: int, judge_runs: int, drop: float) -> str:
    """What the latest `runs` runs say about the training: "best" (a new best win rate: keep this network),
    "collapse" (play has fallen far below the best: final ante `drop` lower, or under half its win rate), or "".
    Nothing is judged on fewer than `judge_runs` runs."""
    if runs < judge_runs:
        return ""
    if best is None or win > best["win"]:
        return "best"
    if ante < best["ante"] - drop or (best["win"] >= 0.05 and win < 0.5 * best["win"]):
        return "collapse"
    return ""


def compile_encode(torch, model, device, x, micro: int) -> str:
    """Has PyTorch compile the network's main part (`encode`: the objects into tokens and the transformer), which
    fuses its many small steps into fewer GPU kernels. It is tried first on real positions, as training uses it
    (learning on `micro` positions; playing on all of `x` without gradients); if compiling or those first calls fail
    (a missing compiler or header on this machine, say), `encode` stays plain PyTorch. Says which."""
    eager = model.encode
    was = model.training
    try:
        fn = torch.compile(eager, dynamic=False)
        model.train()
        with autocast(torch, device):
            h, _, cls = fn(x[:micro])
            (h.float().mean() + cls.float().mean()).backward()
        model.zero_grad(set_to_none=True)
        model.eval()
        with torch.no_grad(), autocast(torch, device):
            fn(x)
    except Exception as exc:  # noqa: BLE001 - any failure: train as before
        model.encode = eager
        model.zero_grad(set_to_none=True)
        model.train(was)
        first = (str(exc).strip().splitlines() or [""])[0][:200]
        return f"not compiled ({type(exc).__name__}: {first}); training without it"
    model.encode = fn
    model.train(was)
    return "compiled"


def value_only(model) -> None:
    """Keep only the value head's gradients: the optimizer then leaves everything else as it is. (The value head
    shares the network's layers with the move choices; training it through them would change how it plays.)"""
    keep = {id(q) for q in model.value_head.parameters()}
    for q in model.parameters():
        if id(q) not in keep:
            q.grad = None


def ppo(a) -> dict:
    torch = need_torch()
    fast_float32(torch, a)
    from balatro_meta.env import VecEnv

    e = engine()
    device = device_of(torch, a.cpu)
    folder = Path(a.dir)
    latest = folder / "latest.pt"
    best_path = folder / "best.pt"
    rng = random.Random(a.seed)
    if latest.exists():
        model, ck = load_model(torch, latest, device)
        state = ck["state"]
        print(f"Resuming from {latest}: {state['updates']} updates, {state['stats']['runs']:,} runs.", flush=True)
        if state.get("micro") and state["micro"] < a.micro:
            a.micro = state["micro"]  # a micro-batch halved for memory stays halved
    else:
        if a.init:
            model, _ = load_model(torch, Path(a.init), device)
            print(f"Starting from {a.init}.", flush=True)
        else:
            model = build_model(torch, a.width, a.layers, a.heads, a.width * 3).to(device)
            print("Starting from scratch.", flush=True)
        ck = None
        state = {"updates": 0, "steps": 0, "seconds": 0.0, "stats": new_stats(), "history": [], "recent": [],
                 "book": None}
    opt = torch.optim.Adam(model.parameters(), lr=a.lr, eps=1e-5)
    if ck and "opt" in ck:
        opt.load_state_dict(ck["opt"])
    pp = Parts(torch, model, value_detached=True)
    N, T = a.envs, a.steps
    # The learner's engine (lab #38): the engine makes the moves with no decision in them and refuses a null move.
    env = VecEnv(N, seed=a.seed * 1_000_003 + state["updates"], threads=a.threads, auto=True, skips=a.skips)
    # Training never plays a seed the evaluator or the exam plays (B, lab #38): the engine passes over them.
    held_out = eval_seeds(e, EVAL_BASE, max(a.eval_runs, EVAL_RUNS)) + eval_seeds(e, EXAM_BASE, EVAL_RUNS)
    env.reject_seeds(held_out)
    held_set = set(held_out)
    if a.compile:
        t0 = time.time()
        how = compile_encode(torch, model, device, torch.from_numpy(env.observe().copy()).to(device),
                             min(a.micro, a.minibatch))
        print(f"The network: {how} ({time.time() - t0:.0f} s).", flush=True)
    lay = layout()
    obs_len = e.OBS_LEN
    buf_x = torch.zeros(T, N, obs_len, dtype=torch.float16, device=device)
    buf_legal = torch.zeros(T, N, N_KINDS, dtype=torch.bool, device=device)
    buf_kind = torch.zeros(T, N, dtype=torch.long, device=device)
    buf_slot = torch.zeros_like(buf_kind)
    buf_to = torch.zeros_like(buf_kind)
    buf_cards = torch.zeros(T, N, MAX_HAND, dtype=torch.bool, device=device)
    buf_logp = torch.zeros(T, N, device=device)
    buf_v = torch.zeros(T, N, N_VALUES, device=device)  # the heads' values, each in units of its spread
    buf_r = torch.zeros(T, N, N_VALUES, device=device)  # the win reward and the depth reward
    buf_done = torch.zeros(T, N, device=device)
    buf_gamma = torch.ones(T, N, device=device)  # each transition's discount: --gamma-blind if it ends a blind, else 1
    buf_shop = torch.zeros(T, N, dtype=torch.bool, device=device)  # shop and pack states (more entropy there)
    del lay
    moves = np.zeros(N, np.int64)
    blinds = np.zeros(N, np.int64)
    rec: dict[int, tuple[str, list]] = {}
    watch = Watch(Path(os.environ["BM_WATCH_DIR"]) if os.environ.get("BM_WATCH_DIR") else None,
                  "network (PPO)", None, a.threads)
    watch.history = state.get("history", [])
    stats = state["stats"]
    recent: list = state.get("recent", [])
    book = RunBook(e.HAND_NAMES, state.get("book"), block_runs=a.block_runs)
    stop = {"now": False}

    def on_signal(*_):
        stop["now"] = True

    signal.signal(signal.SIGINT, on_signal)
    signal.signal(signal.SIGTERM, on_signal)
    t_start = time.time()
    seconds0 = state["seconds"]
    last_save = time.time()
    trunc = 0

    def finish(i: int, won: bool, ante: int) -> None:
        stage = 24 if won else ended_stage(int(blinds[i]), ante)
        final = 9 if won else min(max(ante, 1), 8)
        stats["runs"] += 1
        stats["wins"] += int(won)
        stats["moves"] += int(moves[i])
        stats["ante_hist"][final - 1] += 1
        recent.append((int(won), final))
        if len(recent) > 10000:
            del recent[:-5000]
        if i in rec:
            seed, mv = rec.pop(i)
            watch.sample(seed, mv, won, final, stage)

    def save_all() -> None:
        state["seconds"] = seconds0 + time.time() - t_start
        state["stats"], state["recent"], state["history"] = stats, recent[-5000:], watch.history
        state["book"] = book.state()
        save(torch, latest, model, {"opt": opt.state_dict(), "state": state})

    guard = Guard(N, skips=a.skips, shop_cap=a.shop_cap)
    spread = Spread(state=state.get("spread"))
    value_params = list(model.value_head.parameters())
    value_ids = {id(q) for q in value_params}
    policy_params = [q for q in model.parameters() if id(q) not in value_ids]
    state.setdefault("beta", 1.0)
    last_eval = None
    # The potential: each run's high-water mark of position and its last potential (lab #38).
    high = np.zeros(N, np.int64)
    phi = np.zeros(N, np.float64)
    last_losses: dict = {}

    def live() -> dict:
        """progress.json's numbers: the runs broken down (RunBook), and the latest 2,000 runs as the headline."""
        last = recent[-2000:]
        won = sum(w for w, _ in last)
        mean = sum(an for _, an in last) / max(len(last), 1)
        lo, hi = wilson(won, len(last))
        return {**book.fields(), "updates": state["updates"], "steps": state["steps"], "recent_runs": len(last),
                "recent_win_rate": won / max(len(last), 1), "recent_mean_ante": mean, "losses": last_losses,
                "win_rate": won / max(len(last), 1), "win_ci": [lo, hi], "mean_ante": mean}

    x_np = env.observe()
    first = True
    while not stop["now"]:
        if seconds0 + time.time() - t_start >= a.hours * 3600:
            break
        if a.max_updates is not None and state["updates"] >= a.max_updates:
            break
        model.eval()
        t_play = time.time()
        for t in range(T):
            if stop["now"]:
                break
            x = torch.from_numpy(x_np).to(device)
            legal_np = guard.mask(env.legal_kinds())
            legal = torch.from_numpy(legal_np).to(device)
            with torch.no_grad(), autocast(torch, device):
                kind, slot, to, cards, logp, v = pp.act(x, legal)
            kind_np = kind.cpu().numpy()
            slot_np, to_np, cards_np = slot.cpu().numpy(), to.cpu().numpy(), cards.cpu().numpy()
            args = to_moves(kind_np, slot_np, to_np, cards_np)
            _, done, won, refused, ante = env.step(kind_np, args)
            cleared = env.cleared
            moves += 1
            blinds += cleared
            # The potential after the move (before a finished run is replaced): the high-water mark of position,
            # and the round's progress. Discounted by blinds: --gamma-blind on a transition that ends a blind or the
            # run, 1 on every other.
            r, g, high_now, phi_now = shaped(high, phi, env.position, env.progress, cleared, done, won, a.gamma_blind)
            r_win = won * WIN_REWARD
            r_depth = r - r_win - refused * REFUSED_COST - np.isin(kind_np, FREE) * FREE_COST
            cut = (moves >= MAX_MOVES) & ~done
            if cut.any():
                # A run cut short isn't lost: its last transition is bootstrapped with the value of where it stood
                # (B, lab #38: a cut valued at 0 is the time-limit-as-terminal error).
                after = torch.from_numpy(env.observe().copy()).to(device)
                idx = np.flatnonzero(cut)
                with torch.no_grad(), autocast(torch, device):
                    _, _, cls_cut = model.encode(after[idx])
                    v_cut = model.value(cls_cut).float().cpu().numpy() * np.array(spread.sigma())
                r_win = r_win.astype(np.float64)
                r_win[idx] += g[idx] * v_cut[:, WIN]
                r_depth[idx] += g[idx] * v_cut[:, DEPTH]
            ended = done | cut
            high = np.where(ended, 0, high_now)
            phi = np.where(ended, 0.0, phi_now)
            seconds = seconds0 + time.time() - t_start
            if done.any():
                for _, end in env.take_endings():
                    book.add(end, seconds)
            cut_logs = {}
            for i in np.flatnonzero(cut):
                book.add(env.cut_run(int(i)), seconds)
                if int(i) in rec:
                    cut_logs[int(i)] = env.run_moves(int(i))
                seed_next = e.run_seed(a.seed + 7919, trunc)
                while seed_next in held_set:
                    trunc += 1
                    seed_next = e.run_seed(a.seed + 7919, trunc)
                env.set_seed(int(i), seed_next)
                trunc += 1
            guard.after(kind_np, refused, ended)
            guard.count_shop(legal_np, ended)
            # A saved run keeps every move it made, the engine's own among them (Env's log), so it replays.
            for i in list(rec):
                if ended[i]:
                    log = cut_logs.get(i) if cut[i] else env.ended_moves(i)
                    rec[i] = (rec[i][0], [[int(k), int(v)] for k, v in log])
            for i in np.flatnonzero(ended):
                finish(int(i), bool(won[i]), int(ante[i]))
                moves[i], blinds[i] = 0, 0
            if ended.any() and watch.dir and len(rec) < 4:
                fresh = [int(i) for i in np.flatnonzero(ended) if rng.random() < 0.02]
                if fresh:
                    seeds = env.seeds()
                    for i in fresh:
                        rec[i] = (seeds[i], [])
            buf_x[t] = x.half()
            buf_legal[t] = legal
            buf_kind[t], buf_slot[t], buf_to[t], buf_cards[t] = kind, slot, to, cards
            buf_logp[t], buf_v[t] = logp.float(), v.float()
            buf_r[t] = torch.from_numpy(np.stack([r_win, r_depth], -1).astype(np.float32)).to(device)
            buf_done[t] = torch.from_numpy(ended.astype(np.float32)).to(device)
            buf_gamma[t] = torch.from_numpy(g.astype(np.float32)).to(device)
            buf_shop[t] = torch.from_numpy(Guard.in_shop(legal_np)).to(device)
            x_np = env.observe()
            if ended.any():
                stats["elapsed"] = seconds
                stats["rate"] = stats["runs"] / max(seconds, 1e-9)
                watch.report(stats, extra=live)  # at most once a second
        if stop["now"]:
            break  # a part-played batch isn't learned from; the runs start afresh on the next start
        # Advantages (GAE) from the value of where the runs are now.
        with torch.no_grad(), autocast(torch, device):
            _, _, last_cls = model.encode(torch.from_numpy(x_np).to(device))
            last_v = model.value(last_cls).float()
        # Each head's GAE in returns (its values times its spread), discounted by each transition's own discount.
        sig = torch.tensor(spread.sigma(), device=device)
        vals = buf_v * sig
        last_v = last_v * sig
        adv2 = gae(torch, buf_r, vals, last_v, buf_done, buf_gamma, a.lam)
        ret2 = adv2 + vals
        # Each head's explained variance on this batch (S, lab #38): 1 - Var(return - value) / Var(return).
        explained = [round(float(1.0 - (ret2[..., h] - vals[..., h]).var() / ret2[..., h].var()), 4)
                     if float(ret2[..., h].var()) > 1e-12 else float("nan") for h in range(N_VALUES)]  # nan: no spread
        # The play's advantage: the win head's plus beta times the depth head's (normalised below).
        adv = adv2[..., WIN] + state["beta"] * adv2[..., DEPTH]
        # The spreads take in this batch's returns; the value targets are in the new units.
        spread.update(torch, model, ret2)
        state["spread"] = spread.state()
        ret = ret2 / torch.tensor(spread.sigma(), device=device)
        # The update.
        t_learn = time.time()
        model.train()
        flat = lambda z: z.reshape(T * N, *z.shape[2:])
        fx, fl, fk, fs, ft, fc = (flat(buf_x), flat(buf_legal), flat(buf_kind), flat(buf_slot), flat(buf_to),
                                  flat(buf_cards))
        fsh = flat(buf_shop)
        flp, fadv, fret = flat(buf_logp), flat(adv), ret.reshape(T * N, N_VALUES)
        fadv = (fadv - fadv.mean()) / (fadv.std() + 1e-8)
        losses = {"policy": 0.0, "value": 0.0, "entropy": 0.0, "clip": 0.0, "kl": 0.0, "entropy_shop": 0.0,
                  "entropy_round": 0.0}
        nb = 0
        # A small steady learning rate (lab #38; antezero's later segments ran flat), halved at each fall back to the
        # best network; --lr-fall restores the old fall to a tenth over the run.
        done_frac = min(1.0, (seconds0 + time.time() - t_start) / max(a.hours * 3600, 1.0))
        for group in opt.param_groups:
            group["lr"] = a.lr * state.get("lr_scale", 1.0) * (1.0 - 0.9 * done_frac if a.lr_fall else 1.0)
        # The first updates from a copied network train its value head only, on the network's own runs: how it
        # plays stays exactly the copy until the value can judge its moves.
        warm = state["updates"] < state.setdefault("warm_until", a.value_warmup if a.init else 0)
        batch = (fx, fl, fk, fs, ft, fc, flp, fadv, fret, fsh)
        braked = False
        brake_at = None
        breaches = 0
        first_kl = None
        # The brake (S's default, lab #38, after the smoke run threw most batches away on 171 of 300 updates): a
        # breach lowers the rate for the rest of the pass, x --brake-factor each time down to --brake-floor of the
        # update's rate, and the batch is always learned from in full. `--brake stop` is the old brake (the rest of the
        # batch thrown away). rate_share: the mean of the rate used over the pass's steps, as a share of the update's.
        base_lr = [g["lr"] for g in opt.param_groups]
        scale = 1.0
        scale_sum = 0.0
        for _ in range(a.epochs):
            perm = torch.randperm(T * N, device=device)
            for k in range(0, T * N, a.minibatch):
                opt.zero_grad(set_to_none=True)
                # Out of GPU memory (lab #38: the candidates' tokens add to every step, and a run started unattended
                # must not crash on it): the micro-batch halves and the minibatch is tried again. The micro-batch
                # only splits the gradient's sum, so the step is the same.
                while True:
                    try:
                        got = ppo_grads(torch, pp, batch, perm[k:k + a.minibatch], a.micro, device, a.clip,
                                        a.value_coef, a.ent_coef, warm, a.shop_ent_mult)
                        break
                    except torch.cuda.OutOfMemoryError:
                        if a.micro <= 128:
                            raise
                        opt.zero_grad(set_to_none=True)
                        torch.cuda.empty_cache()
                        a.micro //= 2
                        state["micro"] = a.micro
                        print(f"Out of GPU memory: the micro-batch is now {a.micro}.", flush=True)
                if first_kl is None:
                    # Before any step the play hasn't moved, so this is about 0 unless acting and learning disagree
                    # (B, lab #38: precision, a stale input).
                    first_kl = got["kl"]
                if not warm and a.target_kl and got["kl"] > 1.5 * a.target_kl:
                    breaches += 1
                    if not braked:
                        braked, brake_at = True, nb
                    if a.brake == "stop":
                        # The old brake: this update has already moved the play as far as one update should.
                        opt.zero_grad(set_to_none=True)
                        break
                    scale = max(scale * a.brake_factor, a.brake_floor)
                    for g, lr0 in zip(opt.param_groups, base_lr, strict=True):
                        g["lr"] = lr0 * scale
                scale_sum += scale
                if warm:
                    value_only(model)
                # The value head's gradient is clipped apart from the policy's (B, lab #38), so a spike in the value
                # loss (the first wins) can't shrink the policy's step.
                torch.nn.utils.clip_grad_norm_(policy_params, a.max_grad)
                torch.nn.utils.clip_grad_norm_(value_params, a.max_grad)
                opt.step()
                for key in losses:
                    losses[key] += got[key]
                nb += 1
            if braked and a.brake == "stop":
                break
        for g, lr0 in zip(opt.param_groups, base_lr, strict=True):
            g["lr"] = lr0
        state["updates"] += 1
        state["braked"] = state.get("braked", 0) + int(braked)
        # B's rate rule (lab #38), applied once at update --lr-rule-at: no brake and under 3% clipped over the
        # updates so far doubles the rate; the brake on more than a quarter of them halves it.
        state.setdefault("early", []).append([int(braked), losses["clip"] / max(nb, 1)])
        if a.lr_rule_at and state["updates"] == a.lr_rule_at and "lr_rule" not in state:
            early = state["early"][-a.lr_rule_at:]
            brakes = sum(b for b, _ in early)
            clip_mean = sum(c for _, c in early) / max(len(early), 1)
            factor = 2.0 if brakes == 0 and clip_mean < 0.03 else 0.5 if brakes > len(early) / 4 else 1.0
            state["lr_scale"] = state.get("lr_scale", 1.0) * factor
            state["lr_rule"] = {"brakes": brakes, "clip": round(clip_mean, 4), "factor": factor}
            print(f"B's rate rule at update {state['updates']}: {brakes} brakes, clipped {clip_mean:.3f} on average: "
                  f"rate x{factor}", flush=True)
        if first:
            # How long an update takes, and how close it came to the GPU's memory.
            first = False
            mem = ""
            if device.type == "cuda":
                peak = torch.cuda.max_memory_allocated(device) / 2 ** 30
                total = torch.cuda.get_device_properties(device).total_memory / 2 ** 30
                mem = f"; GPU memory peak {peak:.1f} of {total:.1f} GB"
                if peak > 0.9 * total:
                    mem += " (close to full: if updates crawl, rerun with a smaller --micro)"
            print(f"update {state['updates']} took {time.time() - t_play:.0f} s: playing {t_learn - t_play:.0f} s, "
                  f"learning {time.time() - t_learn:.0f} s{mem}", flush=True)
        state["steps"] += T * N
        el_s = seconds0 + time.time() - t_start
        stats["elapsed"] = el_s
        stats["rate"] = stats["runs"] / max(el_s, 1e-9)
        last = recent[-2000:]
        win_recent = sum(w for w, _ in last) / max(len(last), 1)
        ante_recent = sum(an for _, an in last) / max(len(last), 1)
        if not watch.history or el_s - watch.history[-1][0] >= 60:
            watch.history.append([round(el_s, 1), stats["runs"], round(ante_recent, 4), round(win_recent, 5)])
        last_losses = {k: round(v / max(nb, 1), 5) for k, v in losses.items()}
        last_losses["first_kl"] = round(first_kl or 0.0, 6)
        last_losses["explained_win"], last_losses["explained_depth"] = explained
        # Where the update's time went (lab #38: CPU or GPU decides what hardware helps).
        play_s, learn_s = t_learn - t_play, time.time() - t_learn
        last_losses["play_s"], last_losses["learn_s"] = round(play_s, 1), round(learn_s, 1)
        state.setdefault("timing", []).append([round(play_s, 2), round(learn_s, 2)])
        state["timing"] = state["timing"][-50:]
        # The training's log on disk, every update (S, lab #38: the smoke run left none): the brake and the share of
        # the batch learned from, the time, the losses and each head's explained variance.
        per_pass = -(-(T * N) // a.minibatch)
        row = {"update": state["updates"], "steps": state["steps"], "seconds": round(seconds0 + time.time() - t_start, 1),
               "braked": braked, "brake_at": brake_at, "breaches": breaches, "minibatches": nb,
               "learned_share": round(nb / max(per_pass * a.epochs, 1), 4),
               "rate_share": round(scale_sum / max(nb, 1), 4),
               "micro": a.micro,
               "lr": round(opt.param_groups[0]["lr"], 8), "beta": state["beta"],
               "spreads": [round(x, 5) for x in spread.sigma()], **last_losses,
               "recent_win_rate": round(sum(w for w, _ in recent[-2000:]) / max(len(recent[-2000:]), 1), 5),
               "recent_mean_ante": round(sum(an for _, an in recent[-2000:]) / max(len(recent[-2000:]), 1), 4)}
        folder.mkdir(parents=True, exist_ok=True)
        with open(folder / "train_log.jsonl", "a") as fh:
            fh.write(json.dumps(row) + "\n")
        if a.report:
            log_copy = Path(a.report).with_name(Path(a.report).stem + "-log.jsonl")
            log_copy.parent.mkdir(parents=True, exist_ok=True)
            with open(log_copy, "a") as fh:
                fh.write(json.dumps(row) + "\n")
        if a.eval_every and state["updates"] % a.eval_every == 0:
            # The evaluator: the evaluator's seeds with the likeliest moves, the checkpoint kept beside its numbers.
            model.eval()
            t_ev = time.time()
            ev_stats, ev_stages, _ = play_seeds(torch, pp, device, e, eval_seeds(e, EVAL_BASE, a.eval_runs),
                                                a.eval_envs, a.threads, a.skips, a.shop_cap)
            lo_e, hi_e = wilson(ev_stats["wins"], ev_stats["runs"])
            ev = {"updates": state["updates"], "steps": state["steps"], "runs": ev_stats["runs"],
                  "win_rate": ev_stats["wins"] / max(ev_stats["runs"], 1), "win_ci": [lo_e, hi_e],
                  "mean_ante": sum((i + 1) * k for i, k in enumerate(ev_stats["ante_hist"])) / max(ev_stats["runs"], 1),
                  "survival": survival(ev_stats["ante_hist"]), "run_stages": "".join(ev_stages),
                  "seconds": round(time.time() - t_ev, 1), "explained": explained}
            # The depth head's weight steps down as the win rate rises (B's rule; undone after a 2 SE fall).
            ev["beta_before"] = state["beta"]
            state["beta"], why = next_beta(state["beta"], last_eval, ev)
            ev["beta"] = state["beta"]
            if why:
                print(why, flush=True)
            last_eval = ev
            folder.mkdir(parents=True, exist_ok=True)
            with open(folder / "evals.jsonl", "a") as fh:
                fh.write(json.dumps(ev) + "\n")
            save(torch, folder / f"eval-{state['updates']:06d}.pt", model,
                 {"eval": ev, "state": {"spread": spread.state(), "beta": state["beta"]}})
            print(f"eval at update {state['updates']} ({state['steps']:,} moves): won {ev['win_rate']:.2%} "
                  f"({lo_e:.2%} to {hi_e:.2%}) of {ev['runs']:,}, mean final ante {ev['mean_ante']:.2f}, alive after "
                  f"each ante {ev['survival']} ({ev['seconds']:.0f} s)", flush=True)
            state.setdefault("evals", []).append({k: v for k, v in ev.items() if k != "run_stages"})
        # Keep the best network so far; if play collapses, go back to it and learn more slowly.
        judged = recent[-a.judge_runs:]
        verdict = judge_progress(state.get("best"), sum(w for w, _ in judged) / max(len(judged), 1),
                                 sum(an for _, an in judged) / max(len(judged), 1), len(judged), a.judge_runs,
                                 a.rollback_drop)
        if verdict == "best":
            state["best"] = {"win": sum(w for w, _ in judged) / len(judged),
                             "ante": sum(an for _, an in judged) / len(judged), "update": state["updates"],
                             "runs": stats["runs"]}
            save(torch, best_path, model, {"best": state["best"]})
        elif verdict == "collapse":
            b = state["best"]
            state["rollbacks"] = state.get("rollbacks", 0) + 1
            state["lr_scale"] = state.get("lr_scale", 1.0) * 0.5
            print(f"Play collapsed at update {state['updates']}: won {win_recent:.2%} of its last {len(last):,} runs, "
                  f"final ante {ante_recent:.2f} (best {b['win']:.2%} and {b['ante']:.2f} at update {b['update']}). "
                  f"Back to that network; learning rate now x{state['lr_scale']:g}.", flush=True)
            model.load_state_dict(torch.load(best_path, map_location=device, weights_only=False)["model"])
            opt = torch.optim.Adam(model.parameters(), lr=a.lr * state["lr_scale"], eps=1e-5)
            recent.clear()
            if state["rollbacks"] > a.max_rollbacks:
                print(f"It collapsed {state['rollbacks']} times; stopping here, with its best network kept.", flush=True)
                watch.report(stats, extra=live)
                break
        watch.report(stats, extra=live)
        if state["updates"] % a.print_every == 0:
            b = state.get("best")
            print(f"update {state['updates']}{' (value only)' if warm else ''}: {stats['runs']:,} runs, last "
                  f"{len(last):,}: won {win_recent:.2%}, mean final ante {ante_recent:.2f}; policy "
                  f"{losses['policy'] / max(nb, 1):.4f}, value {losses['value'] / max(nb, 1):.4f}, entropy "
                  f"{losses['entropy'] / max(nb, 1):.3f}, kl {losses['kl'] / max(nb, 1):.4f} (first {first_kl or 0.0:.5f}), "
                  f"clipped {losses['clip'] / max(nb, 1):.3f}, entropy shop/round "
                  f"{losses['entropy_shop'] / max(nb, 1):.3f}/{losses['entropy_round'] / max(nb, 1):.3f}, "
                  f"beta {state['beta']:.2f}, spreads {', '.join(f'{x:.4f}' for x in spread.sigma())}, explained "
                  f"{explained[WIN]:.2f}/{explained[DEPTH]:.2f}, "
                  f"play/learn {sum(t[0] for t in state['timing'][-a.print_every:]) / max(len(state['timing'][-a.print_every:]), 1):.0f}/"
                  f"{sum(t[1] for t in state['timing'][-a.print_every:]) / max(len(state['timing'][-a.print_every:]), 1):.0f} s, "
                  f"{T * N / max(sum(sum(t) for t in state['timing'][-a.print_every:]) / max(len(state['timing'][-a.print_every:]), 1), 1e-9):,.0f} moves/s, braked "
                  f"{state['braked']} so far" + (f"; best {b['win']:.2%} (update {b['update']})" if b else ""),
                  flush=True)
        if time.time() - last_save >= a.save_minutes * 60:
            save_all()
            last_save = time.time()
    save_all()
    if not best_path.exists():
        save(torch, best_path, model, {"best": None})
    last = recent[-2000:]
    win_recent = sum(w for w, _ in last) / max(len(last), 1)
    lo, hi = wilson(sum(w for w, _ in last), len(last))
    fields = book.fields()
    watch.report(stats, done=True, extra={**fields, "updates": state["updates"], "steps": state["steps"],
                                          "recent_runs": len(last), "win_rate": win_recent, "win_ci": [lo, hi],
                                          "mean_ante": sum(an for _, an in last) / max(len(last), 1)})
    report = {"kind": "ppo", "updates": state["updates"], "steps": state["steps"], "runs": stats["runs"],
              "seconds": round(seconds0 + time.time() - t_start, 1), "recent_runs": len(last),
              "recent_win_rate": sum(w for w, _ in last) / max(len(last), 1),
              "recent_mean_ante": sum(an for _, an in last) / max(len(last), 1), "history": watch.history,
              "device": str(device), "envs": N, "steps_per_update": T, "model": str(latest), "best_model": str(best_path),
              "best": state.get("best"), "rollbacks": state.get("rollbacks", 0), "evals": state.get("evals", []),
              "beta": state.get("beta"), "lr_rule": state.get("lr_rule"), "braked": state.get("braked", 0), **fields}
    if a.report:
        write_json(Path(a.report), report)
    if stop["now"] and seconds0 + time.time() - t_start < a.hours * 3600:
        print("Stopped; rerun to carry on from the last checkpoint.", flush=True)
        sys.exit(130)
    print(f"Done: {state['updates']} updates, {stats['runs']:,} runs.", flush=True)
    return report


# ---------------------------------------------------------------------------------------------- command line
def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    net = argparse.ArgumentParser(add_help=False)
    net.add_argument("--width", type=int, default=192)
    net.add_argument("--layers", type=int, default=4)
    net.add_argument("--heads", type=int, default=4)
    net.add_argument("--cpu", action="store_true")
    net.add_argument("--seed", type=int, default=1)
    net.add_argument("--threads", type=int, default=0)
    net.add_argument("--bf16", action="store_true", help="bf16 autocast on the GPU (Stage 0's default is float32/TF32)")
    i = sub.add_parser("imitate", parents=[net])
    i.add_argument("--data", required=True)
    i.add_argument("--out", required=True)
    i.add_argument("--report")
    i.add_argument("--epochs", type=int, default=4)
    i.add_argument("--batch", type=int, default=1024)
    i.add_argument("--lr", type=float, default=5e-4)
    i.add_argument("--value-coef", type=float, default=0.5)
    i.add_argument("--gamma", type=float, default=0.999)
    i.add_argument("--max-runs", type=int)
    i.add_argument("--judge-rows", type=int, default=100_000)
    i.add_argument("--report-every", type=int, default=500)
    p = sub.add_parser("ppo", parents=[net])
    p.add_argument("--dir", required=True, help="checkpoints (latest.pt); a rerun resumes from it")
    p.add_argument("--init", help="start from this network (imitation) when there is no checkpoint")
    p.add_argument("--report")
    p.add_argument("--hours", type=float, default=8.0)
    p.add_argument("--max-updates", type=int, help="stop after this many updates in all (tests)")
    # Stage 0's recipe (lab #38): big batches used once (antezero: 4,096 games x 128 moves, one pass).
    p.add_argument("--envs", type=int, default=4096)
    p.add_argument("--steps", type=int, default=128)
    p.add_argument("--epochs", type=int, default=1)
    p.add_argument("--minibatch", type=int, default=4096)
    p.add_argument("--micro", type=int, default=2048,
                   help="positions worked out at once within a minibatch (GPU memory; the result is the same)")
    p.add_argument("--no-compile", dest="compile", action="store_false",
                   help="don't have PyTorch compile the network (it falls back by itself if compiling fails)")
    p.add_argument("--lr", type=float, default=5e-5)
    p.add_argument("--lr-fall", action="store_true", help="the old schedule: the rate falls to a tenth over the run")
    p.add_argument("--gamma-blind", type=float, default=0.98,
                   help="the discount of a transition that ends a blind or the run (every other: 1)")
    # 0.99 a move (lab #38, A's default pending B): about 100 moves, an ante, where 0.95 reached a single blind.
    p.add_argument("--lam", type=float, default=0.99)
    p.add_argument("--skips", action="store_true", help="offer blind skips (off from Stage 0, lab #38)")
    p.add_argument("--shop-cap", type=int, default=SHOP_CAP, help="moves in one shop before only leaving is offered")
    p.add_argument("--shop-ent-mult", type=float, default=3.0, help="entropy's weight on shop and pack decisions")
    p.add_argument("--lr-rule-at", type=int, default=50, help="the update B's rate rule is applied at (0: never)")
    p.add_argument("--eval-every", type=int, default=100,
                   help="updates between evaluations on the evaluator's seeds (0: never)")
    p.add_argument("--eval-runs", type=int, default=EVAL_RUNS)
    p.add_argument("--eval-envs", type=int, default=2048)
    p.add_argument("--clip", type=float, default=0.2)
    p.add_argument("--value-coef", type=float, default=0.5)
    p.add_argument("--ent-coef", type=float, default=0.003)
    p.add_argument("--max-grad", type=float, default=0.5)
    p.add_argument("--save-minutes", type=float, default=10.0)
    p.add_argument("--value-warmup", type=int, default=20, help="updates that train only the value, from --init")
    p.add_argument("--judge-runs", type=int, default=2000, help="runs the best network and collapses are judged on")
    p.add_argument("--rollback-drop", type=float, default=1.0,
                   help="a fall in final ante below the best that counts as a collapse (back to the best network)")
    p.add_argument("--max-rollbacks", type=int, default=6, help="collapses after which the run stops")
    p.add_argument("--target-kl", type=float, default=0.02,
                   help="the brake's line: a minibatch whose play has moved 1.5 x this far from the play it learns from "
                        "is a breach (0: no brake)")
    p.add_argument("--brake", choices=("rate", "stop"), default="rate",
                   help="rate (S's default, lab #38): a breach lowers the rate for the rest of the pass and the batch "
                        "is learned from in full; stop: the old brake, the rest of the batch thrown away")
    p.add_argument("--brake-factor", type=float, default=0.5, help="the rate's fall at each breach (--brake rate)")
    p.add_argument("--brake-floor", type=float, default=0.125,
                   help="the lowest share of the update's rate the brake goes down to")
    p.add_argument("--print-every", type=int, default=5)
    p.add_argument("--block-runs", type=int, default=BLOCK_RUNS, help="runs per point of the training's progress")
    v = sub.add_parser("eval", parents=[net])
    v.add_argument("--model", required=True)
    v.add_argument("--runs", type=int, default=EVAL_RUNS)
    v.add_argument("--evaluator", action="store_true", help="the evaluator's seeds (EVAL_BASE), as training's evals")
    v.add_argument("--exam", action="store_true", help="the exam's seeds (EXAM_BASE): a gate, reported once")
    v.add_argument("--engine-rounds", action="store_true",
                   help="the teacher plays the rounds, the network the rest (the exchange evaluation)")
    v.add_argument("--envs", type=int, default=1024)
    v.add_argument("--out")
    v.add_argument("--sample", action="store_true", help="sample moves instead of taking the likeliest")
    v.add_argument("--block-runs", type=int, default=BLOCK_RUNS, help="runs per block in the live numbers")
    x = sub.add_parser("explain", parents=[net])
    x.add_argument("--model", required=True)
    x.add_argument("--envs", type=int, default=256)
    x.add_argument("--steps", type=int, default=512)
    x.add_argument("--lam", type=float, default=0.99)
    x.add_argument("--out")
    a = ap.parse_args()
    if a.cmd == "explain":
        explain(a)
        return
    if a.cmd == "imitate":
        imitate(a)
    elif a.cmd == "ppo":
        ppo(a)
    else:
        eval_policy(a)


if __name__ == "__main__":
    main()
