"""Replay every difference the real-game checks saved, on today's engine (make recheck).

    python scripts/recheck.py [.sync/watch/mismatches] [--move-fixed]

Each file in the folder is one difference the checks found (scripts/checker.py `mismatch`): the seed, the moves up
to the difference, and the game's state then. The engine plays the same moves again and its view is set against
that saved state with the same comparison the checks use (watch.differences), so a difference the engine has
since fixed reads as fixed, and the ones still open are listed, grouped by what differs. A file that doesn't say
its stake (older files) is tried at every stake and the one that differs least is taken. `--move-fixed` moves the
fixed files into a `fixed/` folder beside the rest. Nothing from the game is printed but what the checks already
save: keys, counts and scores.
"""
from __future__ import annotations

import argparse
import collections
import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import watch

ROOT = Path(__file__).resolve().parents[1]
DEFAULT = ROOT / ".sync" / "watch" / "mismatches"


def replay(seed: str, moves: list, stake: int):
    """The engine after these moves at this stake, or None if a move was refused."""
    from balatro_meta._engine import Game

    game = Game(seed, stake)
    for kind, arg in moves:
        if not game.step(int(kind), int(arg)):
            return None
    return game


def recheck_file(path: Path) -> dict:
    """One saved difference replayed: {"file", "seed", "move", "what", "was", "stake", "now": [...] (empty: fixed)
    or "error"}."""
    d = json.loads(path.read_text())
    out = {"file": path.name, "seed": d.get("seed"), "move": d.get("move"), "what": d.get("what"),
           "was": list(d.get("differences") or [])}
    gs = d.get("game")
    moves = d.get("moves")
    if not isinstance(gs, dict) or moves is None:
        out["error"] = "no game state or moves saved"
        return out
    g = watch.game_view(gs)
    stakes = [int(d["stake"])] if d.get("stake") else list(range(1, 9))
    best = None
    for stake in stakes:
        try:
            game = replay(str(d["seed"]), moves, stake)
        except Exception as e:  # noqa: BLE001 - a replay that can't start is reported, not raised
            out["error"] = f"replay failed: {e}"
            return out
        if game is None:
            continue
        now = watch.differences(watch.engine_view(game), g)
        if best is None or len(now) < len(best[1]):
            best = (stake, now)
        if not now:
            break
    if best is None:
        out["error"] = "the engine refused a saved move at every stake"
        return out
    out["stake"], out["now"] = best
    return out


def field_of(diff: str) -> str:
    return diff.split(":")[0]


def summarise(results: list[dict]) -> dict:
    fixed = [r for r in results if "now" in r and not r["now"]]
    open_ = [r for r in results if r.get("now")]
    errors = [r for r in results if "error" in r]
    by_field: dict[str, int] = collections.Counter(field_of(r["now"][0]) for r in open_)
    return {"files": len(results), "fixed": len(fixed), "open": len(open_), "errors": len(errors),
            "open_by_field": dict(by_field.most_common()),
            "open_files": [{k: r[k] for k in ("file", "seed", "move", "what", "stake")} | {"now": r["now"][:3]}
                           for r in open_],
            "errors_files": [{"file": r["file"], "error": r["error"]} for r in errors]}


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("folder", nargs="?", type=Path, default=DEFAULT)
    ap.add_argument("--move-fixed", action="store_true", help="move the fixed files into fixed/ beside the rest")
    ap.add_argument("--out", type=Path, help="where to write the summary (JSON)")
    a = ap.parse_args()
    files = sorted(p for p in a.folder.glob("*.json"))
    if not files:
        print(f"No saved differences in {a.folder}.")
        return
    results = [recheck_file(p) for p in files]
    s = summarise(results)
    print(f"{s['files']} saved differences replayed on today's engine: {s['fixed']} fixed, {s['open']} still open"
          + (f", {s['errors']} couldn't be replayed" if s["errors"] else "") + ".")
    if s["open_by_field"]:
        print("  open, by what differs: " + ", ".join(f"{k} {n}" for k, n in s["open_by_field"].items()))
    for r in s["open_files"]:
        print(f"  {r['file']}: after move {r['move']} ({r['what']}), stake {r['stake']}: {'; '.join(r['now'])[:200]}")
    for r in s["errors_files"]:
        print(f"  {r['file']}: {r['error']}")
    if a.move_fixed:
        dest = a.folder / "fixed"
        dest.mkdir(exist_ok=True)
        for r in results:
            if "now" in r and not r["now"]:
                (a.folder / r["file"]).rename(dest / r["file"])
        print(f"Moved {s['fixed']} fixed files to {dest}.")
    if a.out:
        a.out.parent.mkdir(parents=True, exist_ok=True)
        a.out.write_text(json.dumps(s, indent=1) + "\n")
        print(f"Summary: {a.out}")


if __name__ == "__main__":
    sys.exit(main())
