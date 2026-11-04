"""A quick turn's table (make boost-quick): how well the correction predicts against the champion's networks alone
on the runs kept apart, stake by stake, and how the champion played with it, run by run on the same seeds.

    python scripts/quick_table.py quick1
"""
from __future__ import annotations

import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
STAKES = ["White", "Red", "Green", "Black", "Blue", "Purple", "Orange", "Gold"]


def load(path: Path) -> dict | None:
    try:
        return json.loads(path.read_text())
    except (OSError, ValueError):
        return None


def training_lines(rep: dict | None) -> list[str]:
    if not rep:
        return ["Training: no report yet."]
    m = rep["measured"]
    before = m.get("before", {})
    head = (f"Training, on {m['runs']:,} runs kept apart: BCE {m['bce']:.4f} against "
            f"{before.get('bce', float('nan')):.4f} by the champion's networks alone (lower is better), AUC {m['auc']} "
            f"against {before.get('auc')}.")
    lines = [head]
    for r in m.get("by_stake", []):
        gain = 100 * (r["bce_before"] - r["bce"]) / r["bce_before"] if r.get("bce_before") else float("nan")
        lines.append(f"  {STAKES[r['stake'] - 1]:<7} {r['rows']:>9,} positions  won {r['won']:.3f}  predicted "
                     f"{r['predicted']:.3f} (alone {r.get('predicted_before', float('nan')):.3f})  BCE {r['bce']:.4f} "
                     f"(alone {r.get('bce_before', float('nan')):.4f}: {gain:+.1f}%)  AUC {r['auc']} "
                     f"(alone {r.get('auc_before')})")
    return lines


def play_line(name: str, c: dict | None) -> str:
    if not c:
        return f"  {name:<7} not played yet"
    lo, hi = c["blinds_beaten_diff_ci"]
    wlo, whi = c["win_rate_diff_ci"]
    clear = "clearly better" if lo > 0 else "clearly worse" if hi < 0 else "not clear"
    return (f"  {name:<7} {c['seeds']:>5} seeds  won {100 * c['a']['win_rate']:5.1f}% -> {100 * c['b']['win_rate']:5.1f}% "
            f"({100 * c['win_rate_diff']:+.1f} [{100 * wlo:+.1f}, {100 * whi:+.1f}])  blinds {c['a']['blinds_beaten']:5.2f}"
            f" -> {c['b']['blinds_beaten']:5.2f} ({c['blinds_beaten_diff']:+.2f} [{lo:+.2f}, {hi:+.2f}]: {clear})")


def table(tag: str, root: Path = ROOT) -> str:
    q = root / "results" / "quick"
    lines = [f"Quick turn {tag}: the champion's networks with a residual correction", ""]
    lines += training_lines(load(q / f"residual-{tag}.json"))
    lines += ["", ("Play, run by run on the same seeds (White against the best teacher's runs, the other stakes "
                   "against 3 November's):")]
    lines.append(play_line("White", load(q / f"white-{tag}-vs-best.json")))
    for s in range(2, 9):
        lines.append(play_line(STAKES[s - 1], load(q / f"stake{s}-{tag}-vs-3nov.json")))
    return "\n".join(lines)


def main() -> None:
    if len(sys.argv) != 2:
        raise SystemExit(__doc__)
    print(table(sys.argv[1]))


if __name__ == "__main__":
    main()
