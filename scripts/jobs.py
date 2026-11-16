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
    PC found it no better."""
    for f in sorted((root / "results" / "value").glob("precise-vs-*.json")):
        try:
            diff = json.loads(f.read_text()).get("win_rate_diff")
        except (OSError, ValueError, AttributeError):
            continue
        if isinstance(diff, (int, float)) and diff <= 0:
            return []
    return ["--teacher-options", "precise"]


# A trial's gain over the precise teacher on the same seeds is trusted for a recording below this p-value.
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
    """The teacher that won most of the first 8,000 usual seeds (on the formula in force, any
    networks and weight: the same seeds, so their rates compare directly), as its win rate and arguments
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
            # A salted report draws the bot's samples afresh: its rate is not paired with the others', so a
            # champion changes on a rule (RUNNING step 37), not by a higher number at another salt.
            if r.get("salt", 0):
                continue
            if (best is None or rate > best[0]) and f.name != leave_out:
                best = (rate, report_args(r), str(f.relative_to(root)))
    return best


def best_recording(root: Path = ROOT) -> list[str]:
    """The teacher a round records with, as its arguments (learned value, weight, options): the one that won most of the
    first 8,000 usual seeds when one has played them (best_8k: each new round's networks were measured at several
    weights there); before that, of the trials on the best learned value (results/tune: each change set
    run by run against the precise teacher on the same 2,000 seeds), the one that won most over it, if it won clearly
    (p below RECORD_WITH_P), with the networks it played with (a trial can be other networks); else the best learned value with recording_options."""
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
# ...and only round 10: it gained nothing over round 9 (plateau since round 6), and in a recording by the champion
# a quarter of the runs on builds it wouldn't choose would spend a quarter of the night on them. Round 11 on explores
# single moves alone.
COMMIT_LAST = 10
# From round 11 a won run plays on into Endless, to ante 12 (baseline --endless), and its positions up to the win are
# labelled with how many blinds it beat after it as well (0 to 12): a win with a build that clears ante 10 is a better
# win than one that scrapes through ante 8, which a win alone can't tell. The networks learn it as a second head
# (train_value.py: depth outputs, learned from those wins alone), and teacher option "margin=K" rates a position by
# its chance of winning with K blinds to spare. Positions after the win are never recorded.
ENDLESS_FROM = 11


def exploring(args: list[str], n: int) -> list[str]:
    """A recording's teacher arguments, with the exploration rounds from EXPLORE_FROM on use (and in rounds
    COMMIT_FROM to COMMIT_LAST, whole-build exploration too)."""
    if n < EXPLORE_FROM:
        return args
    extra = f"{EXPLORE},{COMMIT}" if COMMIT_FROM <= n <= COMMIT_LAST else EXPLORE
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


def stake_data(n: int) -> Path:
    return DATA / f"stakes-{n}"


# More of the champion's runs, one of the recordings a round's networks can learn from.
CHAMPION_2 = DATA / "champion-2"  # .sync/record-champion.sh: more of the champion's runs (seed bases 8001-8008)


def learned_round(n: int, root: Path = ROOT) -> dict[str, list[tuple]]:
    """Round n (3 or later) of the learned value, three jobs. teacher-data-n: the best teacher there is (the round and
    weight that won most, with "precise" unless its trial found it no better: recording_options; from round 4, the
    trial that beat that teacher clearly, with its networks, if one did: best_recording) plays 25,000 new runs (seed
    base 101 (n + 1): 404, 505, ...) and records every position it values. value-net-n: the round's
    network learns from them and from the round before's recording (more positions; only round n's runs judge and
    measure it). teacher-learned-n: the teacher plays the usual 5,000 seeds with it at weights 1 and 2, without
    options, to set against the earlier rounds on the same footing. From ENDLESS_FROM the recording's wins play on
    into Endless, so its networks also learn how far a win would have gone."""
    if n >= SCREEN_FROM:
        return screened_round(n, root)
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


# From round 11 a round is recorded by the champion as it plays (best_8k: its networks, weight, options and search
# budget), exploring single moves (EXPLORE; no whole builds: COMMIT_LAST), its wins played on into Endless. It is
# recorded in ROUND_PARTS parts of ROUND_PART_RUNS runs, each a step with its own seed base and folder
# (value-N/part-K): a recording stopped part way starts that part again, not the night (about 3.5 hours a part at
# the champion's budget). The round's pair learns from its own recording alone: the rounds before were recorded by
# teachers that won 50% to 66%, and outcome labels from a 75% player are the point of the round. Then three screens on
# the first SCREEN_RUNS usual seeds, each set run by run against the champion's own runs: the pair alone at the
# champion's weight and at ROUND_HIGH_WEIGHT, and the champion's networks with the pair. A screen chooses nothing by
# itself (best_8k counts 8,000-seed reports only).
SCREEN_FROM = 11
ROUND_PARTS = 5
ROUND_PART_RUNS = 5000
ROUND_HIGH_WEIGHT = "4"


def round_parts(n: int, data: Path) -> list[Path]:
    """Where round n's recording goes, part by part (from SCREEN_FROM). Code that looks for a round at
    value-N/manifest.json (make diagnose's replayable_recording, the forks') doesn't see a round in parts: teach it
    when those jobs come back."""
    return [data / f"value-{n}" / f"part-{k}" for k in range(1, ROUND_PARTS + 1)]


def screened_round(n: int, root: Path = ROOT, data: Path | None = None) -> dict[str, list[tuple]]:
    """Round n from SCREEN_FROM on, three jobs as before: teacher-data-n (record, part by part), value-net-n (the
    pair, on round n's runs alone) and teacher-learned-n (the screens against the champion)."""
    best = best_8k_report(root)
    record = exploring(best[1] if best else best_recording(root), n)
    parts = round_parts(n, data or DATA)
    pair = f"models/value-{n}.bin,models/value-{n}b.bin"
    steps = [("build", [*MAKE, "build"])]
    for k, folder in enumerate(parts, 1):
        steps.append((f"record-{k}", [*BASELINE, "--bot", "teacher", *record, "--runs", str(ROUND_PART_RUNS),
                                      "--seed", str(101 * (n + 1) + k - 1), "--endless", "--record", str(folder),
                                      "--out", f"results/value/record-{n}-{k}.json"]))
    data_args = [a for f in parts for a in ("--data", str(f))]
    nets = best[1][best[1].index("--value-net") + 1] if best else None
    trials: list[tuple] = [(None, "", pair, f"round{n}"), (ROUND_HIGH_WEIGHT, "", pair, f"round{n}")]
    if nets:
        trials.append((None, "", f"{nets},{pair}", f"champion+round{n}"))
    return {
        f"teacher-data-{n}": steps,
        f"value-net-{n}": [
            ("setup-gpu", [*MAKE, "setup-gpu"]),
            *[(f"train{suffix}", ["scripts/train_value.py", *data_args, "--newest", str(ROUND_PARTS), "--seed", seed,
                                  "--out", f"models/value-{n}{suffix}.bin",
                                  "--report", f"results/value/value-{n}{suffix}.json"])
              for suffix, seed in (("", "1"), ("b", "2"))],
        ],
        f"teacher-learned-{n}": champion_trials(root, tuple(trials), SCREEN_RUNS, salt=SCREEN_SALT),
    }


def ensemble_steps(n: int) -> list[tuple]:
    """The teacher with the networks of rounds 1 to n together, at weights 1 and 1.5 (a better value can take
    more weight: in the cloud, rounds 1 and 2 together with "precise" won 53.25% at 1.5 against 49.75% at 1), on
    the usual 5,000 seeds."""
    return [(f"ensemble-w{w}", [*BASELINE, "--bot", "teacher", "--value-net", ensemble(n), "--value-weight", w,
                                "--runs", "5000", "--seed", "1", "--out", f"results/value/ensemble{n}-w{w}.json"])
            for w in ("1", "1.5")]


# The value formula's numbers in force (crates/bm-core/src/value.rs). Version 2: the planner's edge 0.3, sharpness 2.5,
# the long run 0.35, (59.85% against 53.4% on the same 2,000 seeds). A trial is set only
# against a control measured with the same numbers: their reports carry the version (f2-...). Version 3: the same
# numbers, with "focus" and "fair-skip" the teacher's own play (+1.64 points on 8,000 seeds, 64.24% against
# 62.60%, p = 0.001), so trials from 2 November are set against controls that play that way (f3-...).
FORMULA = 3


def file_safe(options: str) -> str:
    return options.replace(",", "+").replace("=", "-")


SCREEN_RUNS = 2000


# Whether the champion is played again on today's code before its trials (its own report serves otherwise: the engine
# hasn't changed its play since 4 November's fairness fixes, which moved the champion of 2 November by 0.1 point).
CONTROL_AGAIN = False
# A fresh-salt control. The champion's own runs on the screen seeds are the ones that made it champion: on seeds 1 to
# 2,000 they sit about a point above its level there (77.05% against 74.35, 75.85 and 75.00 on the next three blocks,
# beyond what those seeds' difficulty explains), so a screen against them reads a challenger low. Playing the champion
# again does not help: a run is fixed by its seed, its index, the code and the options, so it replays the same runs.
# With a salt (`baseline --salt K`) the deals stay and the bot's own samples are drawn afresh: the control is the
# champion at salt K on the screen seeds, played once and shared, and every trial plays at the same K, so control and
# trials keep their common samples. From round 11 the rounds' screens use it.
SCREEN_SALT = 1


def champion_trials(root: Path, trials: tuple[tuple, ...], runs: int,
                    control_runs: int | None = None, salt: int = 0) -> list[tuple]:
    """The teacher that won most of the first 8,000 usual seeds (best_8k), at each (weight, option) of `trials` on
    the first `runs` seeds, each set run by run against the champion's own runs (its report; or, with CONTROL_AGAIN
    or an "again" report there, the champion played again on today's code on `control_runs` seeds, once). The
    comparison pairs the first seeds of both, so a 2,000-seed screen compares against the 8,000-seed champion. A
    trial's `name=value` replaces the champion's own setting of that name, so "late-x=6" on a champion with late-x=3
    plays at 6. A weight of None is the champion's own, so the trial changes the option alone. A trial of four,
    (weight, option, networks, name), plays those networks instead of the champion's (named so in its report's
    file: "-on-<name>"); an option of "" keeps the champion's options as they are. With a `salt`, the control is the
    champion played at that salt on `control_runs` seeds (once, "-salt<K>" in its name; SCREEN_SALT) and every trial
    plays at the same salt."""
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

    def with_(w: str, o: str, nets: str | None = None) -> list[str]:
        a = list(args)
        a[a.index("--value-weight") + 1] = w
        if nets:
            a[a.index("--value-net") + 1] = nets
        if "--teacher-options" in a:
            a[a.index("--teacher-options") + 1] = o
        else:
            a += ["--teacher-options", o]
        return a
    # The control: the champion's own report (the runs that made it the champion, on the same seeds), unless it was
    # played on an older engine (an "again" report exists or is wanted: CONTROL_AGAIN), when it is played again on
    # today's code once and that is the control every trial shares.
    # Since 13 November "boss-reroll" and "held-cards" are on by default: a champion report played before then (no
    # "teacher_defaults") is not today's champion, so a screen's control and trials carry DEFAULTS_MARK in their names
    # and the control is played again on today's code; the older reports are never reused by mistake.
    old_code = not played_with_defaults(root / report)
    mark = DEFAULTS_MARK if old_code else ""
    again = f"results/tune/{tag}-{file_safe(opts)}-{control_size}-w{w0}-again{mark}-on-{nets_stem}.json"
    salted, salt_args = (f"-salt{salt}{mark}", ["--salt", str(salt)]) if salt else (mark, [])
    if salt:
        control = f"results/tune/{tag}-{file_safe(opts)}-{control_size}-w{w0}{salted}-on-{nets_stem}.json"
    else:
        control = report if not (CONTROL_AGAIN or old_code) and not (root / again).exists() else again
    if control != report and not (root / control).exists():
        steps.append(("champion", [*BASELINE, "--bot", "teacher", *with_(w0, opts), "--runs", str(control_runs),
                                   "--seed", "1", *salt_args, "--out", control]))
    for w, extra, *other in trials:
        w = w or w0
        nets, stem = (other[0], other[1]) if other else (None, nets_stem)
        names = {e.split("=")[0] for e in extra.split(",") if "=" in e}
        kept = [e for e in opts.split(",") if e and e.split("=")[0] not in names]
        o = ",".join([*kept, extra]) if extra else opts
        out = f"results/tune/{tag}-{file_safe(o)}-{size}-w{w}{salted}-on-{stem}.json"
        label = f"w{w}-{extra or stem}" if not (other and extra) else f"w{w}-{extra}-{stem}"
        if not (root / out).exists():
            steps.append((label, [*BASELINE, "--bot", "teacher", *with_(w, o, nets), "--runs", str(runs),
                                  "--seed", "1", *salt_args, "--out", out]))
        steps.append((f"compare-{label}", ["scripts/compare_runs.py", control, out, "--out",
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


# Screens, looks and the league play salt 2; Green Stake is stake 3.
BUNDLE_SALT = 2
BUNDLE_GREEN = 3


# Where the looks write their tallies (`baseline --leaks`, read by scripts/leaks.py).
LEAK_DIR = ".sync/leaks"
# Screens' file names once the champion's report predates the options on by default (champion_trials).
DEFAULTS_MARK = "-d5"


def played_with_defaults(report: Path) -> bool:
    """Whether a report was played with today's options on by default (its "teacher_defaults")."""
    try:
        return json.loads(report.read_text()).get("teacher_defaults") == CODE_DEFAULTS
    except (OSError, ValueError):
        return False
CODE_DEFAULTS = ["boss-reroll", "held-cards", "glass", "five-kind", "freeze-money-last", "round-fixes", "shop-fixes",
                 "joker-fixes", "card-sharp", "card-tiers", "fool-copies", "open-packs"]
# All of them turned off (tests replay old runs this way).
DEFAULTS_OFF = ",".join(f"no-{d}" for d in CODE_DEFAULTS)


# make look: the gate before any long run (lab #27): 400 runs at Green and 400 at White with every default on and the
# tally, each tally's bars printed pass or fail, the runs kept for the viewer. A look, not a measurement.
LOOK_RUNS = 400


def look_steps(root: Path = ROOT, runs: int = LOOK_RUNS) -> list[tuple]:
    """make look: the champion as it now plays on the first `runs` salt-2 seeds at Green and at White, tallied."""
    steps: list[tuple] = [("build", [*MAKE, "build"]), ("folder", ["mkdir", "-p", LEAK_DIR])]
    best = best_8k_report(root)
    if best is None:
        return steps
    a = list(best[1])
    for name, stake in (("green", ["--stake", str(BUNDLE_GREEN)]), ("white", [])):
        lines = f"{LEAK_DIR}/look-{name}.jsonl"
        steps.append((name, [*BASELINE, "--bot", "teacher", *a, "--runs", str(runs), "--seed", "1", "--salt",
                             str(BUNDLE_SALT), *stake, "--leaks", lines, "--out", f"{LEAK_DIR}/look-{name}.json"]))
        steps.append((f"{name}-bars", ["scripts/leaks.py", lines, "--out", f"{LEAK_DIR}/look-{name}-summary.json"]))
    return steps


CALIB_FROM, CALIB_TO = 2001, 8000


PACK_AUDIT_RUNS = 300


def pack_audit_steps(root: Path = ROOT, runs: int = PACK_AUDIT_RUNS) -> list[tuple]:
    """make pack-audit (lab #30, the Arcana gap): the bot as it now plays, networks and all, on the first `runs` Green
    seeds at salt 2 with the shop audit on at every ante and the tally: what it thinks each pack is worth where it could
    open one, in its own dollars (scripts/pack_audit.py). The cloud's check had no networks: the formula alone valued
    an Arcana pack well above its price from ante 4 (median +$8 to +$15) and the bot still opened under half."""
    steps: list[tuple] = [("build", [*MAKE, "build"]), ("folder", ["mkdir", "-p", LEAK_DIR, ".sync/audit"])]
    best = best_8k_report(root)
    if best is None:
        return steps
    audit, lines = ".sync/audit/pack-audit-green.jsonl", f"{LEAK_DIR}/pack-audit-green.jsonl"
    steps.append(("pack-audit-green", [*BASELINE, "--bot", "teacher", *best[1], "--runs", str(runs), "--seed", "1",
                                       "--salt", str(BUNDLE_SALT), "--stake", str(BUNDLE_GREEN), "--audit-shops", audit,
                                       "--audit-antes", "1-8", "--leaks", lines,
                                       "--out", ".sync/audit/pack-audit-green.json"]))
    steps.append(("pack-audit-summary", ["scripts/pack_audit.py", audit, "--out", "results/audit/pack-audit-green.json"]))
    steps.append(("pack-audit-tally", ["scripts/leaks.py", lines, "--out", "results/leaks/pack-audit-green.json"]))
    return steps


def green_bench_steps(root: Path = ROOT, runs: int = SCREEN_RUNS) -> list[tuple]:
    """make green-bench (S and B, lab #33): the bench's validation. The base as it now is (-d5) on the same 2,000 Green
    seeds with `--bench`: each build scored as each ante from 5 began, and the final builds at antes 5 to 8, read by
    scripts/bench_check.py. Play is unchanged by the bench, so the report must match green-packs' open-packs arm seed for
    seed (its compare is a check: 0.00 points)."""
    steps: list[tuple] = [("build", [*MAKE, "build"]), ("folder", ["mkdir", "-p", ".sync/bench"])]
    best = best_8k_report(root)
    if best is None:
        return steps
    args = list(best[1])
    w = args[args.index("--value-weight") + 1]
    stem = Path(best[2]).stem.split("-on-")[-1]
    sl = f"salt{BUNDLE_SALT}"
    control = f"results/stakes/stake{BUNDLE_GREEN}-{runs // 1000}k-f{FORMULA}-d4-open-packs-w{w}-{sl}-on-{stem}.json"
    name = f"bench-green-{runs // 1000}k-{sl}"
    out, bench = f"results/bench/{name}.json", f".sync/bench/{name}.jsonl"
    if not (root / out).exists():
        steps.append(("bench-green", [*BASELINE, "--bot", "teacher", *args, "--runs", str(runs), "--seed", "1", "--salt",
                                      str(BUNDLE_SALT), "--stake", str(BUNDLE_GREEN), "--bench", bench, "--out", out]))
    steps.append(("bench-compare", ["scripts/compare_runs.py", control, out, "--out", f"results/bench/{name}-vs-d5.json"]))
    steps.append(("bench-check", ["scripts/bench_check.py", bench, "--out", f"results/bench/{name}-check.json"]))
    return steps


# make diagnose-control: is the play at the bosses worth more search? A lost blind played again at the champion's own
# budget is beaten some of the time by luck alone, and one try a blind leaves that luck in. This plays every lost blind of
# the same recording again CONTROL_TRIES times at the usual samples (the control) and CONTROL_TRIES times at DEEP_X
# (each try its own samples), so each blind's chance at each budget is measured and what deeper search adds is the
# difference (diagnose.py --control: by boss, and as points of win rate at stake, an upper bound). Minutes: the
# diagnosis at 4x took 49 seconds.
CONTROL_TRIES = 8


def diagnose_control_steps(root: Path = ROOT, data: Path = DATA) -> list[tuple]:
    steps: list[tuple] = [("build", [*MAKE, "build"])]
    best = best_8k_report(root)
    if best is None:
        return steps
    folder = data / "diagnose-champion" / "stake1"
    out = "results/diagnose/champion"
    for name, x in (("control", 1), (f"deep{DEEP_X}", DEEP_X)):
        steps.append((name, [*BASELINE, "--bot", "teacher", *best[1], "--replay", str(folder),
                             "--diagnose", f"{out}-{name}.csv", "--deep", str(x), "--deep-tries", str(CONTROL_TRIES),
                             "--deep-share", str(DEEP_SHARE), "--out", f"{out}-{name}.json"]))
    steps.append(("table-control", ["scripts/diagnose.py", f"{out}-control.csv",
                                    "--out", f"{out}-control-summary.json"]))
    steps.append(("table", ["scripts/diagnose.py", f"{out}-deep{DEEP_X}.csv", "--control",
                            f"{out}-control-summary.json", "--out", f"{out}-deep{DEEP_X}-summary.json"]))
    return steps


FORK_ROUNDS = 4


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
    "diagnose-control": diagnose_control_steps(),
    "look": look_steps(),
    "pack-audit": pack_audit_steps(),
    "green-bench": green_bench_steps(),
    "diagnose": diagnose_steps(),
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
    if r.get("play_on"):  # lost blinds played on (a recording for the forecast): never a result
        return None
    code, title = code_of(commit)
    net, weight = arg("--value-net"), arg("--value-weight")
    w = float(weight) if weight else (1.0 if net else None)
    bot = str(r.get("bot") or "bot")
    rnd = re.search(r"value-(\d+)\.bin$", net or "")
    if net and "," in net:
        rounds = re.findall(r"value-(\d+)\.bin", net)
        name = f"Learned values {'+'.join(rounds)} \u00d7{w:g}"
    elif net:
        name = f"Learned value{' ' + rnd[1] if rnd and rnd[1] != '1' else ''} \u00d7{w:g}"
    elif bot == "network":
        name = "Network, copying the teacher" if step == "eval-imitate" else "Network after PPO"
    else:
        name = bot[:1].upper() + bot[1:].removesuffix(" bot")
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


READ_ONLY_STEPS = ("scripts/compare_runs.py", "scripts/leaks.py", "scripts/pack_audit.py", "scripts/bench_check.py",
                   "scripts/danger_check.py", "gzip")
WRITTEN = (".json", ".jsonl", ".gz", ".csv", ".npy", ".bin", ".txt")
NAME_MAX = 250


def read_only(argv: list[str]) -> bool:
    """A step that only reads results and writes a summary."""
    return bool(argv) and argv[0] in READ_ONLY_STEPS


def path_problems(steps: list) -> list[str]:
    """Paths a job's steps would write that the file system refuses: a name over NAME_MAX characters, or a folder
    that can't be made."""
    out = []
    for name, argv, *_ in steps:
        for x in argv:
            if not isinstance(x, str) or "/" not in x or not x.endswith(WRITTEN) or x.startswith(("http", "-")):
                continue
            p = Path(x)
            if len(p.name) > NAME_MAX:
                out.append(f"{name}: a file name of {len(p.name)} characters ({p.name[:60]}...)")
            elif not p.is_absolute():
                folder = ROOT / p.parent
                try:
                    folder.mkdir(parents=True, exist_ok=True)
                except OSError as e:
                    out.append(f"{name}: can't make {p.parent} ({e})")
    return out


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
    # Every path the job will write, checked before its first step (S, #27: a night once stopped after two hours on
    # a file name too long for the file system).
    bad = path_problems(steps)
    if bad:
        raise SystemExit("The job would write paths the file system refuses, so it didn't start:\n  "
                         + "\n  ".join(bad[:10]))
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
        if code != 0 and read_only(argv):
            # A step that only reads results (a compare, a tally, a summary, a gzip) never stops a chain (S, #27):
            # it is recorded as failed and the next step starts; rerunning the job tries it again.
            st["steps"][name] = {"done": False, "seconds": secs, "exit_code": code, "read_only": True}
            save_state(job, st)
            print(f"    failed (exit code {code}), a read-only step: going on; the log is .sync/logs/{log.name}",
                  flush=True)
            continue
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
