"""The judge test from a fork recording alone (make forks-table): how often each judge's choice was the option whose
play-outs went best, and what its choice gave away in win rate.

    python scripts/fork_table.py ~/.cache/balatro-meta/data/forks-quick [more fork folders...] [--out summary.json]

A fork recording is what `baseline --forks` writes (forks.rs): at a shop's or an open pack's first decision, the
teacher's best options and doing nothing (and its own choice and the run's move, if others), each played out to the
end on the same few fair copies. The judges: the networks the forks were made with (the mean of their log-odds for
the position each option left: the champion's judgement without the formula), the teacher's quick look (its first
screen of the options, formula and networks), the teacher's own choice (its full judgement, decided at the fork), the
move the run made (the recording's teacher, which may have been exploring), and picking at random. "Best" is the
option with most wins among the copies, then the furthest losses; with few copies that is noisy, which inflates every
judge's regret alike, so the judges compare with each other, not with zero.
"""
from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parent))
import train_value as tv

JUDGES = ("ensemble", "quick", "chosen", "made", "random")


def judge_table(fs: tv.ForkSet, ids: np.ndarray) -> dict:
    """Each judge's picked-best share and regret on these forks (the judges that had an opinion on each)."""
    seen: dict[str, list[tuple[float, float]]] = {j: [] for j in JUDGES}
    for i in ids:
        f = fs.forks[i]
        opts = f["options"]
        m = len(opts)
        wr = np.array([o["won"].mean() for o in opts])
        score = np.array([o["key"].sum() for o in opts])
        best = int(np.lexsort((-np.arange(m), score, wr))[-1])
        logits = np.array([o["logit"] for o in opts])
        quick = np.array([o["quick"] for o in opts])
        picks = {
            "ensemble": int(np.nanargmax(logits)) if np.isfinite(logits).any() else None,
            "quick": int(np.nanargmax(quick)) if np.isfinite(quick).any() else None,
            "chosen": next((k for k, o in enumerate(opts) if o["chosen"]), None),
            "made": next((k for k, o in enumerate(opts) if o["made"]), None),
        }
        for name, pick in picks.items():
            if pick is not None:
                seen[name].append((float(pick == best), float(wr[best] - wr[pick])))
        seen["random"].append((1.0 / m, float(wr[best] - wr.mean())))
    out = {"forks": len(ids)}
    for name, rows in seen.items():
        if rows:
            out[name] = {"judged": len(rows), "picked_best": round(float(np.mean([b for b, _ in rows])), 4),
                         "regret": round(float(np.mean([r for _, r in rows])), 4)}
    return out


def summarise(fs: tv.ForkSet) -> dict:
    ids = np.arange(len(fs))
    antes = np.array([f["ante"] for f in fs.forks])
    out = {"all": judge_table(fs, ids), "by_ante": {}, "options_mean": round(float(np.mean(
        [len(f["options"]) for f in fs.forks])), 2) if len(fs) else None}
    for a in sorted(set(antes.tolist())):
        out["by_ante"][int(a)] = judge_table(fs, ids[antes == a])
    # How far apart the options were: the share of forks where the best and worst option differ by a copy or more.
    spread = [max(o["won"].mean() for o in f["options"]) - min(o["won"].mean() for o in f["options"]) for f in fs.forks]
    out["spread"] = {"mean": round(float(np.mean(spread)), 4) if spread else None,
                     "a_copy_or_more": round(float(np.mean([s > 1e-9 for s in spread])), 4) if spread else None}
    return out


def lines(s: dict, folders: list[Path]) -> list[str]:
    t = s["all"]
    o = [f"The judge test on {t['forks']:,} forked decisions ({', '.join(f.name for f in folders)}; "
         f"{s['options_mean']} options a fork): how often each judge's choice was the option whose play-outs went "
         f"best, and what its choice gave away in win rate."]
    for name in JUDGES:
        if name in t:
            j = t[name]
            o.append(f"  {name:9} picked best {100 * j['picked_best']:5.1f}%   regret {j['regret']:.3f}   "
                     f"({j['judged']:,} forks)")
    sp = s["spread"]
    if sp["mean"] is not None:
        o.append(f"  the options' win rates differed by {sp['mean']:.3f} on average; by a copy or more at "
                 f"{100 * sp['a_copy_or_more']:.0f}% of the forks")
    o.append("  by ante (chosen: picked best / regret):")
    for a, t in s["by_ante"].items():
        c = t.get("chosen")
        e = t.get("ensemble")
        o.append(f"    ante {a}: {t['forks']:,} forks  chosen {100 * c['picked_best']:.0f}% / {c['regret']:.3f}"
                 + (f"  networks {100 * e['picked_best']:.0f}% / {e['regret']:.3f}" if e else ""))
    return o


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("folders", nargs="+", type=Path, help="fork recordings (baseline --forks DIR)")
    ap.add_argument("--out", type=Path, help="where to write the summary (JSON)")
    a = ap.parse_args()
    folders = [f.expanduser() for f in a.folders]
    fs = tv.ForkSet(tv.load_data(folders))
    s = summarise(fs)
    print("\n".join(lines(s, folders)))
    if a.out:
        a.out.parent.mkdir(parents=True, exist_ok=True)
        s["folders"] = [tv.shown(f) for f in folders]
        a.out.write_text(json.dumps(s, indent=1) + "\n")
        print(f"Summary: {a.out}")


if __name__ == "__main__":
    sys.exit(main())
