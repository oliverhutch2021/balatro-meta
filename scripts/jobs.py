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
# How results name the machine that ran them (committed, so not the computer's own name).
HOST = os.environ.get("BM_HOST", "PC")
PY = sys.executable
MAKE = ["make", "--no-print-directory"]

BASELINE = ["cargo", "run", "--release", "-q", "-p", "bm-core", "--example", "baseline", "--"]


# Each round of the learned value: its network, and the reports of the teacher playing with it at each weight on the
# usual 5,000 seeds (round 1: make teacher-learned; round 2: make teacher-learned-2).
# Rounds 3 and later are all alike (learned_round), up to this one.
LAST_ROUND = 12


def ensemble(n: int) -> str:
    """The networks of rounds 1 to n together (the learned value is then the mean of their chances of winning): each
    round's, and from round 3 on the second network each round trains (value-Nb, another seed on the same runs)."""
    nets = [f"models/value-{k}.bin" for k in range(1, n + 1)]
    nets += [f"models/value-{k}b.bin" for k in range(3, n + 1)]
    return ",".join(nets)


# Each learned value tried on the usual seeds, with its reports (one per weight): each round's network, and from
# round 2 on, the networks of all rounds so far together (an ensemble: on the cloud's 400 seeds, rounds 1 and 2
# together with "precise" won 49.75% against 45.0% for round 1's alone).
LEARNED_ROUNDS = (("models/value-1.bin", "results/value/teacher-w{w}.json"),
                  *((f"models/value-{n}.bin", f"results/value/teacher{n}-w{{w}}.json")
                    for n in range(2, LAST_ROUND + 1)),
                  *((ensemble(n), f"results/value/ensemble{n}-w{{w}}.json") for n in range(2, LAST_ROUND + 1)))


# The weights a learned value is tried at, in the order a tie goes to.
WEIGHTS = ("0.5", "1", "1.5", "2")


def best_learned(root: Path = ROOT) -> tuple[str, str, str] | None:
    """The learned value and weight that won most often on the usual seeds (the reports LEARNED_ROUNDS names; the
    earlier entry and the lower weight on a tie): its network(s), its weight and that report. None before any has
    been measured or without its networks."""
    best = None
    for net, report in LEARNED_ROUNDS:
        if not all((root / n).exists() for n in net.split(",")):
            continue
        for w in WEIGHTS:
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


# A trial's gain over the precise teacher on the same seeds (make teacher-tune-2, make teacher-ideas) is trusted for
# a recording below this p-value.
RECORD_WITH_P = 0.1


def report_args(r: dict) -> list[str]:
    """The teacher a report played, as baseline's arguments: its networks, weight and options, and its stake
    correction if it had one."""
    options = r.get("teacher_options") or []
    options = options.split(",") if isinstance(options, str) else list(options)
    args = ["--value-net", str(r["value_net"]), "--value-weight", f"{float(r['value_weight']):g}"]
    args += ["--teacher-options", ",".join(options)] if options else []
    if r.get("stake_net"):
        args += ["--stake-net", str(r["stake_net"]), "--stake-mode", str(r.get("stake_mode") or "residual")]
    return args


def best_8k(root: Path = ROOT) -> tuple[float, list[str]] | None:
    best = best_8k_report(root)
    return None if best is None else best[:2]


def best_8k_report(root: Path = ROOT, leave_out: str = "") -> tuple[float, list[str], str] | None:
    """The teacher that won most of the first 8,000 usual seeds (make teacher-weights and teacher-big on the formula in
    force, any networks and weight: the same seeds, so their rates compare directly), as its win rate and arguments
    (networks, weight, options); None before any. Exploring runs never count (recordings use other seeds anyway)."""
    best = None
    for folder in ("value", "tune"):
        for f in sorted((root / "results" / folder).glob(f"f{FORMULA}-*-8k-*.json")):
            if "-vs-" in f.name:
                continue
            try:
                r = json.loads(f.read_text())
                rate, runs, base = float(r["win_rate"]), int(r["runs"]), int(r.get("seed_base", 1))
                options = r.get("teacher_options") or []
                options = options.split(",") if isinstance(options, str) else list(options)
            except (OSError, ValueError, KeyError, TypeError):
                continue
            if runs < 8000 or base != 1 or not r.get("value_net") or any(o.startswith("explore") for o in options):
                continue
            if (best is None or rate > best[0]) and f.name != leave_out:
                best = (rate, report_args(r), str(f.relative_to(root)))
    return best


def best_recording(root: Path = ROOT) -> list[str]:
    """The teacher a round records with, as its arguments (learned value, weight, options): the one that won most of the
    first 8,000 usual seeds when one has played them (best_8k: make teacher-weights measures each new round's networks
    at several weights there); before that, of the trials on the best learned value (results/tune: each change set
    run by run against the precise teacher on the same 2,000 seeds), the one that won most over it, if it won clearly
    (p below RECORD_WITH_P), with the networks it played with (a trial can be other networks: make teacher-next's
    "values 1-4"); else the best learned value with recording_options."""
    eight = best_8k(root)
    if eight is not None:
        return eight[1]
    base = [*learned_value_args(root), *recording_options(root)]
    best = best_learned(root)
    if best is None or not recording_options(root):
        return base
    stem = Path(best[2]).stem
    top = None
    # Only trials on the formula in force (an older formula's winners are in its numbers now, or were beaten).
    for f in sorted((root / "results" / "tune").glob(f"f{FORMULA}-*-vs-*-on-{stem}.json")):
        try:
            d = json.loads(f.read_text())
            diff, p = float(d["win_rate_diff"]), float(d["p_value"])
            trial = json.loads((root / d["b"]["report"]).read_text())
            options = trial["teacher_options"]
        except (OSError, ValueError, KeyError, TypeError):
            continue
        if diff > 0 and p < RECORD_WITH_P and options and (top is None or diff > top[0]):
            nets = [best[0], best[1]]
            if trial.get("value_net"):
                nets = [str(trial["value_net"]), f"{float(trial.get('value_weight', best[1])):g}"]
            top = (diff, ["--value-net", nets[0], "--value-weight", nets[1], "--teacher-options", ",".join(options)])
    return base if top is None else top[1]


# From round 7 the recordings explore (teacher option "explore": a shop or pack decision takes, one time in ten,
# another option the value rates nearly as high as the best), so the learned value also sees where the choices the
# teacher doesn't make lead. Rounds 4 to 6 gained less each time (+1.75, then +0.4 points on the same 2,000 seeds):
# a value learned only from the teacher's own choices can only refine its habits. Measuring never explores.
EXPLORE = "explore=0.1"
EXPLORE_FROM = 7
# From round 10 a quarter of the recorded runs also commit, from ante 3 (the shop after ante 2's boss), to one hand
# type picked at random by the run's number (teacher option "commit": its Planets taken and bought as the focus's):
# whole builds the teacher wouldn't choose, played to the end, where "explore" only tries single moves along its own
# habits.
COMMIT = "commit=0.25"
COMMIT_FROM = 10
# From round 11 a won run plays on into Endless, to ante 12 (baseline --endless), and its positions up to the win are
# labelled with how many blinds it beat after it as well (0 to 12): a win with a build that clears ante 10 is a better
# win than one that scrapes through ante 8, which a win alone can't tell. The networks learn it as a second head
# (train_value.py: depth outputs, learned from those wins alone), and teacher option "margin=K" rates a position by
# its chance of winning with K blinds to spare (make teacher-margin). Positions after the win are never recorded.
ENDLESS_FROM = 11


def exploring(args: list[str], n: int) -> list[str]:
    """A recording's teacher arguments, with the exploration rounds from EXPLORE_FROM on use (and from COMMIT_FROM,
    whole-build exploration too)."""
    if n < EXPLORE_FROM:
        return args
    extra = f"{EXPLORE},{COMMIT}" if n >= COMMIT_FROM else EXPLORE
    args = list(args)
    if "--teacher-options" in args:
        i = args.index("--teacher-options") + 1
        args[i] = f"{args[i]},{extra}"
    else:
        args += ["--teacher-options", extra]
    return args


# make teacher-exam: the headline number. Ideas are compared on the usual seeds (seed base 1) and the winners kept,
# which flatters those seeds a little; the exam's seeds (a base no recording or trial uses) only ever measure the
# best teacher there is, never choose anything.
EXAM_SEED = 5003
EXAM_RUNS = 8000


def exam_steps(root: Path = ROOT, runs: int = EXAM_RUNS) -> list[tuple]:
    """make teacher-exam: the teacher a round would record with (best_recording: the best networks, weight and
    options, without exploring) on the exam's seeds."""
    args = best_recording(root)
    net = args[args.index("--value-net") + 1] if "--value-net" in args else ""
    rounds = sorted({int(n) for n in re.findall(r"value-(\d+)b?\.bin", net)})
    w = args[args.index("--value-weight") + 1] if "--value-weight" in args else "0"
    opts = args[args.index("--teacher-options") + 1] if "--teacher-options" in args else ""
    name = f"exam-{runs // 1000}k-f{FORMULA}-{file_safe(opts) or 'plain'}-rounds{rounds[-1] if rounds else 0}-w{w}"
    return [("build", [*MAKE, "build"]),
            ("exam", [*BASELINE, "--bot", "teacher", *args, "--runs", str(runs), "--seed", str(EXAM_SEED),
                      "--out", f"results/exam/{name}.json"])]


# make teacher-stakes: the best teacher there is (the exam's: best_recording, without exploring) at every stake, White
# to Gold, on the first 2,000 usual seeds each. Each stake adds a rule (Red: the Small Blind pays nothing; Green and
# Purple: blinds grow faster; Black: eternal jokers; Blue: a discard fewer; Orange: perishable jokers; Gold: rental
# jokers), so the steps from one stake to the next show what each rule costs. The networks learned White Stake's runs
# alone, so this is where the teacher stands before learning any other.
STAKE_RUNS = 2000


def stakes_steps(root: Path = ROOT, runs: int = STAKE_RUNS) -> list[tuple]:
    args = best_recording(root)
    net = args[args.index("--value-net") + 1] if "--value-net" in args else ""
    rounds = sorted({int(n) for n in re.findall(r"value-(\d+)b?\.bin", net)})
    w = args[args.index("--value-weight") + 1] if "--value-weight" in args else "0"
    opts = args[args.index("--teacher-options") + 1] if "--teacher-options" in args else ""
    tail = f"{runs // 1000}k-f{FORMULA}-{file_safe(opts) or 'plain'}-rounds{rounds[-1] if rounds else 0}-w{w}"
    return [("build", [*MAKE, "build"]),
            *[(f"stake-{s}", [*BASELINE, "--bot", "teacher", *args, "--runs", str(runs), "--seed", "1",
                              "--stake", str(s), "--out", f"results/stakes/stake{s}-{tail}.json"])
              for s in range(8, 0, -1)]]  # Gold first: the stakes furthest from what the networks learned


# make teacher-inputs: does seeing the deck card by card (input layout 2: which ranks and suits carry which
# enhancement, seal and edition) make better networks than layout 1's totals? Round 11 is the first recording to hold
# layout 2 (rounds 1 to 10 hold layout 1 alone), so both pairs learn from round 11's runs alone, with the seeds every
# round's pair uses, and play at the same weight on the first 8,000 usual seeds, set run by run: the same runs and
# the same compute, so a difference is the input's. Round 11's own networks (make value-net-11, rounds 10 and 11 read
# as layout 1, the layout they share) stay the ensemble's.
INPUT_ROUND = 11
INPUT_WEIGHT = "2.5"


def input_steps(n: int = INPUT_ROUND, w: str = INPUT_WEIGHT, runs: int | None = None) -> list[tuple]:
    runs = BIG_RUNS if runs is None else runs
    data = DATA / f"value-{n}"
    steps: list[tuple] = [("setup-gpu", [*MAKE, "setup-gpu"]), ("build", [*MAKE, "build"])]
    plays = {}
    for layout in (1, 2):
        nets = [f"models/value-{n}-layout{layout}{suffix}.bin" for suffix in ("", "b")]
        for net, seed in zip(nets, ("1", "2")):
            steps.append((f"train-layout{layout}{'' if seed == '1' else 'b'}", [
                "scripts/train_value.py", "--data", str(data), "--input-layout", str(layout), "--seed", seed,
                "--out", net, "--report", f"results/value/{Path(net).stem}.json"]))
        plays[layout] = f"results/tune/f{FORMULA}-precise-{runs // 1000}k-w{w}-on-value-{n}-layout{layout}.json"
        steps.append((f"play-layout{layout}", [*BASELINE, "--bot", "teacher", "--value-net", ",".join(nets),
                                               "--value-weight", w, "--teacher-options", "precise",
                                               "--runs", str(runs), "--seed", "1", "--out", plays[layout]]))
    steps.append(("compare", ["scripts/compare_runs.py", plays[1], plays[2], "--out",
                              f"results/tune/f{FORMULA}-precise-{runs // 1000}k-w{w}-value-{n}-layout2-vs-layout1.json"]))
    return steps


# make stakes-night (.sync/night.sh starts it): networks that know the stake. The stake results of 3 November
# (make teacher-stakes: the best teacher, which learned White's runs alone, at every stake) lost most where the
# blinds grow faster (Green -3.2 blinds beaten, Purple -1.7) and where jokers carry a future (Gold's rentals -2.4,
# Black's eternals -1.2). The networks never saw a stake, and the trainer leaves out inputs that never vary, so the
# jokers' eternal, perishable and rental marks (all off at White) were left out too. Rounds of expert iteration at
# every stake fix both: the networks read the stake as its rules in force (input layout 3) and learn runs at all
# eight, most where the result is least certain (the most to learn from a run: White and Red, Green, Black), and
# each round is judged first at White, on the first 8,000 usual seeds against the best teacher there: harder stakes
# are worth learning from only if they make the play better all round.
STAKE_BASELINE = "2k-f3-precise-rounds8-w2.5"  # make teacher-stakes's reports of 3 November, results/stakes/
NIGHT_RECORDING = DATA / "stakes-1-layout2"  # .sync/night.sh: round 1, recorded before layout 3 existed
STAKE_RUNS_1 = {1: 3000, 2: 4000, 3: 6000, 4: 6000, 5: 5000, 6: 5000, 7: 4000, 8: 4000}  # night.sh's
STAKE_RUNS_N = {1: 6000, 2: 6000, 3: 6000, 4: 5000, 5: 3000, 6: 2000, 7: 1500, 8: 1500}  # rounds 2 on
STAKE_ROUNDS = 12
STAKE_WEIGHT = "2.5"
STAKE_LAYOUT = "3"
# Trials on the best teacher of 3 November, each at the stake that brings what it changes, set run by run against
# that stake's report (same seeds, same networks): a joker type the teacher would do better never to buy is one
# its value overrates; such an option is kept for the rounds after (stake_options). "reach=6" (the learned term
# from the networks' chance of getting 6 blinds further, not of winning) is tried where wins are rare.
STAKE_TRIALS = (("avoid-eternal", 4), ("avoid-perishable", 7), ("avoid-rental", 8), ("reach=6", 3), ("reach=6", 6))


def stake_data(n: int) -> Path:
    return DATA / f"stakes-{n}"


def stake_nets(n: int) -> str:
    return f"models/value-stakes-{n}.bin,models/value-stakes-{n}b.bin"


def stake_baseline(s: int) -> str:
    return f"results/stakes/stake{s}-{STAKE_BASELINE}.json"


def baseline_args(root: Path = ROOT, s: int = 1) -> list[str]:
    """The teacher of a stake baseline report, as its arguments (networks, weight, options)."""
    try:
        r = json.loads((root / stake_baseline(s)).read_text())
        return report_args(r)
    except (OSError, ValueError, KeyError, TypeError):
        return best_recording(root)


def stake_trial_name(option: str, s: int) -> str:
    return f"results/stakes/stake{s}-{STAKE_BASELINE}-{file_safe(option)}"


def stake_options(root: Path = ROOT) -> list[str]:
    """The trials' avoid- options that took their stake's runs clearly further (paired blinds beaten, the 95%
    interval above 0). Each changes only the stakes that bring its jokers. ("reach" is not taken up: it changes
    White's play too, where the win chance tells positions apart well.)"""
    kept = []
    for option, s in STAKE_TRIALS:
        if not option.startswith("avoid-"):
            continue
        try:
            c = json.loads((root / f"{stake_trial_name(option, s)}-vs.json").read_text())
            low = float(c["blinds_beaten_diff_ci"][0])
        except (OSError, ValueError, KeyError, TypeError, IndexError):
            continue
        if low > 0 and option not in kept:
            kept.append(option)
    return kept


def stake_teacher(n: int, root: Path = ROOT, explore: bool = False) -> list[str]:
    """Round n's stake-aware teacher: its two networks, the usual weight, "precise" and the avoid- options the
    trials kept; recording, exploring as round 11 on would (explore, commit)."""
    options = ["precise", *stake_options(root)] + ([EXPLORE, COMMIT] if explore else [])
    return ["--value-net", stake_nets(n), "--value-weight", STAKE_WEIGHT, "--teacher-options", ",".join(options)]


def stake_trials_steps(root: Path = ROOT, runs: int = STAKE_RUNS) -> list[tuple]:
    """make stakes-trials (about 35 minutes)."""
    steps: list[tuple] = [("build", [*MAKE, "build"])]
    for option, s in STAKE_TRIALS:
        args = baseline_args(root, s)
        i = args.index("--teacher-options") + 1 if "--teacher-options" in args else None
        if i is None:
            args = [*args, "--teacher-options", option]
        else:
            args = [*args[:i], f"{args[i]},{option}", *args[i + 1:]]
        out = stake_trial_name(option, s)
        steps += [(f"{file_safe(option)}-stake{s}", [*BASELINE, "--bot", "teacher", *args, "--runs", str(runs),
                                                     "--seed", "1", "--stake", str(s), "--out", f"{out}.json"]),
                  (f"compare-{file_safe(option)}-stake{s}", ["scripts/compare_runs.py", stake_baseline(s),
                                                             f"{out}.json", "--out", f"{out}-vs.json"])]
    return steps


def stake_round(n: int, root: Path = ROOT, runs: int = STAKE_RUNS, white: int | None = None
                ) -> dict[str, list[tuple]]:
    """Round n of the stake-aware networks, three jobs. stakes-data-n: runs at every stake, each with every position
    it values; round 1's are .sync/night.sh's recording (the best teacher of 3 November, exploring), encoded afresh
    in layout 3 (baseline --replay), or recorded here if it isn't there; from round 2 the stake-aware teacher of
    the round before records (seed bases 7000 + 100 (n - 1) + stake). stakes-net-n: two networks learn layout 3 from
    the round's runs at all eight stakes and the round before's (the round's own judge and measure them).
    stakes-play-n: the pair at every stake on the 2,000 seeds of 3 November, each set run by run against that day's
    report; at White on the first 8,000 usual seeds against the best teacher there (best_8k); at Green at weights
    1.5 and 4 (the usual 2.5 was chosen at White); and with "reach=6" at Green and Purple."""
    white = BIG_RUNS if white is None else white
    data = stake_data(n)
    sizes = STAKE_RUNS_1 if n == 1 else STAKE_RUNS_N
    record: list[tuple] = [("build", [*MAKE, "build"])]
    for s in range(1, 9):
        dst = data / f"stake{s}"
        night = NIGHT_RECORDING / f"stake{s}"
        if n == 1 and (night / "manifest.json").exists():
            record.append((f"replay-stake{s}", [*BASELINE, "--replay", str(night), "--record", str(dst),
                                                "--out", f"results/stakes/replay-{n}-stake{s}.json"]))
            continue
        teacher = exploring(best_recording(root), 11) if n == 1 else stake_teacher(n - 1, root, explore=True)
        record.append((f"record-stake{s}", [*BASELINE, "--bot", "teacher", *teacher, "--runs", str(sizes[s]),
                                            "--seed", str(7000 + 100 * (n - 1) + s), "--stake", str(s),
                                            "--record", str(dst),
                                            "--out", f"results/stakes/record-{n}-stake{s}.json"]))
    folders = [stake_data(m) / f"stake{s}" for m in ([n - 1, n] if n > 1 else [n]) for s in range(1, 9)]
    train = [(f"train{suffix}", ["scripts/train_value.py", *[x for f in folders for x in ("--data", str(f))],
                                 "--newest", "8", "--input-layout", STAKE_LAYOUT, "--seed", seed,
                                 "--out", f"models/value-stakes-{n}{suffix}.bin",
                                 "--report", f"results/value/value-stakes-{n}{suffix}.json"])
             for suffix, seed in (("", "1"), ("b", "2"))]
    if n == 1:
        # The control for White: a pair that learns the round's White runs alone, the same seeds (does learning
        # the other stakes too make the White play better, or worse?).
        white_data = str(data / "stake1")
        train += [(f"train-white{suffix}", ["scripts/train_value.py", "--data", white_data, "--input-layout",
                                            STAKE_LAYOUT, "--seed", seed,
                                            "--out", f"models/value-stakes-{n}-white{suffix}.bin",
                                            "--report", f"results/value/value-stakes-{n}-white{suffix}.json"])
                  for suffix, seed in (("", "1"), ("b", "2"))]
    teacher = stake_teacher(n, root)
    tail = f"{runs // 1000}k-f{FORMULA}-precise-stakes{n}-w{STAKE_WEIGHT}"
    play: list[tuple] = [("build", [*MAKE, "build"])]
    # White first: the round's test.
    white_out = f"results/value/f{FORMULA}-precise-{white // 1000}k-w{STAKE_WEIGHT}-stakes{n}.json"
    best = best_8k_report(root, leave_out=Path(white_out).name)
    play.append(("white", [*BASELINE, "--bot", "teacher", *teacher, "--runs", str(white), "--seed", "1",
                           "--out", white_out]))
    if best is not None:
        play.append(("compare-white", ["scripts/compare_runs.py", best[2], white_out, "--out",
                                       white_out.replace(".json", "-vs-best.json")]))
    if n == 1:
        control = white_out.replace(".json", "-white-only.json")
        args = [*teacher]
        args[args.index("--value-net") + 1] = f"models/value-stakes-{n}-white.bin,models/value-stakes-{n}-whiteb.bin"
        play += [("white-only", [*BASELINE, "--bot", "teacher", *args, "--runs", str(white), "--seed", "1",
                                 "--out", control]),
                 ("compare-white-only", ["scripts/compare_runs.py", control, white_out, "--out",
                                         white_out.replace(".json", "-vs-white-only.json")])]
    for s in range(1, 9):
        out = f"results/stakes/stake{s}-{tail}.json"
        play += [(f"stake-{s}", [*BASELINE, "--bot", "teacher", *teacher, "--runs", str(runs), "--seed", "1",
                                 "--stake", str(s), "--out", out]),
                 (f"compare-stake-{s}", ["scripts/compare_runs.py", stake_baseline(s), out, "--out",
                                         out.replace(".json", f"-vs-{STAKE_BASELINE}.json")])]
        if n > 1:
            before = f"results/stakes/stake{s}-{runs // 1000}k-f{FORMULA}-precise-stakes{n - 1}-w{STAKE_WEIGHT}.json"
            play.append((f"compare-stake-{s}-round", ["scripts/compare_runs.py", before, out, "--out",
                                                      out.replace(".json", f"-vs-stakes{n - 1}.json")]))
    green = f"results/stakes/stake3-{tail}.json"
    for w in ("1.5", "4"):
        out = green.replace(f"-w{STAKE_WEIGHT}", f"-w{w}")
        args = [*teacher]
        args[args.index("--value-weight") + 1] = w
        play += [(f"green-w{w}", [*BASELINE, "--bot", "teacher", *args, "--runs", str(runs), "--seed", "1",
                                  "--stake", "3", "--out", out]),
                 (f"compare-green-w{w}", ["scripts/compare_runs.py", green, out, "--out",
                                          out.replace(".json", f"-vs-w{STAKE_WEIGHT}.json")])]
    for s in (3, 6):
        base = f"results/stakes/stake{s}-{tail}.json"
        out = base.replace(".json", "-reach-6.json")
        args = [*teacher]
        i = args.index("--teacher-options") + 1
        args[i] = f"{args[i]},reach=6"
        play += [(f"reach-stake{s}", [*BASELINE, "--bot", "teacher", *args, "--runs", str(runs), "--seed", "1",
                                      "--stake", str(s), "--out", out]),
                 (f"compare-reach-stake{s}", ["scripts/compare_runs.py", base, out, "--out",
                                              out.replace(".json", "-vs.json")])]
    return {f"stakes-data-{n}": record,
            f"stakes-net-{n}": [("setup-gpu", [*MAKE, "setup-gpu"]), *train],
            f"stakes-play-{n}": play}


# make boost-night: the champion's judgement, corrected where the runs show it wrong. The stake-aware networks of
# 4 November (make stakes-night) lost to the champion at every stake but Gold (level): a pair that learns 37,000 to
# 68,000 runs can't match fourteen networks that learned about 195,000 White runs, and from round 2 each round
# recorded with the round before's pair, weaker than the champion, so the rounds went in circles (White: 54.2%,
# 58.8%, 55.9% against its 66.5%). What they showed: the other stakes' runs teach White's play too (the pair that
# learned all eight stakes won 54.2% at White, one that learned the same White runs alone 41.6%); only rentals are
# overrated (never buying them: +0.56 blinds beaten at Gold; never buying eternals -1.16 at Black, perishables -0.66
# at Orange); a stake-aware network wants less say (weight 1.5 beat 2.5 at Green each round).
# So the champion's networks stay, and a residual pair learns only what they get wrong, at every stake
# (train_value.py --offset, on recordings that keep the champion's log-odds of each position: baseline
# --offsets-into), starting at no correction and keeping one only as far as its judging runs bear out. It learns
# only from the champion's runs or the best teacher's: each round records every stake with the best teacher measured
# there so far (best_at_stake), so no round learns from a teacher worse than the last.
CHAMPION_2 = DATA / "champion-2"  # .sync/record-champion.sh: more of the champion's runs (seed bases 8001-8008)
BOOST_BUDGET = 110_000  # runs one training holds at most (about 90 KB a run on the GPU's 12 GB)
BOOST_ROUNDS = 12
DELTA_NETS = ",".join(f"models/value-stakes-{n}{b}.bin" for n in (1, 2, 3) for b in ("", "b"))


def residual_nets(n: int) -> str:
    return f"models/value-residual-{n}.bin,models/value-residual-{n}b.bin"


def boost_data(n: int) -> Path:
    """Round n's own recording (from round 2; round 1 learns the night's, stakes-1, and champion-2)."""
    return DATA / f"boost-{n}"


def recorded_runs(folder: Path, planned: int) -> int:
    try:
        return int(json.loads((folder / "manifest.json").read_text())["runs"])
    except (OSError, ValueError, KeyError, TypeError):
        return planned


def boost_folders(n: int) -> tuple[list[Path], int]:
    """The recordings round n's correction learns from, oldest first, and how many of the last are the round's own
    (split to judge and measure): the round's own runs at every stake, then the rounds' before it back to the
    champion's own (champion-2, then stakes-1), as far as BOOST_BUDGET holds. Round 1's own are the champion's,
    both recordings; recordings that aren't there (a stake record-champion.sh didn't get to) are left out."""
    sets = [(boost_data(m), STAKE_RUNS_N) for m in range(n, 1, -1)]
    sets += [(CHAMPION_2, STAKE_RUNS_N), (stake_data(1), STAKE_RUNS_1)]
    chosen, total = [], 0
    for k, (base, sizes) in enumerate(sets):
        folders = [base / f"stake{s}" for s in range(1, 9)]
        if n == 1 or k > 0:
            folders = [f for f in folders if (f / "manifest.json").exists()]
        runs = sum(recorded_runs(f, sizes[int(f.name[5:])]) for f in folders)
        if k > (1 if n == 1 else 0) and total + runs > BOOST_BUDGET:
            break
        chosen.append(folders)
        total += runs
    ordered = [f for group in reversed(chosen) for f in group]
    own = sum(len(g) for g in chosen[:2]) if n == 1 else len(chosen[0]) if chosen else 0
    return ordered, max(1, min(own, len(ordered)))


def best_at_stake(s: int, root: Path = ROOT) -> tuple[list[str], str | None]:
    """The ratchet: the teacher to record stake s with, and its report. At White, the best on the first 8,000 usual
    seeds (best_recording, best_8k). Above it, of every teacher set run by run against 3 November's at that stake
    (the same 2,000 seeds: results/stakes/stake{s}-*-vs-<baseline>.json), the one that went furthest, if it went
    clearly further (paired blinds beaten, the 95% interval above 0); else 3 November's, with the avoid- options the
    trials kept."""
    if s == 1:
        best = best_8k_report(root)
        return (best[1], best[2]) if best else (best_recording(root), None)
    top = None
    for f in sorted((root / "results" / "stakes").glob(f"stake{s}-*-vs-{STAKE_BASELINE}.json")):
        try:
            c = json.loads(f.read_text())
            low, diff = float(c["blinds_beaten_diff_ci"][0]), float(c["blinds_beaten_diff"])
            report = str(c["b"]["report"])
            args = report_args(json.loads((root / report).read_text()))
        except (OSError, ValueError, KeyError, TypeError, IndexError):
            continue
        if low > 0 and (top is None or diff > top[0]):
            top = (diff, args, report)
    if top is not None:
        return top[1], top[2]
    args = baseline_args(root, s)
    kept = stake_options(root)
    if kept:
        if "--teacher-options" in args:
            i = args.index("--teacher-options") + 1
            args = [*args[:i], ",".join([args[i], *kept]), *args[i + 1:]]
        else:
            args = [*args, "--teacher-options", ",".join(kept)]
    # A kept option tried at this very stake: its trial is the report of the teacher this is.
    trials = [stake_trial_name(o, st) for o, st in STAKE_TRIALS if st == s and o in kept]
    return args, f"{trials[0]}.json" if len(trials) == 1 else stake_baseline(s)


def with_correction(args: list[str], nets: str, mode: str) -> list[str]:
    """A teacher's arguments with this stake correction in place of any it had."""
    out, skip = [], False
    for a in args:
        if skip:
            skip = False
            continue
        if a in ("--stake-net", "--stake-mode"):
            skip = True
            continue
        out.append(a)
    return [*out, "--stake-net", nets, "--stake-mode", mode]


def champion_with_kept(root: Path = ROOT) -> list[str]:
    """The champion of 3 November with the avoid- options the trials kept."""
    args = baseline_args(root, 1)
    kept = stake_options(root)
    if kept:
        i = args.index("--teacher-options") + 1 if "--teacher-options" in args else None
        args = [*args, "--teacher-options", ",".join(kept)] if i is None else \
            [*args[:i], ",".join([args[i], *kept]), *args[i + 1:]]
    return args


def with_weight(args: list[str], w: str) -> list[str]:
    out = list(args)
    out[out.index("--value-weight") + 1] = w
    return out


def boost_teacher(n: int, root: Path = ROOT) -> list[str]:
    """Round n's teacher: the champion (3 November's: its networks, weight and options, with the avoid- options the
    trials kept), its chance corrected by round n's residual pair."""
    return with_correction(champion_with_kept(root), residual_nets(n), "residual")


def boost_round(n: int, root: Path = ROOT, runs: int = STAKE_RUNS, white: int | None = None
                ) -> dict[str, list[tuple]]:
    """Round n of the boost, three jobs. boost-data-n: round 1 adds the champion's log-odds to every position of the
    champion's own recordings (stakes-1, champion-2); from round 2 each stake records with the best teacher measured
    there so far (best_at_stake, exploring as round 11 on would; seed bases 9000 + 100 (n - 1) + stake), then adds
    them too. boost-net-n: the residual pair learns, on those and the rounds' before them (boost_folders), what the
    champion's networks get wrong. boost-play-n: the champion with the pair's correction at White on the first 8,000
    usual seeds against the best teacher there, then at every stake above on the 2,000 seeds of 3 November, run by
    run against that day's teacher and against the best so far; round 1 also tries the delta of the stake-aware
    pairs of rounds 1 to 3 (their chance at the stake less at White) on the champion."""
    white = BIG_RUNS if white is None else white
    champion = baseline_args(root, 1)
    c_nets = champion[champion.index("--value-net") + 1]
    w = champion[champion.index("--value-weight") + 1]
    data: list[tuple] = [("build", [*MAKE, "build"])]
    if n == 1:
        # The champion's runs: the night's (stakes-1), and more at every stake (champion-2: recorded here unless
        # .sync/record-champion.sh did, the same runs).
        targets = [f for f in (stake_data(1) / f"stake{s}" for s in range(1, 9)) if (f / "manifest.json").exists()]
        recorder = exploring(baseline_args(root, 1), 11)
        for s in range(1, 9):
            dst = CHAMPION_2 / f"stake{s}"
            if not (dst / "manifest.json").exists():
                data.append((f"record-champion-stake{s}", [*BASELINE, "--bot", "teacher", *recorder,
                                                            "--runs", str(STAKE_RUNS_N[s]), "--seed", str(8000 + s),
                                                            "--stake", str(s), "--record", str(dst),
                                                            "--out", f"results/stakes/champion-2-stake{s}.json"]))
            targets.append(dst)
    else:
        targets = []
        for s in range(1, 9):
            dst = boost_data(n) / f"stake{s}"
            data.append((f"record-stake{s}", [*BASELINE, "--bot", "teacher", *exploring(best_at_stake(s, root)[0], 11),
                                              "--runs", str(STAKE_RUNS_N[s]), "--seed", str(9000 + 100 * (n - 1) + s),
                                              "--stake", str(s), "--record", str(dst),
                                              "--out", f"results/stakes/boost-record-{n}-stake{s}.json"]))
            targets.append(dst)
    for t in targets:
        label = f"{t.parent.name}-{t.name}"
        data.append((f"offsets-{label}", [*BASELINE, "--offsets-into", str(t), "--value-net", c_nets,
                                          "--out", f"results/stakes/offsets-{label}.json"]))
    folders, newest = boost_folders(n)
    train = [(f"train{suffix}", ["scripts/train_value.py", *[x for f in folders for x in ("--data", str(f))],
                                 "--newest", str(newest), "--offset", "--input-layout", STAKE_LAYOUT, "--seed", seed,
                                 "--out", f"models/value-residual-{n}{suffix}.bin",
                                 "--report", f"results/value/value-residual-{n}{suffix}.json"])
             for suffix, seed in (("", "1"), ("b", "2"))]
    teacher = boost_teacher(n, root)
    play: list[tuple] = [("build", [*MAKE, "build"])]
    white_out = f"results/value/f{FORMULA}-precise-{white // 1000}k-w{w}-boost{n}.json"
    best = best_8k_report(root, leave_out=Path(white_out).name)
    play.append(("white", [*BASELINE, "--bot", "teacher", *teacher, "--runs", str(white), "--seed", "1",
                           "--out", white_out]))
    if best is not None:
        play.append(("compare-white", ["scripts/compare_runs.py", best[2], white_out, "--out",
                                       white_out.replace(".json", "-vs-best.json")]))
    variants = [(f"boost{n}", teacher)]
    if n == 1:
        variants.append(("delta-stakes1-3", with_correction(teacher, DELTA_NETS, "delta")))
    for name, args in variants:
        for s in range(2, 9):
            out = f"results/stakes/stake{s}-{runs // 1000}k-f{FORMULA}-precise-{name}-w{w}.json"
            play += [(f"{name}-stake{s}", [*BASELINE, "--bot", "teacher", *args, "--runs", str(runs), "--seed", "1",
                                           "--stake", str(s), "--out", out]),
                     (f"compare-{name}-stake{s}", ["scripts/compare_runs.py", stake_baseline(s), out, "--out",
                                                   out.replace(".json", f"-vs-{STAKE_BASELINE}.json")])]
            so_far = best_at_stake(s, root)[1]
            if so_far and so_far != stake_baseline(s) and so_far != out:
                play.append((f"compare-{name}-stake{s}-best", ["scripts/compare_runs.py", so_far, out, "--out",
                                                               out.replace(".json", "-vs-best.json")]))
    if n == 1:
        # The stake-aware pairs did better with less say at Green; the champion's networks are as far from what
        # they learned there. Both at weight 1.5, against 3 November (a clear gain is a candidate for the ratchet).
        for name, args in (("champion", with_weight(champion_with_kept(root), "1.5")),
                           (f"boost{n}", with_weight(teacher, "1.5"))):
            out = f"results/stakes/stake3-{runs // 1000}k-f{FORMULA}-precise-{name}-w1.5.json"
            play += [(f"{name}-w1.5-stake3", [*BASELINE, "--bot", "teacher", *args, "--runs", str(runs), "--seed",
                                              "1", "--stake", "3", "--out", out]),
                     (f"compare-{name}-w1.5-stake3", ["scripts/compare_runs.py", stake_baseline(3), out, "--out",
                                                      out.replace(".json", f"-vs-{STAKE_BASELINE}.json")])]
    return {f"boost-data-{n}": data,
            f"boost-net-{n}": [("setup-gpu", [*MAKE, "setup-gpu"]), *train],
            f"boost-play-{n}": play}


# make boost-quick: one quick turn of the correction, to build on while someone watches (about 25 minutes): the
# champion's log-odds added to the night's runs (stakes-1: 37,000 of the champion's runs at every stake; skipped once
# there), one residual network trained on them, then the champion with it at White on the first 1,000 usual seeds
# (against the best teacher's 8,000, run by run on those) and at every stake above on the first 500 (against 3
# November's). 500 seeds tell a stake's blinds beaten to about +-0.55, enough to see a big gain or loss; a small
# one is confirmed on 2,000 (make boost-night's sizes). Each turn's files are named by QUICK (results/quick/), so
# turns stay side by side; scripts/quick_table.py prints a turn's table.
QUICK = "quick1"
QUICK_WHITE = 1000
QUICK_RUNS = 500


def quick_steps(root: Path = ROOT, tag: str = QUICK, white: int = QUICK_WHITE, runs: int = QUICK_RUNS
                ) -> list[tuple]:
    champion = baseline_args(root, 1)
    c_nets = champion[champion.index("--value-net") + 1]
    steps: list[tuple] = [("build", [*MAKE, "build"])]
    folders = [f for f in (stake_data(1) / f"stake{s}" for s in range(1, 9)) if (f / "manifest.json").exists()]
    for f in folders:
        steps.append((f"offsets-{f.name}", [*BASELINE, "--offsets-into", str(f), "--value-net", c_nets,
                                            "--out", f"results/quick/offsets-{f.name}.json"]))
    net = f"models/value-residual-{tag}.bin"
    steps += [("setup-gpu", [*MAKE, "setup-gpu"]),
              ("train", ["scripts/train_value.py", *[x for f in folders for x in ("--data", str(f))],
                         "--newest", str(max(1, len(folders))), "--offset", "--input-layout", STAKE_LAYOUT,
                         "--seed", "1", "--out", net, "--report", f"results/quick/residual-{tag}.json"])]
    teacher = with_correction(champion_with_kept(root), net, "residual")
    white_out = f"results/quick/white-{tag}.json"
    steps.append(("white", [*BASELINE, "--bot", "teacher", *teacher, "--runs", str(white), "--seed", "1",
                            "--out", white_out]))
    best = best_8k_report(root)
    if best is not None:
        steps.append(("compare-white", ["scripts/compare_runs.py", best[2], white_out, "--out",
                                        white_out.replace(".json", "-vs-best.json")]))
    for s in range(2, 9):
        out = f"results/quick/stake{s}-{tag}.json"
        steps += [(f"stake-{s}", [*BASELINE, "--bot", "teacher", *teacher, "--runs", str(runs), "--seed", "1",
                                  "--stake", str(s), "--out", out]),
                  (f"compare-stake-{s}", ["scripts/compare_runs.py", stake_baseline(s), out, "--out",
                                          out.replace(".json", "-vs-3nov.json")])]
    steps.append(("table", ["scripts/quick_table.py", tag]))
    return steps


def learned_round(n: int, root: Path = ROOT) -> dict[str, list[tuple]]:
    """Round n (3 or later) of the learned value, three jobs. teacher-data-n: the best teacher there is (the round and
    weight that won most, with "precise" unless its trial found it no better: recording_options; from round 4, the
    trial that beat that teacher clearly, with its networks, if one did: best_recording) plays 25,000 new runs (seed
    base 101 (n + 1): 404, 505, ...) and records every position it values. value-net-n: the round's
    network learns from them and from the round before's recording (more positions; only round n's runs judge and
    measure it). teacher-learned-n: the teacher plays the usual 5,000 seeds with it at weights 1 and 2, without
    options, to set against the earlier rounds on the same footing. From ENDLESS_FROM the recording's wins play on
    into Endless, so its networks also learn how far a win would have gone."""
    data = DATA / f"value-{n}"
    endless = ["--endless"] if n >= ENDLESS_FROM else []
    return {
        f"teacher-data-{n}": [
            ("build", [*MAKE, "build"]),
            ("record", [*BASELINE, "--bot", "teacher",
                        *exploring(best_recording(root) if n >= 4
                                   else [*learned_value_args(root), *recording_options(root)], n),
                        "--runs", "25000", "--seed", str(101 * (n + 1)), *endless, "--record", str(data),
                        "--out", f"results/value/record-{n}.json"]),
        ],
        f"value-net-{n}": [
            ("setup-gpu", [*MAKE, "setup-gpu"]),
            *[(f"train{suffix}", ["scripts/train_value.py", "--data", str(DATA / f"value-{n - 1}"), "--data", str(data),
                                  "--seed", seed, "--out", f"models/value-{n}{suffix}.bin",
                                  "--report", f"results/value/value-{n}{suffix}.json"])
              for suffix, seed in (("", "1"), ("b", "2"))],
        ],
        f"teacher-learned-{n}": [
            ("build", [*MAKE, "build"]),
            ("weight-1", [*BASELINE, "--bot", "teacher", "--value-net", f"models/value-{n}.bin", "--value-weight", "1",
                          "--runs", "5000", "--seed", "1", "--out", f"results/value/teacher{n}-w1.json"]),
            *ensemble_steps(n),
        ],
    }


def ensemble_steps(n: int) -> list[tuple]:
    """The teacher with the networks of rounds 1 to n together, at weights 1 and 1.5 (a better value can take
    more weight: in the cloud, rounds 1 and 2 together with "precise" won 53.25% at 1.5 against 49.75% at 1), on
    the usual 5,000 seeds."""
    return [(f"ensemble-w{w}", [*BASELINE, "--bot", "teacher", "--value-net", ensemble(n), "--value-weight", w,
                                "--runs", "5000", "--seed", "1", "--out", f"results/value/ensemble{n}-w{w}.json"])
            for w in ("1", "1.5")]


# The value formula's numbers tried by make teacher-tune, each beside its usual value (crates/bm-core/src/value.rs:
# PARAMS). In the cloud the long-run term at twice its weight won 2.75 points less, at half its weight 2.75 more
# (400 seeds each; neither clear). A quarter of the precise teacher's losses there came with $25 or more unspent,
# so the money numbers are tried towards spending: money past the interest cap counted at a quarter (52.25% against
# 49.75% with rounds 1 and 2's networks and "precise"; not clear), and faster-falling money (less buying power won
# no more there: 48.75%). The planner's edge over the value's simple players at 0.3 instead of 0.5 won 51.5% there,
# with 0.42 more blinds beaten a run (not clear); the value's chance of beating the next blind is well matched to
# what happens (60 runs: 94.3% predicted for bosses, 94.2% beaten).
TUNE = ("above-cap=0.25", "money-scale=15", "planner-edge=0.3", "long-run=0.35", "sharpness=2.5", "sharpness=5")
# make teacher-tune-2: on the PC (2,000 seeds against the precise teacher on rounds 1 and 2's networks, 49.4%) the
# planner's edge at 0.3 won 52.05% (p = 0.02), sharpness 2.5 51.6% (p = 0.07), long run 0.35 50.7% (not clear), and
# sharpness 5 lost (46.4%). Tried now: the two winners together, with the long run too, and each pushed further.
TUNE_2 = ("planner-edge=0.3,sharpness=2.5", "planner-edge=0.3,sharpness=2.5,long-run=0.35", "planner-edge=0.2",
          "sharpness=2")
# make teacher-ideas: what a player's winning runs (make record-me, make compare-me) showed the bot missing. It
# sold Dusk again and again where the player played rounds down to it on purpose ("final-hand"); the player counts
# Overstock among the strongest vouchers (the value counts it as 15% more for each dollar) and built an economy on
# Midas Mask (counted as $1 a round). Each changes only runs where its joker or voucher shows up, so the paired runs
# show its effect with less noise than its share of all runs would suggest. "five-kind": the player won a run built
# on Five of a Kind (copies of Kings), which the quick search for a round's best hand never played.
IDEAS = ("final-hand", "five-kind", "overstock=0.4", "midas=3")
# The value formula's numbers in force (crates/bm-core/src/value.rs). Version 2: the planner's edge 0.3, sharpness 2.5,
# the long run 0.35, from make teacher-tune-2 (59.85% against 53.4% on the same 2,000 seeds). A trial is set only
# against a control measured with the same numbers: their reports carry the version (f2-...). Version 3: the same
# numbers, with "focus" and "fair-skip" the teacher's own play (make teacher-big: +1.64 points on 8,000 seeds, 64.24%
# against 62.60%, p = 0.001), so trials from 2 November are set against controls that play that way (f3-...).
FORMULA = 3
# make teacher-next, on formula 2: twice the samples behind the teacher's first (quick) look at each option, which on
# the player's 1,325 recorded shop decisions chose better 47 times and worse 12 (judged alike, examples/judge.rs
# --phase SHOP); the player's four ideas together (each won a little alone, none clearly); and two of the player's
# rules: x mult held counts more as the run goes on (42% of the teacher's late losses left a shop at antes 4-7 with $25+
# and no x mult joker), and a clipped deck counts more (the teacher's decks grow to 55 cards, the player's shrink to 46).
NEXT = ("quick-x=2", "final-hand,five-kind,overstock=0.4,midas=3", "xmult-bonus=0.5", "xmult-bonus=1", "thin=0.03")
# make teacher-ideas-2, on formula 2, each against the formula 2 control: from the player's runs and their word.
# "deck-fix": held Tarots and Spectrals that change cards priced by what they would do to the deck (value.rs
# fix_dollars; the player used 47 to 69 Tarots a run, fixing the deck towards its strongest cards); with "glass", a
# card's x mult counted by the times it scores (Glass with a retrigger: "a free win"). "xmult-fit": held x mult that
# fires with the hands the run plays (The Duo with Pairs), at two weights (the plain x mult bonus, which paid for any,
# lost). "card-sharp": the planner tries each hand type first so the hands after it get Card Sharp's x3 (on the 55
# of 300 cloud runs that held Card Sharp: 43 won against 40). Then all of them together.
IDEAS_2 = ("deck-fix", "deck-fix,glass", "xmult-fit=0.3", "xmult-fit=0.6", "card-sharp",
           "deck-fix,glass,xmult-fit=0.3,card-sharp")
# make teacher-focus, on formula 2, each against the control: the player's "focus on one hand". "focus": the hand
# the run has played most is its focus, and its Planets are taken from Celestial packs and bought in the shop
# whenever offered (the teacher's planets went a third to its main hand; the learned value's differences between
# Planets are large and noisy). "fair-skip": a pack's picks judged against the run once the pack is skipped, not
# against the position in the pack, whose learned value also reads the cards on offer. Then both.
FOCUS = ("focus", "fair-skip", "focus,fair-skip")


def file_safe(options: str) -> str:
    return options.replace(",", "+").replace("=", "-")


def tune_control(root: Path = ROOT, tag: str = "") -> tuple[list[str], str, str, str]:
    """The teacher trials are set against: the best learned value's arguments, its report's stem, the options the
    recordings use, and where that teacher's own runs on the first 2,000 usual seeds are kept (with the formula's
    version in the name when a tag is given)."""
    best = best_learned(root)
    net_args, stem = (["--value-net", best[0], "--value-weight", best[1]], Path(best[2]).stem) if best else \
        ([], "teacher-plain")
    base = recording_options(root)[1:]
    base_opts = base[0] if base else ""
    pre = f"{tag}-" if tag else ""
    control = f"results/value/{pre}{file_safe(base_opts)}-on-{stem}.json" if base_opts else \
        f"results/value/{pre}{stem}.json"
    return net_args, stem, base_opts, control


def tune_steps(root: Path = ROOT, changes: tuple[str, ...] = TUNE, tag: str = "") -> list[tuple]:
    """make teacher-tune: the best teacher there is, with the options recordings use (recording_options), tried with
    each of the formula's numbers in TUNE changed (make teacher-tune-2: TUNE_2), on the first 2,000 of the usual
    seeds, each set run by run against the same teacher without the change. That teacher's own runs come from make
    teacher-precise when it has run (the same seeds and options), else this job plays them first. With a tag (the
    formula's version, "f2"), the control and the trials carry it, so a control measured with other numbers is
    never used."""
    net_args, stem, base_opts, control = tune_control(root, tag)
    pre = f"{tag}-" if tag else ""
    steps: list[tuple] = [("build", [*MAKE, "build"])]
    if base_opts and not (root / control).exists():
        steps.append((base_opts, [*BASELINE, "--bot", "teacher", *net_args, "--teacher-options", base_opts,
                                  "--runs", "2000", "--seed", "1", "--out", control]))
    for change in changes:
        opts = f"{base_opts},{change}" if base_opts else change
        out = f"results/tune/{pre}{file_safe(opts)}-on-{stem}.json"
        steps.append((change, [*BASELINE, "--bot", "teacher", *net_args, "--teacher-options", opts, "--runs", "2000",
                               "--seed", "1", "--out", out]))
        steps.append((f"compare {change}", ["scripts/compare_runs.py", control, out, "--out",
                                            f"results/tune/{pre}{file_safe(opts)}-vs-{Path(control).stem}.json"]))
    return steps


def next_steps(root: Path = ROOT) -> list[tuple]:
    """make teacher-next: the trials in NEXT on the formula in force (tune_steps with its version), and round 4's
    networks with the teacher whose numbers made them (all rounds' together at weight 1.5, with "precise"), each
    against the same control on the same 2,000 seeds."""
    tag = f"f{FORMULA}"
    steps = tune_steps(root, NEXT, tag=tag)
    *_, control = tune_control(root, tag)
    out = f"results/tune/{tag}-precise-on-ensemble4-w1.5.json"
    steps.append(("values 1-4", [*BASELINE, "--bot", "teacher", "--value-net", ensemble(4), "--value-weight", "1.5",
                                 "--teacher-options", "precise", "--runs", "2000", "--seed", "1", "--out", out]))
    steps.append(("compare values 1-4", ["scripts/compare_runs.py", control, out, "--out",
                                         f"results/tune/{Path(out).stem}-vs-{Path(control).stem}.json"]))
    return steps


# make teacher-big: the most promising small gain so far (make teacher-focus: focus and fair-skip together, +1.05
# points, p = 0.33 on 2,000 seeds) on 8,000 seeds, where a 1-point gain shows (95% range about +-1 point).
BIG = "focus,fair-skip"
BIG_RUNS = 8000


def big_steps(root: Path = ROOT, change: str = BIG, runs: int = BIG_RUNS) -> list[tuple]:
    """make teacher-big: `change` on the precise teacher with the best learned value, against the same teacher, both
    on the first `runs` usual seeds, run by run. Named so a clear gain is what the next round records with
    (best_recording reads every f2-...-vs-...-on-<networks> comparison)."""
    tag = f"f{FORMULA}"
    net_args, stem, base_opts, _ = tune_control(root, tag)
    size = f"{runs // 1000}k"
    control = f"results/value/{tag}-{file_safe(base_opts)}-{size}-on-{stem}.json"
    opts = f"{base_opts},{change}" if base_opts else change
    out = f"results/tune/{tag}-{file_safe(opts)}-{size}-on-{stem}.json"
    steps: list[tuple] = [("build", [*MAKE, "build"])]
    if not (root / control).exists():
        steps.append(("control", [*BASELINE, "--bot", "teacher", *net_args, "--teacher-options", base_opts,
                                  "--runs", str(runs), "--seed", "1", "--out", control]))
    steps.append((change, [*BASELINE, "--bot", "teacher", *net_args, "--teacher-options", opts,
                           "--runs", str(runs), "--seed", "1", "--out", out]))
    versus = f"results/tune/{tag}-{file_safe(opts)}-{size}-vs-{tag}-{file_safe(base_opts)}-{size}-on-{stem}.json"
    steps.append(("compare", ["scripts/compare_runs.py", control, out, "--out", versus]))
    return steps


# make teacher-weights: how much say the learned value gets against the formula. Round 8's networks won more than
# round 7's when the teacher played with them alone (47.9% against 43.3% on the usual 5,000 seeds, ensembles 57.0%
# against 54.9%), yet not with the precise teacher at weight 1.5 (64.0% against 65.75% on 2,000 seeds, p = 0.12):
# stronger networks may need more weight. Each weight on the first 8,000 usual seeds, where the noise is half that
# of 2,000, set run by run against weight 1.5 (and that against round 7's networks on the same seeds).
WEIGHT_TRIALS = ("2.5", "4")
ROUND_7_8K = "results/tune/f2-precise+focus+fair-skip-8k-on-ensemble7-w1.5.json"  # formula 3's play, before it had a name

# make teacher-weights-high: can the networks carry the teacher with the formula all but out? Round 10's networks went
# 63.5%, 64.9%, 66.1% at weights 1.5, 2.5 and 4 on the first 8,000 usual seeds, still rising. At 8 and 16 the
# formula's numbers barely move a decision. If the rate holds or climbs, the strength is in the networks, and another
# stake or deck is a matter of training them there, not of tuning the formula again by hand. Each is set run by run
# against weight 4 as well as 1.5.
HIGH_WEIGHT_TRIALS = ("8", "16")
HIGH_AGAINST = "4"


def weight_steps(root: Path = ROOT, weights: tuple[str, ...] = WEIGHT_TRIALS, runs: int = BIG_RUNS,
                 against: str | None = None) -> list[tuple]:
    """make teacher-weights: the precise teacher on the newest round's networks (all rounds' together; before any
    ensemble, the best learned value) at weight 1.5 and at each of `weights`,
    on the first `runs` usual seeds, each set run by run against the first (a clear gain is what the next round
    records with: best_recording reads each f3-...-vs-...-on-<networks> comparison and the trial's own weight).
    Runs already played on the same networks are only compared again, so after a new round the job measures that
    round's networks, and before one it costs nothing. `against`: a weight already played on those networks that
    each of `weights` is also set against (make teacher-weights-high: against 4)."""
    tag = f"f{FORMULA}"
    net_args, stem, base_opts, _ = tune_control(root, tag)
    newest = next((n for n in range(LAST_ROUND, 1, -1) if all((root / m).exists() for m in ensemble(n).split(","))),
                  None)
    if newest is not None:  # the newest round's networks: the 5,000-seed tests at weights 1 and 1.5 can miss them
        net_args, stem = ["--value-net", ensemble(newest), "--value-weight", "1.5"], f"ensemble{newest}-w1.5"
    size = f"{runs // 1000}k"
    opts = base_opts or "precise"
    control = f"results/value/{tag}-{file_safe(opts)}-{size}-on-{stem}.json"
    steps: list[tuple] = [("build", [*MAKE, "build"])]
    if not (root / control).exists():
        steps.append(("control", [*BASELINE, "--bot", "teacher", *net_args, "--teacher-options", opts,
                                  "--runs", str(runs), "--seed", "1", "--out", control]))
    if (root / ROUND_7_8K).exists() and "ensemble7-" not in stem:
        steps.append(("vs-round-7", ["scripts/compare_runs.py", ROUND_7_8K, control, "--out",
                                     f"results/tune/{Path(control).stem}-vs-round-7.json"]))
    if "--value-net" not in net_args:
        return steps
    nets = net_args[net_args.index("--value-net") + 1]
    for w in weights:
        out = f"results/tune/{tag}-{file_safe(opts)}-{size}-w{w}-on-{stem}.json"
        if not (root / out).exists():  # already played on these networks: only compared again
            steps.append((f"weight-{w}", [*BASELINE, "--bot", "teacher", "--value-net", nets, "--value-weight", w,
                                          "--teacher-options", opts, "--runs", str(runs), "--seed", "1",
                                          "--out", out]))
        steps.append((f"compare-{w}", ["scripts/compare_runs.py", control, out, "--out",
                                       f"results/tune/{tag}-{file_safe(opts)}-{size}-w{w}-vs-"
                                       f"{tag}-{file_safe(opts)}-{size}-on-{stem}.json"]))
        if against is not None and against != w:
            ref = f"results/tune/{tag}-{file_safe(opts)}-{size}-w{against}-on-{stem}.json"
            vs = f"results/tune/{tag}-{file_safe(opts)}-{size}-w{w}-vs-w{against}-on-{stem}.json"
            steps.append((f"compare-{w}-vs-{against}", ["scripts/compare_runs.py", ref, out, "--out", vs]))
    return steps


# make teacher-doubt: the networks' disagreement as the brake. More say for the networks helped up to weight 4 and then
# fell away (66.1% at 4, 63% at 8, 53.8% at 16 for round 10's): picking the best of many options with a judge that
# errs picks its errors, and the formula is what holds that back. The ensemble's networks disagree most where they
# know least, so teacher option "doubt=k" moves its chance down by k standard deviations of theirs: options they
# disagree about count less. If that is the brake, the networks can take more say (weight 4 with doubt beating the
# champion's 2.5 without). Each against the champion played again on today's code, run by run on the same seeds.
DOUBT_TRIALS = (("2.5", "doubt=1"), ("4", "doubt=1"), ("4", "doubt=2"))


def doubt_steps(root: Path = ROOT, trials: tuple[tuple[str, str], ...] = DOUBT_TRIALS,
                runs: int = BIG_RUNS) -> list[tuple]:
    """make teacher-doubt: the teacher that won most of the first `runs` usual seeds (best_8k) played again, then at
    each (weight, option) of `trials`, each set run by run against it (champion_trials)."""
    return champion_trials(root, trials, runs)


# make teacher-late: more care where the runs are lost. 70% of the champion's losses come at antes 5 to 8, and it
# thinks as hard about ante 1's shop as ante 7's. "late-x=3" triples every count behind its decisions (deals, sample
# contents, tries in rounds) from ante 6 ("late-from"), at about 1.5 times the time a run takes. Against the champion
# played again, run by run.
LATE_TRIALS = (("2.5", "late-x=3"), ("2.5", "late-x=3,late-from=5"))


def late_steps(root: Path = ROOT) -> list[tuple]:
    return champion_trials(root, LATE_TRIALS, BIG_RUNS)


# make teacher-boss: 82% of the champion's losses come at a boss, and the lost blind played again from its start with
# four times the samples was beaten 36% of the time (make diagnose: half of the near misses). "boss-x=4" gives the
# boss blinds' rounds four times the samples at every ante, cheaper than the late budget and aimed where the runs end.
BOSS_TRIALS = (("2.5", "boss-x=4"),)


def boss_steps(root: Path = ROOT) -> list[tuple]:
    return champion_trials(root, BOSS_TRIALS, BIG_RUNS)


# make teacher-budget: how far the search budget goes, screened on 2,000 seeds against the champion's 8,000-seed
# control (a 3-point gain shows on 2,000: the ante-5 budget's 8,000 seeds split 871 to 526). Three times the samples
# from ante 6 won +3.1 points, from ante 5 +4.3 (5 November). Tried: twice from ante 5 (is three the sweet spot, at
# two thirds of the cost?), three times from ante 3 (does starting earlier keep paying?), six times from ante 5 (how
# far does it scale?), and three times everywhere (is it the late antes, or under-searching all along?). The winner
# is then confirmed on 8,000 (make teacher-budget-big), which is what makes it the champion.
BUDGET_TRIALS = (("2.5", "late-x=2"), ("2.5", "late-from=3"), ("2.5", "late-x=6"), ("2.5", "late-from=1"))
SCREEN_RUNS = 2000


def budget_steps(root: Path = ROOT, runs: int = SCREEN_RUNS) -> list[tuple]:
    return champion_trials(root, BUDGET_TRIALS, runs, control_runs=BIG_RUNS)


# make teacher-budget-confirm: the screen's winner on 8,000 seeds, which is what makes it the champion. 5 November's
# screen, against the ante-5 champion's 71.3% on 2,000 seeds: twice from ante 5 71.6% (level, at two thirds of the
# cost), three times from ante 3 73.2% (p = 0.08), six times from ante 5 73.8% (p = 0.004, 163 minutes), three times
# everywhere 77.1% (p < 0.0001, +0.55 blinds a run, 83 minutes): the bot was short of thought at every decision, the
# early antes most of all.
BUDGET_WINNER = (("2.5", "late-from=1"),)


def budget_confirm_steps(root: Path = ROOT) -> list[tuple]:
    return champion_trials(root, BUDGET_WINNER, BIG_RUNS)


# make teacher-skips: skipping a blind for its tag as a move judged by value like any other (teacher option "skips":
# at the blind choice, the skip and the blind are each played out by the teacher on fair copies to where the tag has
# acted, the Boss Blind's choice or the next ante's first, and the positions reached are compared; the skip wins when
# its gain clears the copies' noise). The bot has never skipped (0 a run); its human teacher skips an ante 1 blind for
# an Investment tag, and holds Double tags, and the diagnosis of 5 November put the run's outcome in its economy at
# antes 2 and 3, which is what a tag buys. Screened on 2,000 seeds against the champion (about 30% slower a run).
SKIP_TRIALS = (("2.5", "skips"),)


def skips_steps(root: Path = ROOT, runs: int = SCREEN_RUNS) -> list[tuple]:
    return champion_trials(root, SKIP_TRIALS, runs, control_runs=BIG_RUNS)


# make teacher-budget-6: six times the samples everywhere, screened on 2,000 seeds against the champion of 6 November
# (three times everywhere). Six times from ante 5 beat three times from ante 5 by 3.1 points on the 2,000-seed screen
# of 5 November (p = 0.004), so the budget may not have stopped paying; a run takes about twice as long again.
BUDGET_6 = (("2.5", "late-x=6"),)


def budget_6_steps(root: Path = ROOT, runs: int = SCREEN_RUNS) -> list[tuple]:
    return champion_trials(root, BUDGET_6, runs, control_runs=BIG_RUNS)


# make teacher-budget-confirm-6: six times everywhere on the 8,000 seeds (about 16 hours). The screen of 6 November:
# 79.2% against 77.05% on 2,000 seeds (+2.15, 95% range +0.03 to +4.3, p = 0.053) at four hours for 2,000 runs.
def budget_confirm_6_steps(root: Path = ROOT) -> list[tuple]:
    return champion_trials(root, BUDGET_6, BIG_RUNS)


# make teacher-close: the adaptive budget, screened on 2,000 seeds against the champion of 6 November. Six times the
# samples everywhere gained about 2 points over three times (79.2% against 77.05%, p = 0.053) at twice the time.
# "close-x=K": when a decision's two best options are close (a shop's or pack's within "close-gap" in value, a
# round's two best plans within "close-beats" of the tries in beats), those two are judged again with every count
# scaled by K; the rest of the decisions cost nothing more. On the champion's 3x budget, close-x=2 judges close calls
# at 6x for about 30% more time a run, close-x=3 at 9x for about 45%.
CLOSE_TRIALS = (("2.5", "close-x=2"), ("2.5", "close-x=3"))


def close_steps(root: Path = ROOT, runs: int = SCREEN_RUNS) -> list[tuple]:
    return champion_trials(root, CLOSE_TRIALS, runs, control_runs=BIG_RUNS)


# make teacher-close-2: the same screen of the option as fixed on 7 November. The first screen's reports (CLOSE_TRIALS,
# 6 November: close-x=2 75.3%, close-x=3 74.6% against 77.05%) are of a version that, having judged the two close
# options again, let a third option or doing nothing win at its first, unchecked estimate; the two judged again come
# down on average, so the bot bought less (packs 17.2 and 16.9 a run against the champion's 18.0; money at the end
# 58 and 60 against 55) and lost 1.75 and 2.45 points. Fixed, the choice is between the two alone. The trials carry
# "close-gap" spelt out so that their reports are new files.
CLOSE_TRIALS_2 = (("2.5", "close-x=2,close-gap=0.1"), ("2.5", "close-x=3,close-gap=0.1"))


def close_2_steps(root: Path = ROOT, runs: int = SCREEN_RUNS) -> list[tuple]:
    return champion_trials(root, CLOSE_TRIALS_2, runs, control_runs=BIG_RUNS)


# make skips-stakes: the same option where a skip should be worth more. At White it came out level (step 30: 76.85%
# against 77.05%, 0.34 skips a run), where every blind pays and the blinds are small. From Red Stake up the Small
# Blind pays nothing, so skipping it costs only its shop; at Green and Purple the blinds grow faster, so what a tag
# buys (an Investment's $25, an edition joker) is a larger share of what the run needs. Each stake's best teacher
# (best_at_stake: the ratchet's) with "skips", on the stake's 2,000 seeds against that teacher's own report; a stake
# where the skips take the runs clearly further (paired blinds beaten, the 95% interval above 0) is adopted by the
# ratchet by itself, since the comparison is named as its trials are.
SKIP_STAKES = (2, 3, 6)


def skips_stakes_steps(root: Path = ROOT, stakes: tuple[int, ...] = SKIP_STAKES, runs: int = STAKE_RUNS
                       ) -> list[tuple]:
    steps: list[tuple] = [("build", [*MAKE, "build"])]
    for s in stakes:
        args, report = best_at_stake(s, root)
        if report is None:
            continue
        args = list(args)
        if "--teacher-options" in args:
            i = args.index("--teacher-options") + 1
            if "skips" in args[i].split(","):
                continue
            args[i] = f"{args[i]},skips"
        else:
            args += ["--teacher-options", "skips"]
        stem = Path(report).stem.removeprefix(f"stake{s}-")
        out = f"results/stakes/stake{s}-skips-on-{stem}.json"
        steps += [(f"skips-stake{s}", [*BASELINE, "--bot", "teacher", *args, "--runs", str(runs), "--seed", "1",
                                       "--stake", str(s), "--out", out]),
                  (f"compare-skips-stake{s}", ["scripts/compare_runs.py", report, out, "--out",
                                               f"results/stakes/stake{s}-skips-on-{stem}-vs-{stem}.json"])]
    return steps


# Whether the champion is played again on today's code before its trials (its own report serves otherwise: the engine
# hasn't changed its play since 4 November's fairness fixes, which moved the champion of 2 November by 0.1 point).
CONTROL_AGAIN = False


def champion_trials(root: Path, trials: tuple[tuple[str, str], ...], runs: int,
                    control_runs: int | None = None) -> list[tuple]:
    """The teacher that won most of the first 8,000 usual seeds (best_8k), at each (weight, option) of `trials` on
    the first `runs` seeds, each set run by run against the champion's own runs (its report; or, with CONTROL_AGAIN
    or an "again" report there, the champion played again on today's code on `control_runs` seeds, once). The
    comparison pairs the first seeds of both, so a 2,000-seed screen compares against the 8,000-seed champion. A
    trial's `name=value` replaces the champion's own setting of that name, so "late-x=6" on a champion with late-x=3
    plays at 6."""
    steps: list[tuple] = [("build", [*MAKE, "build"])]
    best = best_8k_report(root)
    if best is None:
        return steps
    _, args, report = best
    args = list(args)
    opts = args[args.index("--teacher-options") + 1] if "--teacher-options" in args else "precise"
    w0 = args[args.index("--value-weight") + 1] if "--value-weight" in args else "1"
    nets_stem = Path(report).stem.split("-on-")[-1]
    control_runs = control_runs or runs
    tag, size, control_size = f"f{FORMULA}", f"{runs // 1000}k", f"{control_runs // 1000}k"

    def with_(w: str, o: str) -> list[str]:
        a = list(args)
        a[a.index("--value-weight") + 1] = w
        if "--teacher-options" in a:
            a[a.index("--teacher-options") + 1] = o
        else:
            a += ["--teacher-options", o]
        return a
    # The control: the champion's own report (the runs that made it the champion, on the same seeds), unless it was
    # played on an older engine (an "again" report exists or is wanted: CONTROL_AGAIN), when it is played again on
    # today's code once and that is the control every trial shares.
    again = f"results/tune/{tag}-{file_safe(opts)}-{control_size}-w{w0}-again-on-{nets_stem}.json"
    control = report if not CONTROL_AGAIN and not (root / again).exists() else again
    if control == again and not (root / again).exists():
        steps.append(("champion", [*BASELINE, "--bot", "teacher", *with_(w0, opts), "--runs", str(control_runs),
                                   "--seed", "1", "--out", control]))
    for w, extra in trials:
        names = {e.split("=")[0] for e in extra.split(",") if "=" in e}
        kept = [e for e in opts.split(",") if e and e.split("=")[0] not in names]
        o = ",".join([*kept, extra])
        out = f"results/tune/{tag}-{file_safe(o)}-{size}-w{w}-on-{nets_stem}.json"
        if not (root / out).exists():
            steps.append((f"w{w}-{extra}", [*BASELINE, "--bot", "teacher", *with_(w, o), "--runs", str(runs),
                                            "--seed", "1", "--out", out]))
        steps.append((f"compare-w{w}-{extra}", ["scripts/compare_runs.py", control, out, "--out",
                                                f"results/tune/{Path(out).stem}-vs-{Path(control).stem}.json"]))
    return steps


# make teacher-margin: aim for wins with blinds to spare. 82% of the champion's losses on the exam's seeds came at a
# boss, most at antes 5 to 8: runs it rated as won until a boss they couldn't take. Teacher option "margin=K" rates a
# position by the chance of winning and then beating K more blinds in Endless (the networks' win chance times their
# depth head's share at K or more), so a build that only just wins counts for less than one with room to spare. K = 3
# asks for ante 9 cleared, K = 6 for ante 10. Only networks from ENDLESS_FROM on have the depth head; the others' win
# chances are used as they are.
MARGIN_TRIALS = ("margin=3", "margin=6")


def margin_steps(root: Path = ROOT, trials: tuple[str, ...] = MARGIN_TRIALS, runs: int = BIG_RUNS) -> list[tuple]:
    """make teacher-margin: the champion's weight and options (best_8k_report) on the newest round's networks that
    learned how far a win goes on (all rounds' together, from ENDLESS_FROM), without and with each of `trials`, on
    the first `runs` usual seeds, each set run by run against it without. The run without is make teacher-weights'
    at that weight on those networks when it has played (same name), else it is played first. Nothing to do before
    those networks exist."""
    steps: list[tuple] = [("build", [*MAKE, "build"])]
    best = best_8k_report(root)
    newest = next((n for n in range(LAST_ROUND, ENDLESS_FROM - 1, -1)
                   if all((root / m).exists() for m in ensemble(n).split(","))), None)
    if best is None or newest is None:
        return steps
    args = best[1]
    w = args[args.index("--value-weight") + 1] if "--value-weight" in args else "1.5"
    opts = args[args.index("--teacher-options") + 1] if "--teacher-options" in args else "precise"
    opts = ",".join(o for o in opts.split(",") if o and not o.startswith("margin=")) or "precise"
    tag, size, stem = f"f{FORMULA}", f"{runs // 1000}k", f"ensemble{newest}-w1.5"

    def play(o: str, out: str) -> list[str]:
        return [*BASELINE, "--bot", "teacher", "--value-net", ensemble(newest), "--value-weight", w,
                "--teacher-options", o, "--runs", str(runs), "--seed", "1", "--out", out]
    control = f"results/tune/{tag}-{file_safe(opts)}-{size}-w{w}-on-{stem}.json"
    if not (root / control).exists():
        steps.append(("without", play(opts, control)))
    for extra in trials:
        o = f"{opts},{extra}"
        out = f"results/tune/{tag}-{file_safe(o)}-{size}-w{w}-on-{stem}.json"
        if not (root / out).exists():
            steps.append((extra, play(o, out)))
        steps.append((f"compare-{extra}", ["scripts/compare_runs.py", control, out, "--out",
                                           f"results/tune/{Path(out).stem}-vs-{Path(control).stem}.json"]))
    return steps


def replayable_recording(data: Path = DATA) -> tuple[str, Path] | None:
    """The newest recording of the best teacher's White Stake runs that can be played again by its moves (its
    manifest says so: recordings keep every move from round 11 on), as (its name, its folder): the newest round's,
    else the champion's own White runs (champion-2/stake1 when .sync/record-champion.sh made it, else the night of 4
    November's stakes-1/stake1: 3,000 runs, exploring). None without one."""
    candidates = [(f"round{n}", data / f"value-{n}") for n in range(LAST_ROUND, 0, -1)]
    candidates += [("champion-2", data / CHAMPION_2.name / "stake1"), ("night", data / stake_data(1).name / "stake1")]
    for name, folder in candidates:
        try:
            m = json.loads((folder / "manifest.json").read_text())
        except (OSError, ValueError):
            continue
        if m.get("moves_kept") is True and int(m.get("stake") or 1) == 1:
            return name, folder
    return None


# make diagnose: where the champion's runs are lost, and why, from the newest recording (its runs played again by
# their moves, in minutes). Every lost blind is played again from its start by the champion with four times the usual
# samples (a blind is a second or so): a blind that deeper play beats was lost to play, not to the build.
DEEP_X = 4
DEEP_SHARE = 1.0


def diagnose_steps(root: Path = ROOT, data: Path = DATA) -> list[tuple]:
    """make diagnose: `baseline --diagnose` on the newest replayable recording (replayable_recording) with the
    champion (best_8k_report: its networks and options play the lost blinds again), then scripts/diagnose.py's
    tables. Nothing before a champion and a recording."""
    steps: list[tuple] = [("build", [*MAKE, "build"])]
    best, rec = best_8k_report(root), replayable_recording(data)
    if best is None or rec is None:
        return steps
    name, folder = rec
    table = f"results/diagnose/{name}.csv"
    steps.append(("replay", [*BASELINE, "--bot", "teacher", *best[1], "--replay", str(folder), "--diagnose", table,
                             "--deep", str(DEEP_X), "--deep-share", str(DEEP_SHARE),
                             "--out", f"results/diagnose/{name}.json"]))
    steps.append(("table", ["scripts/diagnose.py", table, "--out", f"results/diagnose/{name}-summary.json"]))
    return steps


# make diagnose-fresh: the champion's own runs, recorded today and diagnosed. The night recording of 4 November is the
# champion of 5 November exploring (one shop move in ten taken at random among the near-equal), so its base win rate
# is 61% where the champion of 6 November wins 75%, and the page's "how it wins" and "where it loses" read from it.
# This records DIAGNOSE_RUNS runs with the champion as it plays (no exploring, fresh seeds, every move kept), then
# diagnoses them like make diagnose. About 2 hours for the recording at the champion's budget, then the diagnosis.
DIAGNOSE_RUNS = 3000
DIAGNOSE_SEED = 8150


def diagnose_fresh_steps(root: Path = ROOT, data: Path = DATA) -> list[tuple]:
    steps: list[tuple] = [("build", [*MAKE, "build"])]
    best = best_8k_report(root)
    if best is None:
        return steps
    folder = data / "diagnose-champion" / "stake1"
    name = "champion"
    if not (folder / "manifest.json").exists():
        steps.append(("record", [*BASELINE, "--bot", "teacher", *best[1], "--runs", str(DIAGNOSE_RUNS),
                                 "--seed", str(DIAGNOSE_SEED), "--record", str(folder),
                                 "--out", f"results/diagnose/{name}-record.json"]))
    table = f"results/diagnose/{name}.csv"
    steps.append(("replay", [*BASELINE, "--bot", "teacher", *best[1], "--replay", str(folder), "--diagnose", table,
                             "--deep", str(DEEP_X), "--deep-share", str(DEEP_SHARE),
                             "--out", f"results/diagnose/{name}.json"]))
    steps.append(("table", ["scripts/diagnose.py", table, "--out", f"results/diagnose/{name}-summary.json"]))
    return steps


# Forks (crates/bm-core/src/forks.rs): at a share FORK_CHANCE of the shops entered and packs opened in antes 2 to 6
# of the newest replayable recording's runs (played again by their moves; a run enters about 40 of them), the
# champion's FORK_BEST best options and doing nothing (and its own choice and the run's move, if others) are each
# played out to the end by the champion on FORK_COPIES fair copies that share their seeds across the options, the
# play-outs at the lighter shop budget (FORK_PLAY: the same for every option, so the comparison is fair). A fork
# costs about a dozen full games; a night of it (FORK_SECONDS) forks about 7,000 decisions over the night recording's
# 3,000 runs. Each fork says which option went best here, where a recording's one result per run never did; and how
# often the champion's own choice was that option is the judge test, measured in minutes instead of three hours of
# play. The quick turn (make forks-quick) forks for an hour, for the judge test today. The fork files add up: every
# round's train together (fork_files).
# A night's share is set from its time: the night recording's 3,000 runs enter about 120,000 shops and packs, and a
# fork takes about 1.6 seconds on 16 threads, so a share of 0.06 ran out of runs in three hours (4 November's
# night); the share is chosen so the runs last the time given (fork_chance).
FORK_CHANCE = 0.06
FORK_SECONDS_A_FORK = 1.6
FORK_DECISIONS_A_RUN = 40
FORK_COPIES = 4
FORK_BEST = 2
FORK_PLAY = "light"
FORK_SECONDS = 9 * 3600
FORK_QUICK_SECONDS = 3600
FORK_ROUNDS = 4


def fork_chance(seconds: int, runs: int) -> float:
    """The share of decisions forked so that `runs` recorded runs last `seconds` (at least FORK_CHANCE, at most 1)."""
    wanted = seconds / FORK_SECONDS_A_FORK
    return round(min(1.0, max(FORK_CHANCE, wanted / max(runs * FORK_DECISIONS_A_RUN, 1))), 3)


def fork_steps(name: str, seconds: int, root: Path = ROOT, data: Path = DATA) -> list[tuple]:
    """The forks job `name` (forks-quick, forks-1...): the champion forks the newest replayable recording's decisions
    for `seconds` (a share of them chosen so the recording lasts that long: fork_chance), into DATA/<name>, with its
    report (the judge test) in results/forks/<name>.json."""
    steps: list[tuple] = [("build", [*MAKE, "build"])]
    best, rec = best_8k_report(root), replayable_recording(data)
    if best is None or rec is None:
        return steps
    _, folder = rec
    runs = recorded_runs(folder, 3000)
    # Each fork job forks its own decisions of the recording (forks-quick 0, forks-1 1, forks-2 2...).
    salt = 0 if name == "forks-quick" else int(name.rsplit("-", 1)[-1])
    steps.append(("fork", [*BASELINE, "--bot", "teacher", *best[1], "--replay", str(folder), "--forks",
                           str(data / name), "--fork-chance", str(fork_chance(seconds, runs)), "--fork-salt",
                           str(salt), "--fork-copies", str(FORK_COPIES),
                           "--fork-best", str(FORK_BEST), "--fork-play-options", FORK_PLAY, "--fork-antes", "2-6",
                           "--seconds", str(seconds), "--out", f"results/forks/{name}.json"]))
    steps.append(("table", ["scripts/fork_table.py", *map(str, fork_files(name, data)),
                            "--out", f"results/forks/{name}-table.json"]))
    return steps


def fork_table_steps(data: Path = DATA) -> list[tuple]:
    """make forks-table (run directly, beside any job): the judge test from every fork file there is."""
    files = [data / n for n in ["forks-quick", *(f"forks-{k}" for k in range(1, FORK_ROUNDS + 1))]
             if (data / n / "manifest.json").exists()]
    if not files:
        return []
    return [("table", ["scripts/fork_table.py", *map(str, files), "--out", "results/forks/table.json"])]


def fork_net_steps(name: str, root: Path = ROOT, data: Path = DATA) -> list[tuple]:
    """The networks that learn from the forks `name` as well as the newest two rounds' positions (a pair, seeds 1
    and 2: models/value-<name>.bin and -b), and a control pair that learns the same positions without the forks
    (models/value-<name>-control.bin and -b), so the forks' part shows on the same footing. Each report's judge test
    (`measured.forks`) sets the network, the champion's networks, the teacher's quick look and its own choice against
    the option that went best, on forks kept apart."""
    rounds = [n for n in range(LAST_ROUND, 0, -1) if (data / f"value-{n}" / "manifest.json").exists()][:2]
    if not rounds:
        return [("setup-gpu", [*MAKE, "setup-gpu"])]
    positions = [a for n in sorted(rounds) for a in ("--data", str(data / f"value-{n}"))]
    # Every fork file there is, this one last (the quick turn's and the rounds' before: they add up).
    forks = [a for f in fork_files(name, data) for a in ("--forks", str(f))]
    steps: list[tuple] = [("setup-gpu", [*MAKE, "setup-gpu"])]
    for suffix, seed in (("", "1"), ("b", "2")):
        steps.append((f"train{suffix}", ["scripts/train_value.py", *positions, *forks, "--fork-split",
                                         str(FORK_SPLIT), "--seed", seed, "--out", f"models/value-{name}{suffix}.bin",
                                         "--report", f"results/value/value-{name}{suffix}.json"]))
        steps.append((f"control{suffix}", ["scripts/train_value.py", *positions, *forks, "--pair-weight", "0",
                                           "--fork-split", str(FORK_SPLIT), "--seed", seed,
                                           "--out", f"models/value-{name}-control{suffix}.bin",
                                           "--report", f"results/value/value-{name}-control{suffix}.json"]))
    return steps


def fork_files(name: str, data: Path = DATA) -> list[Path]:
    """The fork recordings the networks of `name` learn from: the quick turn's and every round's before `name`
    (those recorded: a manifest there, a partial one too, since a fork recording can be read while it is made),
    then `name`'s. For "forks-now" (make forks-now, between rounds): every fork file there is."""
    order = ["forks-quick", *(f"forks-{k}" for k in range(1, FORK_ROUNDS + 1))]
    upto = order.index(name) if name in order else len(order)
    files = [data / n for n in order[:upto] if (data / n / "manifest.json").exists()]
    return files if name == "forks-now" else [*files, data / name]


# make forks-now: while the night's forks come in, the pair at three pair weights and its control on every fork so
# far (a night's recording can be read while it is made), the judge test of each on forks kept apart (a fifth of
# the forks' runs), then one table (scripts/forks_report.py). About 20 minutes on the GPU, beside the forking.
FORK_SPLIT = 5
NOW_WEIGHTS = ("0.3", "1", "3")


def fork_now_steps(root: Path = ROOT, data: Path = DATA) -> list[tuple]:
    rounds = [n for n in range(LAST_ROUND, 0, -1) if (data / f"value-{n}" / "manifest.json").exists()][:2]
    files = fork_files("forks-now", data)
    if not rounds or not files:
        return [("setup-gpu", [*MAKE, "setup-gpu"])]
    positions = [a for n in sorted(rounds) for a in ("--data", str(data / f"value-{n}"))]
    forks = [a for f in files for a in ("--forks", str(f))]
    steps: list[tuple] = [("setup-gpu", [*MAKE, "setup-gpu"])]
    reports = []
    for w in ("0", *NOW_WEIGHTS):
        report = f"results/value/value-forks-now-pw{w}.json"
        reports.append(report)
        steps.append((f"pw{w}", ["scripts/train_value.py", *positions, *forks, "--pair-weight", w, "--fork-split",
                                 str(FORK_SPLIT), "--seed", "1", "--out", f"models/value-forks-now-pw{w}.bin",
                                 "--report", report]))
    steps.append(("table", ["scripts/forks_report.py", *reports, "--out", "results/forks/now-table.json"]))
    return steps


def fork_play_steps(name: str, root: Path = ROOT, runs: int = BIG_RUNS) -> list[tuple]:
    """How the fork-trained pair plays: on the first `runs` usual seeds, run by run, the fork pair alone against the
    control pair alone (both at the champion's weight: the forks' part in play), and the champion's networks with the
    fork pair added against the champion played again (champion_trials' control)."""
    steps: list[tuple] = [("build", [*MAKE, "build"])]
    best = best_8k_report(root)
    if best is None:
        return steps
    _, args, report = best
    args = list(args)
    w = args[args.index("--value-weight") + 1] if "--value-weight" in args else "2.5"
    opts = args[args.index("--teacher-options") + 1] if "--teacher-options" in args else "precise"
    nets = args[args.index("--value-net") + 1]
    nets_stem = Path(report).stem.split("-on-")[-1]
    tag, size = f"f{FORMULA}", f"{runs // 1000}k"
    pair = f"models/value-{name}.bin,models/value-{name}b.bin"
    control_pair = f"models/value-{name}-control.bin,models/value-{name}-controlb.bin"

    def play(label: str, value_net: str, out: str) -> tuple:
        return (label, [*BASELINE, "--bot", "teacher", "--value-net", value_net, "--value-weight", w,
                        "--teacher-options", opts, "--runs", str(runs), "--seed", "1", "--out", out])
    alone = f"results/tune/{tag}-{file_safe(opts)}-{size}-w{w}-on-{name}.json"
    control = f"results/tune/{tag}-{file_safe(opts)}-{size}-w{w}-on-{name}-control.json"
    steps.append(play("pair", pair, alone))
    steps.append(play("control-pair", control_pair, control))
    steps.append(("compare-pairs", ["scripts/compare_runs.py", control, alone, "--out",
                                    f"results/tune/{Path(alone).stem}-vs-{Path(control).stem}.json"]))
    again = f"results/tune/{tag}-{file_safe(opts)}-{size}-w{w}-again-on-{nets_stem}.json"
    champion = report if not CONTROL_AGAIN and not (root / again).exists() else again
    if champion == again and not (root / again).exists():
        steps.append(play("champion", nets, champion))
    joined = f"results/tune/{tag}-{file_safe(opts)}-{size}-w{w}-on-{nets_stem}+{name}.json"
    steps.append(play("champion-with-pair", f"{nets},{pair}", joined))
    steps.append(("compare-with-champion", ["scripts/compare_runs.py", champion, joined, "--out",
                                            f"results/tune/{Path(joined).stem}-vs-{Path(champion).stem}.json"]))
    return steps


def fork_round(k: int, root: Path = ROOT, data: Path = DATA) -> dict[str, list[tuple]]:
    """Round k of the forks, three jobs (make forks-night runs them in a row): forks-k (a night of forks),
    value-net-forks-k (the pair and its control), teacher-forks-k (how they play)."""
    name = f"forks-{k}"
    return {
        name: fork_steps(name, FORK_SECONDS, root, data),
        f"value-net-{name}": fork_net_steps(name, root, data),
        f"teacher-{name}": fork_play_steps(name, root),
    }


def control_steps(root: Path = ROOT) -> list[tuple]:
    """make teacher-control: the precise teacher on the best learned value there is (best_learned: after a round, its
    networks if they won most) on the first 2,000 usual seeds, the control later trials are set against, and set run
    by run against the control before it (the one on the highest round's networks below it)."""
    tag = f"f{FORMULA}"
    steps = tune_steps(root, (), tag=tag)
    *_, control = tune_control(root, tag)

    def rounds(name: str) -> int:
        digits = "".join(c for c in name.split("-on-")[-1].split("-w")[0] if c.isdigit())
        return int(digits) if digits else 0

    older = [f for f in (root / "results" / "value").glob(f"{tag}-precise-on-*.json")
             if f.name != Path(control).name and rounds(f.name) < rounds(Path(control).name)]
    if older:
        prev = max(older, key=lambda f: (rounds(f.name), f.name))
        prev_rel = f"results/value/{prev.name}"
        steps.append(("compare", ["scripts/compare_runs.py", prev_rel, control, "--out",
                                  f"results/tune/{Path(control).stem}-vs-{prev.stem}.json"]))
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
    # Rounds 1 and 2's networks together (round 2 alone won no more than round 1): the usual 5,000 seeds, to set
    # against each alone (teacher-w1.json, teacher2-w1.json). If it wins most, the jobs after it use it.
    "teacher-ensemble": [("build", [*MAKE, "build"]), *ensemble_steps(2)],
    # The teacher's option "precise" (crates/bm-core/src/value.rs: `Scales`): twice the sample contents behind pack
    # and reroll decisions, its noisiest, and twice the tries behind each plan for a round, at about 1.45 times the
    # time. It is tried on the best teacher there is (option_trial). On 400 seeds in the cloud with round 1's value
    # it won 45.0% against 38.5% (p = 0.03); 2,000 seeds on the PC tell more.
    "teacher-precise": option_trial("precise"),
    # The value formula's numbers, each changed in turn on the best teacher there is (tune_steps, TUNE): about an hour.
    "teacher-tune": tune_steps(),
    "teacher-tune-2": tune_steps(changes=TUNE_2),
    "teacher-ideas": tune_steps(changes=IDEAS),
    "teacher-next": next_steps(),
    "teacher-ideas-2": tune_steps(changes=IDEAS_2, tag=f"f{FORMULA}"),
    "teacher-focus": tune_steps(changes=FOCUS, tag=f"f{FORMULA}"),
    "teacher-control": control_steps(),
    "teacher-big": big_steps(),
    "teacher-weights": weight_steps(),
    "teacher-weights-high": weight_steps(weights=HIGH_WEIGHT_TRIALS, against=HIGH_AGAINST),
    "teacher-doubt": doubt_steps(),
    "teacher-margin": margin_steps(),
    "teacher-late": late_steps(),
    "teacher-boss": boss_steps(),
    "teacher-budget": budget_steps(),
    "teacher-budget-big": budget_steps(runs=BIG_RUNS),
    "teacher-budget-confirm": budget_confirm_steps(),
    "teacher-skips": skips_steps(),
    "skips-stakes": skips_stakes_steps(),
    "teacher-budget-6": budget_6_steps(),
    "teacher-budget-confirm-6": budget_confirm_6_steps(),
    "teacher-close": close_steps(),
    "teacher-close-2": close_2_steps(),
    "diagnose-fresh": diagnose_fresh_steps(),
    "diagnose": diagnose_steps(),
    # An hour of forks and the networks that learn from them: the judge test today (results/forks/forks-quick.json,
    # then results/value/value-forks-quick.json's measured.forks).
    "forks-quick": fork_steps("forks-quick", FORK_QUICK_SECONDS),
    "forks-now": fork_now_steps(),
    "value-net-forks-quick": fork_net_steps("forks-quick"),
    **{name: steps for k in range(1, FORK_ROUNDS + 1) for name, steps in fork_round(k).items()},
    "teacher-inputs": input_steps(),
    "stakes-trials": stake_trials_steps(),
    "boost-quick": quick_steps(),
    # The stake-aware networks, round by round (stake_round): make stakes-night runs round 1, the trials, then
    # rounds 2, 3... until stopped.
    **{name: steps for n in range(1, STAKE_ROUNDS + 1) for name, steps in stake_round(n).items()},
    # The champion with a residual correction, round by round (boost_round): make boost-night runs them until stopped.
    **{name: steps for n in range(1, BOOST_ROUNDS + 1) for name, steps in boost_round(n).items()},
    "teacher-stakes": stakes_steps(),
    "teacher-exam": exam_steps(),
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


def public(text: str) -> str:
    """Text that goes into a committed result with the home folder as ~ (the repo can be public)."""
    home = str(Path.home())
    return text.replace(home + "/", "~/").replace(home, "~") if home not in ("", "/") else text


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
    return {"host": HOST, "platform": platform.platform(), "cpu_threads": os.cpu_count(),
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
    if r.get("bot") == "replayed runs":  # a recording encoded afresh: the runs it already counted
        return None
    code, title = code_of(commit)
    net, weight = arg("--value-net"), arg("--value-weight")
    w = float(weight) if weight else (1.0 if net else None)
    bot = str(r.get("bot") or "bot")
    rnd = re.search(r"value-(\d+)\.bin$", net or "")
    layout = re.search(r"value-(\d+)-layout(\d+)b?\.bin", net or "")
    stakes = re.search(r"value-stakes-(\d+)(-white)?b?\.bin", net or "")
    if stakes:  # make stakes-night: one round's stake-aware pair (or its control, on White's runs alone)
        name = f"{'White-only' if stakes[2] else 'Stake-aware'} values {stakes[1]} \u00d7{w:g}"
    elif layout:  # make teacher-inputs: one round's networks on one input layout
        name = f"Learned value {layout[1]}, input layout {layout[2]} ×{w:g}"
    elif net and "," in net:
        rounds = re.findall(r"value-(\d+)\.bin", net)
        name = f"Learned values {'+'.join(rounds)} \u00d7{w:g}"
    elif net:
        name = f"Learned value{' ' + rnd[1] if rnd and rnd[1] != '1' else ''} \u00d7{w:g}"
    elif bot == "network":
        name = "Network, copying the teacher" if step == "eval-imitate" else "Network after PPO"
    else:
        name = bot[:1].upper() + bot[1:].removesuffix(" bot")
    stake_net = arg("--stake-net")
    if stake_net:  # make boost-night: the champion's chance corrected by a stake-aware pair
        rr = re.search(r"value-residual-([a-z]*\d+)b?\.bin", stake_net)
        name += f" + residual {rr[1]}" if rr else " + stake delta"
    options = arg("--teacher-options")
    if options:
        name += ", " + options.replace(",", ", ")
    if r.get("stake") and r.get("stake") != "White Stake":  # the Progress panel's lines are White's otherwise
        name += f", {r['stake']}"
    return {"time": now(), "name": name, "job": job, "step": step, "code": code, "title": title, "out": out,
            "bot": r.get("bot"), "stake": int(r.get("stake_number") or 1),
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
             "commands": {s[0]: public(" ".join(f"{k}={v}" for k, v in (s[2] if len(s) > 2 else {}).items())
                                       + (" " if len(s) > 2 else "") + shlex.join(s[1])) for s in JOBS[job]}}
    runs = ROOT / "results" / "runs"
    runs.mkdir(parents=True, exist_ok=True)
    (runs / f"{job}.json").write_text(json.dumps(stamp, indent=1) + "\n")
    git("add", "--all", "results")
    if git("diff", "--cached", "--name-only"):
        git("commit", "--quiet", "-m", f"results({job}): {stamp['machine']['cpu_threads']} threads, "
                                       f"code {st['commit'][:9]}")
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
