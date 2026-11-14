#!/usr/bin/env python3
"""What the bot thinks each pack is worth where it could open one (lab #30, the Arcana gap), from the shop audit
(`baseline --audit-shops FILE --audit-antes 1-8`).

For each shop decision with a pack on sale: the quick pass's value of opening it, less the value of leaving as things
stand (the pack's price already paid in that value), turned into the value's own dollars by what $4 more is worth
there (`here_plus4`). Then, by pack kind and ante, the median and the share above zero; and, where an Arcana pack
was worth more than leaving, what the bot did instead: opened it, made a move it valued higher, left the shop, or
made a move the full pass preferred.

    scripts/pack_audit.py AUDIT.jsonl --out SUMMARY.json
"""
from __future__ import annotations

import argparse
import json
import statistics
from collections import Counter, defaultdict
from pathlib import Path

KINDS = ("Arcana", "Buffoon", "Celestial", "Standard", "Spectral")


def quick_note(d: dict) -> dict | None:
    for n in d.get("notes", []):
        n = json.loads(n) if isinstance(n, str) else n
        if "quick" in n:
            return n["quick"]
    return None


def summarise(lines: list[dict]) -> dict:
    worth: dict = defaultdict(list)
    when: Counter = Counter()
    for d in lines:
        q = quick_note(d)
        if not q or "here_plus4" not in q:
            continue
        per = (q["here_plus4"] - q["here"]) / 4
        if per <= 1e-9:
            continue
        slots = {p[0]: (p[1], p[2]) for p in d.get("packs", [])}
        arcana = []
        for m, v in q["options"]:
            if m.startswith("OpenBooster"):
                i = int(m.split()[1])
                kind = slots.get(i, ("?", 0))[0]
                worth[(kind, min(d["ante"], 8))].append((v - q["here"]) / per)
                if kind == "Arcana":
                    arcana.append(v)
        if not arcana:
            continue
        best = max(arcana)
        if best <= q["here"]:
            when["arcana_below_leaving"] += 1
            continue
        when["arcana_above_leaving"] += 1
        ch = d["chosen"]
        if ch.startswith("OpenBooster") and slots.get(int(ch.split()[1]), ("", 0))[0] == "Arcana":
            when["opened"] += 1
        elif ch.startswith("LeaveShop"):
            when["left_the_shop"] += 1
        elif max(v for _, v in q["options"]) > best + 1e-9:
            when["another_move_valued_higher"] += 1
        else:
            when["another_move_the_full_pass_preferred"] += 1
    table = {}
    for kind in KINDS:
        table[kind] = {}
        for a in range(1, 9):
            v = worth.get((kind, a), [])
            if v:
                table[kind][a] = {"n": len(v), "median_dollars": round(statistics.median(v), 2),
                                  "above_zero": round(sum(x > 0 for x in v) / len(v), 3)}
    return {"by_kind_and_ante": table, "when_arcana_beats_leaving": dict(when)}


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("audit", type=Path)
    ap.add_argument("--out", type=Path, required=True)
    a = ap.parse_args(argv)
    lines = [json.loads(x) for x in a.audit.read_text().splitlines() if x.strip()]
    s = {"audit": a.audit.name, "decisions": len(lines), **summarise(lines)}
    a.out.parent.mkdir(parents=True, exist_ok=True)
    a.out.write_text(json.dumps(s, indent=1) + "\n")
    print("A pack's worth beyond its price, in the value's own dollars (median, share above zero), by ante:")
    for kind, t in s["by_kind_and_ante"].items():
        print(f"  {kind}: " + "; ".join(f"a{k} {v['median_dollars']:+.1f} [{100 * v['above_zero']:.0f}%]"
                                         for k, v in t.items()))
    print("Where an Arcana pack beat leaving: " + ", ".join(f"{k} {v}" for k, v in s["when_arcana_beats_leaving"].items()))
    print(f"Summary: {a.out}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
