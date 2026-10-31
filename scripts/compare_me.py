"""Your decisions set against the bot's: make compare-me.

    make compare-me          every run you recorded (.sync/me/runs), judged by the best teacher there is

Each run recorded by `make record-me` is replayed in the engine from its seed (scripts/replay_me.py finds your
moves), and at each of your decisions the best teacher (the value network, weight and options the jobs found best)
says what it would have done there, and what it thinks each choice is worth: in a round, the chance of beating
the blind after it; between rounds, the value of the position after it (a pack with its best pick, a reroll with
its best buy). The teacher sees only what the screen showed you, as when it plays (examples/judge.rs).

Written to .sync/me/ (this PC only): judged.jsonl, one line per decision, and report.txt: how often you and the
bot agree by kind of decision, the swaps you make most (you did X where the bot would do Y), and the decisions
where the bot thinks your choice was worth least and most against its own. A difference in value is the bot's
opinion, not a verdict: the patterns that recur are then tried as changes to the bot on thousands of seeds.
"""
from __future__ import annotations

import argparse
import collections
import json
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))

import replay_me

ME = ROOT / ".sync" / "me"
RUNS = ME / "runs"
JUDGE = ["cargo", "run", "--release", "-q", "-p", "bm-core", "--example", "judge", "--"]
KINDS = ("select blind", "skip blind", "cash out", "leave shop", "sort by rank", "sort by suit", "play", "discard",
         "move a card", "move a joker", "sell a joker", "reroll the shop", "buy", "buy and use", "redeem a voucher",
         "open a pack", "sell a consumable", "use a consumable", "take a pack card", "skip the pack", "reroll the boss")
PHASES = {"BLIND_SELECT": "blind choice", "SELECTING_HAND": "round", "ROUND_EVAL": "cash-out", "SHOP": "shop",
          "SMODS_BOOSTER_OPENED": "pack"}


def run_files(folder: Path = RUNS) -> list[Path]:
    return sorted([*folder.glob("*.jsonl.gz"), *folder.glob("*.jsonl")])


def replay_file(path: Path) -> dict:
    """A recorded run replayed: its seed, the engine's moves state by state, and how far the replay got."""
    header, states = replay_me.read_run(path)
    r = replay_me.replay_states(states)
    if r is None:
        return {"name": path.name, "seed": header.get("seed"), "steps": [], "ok": False, "lost": "no states",
                "states": 0}
    first = r.moves[: len(r.moves) - sum(len(s["moves"]) for s in r.steps)]
    steps = ([first] if first else []) + [s["moves"] for s in r.steps]
    v = r.game.view()
    return {"name": path.name.split(".")[0], "seed": r.game.seed, "steps": steps, "ok": r.ok, "lost": r.lost,
            "states": len(states), "ante": v["ante"], "won": v["won"], "over": r.game.over}


def judge_input(runs: list[dict]) -> str:
    lines = []
    for run in runs:
        steps = "|".join(",".join(f"{k}:{a}" for k, a in s) for s in run["steps"])
        lines.append(f"{run['name']}\t{run['seed']}\t{steps}")
    return "\n".join(lines) + "\n"


def best_teacher(root: Path = ROOT) -> list[str]:
    """The judge's options for the best teacher there is (as the jobs choose it)."""
    import jobs

    return [*jobs.learned_value_args(root), *jobs.recording_options(root)]


def gap(d: dict) -> float | None:
    """How much better the bot thinks its own choice is than yours (0 when you agree)."""
    if d["agree"]:
        return 0.0
    p, b = d.get("player_value"), d.get("bot_value")
    if p is None or b is None:
        return None
    return b - p


def report(decisions: list[dict], runs: list[dict], top: int = 15) -> str:
    out = []
    n = len(decisions)
    won = sum(1 for r in runs if r.get("won"))
    out.append(f"{len(runs)} runs ({won} won), {n} decisions judged.")
    for r in runs:
        how = "won" if r.get("won") else (f"lost at ante {r.get('ante')}" if r.get("over") else
                                          f"stopped at ante {r.get('ante')}")
        note = "" if r["ok"] else f" (the replay stopped part-way: {r['lost']})"
        out.append(f"  {r['name']}: {how}, {r['states']} states{note}")
    if not n:
        return "\n".join(out) + "\n"
    out.append("")
    out.append("How often you and the bot made the same choice:")
    by_phase = collections.defaultdict(list)
    for d in decisions:
        by_phase[PHASES.get(d["phase"], d["phase"])].append(d)
    for phase, ds in sorted(by_phase.items(), key=lambda x: -len(x[1])):
        agree = sum(d["agree"] for d in ds)
        out.append(f"  {phase:13s} {agree:4d} of {len(ds):4d} ({100 * agree / len(ds):.0f}%)")
    out.append("")
    out.append("Your choice where the bot would make another, most common first:")
    swaps = collections.Counter((PHASES.get(d["phase"], d["phase"]), KINDS[d["player_kind"]], KINDS[d["bot_kind"]])
                                for d in decisions if not d["agree"])
    for (phase, mine, bots), c in swaps.most_common(12):
        gaps = [g for d in decisions if not d["agree"] and PHASES.get(d["phase"], d["phase"]) == phase
                and KINDS[d["player_kind"]] == mine and KINDS[d["bot_kind"]] == bots
                and (g := gap(d)) is not None]
        mean = sum(gaps) / len(gaps) if gaps else float("nan")
        out.append(f"  {c:4d}x in the {phase}: you {mine}, the bot would {bots} (mean gap {mean:+.3f})")
    sections = (("value", "between rounds (value)", ""), ("beat", "in rounds (chance to beat the blind)", ""))
    for measure, label, unit in sections:
        ds = [d for d in decisions if not d["agree"] and d["measure"] == measure and gap(d) is not None]
        if not ds:
            continue
        ds.sort(key=gap, reverse=True)
        out.append("")
        out.append(f"Where the bot rates its own choice highest above yours, {label}:")
        for d in ds[:top]:
            out.append(f"  {gap(d):+8.3f}{unit}  ante {d['ante']} {PHASES.get(d['phase'], d['phase'])}, "
                       f"${d['money']}: you {d['player']}; bot {d['bot']}  [{d['run']} step {d['step']}]")
        out.append(f"Where it rates yours above its own, {label}:")
        for d in sorted(ds, key=gap)[:top]:
            if gap(d) >= 0:
                break
            out.append(f"  {gap(d):+8.3f}{unit}  ante {d['ante']} {PHASES.get(d['phase'], d['phase'])}, "
                       f"${d['money']}: you {d['player']}; bot {d['bot']}  [{d['run']} step {d['step']}]")
    return "\n".join(out) + "\n"


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("runs", nargs="*", type=Path, help="recorded runs (default: all in .sync/me/runs)")
    ap.add_argument("--threads", type=int, default=0)
    a = ap.parse_args(argv)
    paths = a.runs or run_files()
    if not paths:
        print("No recorded runs yet: make record-me (RUNNING.md, step 13).")
        return 1
    runs = []
    for p in paths:
        r = replay_file(p)
        runs.append(r)
        print(f"{p.name}: {len(r['steps'])} steps" + ("" if r["ok"] else f" (replay stopped: {r['lost']})"))
    ME.mkdir(parents=True, exist_ok=True)
    inp, out = ME / "judge-input.txt", ME / "judged.jsonl"
    inp.write_text(judge_input([r for r in runs if r["steps"]]))
    cmd = [*JUDGE, "--runs", str(inp), "--out", str(out), *best_teacher()]
    if a.threads:
        cmd += ["--threads", str(a.threads)]
    print("Judging with the best teacher:", " ".join(cmd[len(JUDGE):]))
    subprocess.run(cmd, cwd=ROOT, check=True)
    decisions = [json.loads(line) for line in out.read_text().splitlines() if line.strip()]
    text = report(decisions, runs)
    (ME / "report.txt").write_text(text)
    print(text)
    print(f"Report: {ME / 'report.txt'}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
