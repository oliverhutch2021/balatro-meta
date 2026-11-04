"""One table from several training reports' judge tests (make forks-now): the network of each training against the
champion's networks, the teacher's quick look and its own choice, on the forks kept apart.

    python scripts/forks_report.py results/value/value-forks-now-pw0.json results/value/value-forks-now-pw1.json ...
        [--out results/forks/now-table.json]

Each report is train_value.py's (`measured.forks`: `judge_forks` with the trained network as a judge). The reports
should share their held-out forks (the same fork files and split), so the lines compare.
"""
from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path


def load(path: Path) -> dict | None:
    try:
        return json.loads(path.read_text())
    except (OSError, ValueError):
        return None


def row(name: str, j: dict | None) -> str:
    if not j:
        return f"  {name:28} (no judgement)"
    return (f"  {name:28} picked best {100 * j['picked_best']:5.1f}%   regret {j['regret']:.3f}   gain over random "
            f"{j['gain']:+.3f}   ({j['judged']:,} forks)")


def lines(reports: list[tuple[Path, dict | None]]) -> list[str]:
    o = []
    first = next((r for _, r in reports if r and (r.get("measured") or {}).get("forks", {}).get("forks")), None)
    if first is None:
        return ["No report with a judge test yet."]
    fk = first["measured"]["forks"]
    o.append(f"The judge test on {fk['forks']:,} forks kept apart (every report's network against the same judges):")
    for name in ("ensemble", "quick", "chosen", "random"):
        o.append(row(name, fk.get(name)))
    if "oracle_half_gain" in fk:
        o.append(f"  {'oracle (half the copies)':28} gain over random {fk['oracle_half_gain']:+.3f} on the other half")
    o.append("  the trained networks:")
    for path, r in reports:
        if not r:
            o.append(f"  {path.stem:28} (no report)")
            continue
        m = (r.get("measured") or {}).get("forks") or {}
        pw = (r.get("forks") or {}).get("pair_weight")
        label = f"{path.stem} (pair weight {pw})" if pw is not None else path.stem
        o.append(row(label[:28], m.get("network")) + (f"   pairwise loss {m['pair_bce']}" if "pair_bce" in m else "")
                 + f"   BCE {r['measured']['bce']}")
    return o


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("reports", nargs="+", type=Path)
    ap.add_argument("--out", type=Path)
    a = ap.parse_args()
    reports = [(p, load(p)) for p in a.reports]
    text = lines(reports)
    print("\n".join(text))
    if a.out:
        a.out.parent.mkdir(parents=True, exist_ok=True)
        a.out.write_text(json.dumps({"reports": [str(p) for p, _ in reports], "lines": text,
                                     "judge": {p.stem: ((r or {}).get("measured") or {}).get("forks")
                                               for p, r in reports}}, indent=1) + "\n")
        print(f"Summary: {a.out}")


if __name__ == "__main__":
    sys.exit(main())
