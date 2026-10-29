"""Many Balatro runs stepped at once, for training and baselines.

    env = VecEnv(4096, seed=1)
    obs = env.observe()                 # float32 (n, OBS_LEN): only what the screen shows
    kinds, args = env.random_moves()    # or the bot's moves
    reward, done, won, refused, ante = env.step(kinds, args)

A move is a kind (see MOVE_KINDS) and an argument: a bitmask of hand
positions for play and discard, a joker position to sell, and
`from | to << 8` to move a hand card or joker. Runs restart on a fresh seed
when they end (lost, or won by beating ante 8); `done` marks that step.
"""
from __future__ import annotations

import numpy as np

from balatro_meta import _engine

OBS_LEN: int = _engine.OBS_LEN
MOVE_KINDS = (
    "select_blind",
    "skip_blind",
    "cash_out",
    "leave_shop",
    "sort_rank",
    "sort_suit",
    "play",
    "discard",
    "move_card",
    "move_joker",
    "sell_joker",
)
assert len(MOVE_KINDS) == _engine.N_MOVE_KINDS


class VecEnv:
    def __init__(self, n: int, seed: int = 0, stake: int = 1, threads: int = 0) -> None:
        self.n = n
        self._env = _engine.VecEnv(n, seed, stake, threads)
        self._obs = np.zeros((n, OBS_LEN), np.float32)
        self._legal = np.zeros((n, len(MOVE_KINDS)), np.uint8)
        self._reward = np.zeros(n, np.float32)
        self._done = np.zeros(n, np.uint8)
        self._won = np.zeros(n, np.uint8)
        self._refused = np.zeros(n, np.uint8)
        self._ante = np.zeros(n, np.int32)
        self._hand = np.zeros(n, np.int32)

    def observe(self) -> np.ndarray:
        """The bot's input for every run (a view reused by the next call)."""
        self._env.obs_into(self._obs)
        return self._obs

    def legal_kinds(self) -> np.ndarray:
        """bool (n, len(MOVE_KINDS)): which kinds of move each run allows now."""
        self._env.legal_kinds_into(self._legal)
        return self._legal.astype(bool)

    def hand_len(self) -> np.ndarray:
        self._env.hand_len_into(self._hand)
        return self._hand.copy()

    def random_moves(self) -> tuple[np.ndarray, np.ndarray]:
        kinds = np.zeros(self.n, np.int64)
        args = np.zeros(self.n, np.uint64)
        self._env.random_moves_into(kinds, args)
        return kinds, args

    def step(self, kinds: np.ndarray, args: np.ndarray):
        kinds = np.ascontiguousarray(kinds, dtype=np.int64)
        args = np.ascontiguousarray(args, dtype=np.uint64)
        self._env.step(kinds, args, self._reward, self._done, self._won, self._refused, self._ante)
        return (
            self._reward.copy(),
            self._done.astype(bool),
            self._won.astype(bool),
            self._refused.astype(bool),
            self._ante.copy(),
        )

    def screen(self, i: int) -> dict:
        """What run `i` shows on screen (for viewers and debugging)."""
        return self._env.screen(i)
