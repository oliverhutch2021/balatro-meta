"""Run the heavy jobs on the PC: resumable, low priority, and self-reporting.

    python scripts/jobs.py sync          # fast-forward main to the code Claude left in .sync/claude.bundle
    python scripts/jobs.py run smoke     # run (or resume) a job; `make smoke` calls this
    python scripts/jobs.py status        # which steps of which jobs are done

A job is a list of steps. Each finished step is recorded in .sync/state/<job>.json, so running the
same command again skips it. A job refuses to resume on different code (another commit), because
its checkpoints would mix two versions; pass --force to restart it on the new code.

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
PY = sys.executable
MAKE = ["make", "--no-print-directory"]

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
    # The teacher bot (crates/bm-core/src/teacher.rs: sells for upgrades, rerolls, vouchers, Tarots with
    # targets, joker order; planner.rs: plans its hands, digging) on 100,000 runs: the network's warm start
    # and the next number to beat. Same seeds as the greedy baseline's first 100,000.
    "teacher": [
        ("build", [*MAKE, "build"]),
        ("teacher", ["cargo", "run", "--release", "-q", "-p", "bm-core", "--example", "baseline", "--",
                     "--bot", "teacher", "--runs", "100000", "--seed", "1", "--out", "results/baseline/teacher.json"]),
    ],
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
            for line in proc.stdout:
                sys.stdout.write("    " + line)
                fh.write(line)
            code = proc.wait()
        secs = round(time.time() - t0)
        if code != 0:
            st["steps"][name] = {"done": False, "seconds": secs, "exit_code": code}
            save_state(job, st)
            watch_note(job, st, steps, "failed", step=name)
            raise SystemExit(f"Step '{name}' failed (exit code {code}); the log is .sync/logs/{log.name}. "
                             f"Rerun `make {job}` to resume from this step once it's fixed.")
        st["steps"][name] = {"done": True, "seconds": secs, "finished": now()}
        save_state(job, st)
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
