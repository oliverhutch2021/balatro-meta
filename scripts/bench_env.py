"""Speed of the Python environment: steps per second for a batch of runs,
with random legal moves and the bot's input built after every step.

    python scripts/bench_env.py [--n 4096] [--seconds 20] [--out results/bench/python.json]
"""
from __future__ import annotations

import argparse
import json
import os
import time
from pathlib import Path

from balatro_meta import VecEnv


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--n", type=int, default=4096)
    ap.add_argument("--seconds", type=float, default=20.0)
    ap.add_argument("--threads", type=int, default=0)
    ap.add_argument("--out", default="results/bench/python.json")
    a = ap.parse_args()
    env = VecEnv(a.n, seed=1, threads=a.threads)
    env.observe()
    steps = done = 0
    t0 = time.perf_counter()
    while time.perf_counter() - t0 < a.seconds:
        kinds, args = env.random_moves()
        _, d, _, refused, _ = env.step(kinds, args)
        env.observe()
        assert not refused.any()
        steps += a.n
        done += int(d.sum())
    secs = time.perf_counter() - t0
    report = {
        "load_avg_1min_at_end": round(os.getloadavg()[0], 1),
        "batch": a.n,
        "threads": a.threads or os.cpu_count(),
        "seconds": round(secs, 2),
        "moves_per_s": round(steps / secs),
        "runs_finished_per_s": round(done / secs),
        "includes": "random legal moves, the step, and the bot's input for every run after every step",
    }
    print(json.dumps(report, indent=1))
    out = Path(a.out)
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(json.dumps(report, indent=1) + "\n")


if __name__ == "__main__":
    main()
