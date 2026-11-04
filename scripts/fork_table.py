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


def summarise(fs: tv.ForkSet) -> dict:
    """The judge test on every fork (train_value.judge_forks), and by ante."""
    ids = np.arange(len(fs))
    antes = np.array([f["ante"] for f in fs.forks])
    out = {"all": tv.judge_forks(fs, ids), "by_ante": {}}
    for a in sorted(set(antes.tolist())):
        out["by_ante"][int(a)] = tv.judge_forks(fs, ids[antes == a])
    return out


def lines(s: dict, folders: list[Path]) -> list[str]:
    t = s["all"]
    head = (f"The judge test on {t['forks']:,} forked decisions ({', '.join(f.name for f in folders)}): how often "
            f"each judge's choice was the option whose play-outs went best, what it gave away against that option, "
            f"and its gain over a random option in the play-outs' win rate.")
    o = [head]
    o += ["  " + line for line in tv.judge_lines(t)]
    o.append("  by ante (chosen: picked best / gain; networks: picked best / gain):")
    for a, t in s["by_ante"].items():
        c, e = t.get("chosen"), t.get("ensemble")
        o.append(f"    ante {a}: {t['forks']:,} forks  chosen {100 * c['picked_best']:.0f}% / {c['gain']:+.3f}"
                 + (f"  networks {100 * e['picked_best']:.0f}% / {e['gain']:+.3f}" if e else "")
                 + (f"  oracle {t['oracle_half_gain']:+.3f}" if "oracle_half_gain" in t else ""))
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
