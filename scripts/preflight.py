"""Preflight (lab #38, before renting a GPU): many small checks that the learner's code is correct and runs smoothly,
each with PASS, FAIL or SKIP and what it saw.

    python scripts/preflight.py [--quick] [--cpu] [--only NAME,NAME] [--out results/preflight.json]

`--quick` shrinks every check to run on a CPU in a few minutes (the cloud's check); without it the checks run at the
training's own size on the GPU (the PC's or a rented one: about 30 to 60 minutes). The checks:

  determinism   the same seeds and the same network play the same games, move for move, on 1 and 4 threads
  act-eval      the probabilities the network acts with equal those it learns from (inputs stored as training stores
                them), float32; bf16 reported beside it on a GPU
  checkpoint    a saved network loads back with identical outputs
  evaluator     the same seeds give the same endings twice, and with a different number of parallel runs
  returns       the advantages plus values equal the lambda-returns worked out from their definition
  candidates    plays and discards through candidates are never refused, and both candidates and free picks occur
  oom-guard     a learning step that runs out of GPU memory halves the micro-batch and the run goes on
  resume        3 updates straight against 2 plus 1 resumed: the same first two, what the checkpoint carries, and two
                resumes from one checkpoint identical
  speed         full-size updates: moves a second (at least --min-speed), playing and learning seconds, GPU memory
  known         a kept checkpoint replayed on the evaluator's first seeds, twice on the GPU and once on the CPU
  learns        updates from scratch climb past ante 1

Exit 0 when nothing failed. The training-run checks go through train_policy.py's own command line in a child
process, so they test what a real run runs.
"""
from __future__ import annotations

import argparse
import json
import math
import os
import subprocess
import sys
import tempfile
import time
from pathlib import Path

import numpy as np

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent
sys.path.insert(0, str(HERE))
sys.path.insert(0, str(ROOT / "python"))

import train_policy as tp  # noqa: E402

PY = sys.executable


class Skip(Exception):
    pass


def torch_mod():
    return tp.need_torch()


def device(a):
    torch = torch_mod()
    return tp.device_of(torch, a.cpu)


def small_model(torch, dev, seed=0):
    torch.manual_seed(seed)
    m = tp.build_model(torch, 64, 2, 2, 192).to(dev)
    m.eval()
    return m


def env_rollout(torch, pp, dev, n, steps, seed, threads, greedy, gen_seed=0):
    """Plays `steps` moves on `n` runs from seeds (seed, i); returns the observations, the moves and the outcomes."""
    from balatro_meta.env import VecEnv

    e = tp.engine()
    env = VecEnv(n, seed=0, threads=threads, auto=True, skips=False)
    seeds = tp.eval_seeds(e, seed, n)
    for i, s in enumerate(seeds):
        env.set_seed(i, s)
    guard = tp.Guard(n, skips=False, shop_cap=tp.SHOP_CAP)
    torch.manual_seed(gen_seed)
    out = []
    for _ in range(steps):
        x_np = env.observe().copy()
        legal_np = guard.mask(env.legal_kinds())
        x = torch.from_numpy(x_np).to(dev)
        legal = torch.from_numpy(legal_np).to(dev)
        with torch.no_grad():
            kind, slot, to, cards, logp, _ = pp.act(x, legal, greedy=greedy)
        kind_np = kind.cpu().numpy()
        args = tp.to_moves(kind_np, slot.cpu().numpy(), to.cpu().numpy(), cards.cpu().numpy())
        _, done, won, refused, ante = env.step(kind_np, args)
        if done.any():
            env.take_endings()
        out.append({"x": x_np, "legal": legal_np, "kind": kind_np, "slot": slot.cpu().numpy(),
                    "to": to.cpu().numpy(), "cards": cards.cpu().numpy(), "logp": logp.cpu().numpy(),
                    "args": args, "done": done.copy(), "won": won.copy(), "refused": refused.copy(),
                    "ante": np.asarray(ante).copy()})
        guard.after(kind_np, refused, done.copy())
        guard.count_shop(legal_np, done.copy())
    return out


# ------------------------------------------------------------------------------------------------------ checks
def check_determinism(a):
    torch = torch_mod()
    dev = torch.device("cpu")  # move-for-move identity is a CPU property; the GPU's kernels may round differently
    pp = tp.Parts(torch, small_model(torch, dev))
    n, steps = (16, 60) if a.quick else (64, 300)
    one = env_rollout(torch, pp, dev, n, steps, seed=11, threads=1, greedy=True)
    two = env_rollout(torch, pp, dev, n, steps, seed=11, threads=1, greedy=True)
    four = env_rollout(torch, pp, dev, n, steps, seed=11, threads=4, greedy=True)
    for name, other in (("again", two), ("4 threads", four)):
        for t, (p, q) in enumerate(zip(one, other)):
            if not (np.array_equal(p["x"], q["x"]) and np.array_equal(p["args"], q["args"])
                    and np.array_equal(p["done"], q["done"])):
                return False, f"{name}: first difference at move {t}"
    ends = int(sum(s["done"].sum() for s in one))
    return True, f"{n} runs x {steps} moves identical again and on 4 threads ({ends} runs ended)"


def check_act_eval(a):
    torch = torch_mod()
    dev = device(a)
    pp = tp.Parts(torch, small_model(torch, dev, seed=3))
    n, steps = (16, 40) if a.quick else (128, 120)
    roll = env_rollout(torch, pp, dev, n, steps, seed=21, threads=a.threads, greedy=False, gen_seed=5)
    worst = 0.0
    kl = []
    for s in roll:
        x = torch.from_numpy(s["x"]).to(dev).half().float()  # stored as training stores it (float16)
        legal = torch.from_numpy(s["legal"]).to(dev)
        with torch.no_grad():
            lp, _, _ = pp.evaluate(x, legal, *(torch.from_numpy(s[k]).to(dev) for k in ("kind", "slot", "to")),
                                   torch.from_numpy(s["cards"]).to(dev))
        d = (lp.cpu().numpy() - s["logp"])
        worst = max(worst, float(np.abs(d).max()))
        kl.append(float(np.mean(s["logp"] - lp.cpu().numpy())))
    detail = f"float32: largest log-probability gap {worst:.2e}, mean {np.mean(kl):+.2e} over {n * steps:,} moves"
    ok = worst < 2e-2 and abs(np.mean(kl)) < 1e-3
    if dev.type == "cuda":
        tp.BF16["on"] = True
        try:
            gaps = []
            for s in roll[:20]:
                x = torch.from_numpy(s["x"]).to(dev).half().float()
                legal = torch.from_numpy(s["legal"]).to(dev)
                with torch.no_grad(), tp.autocast(torch, dev):
                    lp, _, _ = pp.evaluate(x, legal,
                                           *(torch.from_numpy(s[k]).to(dev) for k in ("kind", "slot", "to")),
                                           torch.from_numpy(s["cards"]).to(dev))
                gaps.append(float(np.abs(lp.float().cpu().numpy() - s["logp"]).max()))
            detail += f"; bf16 (reported only): largest gap {max(gaps):.2e}"
        finally:
            tp.BF16["on"] = False
    return ok, detail


def check_checkpoint(a):
    torch = torch_mod()
    dev = device(a)
    m = small_model(torch, dev, seed=7)
    from balatro_meta.env import VecEnv

    env = VecEnv(8, seed=3, threads=1, auto=True, skips=False)
    x = torch.from_numpy(env.observe().copy()).to(dev)
    with tempfile.TemporaryDirectory() as d:
        p = Path(d) / "m.pt"
        tp.save(torch, p, m, {"state": {}})
        m2, _ = tp.load_model(torch, p, dev)
        m2.eval()
    with torch.no_grad():
        h1, _, c1 = m.encode(x)
        h2, _, c2 = m2.encode(x)
    same = torch.equal(c1, c2) and torch.equal(h1, h2)
    return same, "identical outputs after save and load" if same else "outputs differ after a reload"


def check_evaluator(a):
    torch = torch_mod()
    dev = torch.device("cpu")
    pp = tp.Parts(torch, small_model(torch, dev, seed=9))
    e = tp.engine()
    runs = 48 if a.quick else 400
    seeds = tp.eval_seeds(e, tp.EVAL_BASE, runs)
    s1 = tp.play_seeds(torch, pp, dev, e, seeds, 16 if a.quick else 128, 1, False, tp.SHOP_CAP)[1]
    s2 = tp.play_seeds(torch, pp, dev, e, seeds, 16 if a.quick else 128, 1, False, tp.SHOP_CAP)[1]
    s3 = tp.play_seeds(torch, pp, dev, e, seeds, 7 if a.quick else 50, 2, False, tp.SHOP_CAP)[1]
    if s1 != s2:
        return False, "the same seeds ended differently the second time"
    if s1 != s3:
        diff = sum(x != y for x, y in zip(s1, s3))
        return False, f"{diff} of {runs} seeds ended differently with another number of parallel runs"
    return True, f"{runs} seeds: same endings twice and with another number of parallel runs and threads"


def lambda_returns_reference(r, v, last_v, done, gamma, lam):
    """G_t = r_t + g_t * (1 - d_t) * ((1 - lam) * V_{t+1} + lam * G_{t+1}), with G_T's tail the value at the end."""
    T = r.shape[0]
    G = np.zeros_like(r)
    nxt_v, nxt_g = last_v, last_v
    for t in range(T - 1, -1, -1):
        nonterm = 1.0 - done[t]
        G[t] = r[t] + gamma[t] * nonterm * ((1 - lam) * nxt_v + lam * nxt_g)
        nxt_v, nxt_g = v[t], G[t]
    return G


def check_returns(a):
    rng = np.random.default_rng(4)
    T, N = 64, 32
    r = rng.normal(size=(T, N)) * (rng.random((T, N)) < 0.3)
    v = rng.normal(size=(T, N))
    last_v = rng.normal(size=N)
    done = (rng.random((T, N)) < 0.05).astype(float)
    gamma = np.where(rng.random((T, N)) < 0.1, 0.98, 1.0)
    lam = 0.99
    adv = tp.gae(np, r[..., None], v[..., None], last_v[:, None], done, gamma, lam)[..., 0]
    ref = lambda_returns_reference(r, v, last_v, done, gamma, lam)
    err = float(np.abs(adv + v - ref).max())
    return err < 1e-9, f"advantages + values against the lambda-returns' definition: largest gap {err:.1e}"


def check_candidates(a):
    torch = torch_mod()
    dev = torch.device("cpu")
    pp = tp.Parts(torch, small_model(torch, dev, seed=13))
    n, steps = (24, 80) if a.quick else (128, 400)
    roll = env_rollout(torch, pp, dev, n, steps, seed=31, threads=a.threads, greedy=False, gen_seed=8)
    c = {"play_cand": 0, "play_free": 0, "disc_cand": 0, "disc_free": 0, "cand_refused": 0, "refused": 0, "moves": 0}
    for s in roll:
        k, sl, ref = s["kind"], s["slot"], s["refused"]
        play, disc = k == tp.PLAY, k == tp.DISCARD
        pc = play & (sl >= 0) & (sl < tp.FREE_SLOT)
        dc = disc & (sl >= 0) & (sl < tp.D_FREE)
        c["play_cand"] += int(pc.sum())
        c["play_free"] += int((play & ~pc).sum())
        c["disc_cand"] += int(dc.sum())
        c["disc_free"] += int((disc & ~dc).sum())
        c["cand_refused"] += int((ref & (pc | dc)).sum())
        c["refused"] += int(ref.sum())
        c["moves"] += len(k)
    ok = c["cand_refused"] == 0 and c["play_cand"] > 0 and c["disc_cand"] > 0 and c["play_free"] > 0
    return ok, (f"plays {c['play_cand']:,} by candidate / {c['play_free']:,} free, discards {c['disc_cand']:,} / "
                f"{c['disc_free']:,}; candidate moves refused {c['cand_refused']}; all refused {c['refused']} of "
                f"{c['moves']:,}")


def tiny_ppo_args(folder, updates, extra=()):
    return ["ppo", "--cpu", "--dir", str(folder), "--envs", "16", "--steps", "32", "--minibatch", "256",
            "--micro", "256", "--width", "32", "--layers", "1", "--heads", "2", "--max-updates", str(updates),
            "--no-compile", "--print-every", "1", *extra]


def run_child(argv, env_extra=None, timeout=3600, code=None):
    env = {**os.environ, "PYTHONPATH": f"{ROOT / 'python'}{os.pathsep}{os.environ.get('PYTHONPATH', '')}"}
    env.update(env_extra or {})
    cmd = [PY, "-c", code, *argv] if code else [PY, str(HERE / "train_policy.py"), *argv]
    p = subprocess.run(cmd, cwd=ROOT, env=env, capture_output=True, text=True, timeout=timeout)
    return p.returncode, p.stdout + p.stderr


def read_log(folder):
    f = Path(folder) / "train_log.jsonl"
    return [json.loads(x) for x in f.read_text().splitlines()] if f.exists() else []


OOM_CHILD = """
import sys, torch
sys.path.insert(0, 'scripts')
import train_policy as tp
real = tp.ppo_grads
hit = {'n': 0}
def flaky(*a, **k):
    if hit['n'] == 0:
        hit['n'] += 1
        raise torch.cuda.OutOfMemoryError('preflight: a simulated out-of-memory')
    return real(*a, **k)
tp.ppo_grads = flaky
sys.argv = ['train_policy.py'] + sys.argv[1:]
tp.main()
"""


def check_oom_guard(a):
    with tempfile.TemporaryDirectory() as d:
        rc, out = run_child(tiny_ppo_args(Path(d) / "run", 1), code=OOM_CHILD, timeout=1800)
        log = read_log(Path(d) / "run")
    if rc != 0:
        return False, "the run stopped: " + out.strip().splitlines()[-1][:200]
    if not log or log[-1].get("micro") != 128 or "micro-batch is now 128" not in out:
        return False, f"no halving seen (micro {log[-1].get('micro') if log else None})"
    return True, "a simulated out-of-memory halved the micro-batch (256 to 128) and the update finished"


CARRIED = ("lr", "beta", "micro")  # what a checkpoint carries into the next update exactly
COMPARED = ("policy", "value", "entropy", "kl", "rate_share", "spreads", "steps", "beta", "lr", "micro")


def check_resume(a):
    """S's check (lab #38, rented machines get interrupted): three updates straight against two plus one resumed.
    The two runs' first two updates are identical (a run repeats for a seed); a resumed run starts its games afresh
    (a straight run carries them on), so its third update plays other games: what the checkpoint carries (rate,
    beta, micro-batch) must match the straight run's exactly, the spreads within 20%; and resuming twice from the
    same checkpoint must give the very same third update."""
    import shutil

    with tempfile.TemporaryDirectory() as d:
        straight, part, again = Path(d) / "straight", Path(d) / "part", Path(d) / "again"
        for folder, n in ((straight, 3), (part, 2)):
            rc, out = run_child(tiny_ppo_args(folder, n), timeout=1800)
            if rc != 0:
                return False, f"{folder.name} failed: " + out.strip().splitlines()[-1][:200]
        shutil.copytree(part, again)
        outs = []
        for folder in (part, again):
            rc, out = run_child(tiny_ppo_args(folder, 3), timeout=1800)
            if rc != 0 or "Resuming" not in out:
                return False, f"resuming {folder.name} failed: " + out.strip().splitlines()[-1][:200]
            outs.append(out)
        s3, p3, g3 = read_log(straight), read_log(part), read_log(again)
    pick = lambda r: {k: r.get(k) for k in COMPARED}  # noqa: E731
    if [pick(r) for r in s3[:2]] != [pick(r) for r in p3[:2]]:
        return False, "the first two updates differ between two runs of one seed"
    if pick(p3[2]) != pick(g3[2]):
        diff = [k for k in COMPARED if p3[2].get(k) != g3[2].get(k)]
        return False, f"two resumes from one checkpoint differ in {diff}"
    off = [k for k in CARRIED if p3[2].get(k) != s3[2].get(k)]
    spread_gap = max(abs(x - y) / max(abs(y), 1e-9) for x, y in zip(p3[2]["spreads"], s3[2]["spreads"]))
    ok = not off and spread_gap < 0.2
    return ok, (f"first two updates identical; two resumes identical; carried {'all equal' if not off else off} "
                f"(rate, beta, micro-batch); spreads within {spread_gap:.1%} of the straight run's")


def full_args(folder, updates):
    return ["ppo", "--dir", str(folder), "--max-updates", str(updates), "--print-every", "1",
            "--threads", str(a_threads())]


def a_threads():
    return ARGS.threads


def check_speed(a):
    torch = torch_mod()
    if a.quick or not torch.cuda.is_available() or a.cpu:
        raise Skip("needs the GPU at full size (run without --quick)")
    with tempfile.TemporaryDirectory() as d:
        rc, out = run_child(full_args(Path(d) / "run", 3), timeout=3600)
        log = read_log(Path(d) / "run")
    if rc != 0 or len(log) < 3:
        return False, "the run failed: " + out.strip().splitlines()[-1][:200]
    last = log[1:]  # update 1 compiles
    play = np.mean([r["play_s"] for r in last])
    learn = np.mean([r["learn_s"] for r in last])
    steps = log[-1]["steps"] - log[0]["steps"]
    secs = log[-1]["seconds"] - log[0]["seconds"]
    peak = next((ln for ln in out.splitlines() if "GPU memory peak" in ln), "")
    peak = peak[peak.find("GPU memory peak"):].split(";")[0] if peak else "GPU memory peak not printed"
    rate = steps / max(secs, 1e-9)
    return rate >= a.min_speed, (f"line {a.min_speed:,.0f}: " f"{rate:,.0f} moves a second; playing {play:.0f} s and learning {learn:.0f} s an update; "
                         f"{peak}; micro-batch {log[-1].get('micro')}")


def known_checkpoint(a):
    """--known, else the newest evaluation checkpoint under models/policy/ this code can load."""
    torch = torch_mod()
    if a.known:
        return Path(a.known)
    found = sorted((ROOT / "models" / "policy").glob("*/eval-*.pt"), key=lambda p: p.stat().st_mtime, reverse=True)
    for p in found:
        try:
            ck = torch.load(p, map_location="cpu", weights_only=False)
        except Exception:
            continue
        if ck.get("layout_version") == tp.LAYOUT_VERSION:
            return p
    return None


def check_known(a):
    """S's known number (lab #38): a kept checkpoint replayed on the evaluator's first seeds, twice on the GPU and
    once on the CPU, with the share of seeds that end differently: the natural rate before another machine (a
    rented one) is compared with this one."""
    torch = torch_mod()
    path = known_checkpoint(a)
    if path is None:
        raise Skip("no evaluation checkpoint of this input layout yet (--known PATH)")
    e = tp.engine()
    runs = 200 if a.quick else 2000
    seeds = tp.eval_seeds(e, tp.EVAL_BASE, runs)

    def play(dev):
        model, _ = tp.load_model(torch, path, dev)
        model.eval()
        stats, stages, _ = tp.play_seeds(torch, tp.Parts(torch, model), dev, e, seeds, min(runs, 1024), a.threads,
                                         False, tp.SHOP_CAP)
        return stages, stats["wins"] / max(stats["runs"], 1)

    cpu, cpu_win = play(torch.device("cpu"))
    detail = f"{path.relative_to(ROOT) if path.is_relative_to(ROOT) else path}, {runs} seeds: CPU won {cpu_win:.2%}"
    if torch.cuda.is_available() and not a.cpu:
        g1, w1 = play(torch.device("cuda"))
        g2, _ = play(torch.device("cuda"))
        rep_diff = sum(x != y for x, y in zip(g1, g2)) / runs
        dev_diff = sum(x != y for x, y in zip(g1, cpu)) / runs
        detail += (f"; GPU won {w1:.2%}; seeds ending differently: GPU against GPU {rep_diff:.2%}, GPU against CPU "
                   f"{dev_diff:.2%}")
    return True, detail


def check_learns(a):
    torch = torch_mod()
    with tempfile.TemporaryDirectory() as d:
        if a.quick or not torch.cuda.is_available() or a.cpu:
            rc, out = run_child(tiny_ppo_args(Path(d) / "run", 3), timeout=3600)
            log = read_log(Path(d) / "run")
            finite = all(math.isfinite(r["policy"]) and math.isfinite(r["value"]) for r in log)
            return rc == 0 and len(log) == 3 and finite, (
                f"CPU, tiny: 3 updates, losses finite {finite} (the climb is checked at full size)")
        rc, out = run_child(full_args(Path(d) / "run", 12), timeout=3 * 3600)
        log = read_log(Path(d) / "run")
    if rc != 0 or not log:
        return False, "the run failed: " + out.strip().splitlines()[-1][:200]
    antes = [round(r["recent_mean_ante"], 2) for r in log]
    return antes[-1] > 1.2, f"recent mean ante by update: {antes}"


CHECKS = [("determinism", check_determinism), ("act-eval", check_act_eval), ("checkpoint", check_checkpoint),
          ("evaluator", check_evaluator), ("returns", check_returns), ("candidates", check_candidates),
          ("oom-guard", check_oom_guard), ("resume", check_resume), ("speed", check_speed),
          ("known", check_known), ("learns", check_learns)]
ARGS = None


def main() -> int:
    global ARGS
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("--quick", action="store_true", help="small sizes, CPU friendly (minutes)")
    p.add_argument("--cpu", action="store_true")
    p.add_argument("--threads", type=int, default=0, help="engine threads (0: all)")
    p.add_argument("--only", help="comma-separated check names")
    p.add_argument("--min-speed", type=float, default=5000,
                   help="the speed check's line, moves a second (S: 5,000 on the PC; 15,000 on a rented machine)")
    p.add_argument("--known", help="the known-number checkpoint (default: the newest loadable evaluation checkpoint)")
    p.add_argument("--out", default="results/preflight.json")
    a = p.parse_args()
    ARGS = a
    only = set(a.only.split(",")) if a.only else None
    results = []
    t_all = time.time()
    for name, fn in CHECKS:
        if only and name not in only:
            continue
        t0 = time.time()
        try:
            ok, detail = fn(a)
            status = "PASS" if ok else "FAIL"
        except Skip as s:
            status, detail = "SKIP", str(s)
        except Exception as ex:  # a check that crashes is a failure, with what crashed
            status, detail = "FAIL", f"crashed: {type(ex).__name__}: {str(ex)[:300]}"
        secs = time.time() - t0
        results.append({"check": name, "status": status, "detail": detail, "seconds": round(secs, 1)})
        print(f"{status:4}  {name:12} {detail}  ({secs:.0f} s)", flush=True)
    fails = [r for r in results if r["status"] == "FAIL"]
    torch = torch_mod()
    report = {"quick": a.quick, "device": "cuda" if torch.cuda.is_available() and not a.cpu else "cpu",
              "gpu": torch.cuda.get_device_name(0) if torch.cuda.is_available() else None,
              "passed": sum(r["status"] == "PASS" for r in results), "failed": len(fails),
              "skipped": sum(r["status"] == "SKIP" for r in results), "seconds": round(time.time() - t_all, 1),
              "checks": results}
    tp.write_json(ROOT / a.out, report)
    print(f"\npreflight: {report['passed']} passed, {report['failed']} failed, {report['skipped']} skipped "
          f"({report['seconds']:.0f} s); written to {a.out}", flush=True)
    return 1 if fails else 0


if __name__ == "__main__":
    sys.exit(main())
