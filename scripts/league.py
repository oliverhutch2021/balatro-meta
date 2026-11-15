#!/usr/bin/env python3
"""The league (S, lab #33, Move 2), generation by generation: whole strategies, each an option string on the base,
played on common Green seeds against the base as reference, cut and promoted by B's rules, so luck is never bred from.

A generation (`make league GEN=n`, resumable: every member's block is a report on disk, the ledger is rewritten after
each one):
1. MEMBERS members (generation 0: drawn at random across the genes' ranges; later: bred from the last generation's
   parents, plus fresh random members) and the reference (the base, no genes) play block A, BLOCKS[0] seeds.
2. Cut by bound: a member whose paired difference's upper 95% bound is below zero is dropped.
3. The best PROMOTE[0] survivors by the shrunk estimate play block B (BLOCKS[1] more seeds), and the reference too.
4. The best PROMOTE[1] by the shrunk estimate at block A+B play block C (BLOCKS[2] more), and the reference too.
5. Parents: members at all three blocks whose paired difference is Level or Better (upper bound above zero).

Paired difference: per seed, member won less reference won (-1, 0, 1), pooled over the blocks played; its SE from
the share of seeds that differ. Shrunk estimate: the generation's mean plus (est - mean) x tau^2 / (tau^2 + SE^2),
tau^2 the variance of the estimates less their mean SE^2 (at least 0). Each generation plays a fresh block of seeds
(GEN_SEEDS apart), salt 2, Green Stake; every run is recorded with `--calibrate` (real runs, real wins).

    scripts/league.py --gen 0           play (or resume) generation 0
    scripts/league.py --gen 0 --plan    print the members and what is left to play, play nothing
"""
from __future__ import annotations

import argparse
import hashlib
import json
import math
import random
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import jobs  # noqa: E402

ROOT = jobs.ROOT
MEMBERS = 40
RANDOM_EACH_GEN = 10
BLOCKS = (200, 800, 3000)
PROMOTE = (12, 4)
SEED_BASE = 30001
GEN_SEEDS = 10000
SALT = 2
STAKE = 3
OUT = Path("results/league")
REC = Path(".sync/league")

# The genes: (option, kind, range). "switch" genes are present or not; "level" genes take a number. Each is drawn in
# generation 0 with its chance of being set; when not set the base's own value stands.
GENES: list[tuple[str, str, tuple, float]] = [
    # Forced moves.
    ("spend-down", "level", (0, 40, 5), 0.5),
    ("no-open-packs", "switch", (), 0.3),
    ("open-packs-all", "switch", (), 0.15),
    ("skips", "switch", (), 0.2),
    ("ahead", "level", (1, 3, 1), 0.1),
    # The user's judgements, each of which the league may overrule.
    ("no-card-sharp", "switch", (), 0.2),
    ("no-card-tiers", "switch", (), 0.2),
    ("no-fool-copies", "switch", (), 0.2),
    ("no-held-cards", "switch", (), 0.15),
    ("no-round-fixes", "switch", (), 0.15),
    ("no-shop-fixes", "switch", (), 0.15),
    # The value's prices (value::PARAMS), log-uniform about the base's value.
    ("long-power-x", "log", (0.5, 3.0), 0.5),
    ("long-money-x", "log", (0.5, 3.0), 0.5),
    ("buying-power", "log", (0.015, 0.06), 0.5),
    ("money-scale", "log", (12.5, 50.0), 0.5),
    ("above-cap", "uniform", (0.0, 1.0), 0.5),
    ("long-run", "log", (0.175, 0.7), 0.4),
    ("deck-share", "uniform", (0.2, 0.8), 0.4),
]


def draw_gene(name: str, kind: str, rng: tuple, r: random.Random) -> str:
    if kind == "switch":
        return name
    if kind == "level":
        lo, hi, step = rng
        return f"{name}={r.randrange(lo, hi + 1, step)}"
    lo, hi = rng
    v = math.exp(r.uniform(math.log(lo), math.log(hi))) if kind == "log" else r.uniform(lo, hi)
    return f"{name}={v:.3g}"


def random_member(r: random.Random) -> list[str]:
    genes = [draw_gene(n, k, rg, r) for n, k, rg, p in GENES if r.random() < p]
    return genes or [draw_gene(*GENES[0][:3], r)]


def gene_name(g: str) -> str:
    return g.split("=")[0]


def breed(parents: list[list[str]], r: random.Random) -> list[str]:
    """Uniform crossover of two parents' genes, then a mutation: one gene redrawn, added or removed."""
    a, b = r.sample(parents, 2) if len(parents) > 1 else (parents[0], parents[0])
    by = {gene_name(g): g for g in a}
    for g in b:
        if gene_name(g) not in by or r.random() < 0.5:
            by[gene_name(g)] = g
    child = [g for g in by.values() if r.random() < 0.85 or len(by) == 1]
    n, k, rg, _ = r.choice(GENES)
    child = [g for g in child if gene_name(g) != n]
    if r.random() < 0.7:
        child.append(draw_gene(n, k, rg, r))
    return sorted(set(child)) or random_member(r)


def member_id(genes: list[str]) -> str:
    return "ref" if not genes else hashlib.sha1(",".join(sorted(genes)).encode()).hexdigest()[:10]


def members_for(gen: int, ledger_dir: Path = OUT) -> list[list[str]]:
    """Generation `gen`'s members: drawn at random (generation 0), or bred from the last generation's parents plus
    RANDOM_EACH_GEN random ones. The same every time it is asked (seeded by the generation)."""
    r = random.Random(1000 + gen)
    if gen > 0:
        prev = load(gen - 1, ledger_dir)
        parents = [m["genes"] for m in prev.get("members", {}).values() if m.get("parent")]
        if parents:
            kids = [breed(parents, r) for _ in range(MEMBERS - RANDOM_EACH_GEN)]
            return dedupe(parents + kids + [random_member(r) for _ in range(RANDOM_EACH_GEN)])[:MEMBERS]
    return dedupe([random_member(r) for _ in range(MEMBERS)])


def dedupe(ms: list[list[str]]) -> list[list[str]]:
    seen, out = set(), []
    for m in ms:
        k = member_id(sorted(m))
        if k not in seen:
            seen.add(k)
            out.append(sorted(m))
    return out


def block_seeds(gen, block: int, blocks: tuple = None, seed_base: int = None) -> tuple[int, int]:
    """(first seed, runs) of a block: generation n's blocks follow each other from SEED_BASE + n x GEN_SEEDS; a named
    league (the wall) plays from its own seed base."""
    blocks = blocks or BLOCKS
    base = seed_base if seed_base is not None else SEED_BASE + int(gen) * GEN_SEEDS
    return base + sum(blocks[:block]), blocks[block]


def ledger_path(gen, root: Path = OUT) -> Path:
    return root / f"gen-{gen}.json"


def load(gen: int, root: Path = OUT) -> dict:
    p = ROOT / ledger_path(gen, root) if not root.is_absolute() else ledger_path(gen, root)
    return json.loads(p.read_text()) if p.exists() else {}


def report_path(gen: int, mid: str, block: int) -> Path:
    return OUT / f"gen-{gen}" / f"{mid}-b{block}.json"


def stages_of(report: dict) -> str:
    return report.get("run_stages", "")


def paired(member: str, ref: str) -> tuple[int, int, int]:
    """(seeds, only the reference won, only the member won) from two reports' run endings (WIN is 'Y')."""
    n = min(len(member), len(ref))
    win = "Y"
    only_ref = sum(1 for i in range(n) if ref[i] == win and member[i] != win)
    only_mem = sum(1 for i in range(n) if member[i] == win and ref[i] != win)
    return n, only_ref, only_mem


def estimate(counts: list[tuple[int, int, int]]) -> dict:
    n = sum(c[0] for c in counts)
    a = sum(c[1] for c in counts)
    b = sum(c[2] for c in counts)
    if n == 0:
        return {"seeds": 0}
    d = (b - a) / n
    se = math.sqrt(max((a + b) / n - d * d, 0.0) / n)
    return {"seeds": n, "only_ref": a, "only_member": b, "diff": d, "se": se, "lo": d - 1.96 * se, "hi": d + 1.96 * se}


def shrink(ests: dict[str, dict]) -> dict[str, float]:
    xs = {k: e for k, e in ests.items() if e.get("seeds")}
    if not xs:
        return {}
    vals = [e["diff"] for e in xs.values()]
    mean = sum(vals) / len(vals)
    var = sum((v - mean) ** 2 for v in vals) / max(len(vals) - 1, 1)
    tau2 = max(var - sum(e["se"] ** 2 for e in xs.values()) / len(xs), 0.0)
    return {k: mean + (e["diff"] - mean) * (tau2 / (tau2 + e["se"] ** 2) if tau2 + e["se"] ** 2 > 0 else 0.0)
            for k, e in xs.items()}


def run_block(gen, mid: str, genes: list[str], block: int, base_args: list[str], play, blocks: tuple = None,
              seed_base: int = None) -> dict | None:
    """Play one member's block (unless its report is there) and return the report."""
    out = report_path(gen, mid, block)
    if not (ROOT / out).exists():
        first, runs = block_seeds(gen, block, blocks, seed_base)
        a = list(base_args)
        i = a.index("--teacher-options")
        if genes:
            a[i + 1] = f"{a[i + 1]},{','.join(genes)}"
        rec = REC / f"gen-{gen}" / f"{mid}-b{block}.jsonl"
        (ROOT / out).parent.mkdir(parents=True, exist_ok=True)
        (ROOT / rec).parent.mkdir(parents=True, exist_ok=True)
        cmd = [*jobs.BASELINE, "--bot", "teacher", *a, "--runs", str(runs), "--seed", str(first), "--salt", str(SALT),
               "--stake", str(STAKE), "--calibrate", str(rec), "--out", str(out)]
        print(f"  gen {gen} member {mid} block {block}: {runs} seeds from {first} ({','.join(genes) or 'reference'})",
              flush=True)
        if play(cmd) != 0:
            raise SystemExit(f"member {mid} block {block} failed; rerun `make league GEN={gen}` to resume")
        play(["gzip", "-f", str(rec)])
    p = ROOT / out
    return json.loads(p.read_text()) if p.exists() else None


def deaths(stages: str) -> dict[int, float]:
    """The share of runs that ended at each ante (1-8), and won (9)."""
    n = max(len(stages), 1)
    out = {a: 0 for a in range(1, 10)}
    for c in stages:
        i = "ABCDEFGHIJKLMNOPQRSTUVWXY".index(c)
        out[9 if i == 24 else i // 3 + 1] += 1
    return {a: round(v / n, 4) for a, v in out.items()}


def run_generation(gen, play=None, plan_only: bool = False, members_given: list[list[str]] | None = None,
                   blocks: tuple = None, promote: tuple = None, seed_base: int = None, stop_after: int | None = None,
                   cut_first: bool = True) -> dict:
    """Play (or resume) a generation. B's selection rules (lab #33): block A for all, cut by bound; each later block for
    the best of the last block's survivors ranked by the shrunk estimate on the latest block alone (never on the seeds
    they were picked on); parents from the last block alone, Level or Better. `stop_after` stops after that block."""
    play = play or (lambda cmd: subprocess.call(cmd, cwd=ROOT))
    blocks = blocks or BLOCKS
    promote = promote or PROMOTE
    best = jobs.best_8k_report(ROOT)
    if best is None:
        raise SystemExit("No champion report found to play the league on")
    base_args = list(best[1])
    given = members_given if members_given is not None else members_for(gen)
    members = {member_id(m): {"genes": m} for m in given}
    led = load(gen)
    for k, m in led.get("members", {}).items():
        members.setdefault(k, m)
    first_seed = seed_base if seed_base is not None else SEED_BASE + int(gen) * GEN_SEEDS
    ledger = {"gen": gen, "seed_base": first_seed, "blocks": list(blocks), "salt": SALT, "stake": STAKE,
              "base_options": base_args[base_args.index("--teacher-options") + 1], "members": members}

    def save():
        p = ROOT / ledger_path(gen)
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_text(json.dumps(ledger, indent=1) + "\n")

    def counts(k: str, which: list[int]) -> list[tuple[int, int, int]]:
        out = []
        for b in which:
            mr, rr = report_path(gen, k, b), report_path(gen, "ref", b)
            if (ROOT / mr).exists() and (ROOT / rr).exists():
                out.append(paired(stages_of(json.loads((ROOT / mr).read_text())),
                                  stages_of(json.loads((ROOT / rr).read_text()))))
        return out

    def score(ids: list[str], block: int):
        """The estimate on all blocks played (for reading) and on `block` alone (for selection), shrunk."""
        for k in ids:
            members[k]["estimate"] = estimate(counts(k, list(range(block + 1))))
            members[k][f"estimate_block_{block}"] = estimate(counts(k, [block]))
            st = "".join(stages_of(json.loads((ROOT / report_path(gen, k, b)).read_text()))
                         for b in range(block + 1) if (ROOT / report_path(gen, k, b)).exists())
            members[k]["deaths_by_ante"] = deaths(st) if st else {}
        sh = shrink({k: members[k][f"estimate_block_{block}"] for k in ids})
        for k, v in sh.items():
            members[k]["shrunk"] = v

    if plan_only:
        for k, m in members.items():
            print(k, ",".join(m["genes"]))
        return ledger
    ids = list(members)
    alive = ids
    for block in range(len(blocks)):
        if block > 0:
            alive = sorted(alive, key=lambda k: -members[k].get("shrunk", -1))[:promote[block - 1]]
        run_block(gen, "ref", [], block, base_args, play, blocks, first_seed)
        for k in alive:
            run_block(gen, k, members[k]["genes"], block, base_args, play, blocks, first_seed)
            members[k][f"block_{block}"] = True
            score([k], block)
            save()
        score(alive, block)
        ref_st = "".join(stages_of(json.loads((ROOT / report_path(gen, "ref", b)).read_text()))
                         for b in range(block + 1) if (ROOT / report_path(gen, "ref", b)).exists())
        ledger["reference_deaths_by_ante"] = deaths(ref_st) if ref_st else {}
        if block == 0 and cut_first:
            alive = [k for k in alive if members[k]["estimate"].get("hi", -1) >= 0]
            for k in ids:
                members[k]["cut_at_a"] = k not in alive
        save()
        if stop_after is not None and block >= stop_after:
            print(table(ledger))
            return ledger
    last = len(blocks) - 1
    for k in ids:
        e = members[k].get(f"estimate_block_{last}", {})
        members[k]["parent"] = bool(members[k].get(f"block_{last}") and e.get("hi", -1) > 0)
    save()
    print(table(ledger))
    return ledger


# The wall league (S, lab #33, A4): designed members on one block of 1,000 fresh Green seeds, the reference on it, the
# best two by the shrunk estimate to 4,000. Deaths by ante beside every member. Generation 0's four leaders (by their
# block B alone) join as they stand.
WALL_SEEDS = 60001
WALL_BLOCKS = (1000, 3000)
WALL_PROMOTE = (2,)
WALL_MEMBERS = [
    ["danger-spend=0.6"], ["danger-spend=0.75"], ["danger-spend=0.9"], ["danger-x"],
    ["danger-spend=0.75", "danger-x"],
    ["long-power-x=0.5"], ["long-power-x=0.75"], ["long-run=0.2"],
    # The best danger gene with generation 0's leading money genes (danger-spend=0.75 stands for the best until the
    # danger arms have read).
    ["above-cap=0.899", "buying-power=0.0539", "danger-spend=0.75", "deck-share=0.269", "money-scale=14.7"],
]


def wall_members() -> list[list[str]]:
    """WALL_MEMBERS and generation 0's four leaders by their block B alone (all promoted ones if fewer)."""
    g0 = load(0).get("members", {})
    played_b = [(k, m) for k, m in g0.items() if m.get("block_1")]
    def on_b(m):
        e = m.get("estimate_block_1") or {}
        return e.get("diff", m.get("estimate", {}).get("diff", -9))
    leaders = [m["genes"] for _, m in sorted(played_b, key=lambda km: -on_b(km[1]))[:4]]
    return dedupe(WALL_MEMBERS + leaders)


def dba(m: dict, ante: int) -> float:
    """A member's share of runs ended at `ante` (the ledger's keys are numbers in play, strings once reloaded)."""
    d = m.get("deaths_by_ante", {})
    return d.get(ante, d.get(str(ante), 0.0))


def table(ledger: dict) -> str:
    rows = sorted(ledger["members"].items(), key=lambda kv: -(kv[1].get("shrunk") or -9))
    out = [f"Generation {ledger['gen']}: {len(rows)} members against the base (paired, points of win rate)"]
    for k, m in rows:
        e = m.get("estimate", {})
        if not e.get("seeds"):
            continue
        out.append(f"  {k}  {e['seeds']:5d} seeds  {100 * e['diff']:+6.2f} ({100 * e['lo']:+.2f} to {100 * e['hi']:+.2f})"
                   f"  shrunk {100 * m.get('shrunk', 0):+.2f}{'  PARENT' if m.get('parent') else ''}"
                   f"  died at 4/5/6: {'/'.join(str(round(100 * dba(m, a), 1)) for a in (4, 5, 6))}%"
                   f"{'  cut' if m.get('cut_at_a') else ''}  {','.join(m['genes'])}")
    return "\n".join(out)


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--gen", type=int, default=0)
    ap.add_argument("--plan", action="store_true")
    ap.add_argument("--stop-after", type=int, help="stop after this block (0 = A, 1 = B, ...)")
    ap.add_argument("--wall", action="store_true", help="the wall league (S, lab #33, A4) in place of a generation")
    a = ap.parse_args(argv)
    if a.wall:
        run_generation("wall", plan_only=a.plan, members_given=wall_members(), blocks=WALL_BLOCKS,
                       promote=WALL_PROMOTE, seed_base=WALL_SEEDS, stop_after=a.stop_after, cut_first=False)
    else:
        run_generation(a.gen, plan_only=a.plan, stop_after=a.stop_after)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
