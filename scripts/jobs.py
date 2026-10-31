"""Run the heavy jobs on the PC: resumable, low priority, and self-reporting.

    python scripts/jobs.py sync          # fast-forward main to the code Claude left in .sync/claude.bundle
    python scripts/jobs.py run smoke     # run (or resume) a job; `make smoke` calls this
    python scripts/jobs.py status        # which steps of which jobs are done

A job is a list of steps. Each finished step is recorded in .sync/state/<job>.json, so running the
same command again skips it. A job refuses to resume on different code (another commit), because
its checkpoints would mix two versions; pass --force to restart it on the new code.

A step that measures a bot (its report goes to `--out`) adds a line to results/progress.jsonl: its numbers, the
code it ran and its options. make watch charts these as the bot's progress.

When every step has finished, the job
  1. writes a stamp to results/runs/<job>.json (commit, machine, Python, Rust, GPU, timings),
  2. commits everything under results/ on main,
  3. writes .sync/from-pc.bundle (your new commits) and .sync/DONE-<job>.txt for Claude to read.

Same mechanics as monopoly-strategy's runner; steps can also be plain commands (cargo, make).
"""
from __future__ import annotations

import argparse
import json
import os
import platform
import re
import shlex
import shutil
import socket
import subprocess
import sys
import time
from datetime import datetime, timezone
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SYNC = ROOT / ".sync"
STATE = SYNC / "state"
LOGS = SYNC / "logs"
WATCH = SYNC / "watch"  # what the live viewer reads (make watch)
# Every bot a job measures, one JSON line each, appended as its step finishes (make watch charts the progress).
PROGRESS = ROOT / "results" / "progress.jsonl"
# Big recordings live on the WSL disk, like the build: the repo's drive only holds code and results.
DATA = Path(os.environ.get("BM_DATA", Path.home() / ".cache" / "balatro-meta" / "data"))
PY = sys.executable
MAKE = ["make", "--no-print-directory"]

BASELINE = ["cargo", "run", "--release", "-q", "-p", "bm-core", "--example", "baseline", "--"]


# Each round of the learned value: its network, and the reports of the teacher playing with it at each weight on the
# usual 5,000 seeds (round 1: make teacher-learned; round 2: make teacher-learned-2).
# Rounds 3 and later are all alike (learned_round), up to this one.
LAST_ROUND = 9
LEARNED_ROUNDS = (("models/value-1.bin", "results/value/teacher-w{w}.json"),
                  *((f"models/value-{n}.bin", f"results/value/teacher{n}-w{{w}}.json")
                    for n in range(2, LAST_ROUND + 1)))


def best_learned(root: Path = ROOT) -> tuple[str, str, str] | None:
    """The round and weight of the learned value that won most often on the usual seeds (the reports LEARNED_ROUNDS
    names; the earlier round and the lower weight on a tie): its network, its weight and that report. None before any
    has been measured or without that round's network."""
    best = None
    for net, report in LEARNED_ROUNDS:
        if not (root / net).exists():
            continue
        for w in ("0.5", "1", "2"):
            try:
                rate = json.loads((root / report.format(w=w)).read_text()).get("win_rate")
            except (OSError, ValueError, AttributeError):
                continue
            if isinstance(rate, (int, float)) and (best is None or rate > best[0]):
                best = (rate, net, w, report.format(w=w))
    return None if best is None else best[1:]


def learned_value_args(root: Path = ROOT) -> list[str]:
    """The teacher's learned-value options for the round and weight that won most often (best_learned), or none."""
    best = best_learned(root)
    return [] if best is None else ["--value-net", best[0], "--value-weight", best[1]]


def recording_options(root: Path = ROOT) -> list[str]:
    """The teacher options a recording uses: "precise" (twice the sample contents behind pack and reroll decisions,
    twice the tries behind each plan for a round), which won more in the cloud tests, unless a trial of it on the
    PC (make teacher-precise) found it no better."""
    for f in sorted((root / "results" / "value").glob("precise-vs-*.json")):
        try:
            diff = json.loads(f.read_text()).get("win_rate_diff")
        except (OSError, ValueError, AttributeError):
            continue
        if isinstance(diff, (int, float)) and diff <= 0:
            return []
    return ["--teacher-options", "precise"]


def learned_round(n: int, root: Path = ROOT) -> dict[str, list[tuple]]:
    """Round n (3 or later) of the learned value, three jobs. teacher-data-n: the best teacher there is (the round and
    weight that won most, with "precise" unless its trial found it no better: recording_options) plays 25,000 new
    runs (seed base 101 (n + 1): 404, 505, ...) and records every position it values. value-net-n: the round's
    network learns from them and from the round before's recording (more positions; only round n's runs judge and
    measure it). teacher-learned-n: the teacher plays the usual 5,000 seeds with it at weights 1 and 2, without
    options, to set against the earlier rounds on the same footing."""
    data = DATA / f"value-{n}"
    return {
        f"teacher-data-{n}": [
            ("build", [*MAKE, "build"]),
            ("record", [*BASELINE, "--bot", "teacher", *learned_value_args(root), *recording_options(root),
                        "--runs", "25000", "--seed", str(101 * (n + 1)), "--record", str(data),
                        "--out", f"results/value/record-{n}.json"]),
        ],
        f"value-net-{n}": [
            ("setup-gpu", [*MAKE, "setup-gpu"]),
            ("train", ["scripts/train_value.py", "--data", str(DATA / f"value-{n - 1}"), "--data", str(data),
                       "--out", f"models/value-{n}.bin", "--report", f"results/value/value-{n}.json"]),
        ],
        f"teacher-learned-{n}": [
            ("build", [*MAKE, "build"]),
            *[(f"weight-{w}", [*BASELINE, "--bot", "teacher", "--value-net", f"models/value-{n}.bin",
                               "--value-weight", w, "--runs", "5000", "--seed", "1",
                               "--out", f"results/value/teacher{n}-w{w}.json"])
              for w in ("1", "2")],
        ],
    }


# The value formula's numbers tried by make teacher-tune, each beside its usual value (crates/bm-core/src/value.rs:
# PARAMS). In the cloud the long-run term at twice its weight won 2.75 points less, at half its weight 2.75 more
# (400 seeds each; neither clear).
TUNE = ("long-run=0.25", "long-run=0.35", "sharpness=2.5", "sharpness=5", "money-scale=40", "buying-power=0.045")


def file_safe(options: str) -> str:
    return options.replace(",", "+").replace("=", "-")


def tune_steps(root: Path = ROOT) -> list[tuple]:
    """make teacher-tune: the best teacher there is, with the options recordings use (recording_options), tried with
    each of the formula's numbers in TUNE changed, on the first 2,000 of the usual seeds, each set run by run against
    the same teacher without the change. That teacher's own runs come from make teacher-precise when it has run (the
    same seeds and options), else this job plays them first."""
    best = best_learned(root)
    net_args, stem = (["--value-net", best[0], "--value-weight", best[1]], Path(best[2]).stem) if best else \
        ([], "teacher-plain")
    base = recording_options(root)[1:]
    base_opts = base[0] if base else ""
    if base_opts:
        control = f"results/value/{file_safe(base_opts)}-on-{stem}.json"
    else:
        control = f"results/value/{stem}.json"
    steps: list[tuple] = [("build", [*MAKE, "build"])]
    if base_opts and not (root / control).exists():
        steps.append((base_opts, [*BASELINE, "--bot", "teacher", *net_args, "--teacher-options", base_opts,
                                  "--runs", "2000", "--seed", "1", "--out", control]))
    for change in TUNE:
        opts = f"{base_opts},{change}" if base_opts else change
        out = f"results/tune/{file_safe(opts)}-on-{stem}.json"
        steps.append((change, [*BASELINE, "--bot", "teacher", *net_args, "--teacher-options", opts, "--runs", "2000",
                               "--seed", "1", "--out", out]))
        steps.append((f"compare {change}", ["scripts/compare_runs.py", control, out, "--out",
                                            f"results/tune/{file_safe(opts)}-vs-{Path(control).stem}.json"]))
    return steps


def option_trial(option: str, root: Path = ROOT) -> list[tuple]:
    """The steps that try a teacher option on the best teacher there is: the teacher with the learned value that won
    most (best_learned; the plain teacher before any) and the option plays the first 2,000 of the usual seeds, and
    scripts/compare_runs.py sets it run by run against that teacher's own report on the same seeds."""
    best = best_learned(root)
    net_args, control = (["--value-net", best[0], "--value-weight", best[1]], best[2]) if best else \
        ([], "results/value/teacher-plain.json")
    stem = Path(control).stem
    out = f"results/value/{option}-on-{stem}.json"
    return [
        ("build", [*MAKE, "build"]),
        (option, [*BASELINE, "--bot", "teacher", *net_args, "--teacher-options", option, "--runs", "2000",
                  "--seed", "1", "--out", out]),
        ("compare", ["scripts/compare_runs.py", control, out, "--out", f"results/value/{option}-vs-{stem}.json"]),
    ]


# A step is (name, argv) or (name, argv, env). argv starting with a .py file runs with this Python;
# anything else runs as a command. env adds variables for that step. Finished steps are skipped on a rerun.
JOBS: dict[str, list[tuple]] = {
    # A two-minute end-to-end check: build the engine, run every test, check the machine.
    "smoke": [
        ("build", [*MAKE, "build"]),
        ("rust-tests", ["cargo", "test", "--release", "-q"]),
        ("python-tests", [PY, "-m", "pytest", "-q", "-p", "no:cacheprovider"]),
        ("doctor", ["scripts/doctor.py", "--quick"]),
    ],
    # Phase 1 gate, RNG part: the Rust RNG matches LuaJIT value for value on 200x the committed cases
    # (about 2.4 million lines of seeds, hashes, streams, rounding and shuffles): first this PC's LuaJIT 2.1,
    # then the game's own lua51.dll (LuaJIT 2.0.5 + the Windows C runtime) through Windows Python.
    "rng-check": [
        ("luajit-stress", [*MAKE, "rng-stress", "STRESS=200"]),
        ("game-runtime", ["scripts/rng_game.py", "--scale", "200"]),
    ],
    # Phase 1 gate, speed part: random legal moves on all threads, in Rust and through Python with the bot's
    # input built after every move. The target is 200,000 moves a second.
    "bench": [
        ("build", [*MAKE, "build"]),
        ("engine-speed", ["cargo", "run", "--release", "-q", "-p", "bm-core", "--example", "bench", "--",
                          "20", "0", "results/bench/engine.json"]),
        ("python-speed", ["scripts/bench_env.py", "--n", "4096", "--seconds", "20",
                          "--out", "results/bench/python.json"]),
    ],
    # The number to beat: the greedy bot plays a million fresh-seed runs (Red Deck, White Stake) on all
    # threads. `make watch` shows it live: runs a second, win rate, how runs end, jokers and hands.
    "baseline": [
        ("build", [*MAKE, "build"]),
        ("greedy", ["cargo", "run", "--release", "-q", "-p", "bm-core", "--example", "baseline", "--",
                    "--runs", "1000000", "--seed", "1", "--out", "results/baseline/greedy.json"]),
    ],
    # The teacher bot (crates/bm-core/src/teacher.rs: every decision by the value model, value.rs; rounds
    # planned and farmed by planner.rs) on 12,000 runs: the network's warm start and the next number to
    # beat. Same seeds as the greedy baseline's first 12,000.
    "teacher": [
        ("build", [*MAKE, "build"]),
        ("teacher", ["cargo", "run", "--release", "-q", "-p", "bm-core", "--example", "baseline", "--",
                     "--bot", "teacher", "--runs", "12000", "--seed", "1", "--out", "results/baseline/teacher.json"]),
    ],
    # The learned value, round 1 (crates/bm-core/src/learned.rs). The teacher plays 20,000 runs on fresh seeds
    # (seed base 101: none of the runs `make teacher` plays), and every position it values between rounds is
    # recorded with how its run ended. Its numbers are also a measure of the teacher on new seeds.
    "teacher-data": [
        ("build", [*MAKE, "build"]),
        ("record", [*BASELINE, "--bot", "teacher", "--runs", "20000", "--seed", "101",
                    "--record", str(DATA / "value-1"), "--out", "results/value/record-1.json"]),
    ],
    # The value network, trained on the GPU from that recording (scripts/train_value.py): the chance of
    # winning from a position. The network itself stays out of git (models/); its report holds its checksum.
    "value-net": [
        ("setup-gpu", [*MAKE, "setup-gpu"]),
        ("train", ["scripts/train_value.py", "--data", str(DATA / "value-1"), "--out", "models/value-1.bin",
                   "--report", "results/value/value-1.json"]),
    ],
    # The network that learns to play (scripts/train_policy.py). First the teacher's moves: 10,000 runs on fresh seeds
    # (seed base 303), every move it makes with what the screen showed. The teacher plays with the learned value at
    # the weight that won most often in `make teacher-learned` (learned_value_args), so the network copies the
    # strongest teacher there is.
    "teacher-moves": [
        ("build", [*MAKE, "build"]),
        ("record", [*BASELINE, "--bot", "teacher", *learned_value_args(), "--runs", "10000", "--seed", "303",
                    "--record-moves", str(DATA / "moves-1"), "--out", "results/train/teacher-moves.json"]),
    ],
    # Then the network: it copies the teacher (imitate), plays 2,000 of the usual seeds to show where that leaves it,
    # learns by PPO on the GPU for 8 hours (resumable: Ctrl+C, then make train again; it keeps its best network and
    # falls back to it if its play collapses), and plays 5,000 of the usual seeds with that best network (the
    # teacher's own on the same seeds: results/value/teacher-plain.json and teacher-w*.json).
    "train": [
        ("build", [*MAKE, "build"]),
        ("setup-gpu", [*MAKE, "setup-gpu"]),
        ("imitate", ["scripts/train_policy.py", "imitate", "--data", str(DATA / "moves-1"),
                     "--out", "models/policy/imitate.pt", "--report", "results/train/imitate.json"]),
        ("eval-imitate", ["scripts/train_policy.py", "eval", "--model", "models/policy/imitate.pt", "--runs", "2000",
                          "--seed", "1", "--out", "results/train/eval-imitate.json"]),
        ("ppo", ["scripts/train_policy.py", "ppo", "--init", "models/policy/imitate.pt", "--dir", "models/policy",
                 "--hours", "8", "--report", "results/train/ppo.json"]),
        ("eval", ["scripts/train_policy.py", "eval", "--model", "models/policy/best.pt", "--runs", "5000",
                  "--seed", "1", "--out", "results/train/eval.json"]),
    ],
    # The teacher without the network and with it at three weights, on the same 5,000 seeds (the first of
    # `make teacher`'s): each report's run_stages compare them run by run.
    "teacher-learned": [
        ("build", [*MAKE, "build"]),
        ("plain", [*BASELINE, "--bot", "teacher", "--runs", "5000", "--seed", "1",
                   "--out", "results/value/teacher-plain.json"]),
        *[(f"weight-{w}", [*BASELINE, "--bot", "teacher", "--value-net", "models/value-1.bin", "--value-weight", w,
                           "--runs", "5000", "--seed", "1", "--out", f"results/value/teacher-w{w}.json"])
          for w in ("0.5", "1", "2")],
    ],
    # The trained network's best version (models/policy/best.pt, kept by make train) with its likeliest moves on the
    # usual 5,000 seeds: the number to set against the teacher's, when its training is stopped early.
    "eval-net": [
        ("build", [*MAKE, "build"]),
        ("setup-gpu", [*MAKE, "setup-gpu"]),
        ("eval", ["scripts/train_policy.py", "eval", "--model", "models/policy/best.pt", "--runs", "5000",
                  "--seed", "1", "--out", "results/train/eval.json"]),
    ],
    # The learned value, round 2 (expert iteration): the teacher with round 1's value at its best weight, a better
    # player, plays 25,000 new runs (seed base 202: none played before) and records every position it values with
    # how its run ended; a new value network learns from that; then the teacher plays the usual 5,000 seeds with it,
    # to set against round 1's (results/value/teacher-w1.json: 39.3%).
    "teacher-data-2": [
        ("build", [*MAKE, "build"]),
        ("record", [*BASELINE, "--bot", "teacher", *learned_value_args(), "--runs", "25000", "--seed", "202",
                    "--record", str(DATA / "value-2"), "--out", "results/value/record-2.json"]),
    ],
    "value-net-2": [
        ("setup-gpu", [*MAKE, "setup-gpu"]),
        ("train", ["scripts/train_value.py", "--data", str(DATA / "value-2"), "--out", "models/value-2.bin",
                   "--report", "results/value/value-2.json"]),
    ],
    "teacher-learned-2": [
        ("build", [*MAKE, "build"]),
        *[(f"weight-{w}", [*BASELINE, "--bot", "teacher", "--value-net", "models/value-2.bin", "--value-weight", w,
                           "--runs", "5000", "--seed", "1", "--out", f"results/value/teacher2-w{w}.json"])
          for w in ("1", "2")],
    ],
    # The learned value, round 3 and later (learned_round): the best teacher records with "precise", the round's
    # network learns from its recording and the round before's, and the teacher plays the usual seeds with it.
    **{name: steps for n in range(3, LAST_ROUND + 1) for name, steps in learned_round(n).items()},
    # The teacher's option "precise" (crates/bm-core/src/value.rs: `Scales`): twice the sample contents behind pack
    # and reroll decisions, its noisiest, and twice the tries behind each plan for a round, at about 1.45 times the
    # time. It is tried on the best teacher there is (option_trial). On 400 seeds in the cloud with round 1's value
    # it won 45.0% against 38.5% (p = 0.03); 2,000 seeds on the PC tell more.
    "teacher-precise": option_trial("precise"),
    # The value formula's numbers, each changed in turn on the best teacher there is (tune_steps, TUNE): about an hour.
    "teacher-tune": tune_steps(),
}


def git(*args: str, check: bool = True, capture: bool = True) -> str:
    r = subprocess.run(["git", *args], cwd=ROOT, check=False, text=True,
                       stdout=subprocess.PIPE if capture else None, stderr=subprocess.PIPE if capture else None)
    if check and r.returncode != 0:
        raise SystemExit(f"git {' '.join(args)} failed:\n{(r.stderr or '').strip()}")
    return (r.stdout or "").strip()


def now() -> str:
    return datetime.now(timezone.utc).isoformat(timespec="seconds")


def dirty_tracked(exclude_results: bool) -> list[str]:
    """Tracked files with real (not line-ending-only) changes."""
    out = git("diff", "--name-only", "--ignore-cr-at-eol", "HEAD")
    files = [f for f in out.splitlines() if f]
    if exclude_results:
        files = [f for f in files if not f.startswith("results/")]
    return files


def require_main() -> None:
    branch = git("rev-parse", "--abbrev-ref", "HEAD")
    if branch != "main":
        raise SystemExit(f"You're on branch '{branch}'. Run `git switch main` first.")


def is_ancestor(a: str, b: str) -> bool:
    return subprocess.run(["git", "merge-base", "--is-ancestor", a, b], cwd=ROOT).returncode == 0


def fetch_bundle() -> bool:
    """Fetch Claude's bundle into refs/remotes/claude/main. True if it holds code main doesn't have yet."""
    bundle = SYNC / "claude.bundle"
    if not bundle.exists():
        return False
    git("bundle", "verify", "--quiet", str(bundle))
    git("fetch", "--quiet", str(bundle), "+main:refs/remotes/claude/main")
    return not is_ancestor("refs/remotes/claude/main", "HEAD")


def sync() -> None:
    if not (SYNC / "claude.bundle").exists():
        raise SystemExit("No .sync/claude.bundle yet: nothing to sync.")
    require_main()
    if not fetch_bundle():
        print("Already up to date with Claude's latest code.")
        return
    changed = dirty_tracked(exclude_results=False)
    if changed:
        raise SystemExit("These tracked files have uncommitted changes, so the update was not applied:\n  "
                         + "\n  ".join(changed[:20])
                         + "\nIf a job was interrupted, finish it first (rerun its make command). "
                           "Otherwise commit or stash the changes and run `make sync` again.")
    base = "refs/remotes/claude/main"
    if is_ancestor("HEAD", base):
        git("merge", "--ff-only", "--quiet", base)
        print(f"Updated to {git('log', '-1', '--format=%h %s')}")
        return
    # main has commits Claude hasn't seen yet. If they only add results, replay them on top of the new code.
    local = [f for f in git("diff", "--name-only", f"{base}...HEAD").splitlines() if f]
    if local and all(f.startswith("results/") for f in local):
        r = subprocess.run(["git", "rebase", "--quiet", base], cwd=ROOT, capture_output=True, text=True)
        if r.returncode != 0:
            subprocess.run(["git", "rebase", "--abort"], cwd=ROOT, capture_output=True)
            raise SystemExit("Your results commits clash with Claude's newer code, so nothing was changed. "
                             "Tell Claude; .sync/from-pc.bundle carries your results.")
        git("bundle", "create", "--quiet", str(SYNC / "from-pc.bundle"), f"{base}..main")
        print(f"Updated to Claude's latest code and replayed your results on top: "
              f"{git('log', '-1', '--format=%h %s')}")
        return
    raise SystemExit("Your main has commits Claude hasn't seen that change more than results/, so it can't "
                     "update automatically. Tell Claude; .sync/from-pc.bundle carries them.")


def load_state(job: str) -> dict | None:
    p = STATE / f"{job}.json"
    return json.loads(p.read_text()) if p.exists() else None


def save_state(job: str, st: dict) -> None:
    STATE.mkdir(parents=True, exist_ok=True)
    tmp = STATE / f"{job}.json.tmp"
    tmp.write_text(json.dumps(st, indent=1))
    tmp.replace(STATE / f"{job}.json")


def _first_line(*cmd: str) -> str | None:
    if not shutil.which(cmd[0]):
        return None
    try:
        r = subprocess.run(cmd, text=True, capture_output=True, timeout=20)
        return (r.stdout or r.stderr).strip().splitlines()[0] if r.returncode == 0 else None
    except (OSError, subprocess.TimeoutExpired, IndexError):
        return None


def machine() -> dict:
    mem = None
    try:
        for line in Path("/proc/meminfo").read_text().splitlines():
            if line.startswith("MemTotal:"):
                mem = round(int(line.split()[1]) / 1024 ** 2, 1)
    except OSError:
        pass
    try:
        load = [round(x, 1) for x in os.getloadavg()]  # how busy Linux was (other jobs slow benchmarks)
    except OSError:
        load = None
    return {"host": socket.gethostname(), "platform": platform.platform(), "cpu_threads": os.cpu_count(),
            "ram_gb": mem, "load_avg_1_5_15": load, "python": platform.python_version(),
            "rustc": _first_line("rustc", "--version"),
            "gpu": _first_line("nvidia-smi", "--query-gpu=name,memory.total,driver_version",
                               "--format=csv,noheader")}


def step_command(argv: list[str]) -> list[str]:
    return [PY, "-u", *argv] if argv[0].endswith(".py") else list(argv)


def code_of(commit: str) -> tuple[str, str]:
    """The code a job ran: the newest commit at or before `commit` that isn't a results commit (short hash,
    subject)."""
    out = git("log", "-1", "--format=%h%x09%s", "--invert-grep", "--grep=^results(", commit, check=False)
    code, _, title = out.partition("\t")
    return code, title


def progress_entry(job: str, step: str, argv: list[str], commit: str, root: Path = ROOT) -> dict | None:
    """The line results/progress.jsonl gets for a step that measured a bot: what its report (`--out`) says,
    with the code it ran and the options that matter (seeds, learned value). None for any other step."""
    def arg(flag: str) -> str | None:
        return argv[argv.index(flag) + 1] if flag in argv[:-1] else None

    out = arg("--out")
    if not out:
        return None
    try:
        r = json.loads((root / out).read_text())
    except (OSError, ValueError):
        return None
    if not isinstance(r, dict) or r.get("kind") != "baseline" or not r.get("runs"):
        return None
    code, title = code_of(commit)
    net, weight = arg("--value-net"), arg("--value-weight")
    w = float(weight) if weight else (1.0 if net else None)
    bot = str(r.get("bot") or "bot")
    rnd = re.search(r"value-(\d+)\.bin$", net or "")
    if net:
        name = f"Learned value{' ' + rnd[1] if rnd and rnd[1] != '1' else ''} \u00d7{w:g}"
    elif bot == "network":
        name = "Network, copying the teacher" if step == "eval-imitate" else "Network after PPO"
    else:
        name = bot[:1].upper() + bot[1:].removesuffix(" bot")
    options = arg("--teacher-options")
    if options:
        name += ", " + options.replace(",", ", ")
    return {"time": now(), "name": name, "job": job, "step": step, "code": code, "title": title, "out": out,
            "bot": r.get("bot"),
            "seed": int(arg("--seed") or 1), "value_net": net, "value_weight": w, "teacher_options": options,
            "runs": r["runs"], "wins": r.get("wins"), "win_rate": r.get("win_rate"), "win_ci": r.get("win_ci"),
            "mean_ante": r.get("mean_ante")}


def record_progress(entry: dict | None, path: Path = PROGRESS) -> None:
    if entry is None:
        return
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("a") as fh:
        fh.write(json.dumps(entry) + "\n")


def run(job: str, force: bool, watch: bool = True) -> None:
    if job not in JOBS:
        raise SystemExit(f"Unknown job '{job}'. Jobs: {', '.join(JOBS)}")
    require_main()
    if load_state(job):  # resuming: finish on the job's own code, even if newer code is waiting
        if fetch_bundle():
            print("Note: newer code from Claude is waiting. Finishing this job on its original code first; "
                  "run `make sync` afterwards.")
    elif (SYNC / "claude.bundle").exists():
        before = git("rev-parse", "HEAD")
        sync()
        if git("rev-parse", "HEAD") != before and not os.environ.get("BM_JOBS_REEXEC"):
            # The update may have changed this script and the job's steps: start again on the new code,
            # so the job runs the steps the new code defines rather than the ones loaded before the update.
            os.environ["BM_JOBS_REEXEC"] = "1"
            sys.stdout.flush()
            os.execv(sys.executable, [sys.executable, str(Path(__file__).resolve()), *sys.argv[1:]])
    changed = dirty_tracked(exclude_results=True)
    if changed:
        raise SystemExit("Uncommitted changes outside results/ (" + ", ".join(changed[:5]) + "). "
                         "Commit or stash them first, so the results record the exact code that made them.")
    head = git("rev-parse", "HEAD")
    st = load_state(job)
    if st and st.get("commit") != head and not force:
        raise SystemExit(f"Job '{job}' was started on commit {st['commit'][:9]} but the code is now {head[:9]}. "
                         f"Rerun with FORCE=1 (make {job} FORCE=1) to restart it on the new code.")
    if not st or st.get("commit") != head:
        st = {"job": job, "commit": head, "started": now(), "steps": {}}
        save_state(job, st)
    try:
        os.nice(10)  # keep the PC responsive: jobs yield to anything interactive
    except OSError:
        pass
    LOGS.mkdir(parents=True, exist_ok=True)
    steps = [(*step, {})[:3] for step in JOBS[job]]
    for i, (name, argv, env) in enumerate(steps, 1):
        if st["steps"].get(name, {}).get("done"):
            print(f"[{i}/{len(steps)}] {name}: already done, skipping")
            continue
        cmd = step_command(argv)
        extra = " ".join(f"{k}={v}" for k, v in env.items())
        print(f"[{i}/{len(steps)}] {name}: {extra + ' ' if extra else ''}{shlex.join(argv)}", flush=True)
        t0 = time.time()
        log = LOGS / f"{job}-{name}.log"
        watch_note(job, st, steps, "running", step=name)
        live = {"BM_WATCH_DIR": str(WATCH / job), "BM_STEP": f"{job}/{name}"} if watch else {}
        with log.open("a") as fh:
            fh.write(f"\n=== {now()} {extra} {shlex.join(cmd)}\n")
            fh.flush()
            proc = subprocess.Popen(cmd, cwd=ROOT, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                                    text=True, bufsize=1, env={**os.environ, **live, **env})
            assert proc.stdout is not None
            stopped = False
            try:
                for line in proc.stdout:
                    sys.stdout.write("    " + line)
                    fh.write(line)
                    fh.flush()  # the log is readable while the step runs
            except KeyboardInterrupt:
                # Ctrl+C reached the step too: let it finish saving (the network's training saves its place), and
                # show what it says as it stops. A second Ctrl+C stops it at once.
                stopped = True
                print("\n    Stopping: letting the step save its place...", flush=True)
                try:
                    for line in proc.stdout:
                        sys.stdout.write("    " + line)
                        fh.write(line)
                    proc.wait(timeout=300)
                except (KeyboardInterrupt, subprocess.TimeoutExpired):
                    proc.kill()
            code = proc.wait()
        secs = round(time.time() - t0)
        if stopped:
            st["steps"][name] = {"done": False, "seconds": secs, "stopped": now()}
            save_state(job, st)
            watch_note(job, st, steps, "stopped", step=name)
            raise SystemExit(f"Stopped during step '{name}'. Run `make {job}` to carry on: the steps already done are "
                             "kept, and this one starts again (the network's training carries on from its last save).")
        if code != 0:
            st["steps"][name] = {"done": False, "seconds": secs, "exit_code": code}
            save_state(job, st)
            watch_note(job, st, steps, "failed", step=name)
            raise SystemExit(f"Step '{name}' failed (exit code {code}); the log is .sync/logs/{log.name}. "
                             f"Rerun `make {job}` to resume from this step once it's fixed.")
        st["steps"][name] = {"done": True, "seconds": secs, "finished": now()}
        save_state(job, st)
        record_progress(progress_entry(job, name, list(argv), st["commit"]))
        print(f"    done in {secs // 60} min {secs % 60} s", flush=True)
    watch_note(job, st, steps, "finished")
    finish(job, st)


def watch_note(job: str, st: dict, steps: list, state: str, step: str | None = None) -> None:
    """Tell the live viewer which job and step are running. Never fails the job."""
    try:
        WATCH.mkdir(parents=True, exist_ok=True)
        note = {"job": job, "state": state, "step": step, "time": time.time(), "host": socket.gethostname(),
                "pid": os.getpid(), "started": st["started"], "commit": st["commit"][:9],
                "steps": [{"name": s[0], "done": bool(st["steps"].get(s[0], {}).get("done")),
                           "seconds": st["steps"].get(s[0], {}).get("seconds")} for s in steps]}
        tmp = WATCH / ".current.tmp"
        tmp.write_text(json.dumps(note))
        os.replace(tmp, WATCH / "current.json")
    except OSError:
        pass


def finish(job: str, st: dict) -> None:
    stamp = {"job": job, "commit": st["commit"], "started": st["started"], "finished": now(),
             "machine": machine(), "steps": {k: v["seconds"] for k, v in st["steps"].items()},
             "commands": {s[0]: " ".join(f"{k}={v}" for k, v in (s[2] if len(s) > 2 else {}).items())
                          + (" " if len(s) > 2 else "") + shlex.join(s[1]) for s in JOBS[job]}}
    runs = ROOT / "results" / "runs"
    runs.mkdir(parents=True, exist_ok=True)
    (runs / f"{job}.json").write_text(json.dumps(stamp, indent=1) + "\n")
    git("add", "--all", "results")
    if git("diff", "--cached", "--name-only"):
        git("commit", "--quiet", "-m", f"results({job}): {stamp['machine']['host']}, "
                                       f"{stamp['machine']['cpu_threads']} threads, code {st['commit'][:9]}")
    base = "refs/remotes/claude/main"
    has_base = subprocess.run(["git", "rev-parse", "--verify", "--quiet", base], cwd=ROOT,
                              stdout=subprocess.DEVNULL).returncode == 0
    rng = f"{base}..main" if has_base and git("rev-list", "--count", f"{base}..main") != "0" else "main"
    git("bundle", "create", str(SYNC / "from-pc.bundle"), rng)
    total = sum(stamp["steps"].values())
    lines = [f"Job {job} finished {stamp['finished']} on {stamp['machine']['host']} "
             f"({stamp['machine']['cpu_threads']} threads, Python {stamp['machine']['python']}, "
             f"{stamp['machine']['rustc'] or 'no rustc'}).",
             f"GPU: {stamp['machine']['gpu'] or 'not visible'}",
             f"Code: {st['commit']}", f"Results commit: {git('rev-parse', 'HEAD')}",
             f"Total time: {total // 3600} h {total % 3600 // 60} min {total % 60} s", "", "Steps (seconds):"]
    lines += [f"  {k}: {v}" for k, v in stamp["steps"].items()]
    (SYNC / f"DONE-{job}.txt").write_text("\n".join(lines) + "\n")
    (STATE / f"{job}.json").unlink(missing_ok=True)
    print("\n".join(["", *lines, "", "All done. Tell Claude in the chat; the results are in .sync/ for it to read."]))


def status() -> None:
    for job, steps in JOBS.items():
        st = load_state(job)
        done = (SYNC / f"DONE-{job}.txt").exists()
        if st:
            n = sum(1 for step in steps if st["steps"].get(step[0], {}).get("done"))
            print(f"{job}: running or interrupted, {n}/{len(steps)} steps done (code {st['commit'][:9]})")
        else:
            print(f"{job}: {'finished' if done else 'not started'}")


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    sub.add_parser("sync")
    r = sub.add_parser("run")
    r.add_argument("job")
    r.add_argument("--force", action="store_true")
    r.add_argument("--no-watch", action="store_true", help="don't leave notes for the live viewer")
    sub.add_parser("status")
    a = ap.parse_args()
    if a.cmd == "sync":
        sync()
    elif a.cmd == "run":
        run(a.job, a.force, watch=not a.no_watch)
    else:
        status()


if __name__ == "__main__":
    main()
