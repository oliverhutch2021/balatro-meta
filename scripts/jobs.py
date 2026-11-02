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


def best_recording(root: Path = ROOT) -> list[str]:
    """The teacher a round records with, as its arguments (learned value, weight, options): of the trials on the best
    learned value (results/tune: each change set run by run against the precise teacher on the same 2,000 seeds), the
    one that won most over it, if it won clearly (p below RECORD_WITH_P), with the networks it played with (a trial
    can be other networks: make teacher-next's "values 1-4"); else the best learned value with recording_options."""
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
# From round 10 a quarter of the recorded runs also commit from the start to one hand type, picked at random by the
# run's number (teacher option "commit": its Planets taken and bought as the focus's): whole builds the teacher
# wouldn't choose, played to the end, where "explore" only tries single moves along its own habits.
COMMIT = "commit=0.25"
COMMIT_FROM = 10


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


def learned_round(n: int, root: Path = ROOT) -> dict[str, list[tuple]]:
    """Round n (3 or later) of the learned value, three jobs. teacher-data-n: the best teacher there is (the round and
    weight that won most, with "precise" unless its trial found it no better: recording_options; from round 4, the
    trial that beat that teacher clearly, with its networks, if one did: best_recording) plays 25,000 new runs (seed
    base 101 (n + 1): 404, 505, ...) and records every position it values. value-net-n: the round's
    network learns from them and from the round before's recording (more positions; only round n's runs judge and
    measure it). teacher-learned-n: the teacher plays the usual 5,000 seeds with it at weights 1 and 2, without
    options, to set against the earlier rounds on the same footing."""
    data = DATA / f"value-{n}"
    return {
        f"teacher-data-{n}": [
            ("build", [*MAKE, "build"]),
            ("record", [*BASELINE, "--bot", "teacher",
                        *exploring(best_recording(root) if n >= 4
                                   else [*learned_value_args(root), *recording_options(root)], n),
                        "--runs", "25000", "--seed", str(101 * (n + 1)), "--record", str(data),
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


def weight_steps(root: Path = ROOT, weights: tuple[str, ...] = WEIGHT_TRIALS, runs: int = BIG_RUNS) -> list[tuple]:
    """make teacher-weights: the precise teacher on the best learned value at its weight and at each of `weights`,
    on the first `runs` usual seeds, each set run by run against the first (a clear gain is what the next round
    records with: best_recording reads each f3-...-vs-...-on-<networks> comparison and the trial's own weight).
    Runs already played on the same networks are only compared again, so after a new round the job measures that
    round's networks, and before one it costs nothing."""
    tag = f"f{FORMULA}"
    net_args, stem, base_opts, _ = tune_control(root, tag)
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
    return steps


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
