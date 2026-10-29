"""The batched environment: shapes, legality, determinism and fair input."""
import numpy as np

from balatro_meta import MOVE_KINDS, OBS_LEN, VecEnv


def test_shapes_and_random_play():
    env = VecEnv(64, seed=3, threads=2)
    obs = env.observe()
    assert obs.shape == (64, OBS_LEN) and obs.dtype == np.float32
    assert np.isfinite(obs).all()
    legal = env.legal_kinds()
    assert legal.shape == (64, len(MOVE_KINDS))
    assert legal[:, MOVE_KINDS.index("select_blind")].all()  # every run starts at blind select
    ends = 0
    for _ in range(300):
        kinds, args = env.random_moves()
        assert legal[np.arange(64), kinds].all()
        _, done, _, refused, ante = env.step(kinds, args)
        assert not refused.any()
        assert (ante >= 1).all()
        ends += int(done.sum())
        legal = env.legal_kinds()
    assert ends > 64  # random play loses quickly and runs restart


def test_same_seed_same_runs():
    a, b = VecEnv(16, seed=9, threads=1), VecEnv(16, seed=9, threads=4)
    for _ in range(100):
        ka, aa = a.random_moves()
        kb, ab = b.random_moves()
        assert (ka == kb).all() and (aa == ab).all()
        a.step(ka, aa)
        b.step(kb, ab)
        assert (a.observe() == b.observe()).all()


def test_refused_moves_change_nothing():
    env = VecEnv(4, seed=1)
    before = env.observe().copy()
    kinds = np.full(4, MOVE_KINDS.index("cash_out"))  # not on the cash-out screen yet
    _, done, _, refused, _ = env.step(kinds, np.zeros(4, np.uint64))
    assert refused.all() and not done.any()
    assert (env.observe() == before).all()


def test_screen_dict():
    env = VecEnv(2, seed=5)
    env.step(np.zeros(2, np.int64), np.zeros(2, np.uint64))  # select the Small Blind
    s = env.screen(0)
    assert s["phase"] == "Round"
    assert len(s["hand"]) == 8
    assert s["blind"][1] == 300.0


def test_fast_input_matches_the_reference_path():
    env = VecEnv(256, seed=4, threads=3)
    ref = np.zeros((256, OBS_LEN), np.float32)
    for _ in range(20):
        env.step(*env.random_moves())
        env._env.obs_into_copy(ref)
        assert (env.observe() == ref).all()
