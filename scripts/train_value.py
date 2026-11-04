"""Train the teacher's learned value (crates/bm-core/src/learned.rs) on the GPU.

    python scripts/train_value.py --data ~/.cache/balatro-meta/data/value-1 --out models/value-1.bin \
        --report results/value/value-1.json

The data is what `baseline --record DIR` writes: every position the teacher valued between rounds (the blind
choice, the shop, open packs) as the value network's input (observe.rs: `encode_value`, read from the screen
alone), labelled with how its run ended. The network learns the chance of winning the run from a position
and, alongside, the blind the run ends at, which gives it more to learn from than the wins alone.

With `--offset` the network learns a correction instead: each recording keeps, beside its positions, the value
networks' log-odds of winning as the teacher uses them (`baseline --offsets`, or `--offsets-into` for a recording
made without), and the network's win output is added to them (a residual: what those networks get wrong, at any
stake). It starts at zero, the value networks' own answer, and is kept only as far as it judges better than them;
the report sets the two side by side, stake by stake. The engine adds it with `baseline --stake-net`.

With `--forks DIR` (what `baseline --forks` writes: at forked decisions, the positions the teacher's options
leave, each played out to the end on fair copies that the options share) the network also learns to put a
decision's options in the right order: for each pair of options at a fork, the difference of its win logits
between their positions (each the mean over the option's copies) is trained towards the share of copies where the
first option's play-out ended better (a soft pairwise loss, `--pair-weight` times the positions' loss). That is the
comparison the teacher asks of it, learned from several outcomes per option where a recording gives one per run.
Forks are split by run like positions; on the forks kept apart, the report sets the network's choice (the option
it rates highest) against the one whose play-outs went best, beside the teacher's own choice, its quick look and
the networks the forks were made with (`measured.forks`: the judge test).

Runs are split, not positions (a run's positions share its ending, so a network could learn a run by heart):
one run in twenty (by its number) judges training, which stops when those runs stop improving; another one in
twenty is kept apart to measure the chosen network. A few network sizes and regularisations are tried and the
best kept: a run's positions share its ending, so strong dropout (on the inputs too) matters more than size. The network is written in the engine's format (BMVN, learned.rs) with the input scaling folded into
its first layer, read back and checked against the trained network, and, when the engine module is built,
against the engine's own forward pass.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import math
import struct
import sys
import time
from pathlib import Path

import numpy as np

ROOT = Path(__file__).resolve().parents[1]
FORMAT = 1  # BMVN format version (learned.rs)
STAGES = 25  # a run ends at blind 0..23, or 24 for a win
SPLIT = 20  # one run in twenty judges training, another is kept apart to measure the result
# Networks tried: hidden widths / dropout / weight decay / input dropout. Strong dropout, on the inputs too,
# keeps a network from telling runs apart by their details and learning their endings by heart (on 1,500 runs
# it was the difference between beating what the stage alone predicts and not).
CONFIGS = "256,128/0.5/1e-3/0.5; 512,256/0.5/1e-3/0.5; 128,64/0.3/1e-3/0.3; 64/0.3/1e-3/0.3"

# The columns of each chunk's lab.npy (examples/baseline.rs: LABELS).
RUN, STEP, PHASE, ANTE, STAGE, WON, END_STAGE, END_ANTE = range(8)
# Recordings made with `baseline --endless` add a ninth column: for a win, the blinds it beat after winning (0 to
# DEPTH - 1, how strong the win was); -1 where not known (a loss, or a recording that didn't play on). Older
# recordings have eight columns and read as unknown.
ENDLESS = 8
DEPTH = 13
# Labels every recording has: newer recordings may add columns after them.
BASE_LABELS = 8
# The columns of a fork recording's lab.npy (examples/baseline.rs: FORK_LABELS), one row per option and copy.
(F_RUN, F_STEP, F_FORK, F_OPTION, F_OPTIONS, F_COPY, F_ANTE, F_PHASE, F_CHOSEN, F_MADE, F_QUICK, F_WON, F_END_STAGE,
 F_LOGIT) = range(14)
FORK_LABELS = ["run", "step", "fork", "option", "options", "copy", "ante", "phase", "chosen", "made", "quick", "won",
               "end_stage", "logit"]


# ---- Data ----

class Data:
    """Recorded positions, sparse: row r's nonzero inputs are idx/val[ptr[r]:ptr[r+1]]; lab[r] its labels. With
    several recordings, source[r] is the recording row r came from (the last is the newest), and the manifest is the
    newest one's."""

    def __init__(self, idx: np.ndarray, val: np.ndarray, ptr: np.ndarray, lab: np.ndarray, manifest: dict,
                 source: np.ndarray | None = None, manifests: list[dict] | None = None,
                 off: np.ndarray | None = None, offset_net: str | None = None):
        self.idx, self.val, self.ptr, self.lab, self.manifest = idx, val, ptr, lab, manifest
        self.source = np.zeros(len(lab), np.int8) if source is None else source
        self.manifests = [manifest] if manifests is None else manifests
        # With offsets: each row's log-odds of winning by the value networks they name (offset_net).
        self.off, self.offset_net = off, offset_net

    @property
    def depth(self) -> bool:
        """Whether any recording played its wins on into Endless (so the network learns how strong a win is)."""
        return any(m.get("endless") for m in self.manifests)

    @property
    def stakes(self) -> np.ndarray:
        """Each row's stake (1 White ... 8 Gold), its recording's."""
        by_source = np.array([int(m.get("stake") or 1) for m in self.manifests], np.int8)
        return by_source[self.source]

    @property
    def rows(self) -> int:
        return len(self.lab)

    @property
    def inputs(self) -> int:
        return int(self.manifest["inputs"])

    def dense(self, rows: np.ndarray) -> np.ndarray:
        x = np.zeros((len(rows), self.inputs), np.float32)
        for k, r in enumerate(rows):
            a, b = self.ptr[r], self.ptr[r + 1]
            x[k, self.idx[a:b].astype(np.int64)] = self.val[a:b].astype(np.float32)
        return x


# Run numbers of an older recording are moved up by this much per recording back (the newest keeps its own), so
# runs from different recordings never share a number. Labels are float32, exact to 16,777,216.
RUN_OFFSET = 1_000_000


def shown(path) -> str:
    """A path as reports show it: the home folder as ~ (reports are committed and the repo can be public)."""
    s, home = str(path), str(Path.home())
    return "~" + s[len(home):] if s == home or s.startswith(home + "/") else s


def layouts(manifest: dict) -> dict[int, int]:
    """The input layouts a recording's rows hold, version: floats. Each layout's floats are the first of the next
    one's (observe.rs VALUE_LAYOUTS), so a row of a newer layout holds every older one; a recording made before
    manifests listed them holds its own alone."""
    pairs = manifest.get("layouts") or [[manifest["input_version"], manifest["inputs"]]]
    return {int(v): int(n) for v, n in pairs}


def offset_source(folder: Path, manifest: dict) -> tuple[str, str] | None:
    """The value networks a recording's offsets come from, (paths, fingerprint): `baseline --offsets` names them in
    the manifest, `--offsets-into` in offsets.json. None if it has none."""
    extra = folder / "offsets.json"
    about = json.loads(extra.read_text()) if extra.exists() else manifest
    net, fingerprint = about.get("offset_net"), about.get("offset_net_hash")
    return None if not net or not fingerprint else (str(net), str(fingerprint))


def load_data(folders: Path | list[Path], max_runs: int | None = None, layout: int | None = None,
              offsets: bool = False) -> Data:
    """Every chunk of one or more recordings, each in run order (the recorder writes chunks of whole runs, sorted),
    the newest last. `layout`: the input layout to read the rows in (a newer recording's rows cut to its floats); by
    default the newest every recording holds, so recordings of rounds before and after a new layout train together
    as before, and a network of the new layout comes only from recordings that hold it. `offsets`: each row's
    log-odds by the value networks too (`-off.npy`); every recording must have them, from the same networks."""
    folders = [folders] if isinstance(folders, (str, Path)) else list(folders)
    parts = []
    for folder in folders:
        man_path = folder / "manifest.json"
        if not man_path.exists():
            raise SystemExit(f"{folder} has no manifest.json: the recording is missing or didn't finish "
                             f"(make teacher-data).")
        manifest = json.loads(man_path.read_text())
        chunks = sorted(p.name[: -len("-ptr.npy")] for p in folder.glob("chunk-*-ptr.npy"))
        if manifest.get("partial") and len(chunks) >= manifest["chunks"]:
            chunks = chunks[:manifest["chunks"]]  # still being made: the chunks whole when the manifest was written
        if len(chunks) != manifest["chunks"]:
            raise SystemExit(f"{folder}: the manifest lists {manifest['chunks']} chunks, {len(chunks)} are there.")
        # Sizes first, so the arrays are filled in place (no second copy of the data in memory).
        sizes = []
        for c in chunks:
            ptr = np.load(folder / f"{c}-ptr.npy", mmap_mode="r")
            lab = np.load(folder / f"{c}-lab.npy", mmap_mode="r")
            sizes.append((len(lab), int(ptr[-1])))
        if max_runs is not None:
            keep = max(1, math.ceil(max_runs / manifest["runs_per_chunk"]))
            chunks, sizes = chunks[:keep], sizes[:keep]
        parts.append((folder, manifest, chunks, sizes))
    offset_net = None
    if offsets:
        sources = {}
        for folder, manifest, chunks, _ in parts:
            src = offset_source(folder, manifest)
            missing = [c for c in chunks if not (folder / f"{c}-off.npy").exists()]
            if src is None or missing:
                raise SystemExit(f"{folder} has no offsets{f' for {missing[0]}' if missing else ''}: run "
                                 f"`baseline --offsets-into {folder} --value-net NETS` first.")
            sources[src[1]] = src[0]
        if len(sources) != 1:
            raise SystemExit(f"The recordings' offsets come from different networks: {sorted(sources.values())}.")
        offset_net = next(iter(sources.values()))
    shared = set.intersection(*(set(layouts(m)) for _, m, _, _ in parts))
    version = max(shared, default=None) if layout is None else layout
    if version is None or version not in shared:
        held = "; ".join(f"{f}: {sorted(layouts(m))}" for f, m, _, _ in parts)
        raise SystemExit(f"Not every recording holds input layout {version} ({held}), so they can't be mixed.")
    sizes_of = {layouts(m)[version] for _, m, _, _ in parts}
    if len(sizes_of) != 1:
        raise SystemExit(f"The recordings give input layout {version} different lengths: {sorted(sizes_of)}.")
    inputs = sizes_of.pop()
    first = dict(parts[-1][1], inputs=inputs, input_version=version,
                 layouts=[[v, n] for v, n in sorted(layouts(parts[-1][1]).items()) if v <= version])
    # Labels: every recording's are the first columns of the longest's (columns were only ever added at the end);
    # a recording without one reads -1 there (not known).
    names = max((list(m.get("labels") or []) for _, m, _, _ in parts), key=len)
    kinds = {m.get("kind") or "positions" for _, m, _, _ in parts}
    if len(kinds) != 1:
        raise SystemExit(f"Positions and forks can't be mixed ({sorted(kinds)}).")
    for folder, manifest, _, _ in parts:
        own = list(manifest.get("labels") or [])
        if own != names[:len(own)] or len(own) < BASE_LABELS:
            raise SystemExit(f"{folder}: its labels ({own}) don't start the others' ({names}), so they can't be "
                             f"mixed.")
    first["labels"] = names
    # Run numbers are kept apart by RUN_OFFSET a recording, in float32 labels (whole numbers exact below 2^24).
    if len(parts) > 16:
        raise SystemExit("At most 16 recordings at once.")
    n_rows = sum(r for _, _, _, sizes in parts for r, _ in sizes)
    n_nz = sum(z for _, _, _, sizes in parts for _, z in sizes)
    idx = np.empty(n_nz, np.int16)  # the inputs number fewer than 32,768
    val = np.empty(n_nz, np.float16)
    ptr = np.empty(n_rows + 1, np.int64)
    lab = np.empty((n_rows, len(first["labels"])), np.float32)
    source = np.empty(n_rows, np.int8)
    off = np.empty(n_rows, np.float32) if offsets else None
    ptr[0] = 0
    r0 = z0 = 0
    for k, (folder, manifest, chunks, sizes) in enumerate(parts):
        start = r0
        for c, (nr, nz) in zip(chunks, sizes):
            cp = np.load(folder / f"{c}-ptr.npy")
            if len(cp) != nr + 1 or cp[0] != 0 or cp[-1] != nz:
                raise SystemExit(f"{folder}/{c}: its row pointers don't match its rows.")
            ci, cv = np.load(folder / f"{c}-idx.npy").view(np.int16), np.load(folder / f"{c}-val.npy")
            if int(manifest["inputs"]) > inputs:  # a newer layout's rows, cut to the floats of this one
                keep = (ci >= 0) & (ci < inputs)
                cp = np.concatenate([[0], np.cumsum(keep)])[cp]
                ci, cv, nz = ci[keep], cv[keep], int(cp[-1])
            idx[z0:z0 + nz] = ci
            val[z0:z0 + nz] = cv
            ptr[r0 + 1:r0 + nr + 1] = cp[1:] + z0
            cl = np.load(folder / f"{c}-lab.npy")
            lab[r0:r0 + nr, :cl.shape[1]] = cl
            lab[r0:r0 + nr, cl.shape[1]:] = -1.0
            if off is not None:
                co = np.load(folder / f"{c}-off.npy")
                if co.shape != (nr,):
                    raise SystemExit(f"{folder}/{c}: {len(co)} offsets for {nr} positions.")
                off[r0:r0 + nr] = co
            r0, z0 = r0 + nr, z0 + nz
        if max_runs is not None and r0 > start:
            # Rows are in run order, so the first runs are a prefix; the next recording is written after it.
            r0 = start + int((lab[start:r0, RUN] < lab[start, RUN] + max_runs).sum())
            z0 = int(ptr[r0])
        source[start:r0] = k
        lab[start:r0, RUN] += RUN_OFFSET * (len(parts) - 1 - k)
    idx, val, ptr, lab, source = idx[:z0], val[:z0], ptr[:r0 + 1], lab[:r0], source[:r0]
    off = None if off is None else off[:r0]
    if len(lab) and (idx.min() < 0 or idx.max() >= first["inputs"]):
        raise SystemExit(f"{parts[-1][0]}: an input index is out of range.")
    return Data(idx, val, ptr, lab, first, source, [m for _, m, _, _ in parts], off, offset_net)


def split(runs: np.ndarray, parts: int = SPLIT) -> np.ndarray:
    """Each run's part, by a hash of its number: 0 to train on, 1 to judge training, 2 to measure."""
    h = (runs.astype(np.int64) * 2654435761) % (1 << 32) % parts
    return np.where(h == 0, 1, np.where(h == 1, 2, 0))


def held_out(runs: np.ndarray) -> np.ndarray:
    """Which runs are not trained on."""
    return split(runs) != 0


def parse_configs(text: str) -> list[dict]:
    out = []
    for part in text.split(";"):
        if part.strip():
            fields = part.strip().split("/")
            hidden, dropout, decay = fields[:3]
            out.append({"hidden": [int(h) for h in hidden.split(",") if h], "dropout": float(dropout),
                        "weight_decay": float(decay),
                        "input_dropout": float(fields[3]) if len(fields) > 3 else None})
    return out


# ---- The network ----

def need_torch():
    try:
        import torch
    except ImportError:
        raise SystemExit("PyTorch is missing: run `make setup-gpu` first.") from None
    return torch


def make_net(torch, inputs: int, hidden: list[int], dropout: float, input_dropout: float = 0.0,
             depth: bool = False):
    """Outputs: the win logit, the ending stage's STAGES logits, and with `depth` how far a win goes on after it
    (DEPTH logits, learned only from the wins of recordings that played on)."""
    nn = torch.nn
    layers, width = [nn.Dropout(input_dropout)], inputs
    for h in hidden:
        layers += [nn.Linear(width, h), nn.ReLU(), nn.Dropout(dropout)]
        width = h
    layers.append(nn.Linear(width, 1 + STAGES + (DEPTH if depth else 0)))
    return nn.Sequential(*layers)


class Batches:
    """Dense batches built from the sparse rows on the training device. Positions too many for the GPU beside the
    training stay in memory, each batch built there and sent over (slower, the same numbers)."""

    def __init__(self, torch, data: Data, device):
        self.torch, self.device, self.inputs = torch, device, data.inputs
        self.depth = data.depth
        t = torch.from_numpy
        self.lab = t(data.lab).to(device)
        self.off = None if data.off is None else t(data.off).to(device)
        try:
            self.store = device
            self.idx, self.val, self.ptr = (t(a).to(device) for a in (data.idx, data.val, data.ptr))
        except RuntimeError:  # torch.cuda.OutOfMemoryError
            if device.type != "cuda":
                raise
            self.idx = self.val = self.ptr = None
            torch.cuda.empty_cache()
            print("The positions don't fit on the GPU beside the training: kept in memory, batches sent over.",
                  flush=True)
            self.store = torch.device("cpu")
            self.idx, self.val, self.ptr = (t(a) for a in (data.idx, data.val, data.ptr))

    def x(self, rows):
        """The dense inputs of these rows (a LongTensor of row numbers on the device)."""
        torch = self.torch
        rows = rows.to(self.store)
        start = self.ptr[rows]
        n = self.ptr[rows + 1] - start
        total = int(n.sum())
        out = torch.zeros((len(rows), self.inputs), dtype=torch.float32, device=self.store)
        if total:
            row_of = torch.repeat_interleave(torch.arange(len(rows), device=self.store), n)
            first = torch.repeat_interleave(torch.cumsum(n, 0) - n, n)
            flat = torch.repeat_interleave(start, n) + torch.arange(total, device=self.store) - first
            out[row_of, self.idx[flat].long()] = self.val[flat].float()
        return out.to(self.device)


class ForkSet:
    """Forked decisions (`baseline --forks`): for each fork, its options' rows (one per copy) and how each copy's
    play-out ended, and for each pair of options the share of copies where the first ended better (the pair's
    target). Forks are split by their run like positions (0 to train on, 1 to judge, 2 to measure)."""

    def __init__(self, data: Data, parts: int = SPLIT):
        self.data = data
        lab = data.lab
        if lab.shape[1] < len(FORK_LABELS):
            raise SystemExit("Not a fork recording (baseline --forks).")
        key = np.stack([data.source.astype(np.int64), lab[:, F_RUN].astype(np.int64), lab[:, F_FORK].astype(np.int64)],
                       1)
        _, first, inverse = np.unique(key, axis=0, return_index=True, return_inverse=True)
        inverse = inverse.reshape(-1)
        order = np.argsort(inverse, kind="stable")
        self.forks: list[dict] = []
        start = 0
        for g in range(len(first)):
            end = start + int((inverse == g).sum())
            rows = order[start:end]
            start = end
            opts = lab[rows, F_OPTION].astype(int)
            options = []
            for o in np.unique(opts):
                r = rows[opts == o]
                r = r[np.argsort(lab[r, F_COPY], kind="stable")]
                won = lab[r, F_WON]
                options.append({"rows": r, "won": won, "key": np.where(won > 0.5, 24.0, lab[r, F_END_STAGE]),
                                "chosen": bool(lab[r[0], F_CHOSEN] > 0.5), "made": bool(lab[r[0], F_MADE] > 0.5),
                                "quick": float(lab[r[0], F_QUICK]), "logit": finite_mean(lab[r, F_LOGIT])})
            if len(options) < 2:
                continue
            pairs = []
            for i in range(len(options)):
                for j in range(i + 1, len(options)):
                    ki, kj = options[i]["key"], options[j]["key"]
                    n = min(len(ki), len(kj))
                    t = float(np.mean((ki[:n] > kj[:n]) + 0.5 * (ki[:n] == kj[:n])))
                    pairs.append((i, j, t))
            self.forks.append({"run": int(lab[rows[0], F_RUN]), "ante": int(lab[rows[0], F_ANTE]),
                               "options": options, "pairs": pairs})
        runs = np.array([f["run"] for f in self.forks], np.int64)
        self.part = split(runs, parts) if len(runs) else np.zeros(0, np.int64)

    def __len__(self) -> int:
        return len(self.forks)

    def ids(self, part: int) -> np.ndarray:
        return np.flatnonzero(self.part == part)

    def batch(self, ids) -> tuple[np.ndarray, np.ndarray, int, np.ndarray, np.ndarray, np.ndarray]:
        """These forks' rows, each row's option (numbered through the batch), how many options, and the pairs
        (first option, second option, target share)."""
        rows, opt_of, pa, pb, pt = [], [], [], [], []
        n = 0
        for i in ids:
            f = self.forks[i]
            for k, o in enumerate(f["options"]):
                rows.append(o["rows"])
                opt_of.append(np.full(len(o["rows"]), n + k, np.int64))
            for a_, b_, t in f["pairs"]:
                pa.append(n + a_)
                pb.append(n + b_)
                pt.append(t)
            n += len(f["options"])
        return (np.concatenate(rows), np.concatenate(opt_of), n, np.array(pa, np.int64), np.array(pb, np.int64),
                np.array(pt, np.float32))


def finite_mean(x: np.ndarray) -> float:
    """The mean of the finite values (NaN if none)."""
    x = x[np.isfinite(x)]
    return float(x.mean()) if len(x) else float("nan")


def option_values(torch, z, opt_of, n: int):
    """Each option's value: the mean of its copies' win logits."""
    counts = torch.zeros(n, device=z.device).index_add_(0, opt_of, torch.ones_like(z))
    return torch.zeros(n, device=z.device).index_add_(0, opt_of, z) / counts.clamp(min=1)


def pair_loss(torch, z, opt_of, n: int, pa, pb, pt):
    """The soft pairwise loss over a batch of forks: the difference of two options' values trained towards the
    share of copies where the first ended better."""
    v = option_values(torch, z, opt_of, n)
    return torch.nn.functional.binary_cross_entropy_with_logits(v[pa] - v[pb], pt)


def fork_batch_loss(torch, net, fb: Batches, fs: ForkSet, ids, mean, scale, device):
    rows, opt_of, n, pa, pb, pt = fs.batch(ids)
    t = torch.from_numpy
    z = net((fb.x(t(rows).to(device)) - mean) * scale)[:, 0]
    return pair_loss(torch, z, t(opt_of).to(device), n, t(pa).to(device), t(pb).to(device), t(pt).to(device))


def judge_forks(fs: ForkSet, ids, extra: dict[str, np.ndarray] | None = None) -> dict:
    """The judge test on these forks. For each judge (the networks the forks were made with: "ensemble"; the
    teacher's quick look; the teacher's own choice; the run's move; picking at random; and any `extra` judge, given
    as its pick for each fork, or -1 for none): how often its choice was the option whose play-outs went best
    ("picked_best"), what it gave away against that option ("regret": the best is the best of noisy copies, so this
    is inflated alike for every judge), and its gain over a random option in the play-outs' win rate ("gain": not
    inflated, since no judge looks at the outcomes). An oracle that picks by the even copies, measured on the odd
    ones, gives the ceiling a judge that knew half the play-outs would reach ("oracle_half_gain")."""
    extra = extra or {}
    judges: dict[str, list[tuple[float, float, float]]] = {}
    oracle = []
    spread, options_n = [], []
    for k, i in enumerate(ids):
        f = fs.forks[i]
        opts = f["options"]
        m = len(opts)
        won = [o["won"] for o in opts]
        wr = np.array([w.mean() for w in won])
        score = np.array([o["key"].sum() for o in opts])
        best = int(np.lexsort((-np.arange(m), score, wr))[-1])
        logits = np.array([o["logit"] for o in opts])
        quick = np.array([o["quick"] for o in opts])
        picks = {
            "ensemble": int(np.nanargmax(logits)) if np.isfinite(logits).any() else None,
            "quick": int(np.nanargmax(quick)) if np.isfinite(quick).any() else None,
            "chosen": next((j for j, o in enumerate(opts) if o["chosen"]), None),
            "made": next((j for j, o in enumerate(opts) if o["made"]), None),
        }
        for name, arr in extra.items():
            picks[name] = int(arr[k]) if 0 <= int(arr[k]) < m else None
        for name, pick in picks.items():
            if pick is not None:
                judges.setdefault(name, []).append((float(pick == best), float(wr[best] - wr[pick]),
                                                    float(wr[pick] - wr.mean())))
        judges.setdefault("random", []).append((1.0 / m, float(wr[best] - wr.mean()), 0.0))
        copies = min(len(w) for w in won)
        if copies >= 2:
            even = np.array([w[:copies][0::2].mean() for w in won])
            odd = np.array([w[:copies][1::2].mean() for w in won])
            oracle.append(float(odd[int(np.argmax(even))] - odd.mean()))
        spread.append(float(wr.max() - wr.min()))
        options_n.append(m)
    out: dict = {"forks": len(ids)}
    for name in ("network", "ensemble", "quick", "chosen", "made", "random", *extra):
        rows = judges.get(name)
        if rows and name not in out:
            out[name] = {"judged": len(rows), "picked_best": round(float(np.mean([b for b, _, _ in rows])), 4),
                         "regret": round(float(np.mean([r for _, r, _ in rows])), 4),
                         "gain": round(float(np.mean([g for _, _, g in rows])), 4)}
    if oracle:
        out["oracle_half_gain"] = round(float(np.mean(oracle)), 4)
    if spread:
        out["spread"] = {"mean": round(float(np.mean(spread)), 4),
                         "a_copy_or_more": round(float(np.mean([x > 1e-9 for x in spread])), 4)}
        out["options_mean"] = round(float(np.mean(options_n)), 2)
    return out


def judge_lines(j: dict) -> list[str]:
    """The judge test as lines: each judge's picked-best share, regret and gain over random, then the oracle."""
    out = []
    for name in ("network", "ensemble", "quick", "chosen", "made", "random"):
        if name in j:
            r = j[name]
            out.append(f"{name:9} picked best {100 * r['picked_best']:5.1f}%   regret {r['regret']:.3f}   gain over "
                       f"random {r['gain']:+.3f}   ({r['judged']:,} forks)")
    if "oracle_half_gain" in j:
        out.append(f"an oracle that saw half the copies gains {j['oracle_half_gain']:+.3f} on the other half")
    if "spread" in j:
        out.append(f"the options' win rates differ by {j['spread']['mean']:.3f} on average, by a copy or more at "
                   f"{100 * j['spread']['a_copy_or_more']:.0f}% of the forks; {j['options_mean']} options a fork")
    return out


def fork_measures(torch, net, fb: Batches, fs: ForkSet, ids, mean, scale, device, batch: int) -> dict:
    """The judge test on these forks (`judge_forks`) with the network as a judge (the option whose position it
    rates highest), and the network's pairwise loss."""
    if len(ids) == 0:
        return {"forks": 0}
    net.eval()
    picks = np.full(len(ids), -1, np.int64)
    losses_, n_pairs = 0.0, 0
    with torch.no_grad():
        for k in range(0, len(ids), batch):
            chunk = ids[k:k + batch]
            rows, opt_of, n, pa, pb, pt = fs.batch(chunk)
            t = torch.from_numpy
            z = net((fb.x(t(rows).to(device)) - mean) * scale)[:, 0]
            opt_of_t = t(opt_of).to(device)
            v = option_values(torch, z, opt_of_t, n).cpu().numpy()
            if len(pa):
                losses_ += float(pair_loss(torch, z, opt_of_t, n, t(pa).to(device), t(pb).to(device),
                                           t(pt).to(device))) * len(pa)
                n_pairs += len(pa)
            at = 0
            for j, i in enumerate(chunk):
                m = len(fs.forks[i]["options"])
                picks[k + j] = int(np.argmax(v[at:at + m]))
                at += m
    out = judge_forks(fs, ids, {"network": picks})
    out["pair_bce"] = round(losses_ / max(n_pairs, 1), 5)
    return out


def input_scale(torch, b: Batches, rows, batch: int):
    """Each input's mean and scale over the training rows (a sample of them): x' = (x - mean) * scale.
    Inputs that never vary are left out (scale 0); the scale's divisor is at least 0.25, so a rare one-hot
    doesn't become a huge number."""
    s = torch.zeros(b.inputs, dtype=torch.float64, device=b.device)
    s2 = torch.zeros_like(s)
    for k in range(0, len(rows), batch):
        x = b.x(rows[k:k + batch]).double()
        s += x.sum(0)
        s2 += (x * x).sum(0)
    n = max(len(rows), 1)
    mean = s / n
    sd = (s2 / n - mean * mean).clamp(min=0).sqrt()
    scale = torch.where(sd > 1e-6, 1.0 / sd.clamp(min=0.25), torch.zeros_like(sd))
    return mean.float(), scale.float()


def losses(torch, out, lab, aux: float, off=None):
    """Per row: the win loss, and the loss trained on (with the ending stage's, and for a network with the depth
    outputs, a win's depth where its recording played on). With offsets the win output is a correction to them."""
    f = torch.nn.functional
    z = out[:, 0] if off is None else out[:, 0] + off
    win = f.binary_cross_entropy_with_logits(z, lab[:, WON], reduction="none")
    stage = f.cross_entropy(out[:, 1:1 + STAGES], lab[:, END_STAGE].long(), reduction="none")
    total = win + aux * stage
    if out.shape[1] > 1 + STAGES and lab.shape[1] > ENDLESS:
        known = (lab[:, WON] > 0.5) & (lab[:, ENDLESS] >= 0)
        depth = f.cross_entropy(out[:, 1 + STAGES:], lab[:, ENDLESS].clamp(0, DEPTH - 1).long(), reduction="none")
        total = total + aux * torch.where(known, depth, torch.zeros_like(depth))
    return win, total


def predict(torch, net, b: Batches, rows, mean, scale, batch: int) -> np.ndarray:
    """The network's outputs for these rows (eval mode), as numpy."""
    net.eval()
    outs = []
    with torch.no_grad():
        for k in range(0, len(rows), batch):
            x = (b.x(rows[k:k + batch]) - mean) * scale
            outs.append(net(x).float().cpu())
    return torch.cat(outs).numpy() if outs else np.zeros((0, 1 + STAGES), np.float32)


def depth_measures(out: np.ndarray, lab: np.ndarray) -> dict:
    """How well the depth outputs tell a strong win from a weak one, on the kept-apart wins whose recording played
    on: the cross-entropy against knowing only how often each depth comes up, the mean depth expected and found,
    and the chance of three blinds or more after the win, expected and found."""
    known = (lab[:, WON] > 0.5) & (lab[:, ENDLESS] >= 0) if lab.shape[1] > ENDLESS else np.zeros(len(lab), bool)
    if not known.any():
        return {"wins": 0}
    z = out[known, 1 + STAGES:].astype(np.float64)
    z -= z.max(1, keepdims=True)
    q = np.exp(z)
    q /= q.sum(1, keepdims=True)
    d = lab[known, ENDLESS].astype(int).clip(0, DEPTH - 1)
    freq = np.bincount(d, minlength=DEPTH) / len(d)
    ce = float(-np.mean(np.log(q[np.arange(len(d)), d] + 1e-12)))
    ce_freq = float(-np.mean(np.log(freq[d] + 1e-12)))
    levels = np.arange(DEPTH)
    return {"wins": int(known.sum()), "runs": len(np.unique(lab[known, RUN])), "ce": round(ce, 5),
            "ce_by_frequency_alone": round(ce_freq, 5), "mean_expected": round(float((q * levels).sum(1).mean()), 3),
            "mean_found": round(float(d.mean()), 3), "three_or_more_expected": round(float(q[:, 3:].sum(1).mean()), 4),
            "three_or_more_found": round(float((d >= 3).mean()), 4)}


def win_chance(out: np.ndarray, off: np.ndarray | None) -> np.ndarray:
    """The chance of winning from the network's outputs (and the rows' offsets, if it learned a correction)."""
    z = out[:, 0].astype(np.float64)
    return sigmoid(z if off is None else z + off.astype(np.float64))


# ---- Measures ----

def bce(p: np.ndarray, y: np.ndarray) -> float:
    p = np.clip(p, 1e-7, 1 - 1e-7)
    return float(-np.mean(y * np.log(p) + (1 - y) * np.log(1 - p))) if len(y) else float("nan")


def auc(p: np.ndarray, y: np.ndarray) -> float | None:
    """The chance a won position is rated above a lost one, a tie counting half (ranks averaged over ties: positions
    rated alike, as early positions of many runs can be, say nothing either way, whatever order they come in)."""
    pos, neg = int(y.sum()), int(len(y) - y.sum())
    if pos == 0 or neg == 0:
        return None
    _, inverse, counts = np.unique(p, return_inverse=True, return_counts=True)
    # Distinct values in ascending order: each tie group's ranks run from its start to its end, averaged.
    start = np.concatenate([[0], np.cumsum(counts)[:-1]])
    ranks = (start + (counts + 1) / 2.0)[inverse.reshape(-1)]
    return float((ranks[y == 1].sum() - pos * (pos + 1) / 2) / (pos * neg))


def sigmoid(z: np.ndarray) -> np.ndarray:
    return 1.0 / (1.0 + np.exp(-z))


def stage_rates(lab: np.ndarray) -> np.ndarray:
    """The win rate from each stage (blinds behind the run), the network's yardstick: what knowing only how
    far the run has got predicts."""
    rates = np.full(24, lab[:, WON].mean() if len(lab) else 0.5)
    for s in range(24):
        m = lab[:, STAGE] == s
        if m.sum() >= 20:
            rates[s] = lab[m, WON].mean()
    return rates


def report_measures(p: np.ndarray, lab: np.ndarray, rates: np.ndarray) -> dict:
    y = lab[:, WON]
    base = rates[lab[:, STAGE].astype(int).clip(0, 23)]
    by_ante = []
    for a in range(1, 9):
        m = lab[:, ANTE] == a
        if m.sum() == 0:
            continue
        by_ante.append({"ante": a, "rows": int(m.sum()), "won": round(float(y[m].mean()), 4),
                        "predicted": round(float(p[m].mean()), 4), "bce": round(bce(p[m], y[m]), 4),
                        "bce_by_stage_alone": round(bce(base[m], y[m]), 4),
                        "auc_within_ante": None if auc(p[m], y[m]) is None else round(auc(p[m], y[m]), 4)})
    calib = []
    for lo in np.linspace(0, 0.9, 10):
        m = (p >= lo) & (p < lo + 0.1 + (1e-9 if lo >= 0.9 else 0))
        if m.sum():
            calib.append({"predicted": [round(float(lo), 1), round(float(lo + 0.1), 1)], "rows": int(m.sum()),
                          "mean_predicted": round(float(p[m].mean()), 4), "won": round(float(y[m].mean()), 4)})
    a = auc(p, y)
    return {"rows": len(y), "runs": len(np.unique(lab[:, RUN])), "won": round(float(y.mean()), 4),
            "bce": round(bce(p, y), 5), "bce_by_stage_alone": round(bce(base, y), 5),
            "brier": round(float(np.mean((p - y) ** 2)), 5), "auc": None if a is None else round(a, 4),
            "by_ante": by_ante, "calibration": calib}


def by_stake(p: np.ndarray, lab: np.ndarray, stakes: np.ndarray, before: np.ndarray | None = None) -> list[dict]:
    """The measures stake by stake (`before`: the value networks' own chances, with offsets, beside them)."""
    out = []
    for st in sorted(set(stakes.tolist())):
        m = stakes == st
        y = lab[m, WON]
        a = auc(p[m], y)
        row = {"stake": int(st), "rows": int(m.sum()), "runs": len(np.unique(lab[m, RUN])),
               "won": round(float(y.mean()), 4), "predicted": round(float(p[m].mean()), 4),
               "bce": round(bce(p[m], y), 5), "auc": None if a is None else round(a, 4)}
        if before is not None:
            b = auc(before[m], y)
            row.update({"predicted_before": round(float(before[m].mean()), 4),
                        "bce_before": round(bce(before[m], y), 5), "auc_before": None if b is None else round(b, 4)})
        out.append(row)
    return out


# ---- The engine's format ----

def export_bmvn(path: Path, layers: list[tuple[np.ndarray, np.ndarray, bool]], input_version: int) -> bytes:
    """Write a network as BMVN: `layers` are (weights out x in, biases, ReLU after) in order."""
    inputs = layers[0][0].shape[1]
    out = bytearray(b"BMVN")
    out += struct.pack("<4I", FORMAT, input_version, inputs, len(layers))
    width = inputs
    for w, b, relu in layers:
        o, i = w.shape
        assert i == width and b.shape == (o,), "the layers don't fit together"
        out += struct.pack("<3I", i, o, 1 if relu else 0)
        out += np.ascontiguousarray(w.T, dtype="<f4").tobytes()  # input-major
        out += np.asarray(b, dtype="<f4").tobytes()
        width = o
    data = bytes(out)
    path.parent.mkdir(parents=True, exist_ok=True)
    tmp = path.with_suffix(path.suffix + ".tmp")
    tmp.write_bytes(data)
    tmp.replace(path)
    return data


def read_bmvn(data: bytes) -> tuple[int, int, list[tuple[np.ndarray, np.ndarray, bool]]]:
    """A BMVN file's input layout version, input count and layers (weights out x in)."""
    if data[:4] != b"BMVN":
        raise ValueError("not a value network file")
    fmt, version, inputs, n = struct.unpack_from("<4I", data, 4)
    if fmt != FORMAT:
        raise ValueError(f"unknown format {fmt}")
    at, layers = 20, []
    for _ in range(n):
        i, o, act = struct.unpack_from("<3I", data, at)
        at += 12
        w = np.frombuffer(data, "<f4", i * o, at).reshape(i, o).T
        at += 4 * i * o
        b = np.frombuffer(data, "<f4", o, at)
        at += 4 * o
        layers.append((w, b, act == 1))
    if at != len(data):
        raise ValueError("trailing bytes")
    return version, inputs, layers


def forward_np(layers, x: np.ndarray) -> np.ndarray:
    h = x.astype(np.float32)
    for w, b, relu in layers:
        h = h @ w.T + b
        if relu:
            h = np.maximum(h, 0)
    return h


def folded_layers(net, mean, scale) -> list[tuple[np.ndarray, np.ndarray, bool]]:
    """The trained network's layers with the input scaling folded into the first: W(x - m)s + b =
    (W s) x + (b - W (m s))."""
    import torch
    linears = [m for m in net if isinstance(m, torch.nn.Linear)]
    out = []
    for k, lin in enumerate(linears):
        w = lin.weight.detach().double().cpu().numpy()
        b = lin.bias.detach().double().cpu().numpy()
        if k == 0:
            s = scale.double().cpu().numpy()
            m = mean.double().cpu().numpy()
            b = b - w @ (m * s)
            w = w * s[None, :]
        out.append((w.astype(np.float32), b.astype(np.float32), k < len(linears) - 1))
    return out


def engine_forward():
    """The engine's own forward pass, if the engine module is built."""
    try:
        from balatro_meta._engine import value_net_forward
    except ImportError:
        return None
    return value_net_forward


# ---- Training ----

def fit(torch, b: Batches, lab: np.ndarray, train_rows, judge_rows, mean, scale, cfg: dict, a, device,
        forks: tuple[Batches, ForkSet] | None = None):
    """Train one network; keep it as it was when the judging runs scored best (with forks, their pairwise loss on
    the judging forks counts too, at the pair weight)."""
    torch.manual_seed(a.seed)
    in_drop = a.input_dropout if cfg.get("input_dropout") is None else cfg["input_dropout"]
    net = make_net(torch, b.inputs, cfg["hidden"], cfg["dropout"], in_drop, depth=b.depth).to(device)
    residual = b.off is not None
    if residual:
        # A correction starts at none: the value networks' own answer.
        with torch.no_grad():
            net[-1].weight[0].zero_()
            net[-1].bias[0].zero_()
    decay = [p for n, p in net.named_parameters() if n.endswith("weight")]
    rest = [p for n, p in net.named_parameters() if not n.endswith("weight")]
    opt = torch.optim.AdamW([{"params": decay, "weight_decay": cfg["weight_decay"]},
                             {"params": rest, "weight_decay": 0.0}], lr=a.lr)
    tr = torch.from_numpy(train_rows).to(device)
    ju = torch.from_numpy(judge_rows).to(device)
    per_epoch = max(1, math.ceil(len(train_rows) / a.batch))
    total_steps = a.epochs * per_epoch
    every = max(25, per_epoch // a.evals_per_epoch)
    sched = torch.optim.lr_scheduler.OneCycleLR(opt, max_lr=a.lr, total_steps=total_steps, pct_start=0.1)
    gen = torch.Generator(device="cpu").manual_seed(a.seed)
    best = (math.inf, 0, None)
    history, stale, step, seen, loss_sum = [], 0, 0, 0, 0.0
    y = lab[judge_rows, WON]
    judge_off = None if not residual else b.off[ju].cpu().numpy()
    fork_train = forks[1].ids(0) if forks is not None else np.zeros(0, np.int64)
    fork_judge = forks[1].ids(1) if forks is not None else np.zeros(0, np.int64)
    fork_rng = np.random.default_rng(a.seed)

    def judge() -> tuple[float, float, float | None]:
        """The judging runs' BCE, and with forks the judging forks' pairwise loss, weighed in."""
        out = predict(torch, net, b, ju, mean, scale, a.batch * 4)
        pos = bce(win_chance(out, judge_off), y)
        if forks is None or len(fork_judge) == 0:
            return pos, pos, None
        pairs = fork_measures(torch, net, forks[0], forks[1], fork_judge, mean, scale, device, 64)["pair_bce"]
        return pos + a.pair_weight * pairs, pos, pairs

    if residual:
        # No correction is the one to beat: kept unless training judges better.
        judged, pos, pairs = judge()
        best = (judged, 0, {k2: v.detach().clone() for k2, v in net.state_dict().items()})
        history.append({"step": 0, "epoch": 0.0, "train_loss": None, "judging_bce": round(pos, 5),
                        "judging_pairs": pairs})
    while step < total_steps and stale < a.patience:
        perm = tr[torch.randperm(len(tr), generator=gen).to(device)]
        for k in range(0, len(perm), a.batch):
            net.train()
            rows = perm[k:k + a.batch]
            _, loss = losses(torch, net((b.x(rows) - mean) * scale), b.lab[rows], a.aux,
                             None if b.off is None else b.off[rows])
            total = loss.mean()
            if forks is not None and len(fork_train):
                ids = fork_rng.choice(fork_train, size=min(a.fork_batch, len(fork_train)), replace=False)
                total = total + a.pair_weight * fork_batch_loss(torch, net, forks[0], forks[1], ids, mean, scale,
                                                                device)
            opt.zero_grad(set_to_none=True)
            total.backward()
            opt.step()
            sched.step()
            step += 1
            loss_sum += float(loss.detach().sum())
            seen += len(rows)
            if step % every == 0 or step == total_steps:
                judged, pos, pairs = judge()
                history.append({"step": step, "epoch": round(step / per_epoch, 2),
                                "train_loss": round(loss_sum / max(seen, 1), 5), "judging_bce": round(pos, 5),
                                "judging_pairs": None if pairs is None else round(pairs, 5)})
                loss_sum, seen = 0.0, 0
                if judged < best[0] - 1e-5:
                    best = (judged, step, {k2: v.detach().clone() for k2, v in net.state_dict().items()})
                    stale = 0
                else:
                    stale += 1
                if stale >= a.patience or step == total_steps:
                    break
    net.load_state_dict(best[2])
    net.eval()
    return net, best[0], best[1] / per_epoch, history


def train(a) -> dict:
    torch = need_torch()
    t0 = time.time()
    rng = np.random.default_rng(a.seed)
    folders = [Path(d).expanduser() for d in a.data]
    data = load_data(folders, a.max_runs, a.input_layout, offsets=a.offset)
    man = data.manifest
    if data.rows == 0:
        raise SystemExit("The recording has no positions.")
    # The newest recording's runs (with --newest K, the last K recordings': a round recorded at several stakes,
    # one recording each) are split to train on, judge training and measure; older recordings' runs are all
    # trained on (more positions to learn from), so the measures are the newest teacher's alone.
    if not 1 <= a.newest <= len(folders):
        raise SystemExit(f"--newest {a.newest}: between 1 and the {len(folders)} recordings given.")
    newest = data.source >= len(folders) - a.newest
    part = np.where(newest, split(data.lab[:, RUN], a.split), 0)
    train_rows, judge_rows, test_rows = (np.flatnonzero(part == k) for k in range(3))
    if min(len(train_rows), len(judge_rows), len(test_rows)) == 0:
        raise SystemExit("Too few runs to keep some apart.")
    runs = [len(np.unique(data.lab[r, RUN])) for r in (train_rows, judge_rows, test_rows)]
    device = torch.device("cuda" if torch.cuda.is_available() and not a.cpu else "cpu")
    dev_name = torch.cuda.get_device_name(0) if device.type == "cuda" else "cpu"
    total_runs = sum(int(m["runs"]) for m in data.manifests)
    older = f" (with {len(folders) - 1} older recording{'s' if len(folders) > 2 else ''} trained on)" \
        if len(folders) > 1 else ""
    print(f"{data.rows:,} positions from {total_runs:,} runs{older} ({runs[0]:,} to train on, {runs[1]:,} to judge "
          f"training, {runs[2]:,} to measure); {data.inputs} inputs; {dev_name}", flush=True)
    b = Batches(torch, data, device)
    forks = None
    if a.forks:
        fork_data = load_data([Path(d).expanduser() for d in a.forks], layout=data.manifest["input_version"])
        if fork_data.inputs != data.inputs:
            raise SystemExit(f"The forks have {fork_data.inputs} inputs, the positions {data.inputs}.")
        fs = ForkSet(fork_data, a.fork_split or a.split)
        forks = (Batches(torch, fork_data, device), fs)
        print(f"{len(fs):,} forked decisions ({fork_data.rows:,} positions the options left; {len(fs.ids(0)):,} to "
              f"train on, {len(fs.ids(1)):,} to judge, {len(fs.ids(2)):,} to measure), pair weight {a.pair_weight}",
              flush=True)
    sample = torch.from_numpy(rng.permutation(train_rows)[:a.scale_rows]).to(device)
    mean, scale = input_scale(torch, b, sample, a.batch)
    rates = stage_rates(data.lab[train_rows])
    jl = data.lab[judge_rows]
    judge_base = bce(rates[jl[:, STAGE].astype(int).clip(0, 23)], jl[:, WON])
    print(f"Judging runs: BCE {judge_base:.4f} knowing only the stage.", flush=True)
    judge_before = None
    if data.off is not None:
        judge_before = bce(sigmoid(data.off[judge_rows].astype(np.float64)), jl[:, WON])
        print(f"Judging runs: BCE {judge_before:.4f} by the value networks the correction is for "
              f"({shown(data.offset_net)}).", flush=True)
    tried = []
    best = None
    for cfg in parse_configs(a.configs):
        net, judged, epoch, history = fit(torch, b, data.lab, train_rows, judge_rows, mean, scale, cfg, a, device,
                                          forks)
        tried.append({**cfg, "judging_bce": round(judged, 5), "best_epoch": round(epoch, 2), "history": history})
        print(f"{cfg['hidden']} dropout {cfg['dropout']} decay {cfg['weight_decay']}: judging "
              f"{'loss' if forks else 'BCE'} {judged:.4f} at epoch {epoch:.1f}", flush=True)
        if best is None or judged < best[1]:
            best = (net, judged, cfg)
    net, _, cfg = best

    # Measures on the runs kept apart, against what knowing only how far the run has got predicts.
    out = predict(torch, net, b, torch.from_numpy(test_rows).to(device), mean, scale, a.batch * 4)
    test_off = None if data.off is None else data.off[test_rows]
    p = win_chance(out, test_off)
    measures = report_measures(p, data.lab[test_rows], rates)
    before = None if test_off is None else sigmoid(test_off.astype(np.float64))
    measures["by_stake"] = by_stake(p, data.lab[test_rows], data.stakes[test_rows], before)
    if before is not None:
        measures["before"] = report_measures(before, data.lab[test_rows], rates)
    if out.shape[1] > 1 + STAGES:
        measures["depth"] = depth_measures(out, data.lab[test_rows])
    if forks is not None:
        measures["forks"] = fork_measures(torch, net, forks[0], forks[1], forks[1].ids(2), mean, scale, device, 64)

    # The engine's format, read back and checked.
    layers = folded_layers(net, mean, scale)
    path = Path(a.out)
    blob = export_bmvn(path, layers, int(man["input_version"]))
    version, _inputs, back = read_bmvn(blob)
    x = data.dense(test_rows[:512])
    got = forward_np(back, x)[:, 0]
    want = out[:len(x), 0]
    file_diff = float(np.max(np.abs(got - want))) if len(want) else 0.0
    if file_diff > 2e-3:
        raise SystemExit(f"The written network differs from the trained one by {file_diff}: not saved correctly.")
    engine = engine_forward()
    engine_diff = None
    if engine is not None:
        e = np.array([engine(blob, row.tolist())[0] for row in x[:64]], np.float32)
        engine_diff = float(np.max(np.abs(e - got[:64]))) if len(e) else 0.0
        if engine_diff > 1e-3:
            raise SystemExit(f"The engine reads the network differently (by {engine_diff}).")
    secs = time.time() - t0
    report = {
        "kind": "value-net", "data": shown(a.data[-1]), "recording": {k: man.get(k) for k in (
            "bot", "seed_base", "runs", "rows", "wins", "value_net", "value_weight", "teacher_options")},
        "older_recordings": [{"data": shown(d), **{k: m.get(k) for k in ("bot", "seed_base", "runs", "rows", "wins",
                                                                       "value_net", "value_weight")}}
                             for d, m in zip(a.data[:-1], data.manifests[:-1])],
        "inputs": data.inputs, "input_version": version, "chosen": cfg, "tried": tried,
        "forks": None if forks is None else {"data": [shown(d) for d in a.forks], "forked": len(forks[1]),
                                             "pair_weight": a.pair_weight, "fork_batch": a.fork_batch},
        "residual": None if data.off is None else {
            "offset_net": shown(data.offset_net), "judging_bce_before": round(judge_before, 5),
            "note": "the network's win output is a correction to these networks' log-odds (baseline --stake-net)"},
        "judging_bce_by_stage_alone": round(judge_base, 5),
        "input_dropout": a.input_dropout, "aux_weight": a.aux, "lr": a.lr, "batch": a.batch, "seed": a.seed,
        "runs": {"train": runs[0], "judge": runs[1], "measure": runs[2]},
        "rows": {"train": len(train_rows), "judge": len(judge_rows), "measure": len(test_rows)},
        "measured": measures, "model": str(path), "model_bytes": len(blob),
        "model_sha256": hashlib.sha256(blob).hexdigest(), "check": {"file": file_diff, "engine": engine_diff},
        "device": dev_name, "torch": torch.__version__, "seconds": round(secs, 1),
    }
    if a.report:
        rp = Path(a.report)
        rp.parent.mkdir(parents=True, exist_ok=True)
        rp.write_text(json.dumps(report, indent=1) + "\n")
    m = measures
    print(f"Chosen: {cfg['hidden']} dropout {cfg['dropout']} decay {cfg['weight_decay']}. On the {runs[2]:,} runs "
          f"kept apart: BCE {m['bce']} (knowing only the stage: {m['bce_by_stage_alone']}), AUC {m['auc']}, "
          f"Brier {m['brier']}; {secs / 60:.1f} min.")
    if "before" in m:
        print(f"The value networks alone on the same runs: BCE {m['before']['bce']}, AUC {m['before']['auc']}.")
    for r in m["by_ante"]:
        print(f"  ante {r['ante']}: {r['rows']:,} positions, won {r['won']:.3f}, predicted {r['predicted']:.3f}, "
              f"AUC within the ante {r['auc_within_ante']}")
    for r in m["by_stake"]:
        print(f"  stake {r['stake']}: {r['rows']:,} positions, won {r['won']:.3f}, predicted {r['predicted']:.3f}, "
              f"BCE {r['bce']}" + (f" (alone {r['bce_before']}), AUC {r['auc']} (alone {r['auc_before']})"
                                   if "bce_before" in r else f", AUC {r['auc']}"))
    fk = m.get("forks") or {}
    if fk.get("forks"):
        print(f"  the judge test on {fk['forks']:,} forks kept apart (pairwise loss {fk['pair_bce']}):")
        for line in judge_lines(fk):
            print("   " + line)
    d = m.get("depth") or {}
    if d.get("wins"):
        print(f"  depth after a win ({d['wins']:,} positions of {d['runs']:,} won runs): cross-entropy {d['ce']} "
              f"(knowing only how often each depth comes: {d['ce_by_frequency_alone']}), blinds after the win "
              f"expected {d['mean_expected']}, found {d['mean_found']}; three or more expected "
              f"{d['three_or_more_expected']}, found {d['three_or_more_found']}")
    print(f"Network: {path} ({len(blob):,} bytes), checked against "
          f"{'the engine' if engine_diff is not None else 'its own reading of the file'}.")
    return report


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--data", required=True, action="append",
                    help="a recording (baseline --record DIR); give it more than once to train on older recordings "
                         "too, the newest last (it alone is judged and measured on)")
    ap.add_argument("--out", required=True, help="where to write the network (BMVN)")
    ap.add_argument("--report", help="where to write the training report (JSON)")
    ap.add_argument("--configs", default=CONFIGS,
                    help="networks to try: widths/dropout/decay[/input dropout]; ...")
    ap.add_argument("--input-dropout", type=float, default=0.1, help="for configs that don't give their own")
    ap.add_argument("--aux", type=float, default=1.0, help="weight of the end-stage loss")
    ap.add_argument("--lr", type=float, default=1e-3)
    ap.add_argument("--batch", type=int, default=2048)
    ap.add_argument("--epochs", type=int, default=6, help="the learning rate's schedule (it may stop sooner)")
    ap.add_argument("--evals-per-epoch", type=int, default=8)
    ap.add_argument("--patience", type=int, default=8, help="judgings without improvement to stop at")
    ap.add_argument("--scale-rows", type=int, default=200_000, help="rows sampled for the input scaling")
    ap.add_argument("--max-runs", type=int, help="use only the first runs of the recording")
    ap.add_argument("--input-layout", type=int,
                    help="the input layout to train on (default: the newest every recording holds)")
    ap.add_argument("--newest", type=int, default=1,
                    help="how many of the last recordings are the newest (split to judge and measure; default 1)")
    ap.add_argument("--forks", action="append",
                    help="a fork recording (baseline --forks DIR): the network also learns to put each forked "
                         "decision's options in the order their play-outs went; give it more than once for more")
    ap.add_argument("--pair-weight", type=float, default=1.0, help="the forks' pairwise loss against the positions'")
    ap.add_argument("--fork-batch", type=int, default=16, help="forks in each training step's pairwise batch")
    ap.add_argument("--fork-split", type=int, help="one fork's run in this many judges, another measures (default: "
                                                   "--split; fewer forks than positions want a larger share)")
    ap.add_argument("--offset", action="store_true",
                    help="learn a correction to the value networks' log-odds the recordings keep (baseline --offsets)")
    ap.add_argument("--split", type=int, default=SPLIT, help="one run in this many judges, another measures")
    ap.add_argument("--seed", type=int, default=1)
    ap.add_argument("--cpu", action="store_true", help="train on the CPU even with a GPU")
    a = ap.parse_args()
    train(a)


if __name__ == "__main__":
    sys.exit(main())
