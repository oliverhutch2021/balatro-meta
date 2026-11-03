"""Train the teacher's learned value (crates/bm-core/src/learned.rs) on the GPU.

    python scripts/train_value.py --data ~/.cache/balatro-meta/data/value-1 --out models/value-1.bin \
        --report results/value/value-1.json

The data is what `baseline --record DIR` writes: every position the teacher valued between rounds (the blind
choice, the shop, open packs) as the value network's input (observe.rs: `encode_value`, read from the screen
alone), labelled with how its run ended. The network learns the chance of winning the run from a position
and, alongside, the blind the run ends at, which gives it more to learn from than the wins alone.

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


# ---- Data ----

class Data:
    """Recorded positions, sparse: row r's nonzero inputs are idx/val[ptr[r]:ptr[r+1]]; lab[r] its labels. With
    several recordings, source[r] is the recording row r came from (the last is the newest), and the manifest is the
    newest one's."""

    def __init__(self, idx: np.ndarray, val: np.ndarray, ptr: np.ndarray, lab: np.ndarray, manifest: dict,
                 source: np.ndarray | None = None, manifests: list[dict] | None = None):
        self.idx, self.val, self.ptr, self.lab, self.manifest = idx, val, ptr, lab, manifest
        self.source = np.zeros(len(lab), np.int8) if source is None else source
        self.manifests = [manifest] if manifests is None else manifests

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


def load_data(folders: Path | list[Path], max_runs: int | None = None) -> Data:
    """Every chunk of one or more recordings, each in run order (the recorder writes chunks of whole runs, sorted),
    the newest last."""
    folders = [folders] if isinstance(folders, (str, Path)) else list(folders)
    parts = []
    for folder in folders:
        man_path = folder / "manifest.json"
        if not man_path.exists():
            raise SystemExit(f"{folder} has no manifest.json: the recording is missing or didn't finish "
                             f"(make teacher-data).")
        manifest = json.loads(man_path.read_text())
        chunks = sorted(p.name[: -len("-ptr.npy")] for p in folder.glob("chunk-*-ptr.npy"))
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
    first = parts[-1][1]
    for folder, manifest, _, _ in parts:
        for key in ("inputs", "input_version", "labels"):
            if manifest.get(key) != first.get(key):
                raise SystemExit(f"{folder}: its {key} differ from the newest recording's, so they can't be mixed.")
    if len(parts) > 9:
        raise SystemExit("At most 9 recordings at once.")
    n_rows = sum(r for _, _, _, sizes in parts for r, _ in sizes)
    n_nz = sum(z for _, _, _, sizes in parts for _, z in sizes)
    idx = np.empty(n_nz, np.int16)  # the inputs number fewer than 32,768
    val = np.empty(n_nz, np.float16)
    ptr = np.empty(n_rows + 1, np.int64)
    lab = np.empty((n_rows, len(first["labels"])), np.float32)
    source = np.empty(n_rows, np.int8)
    ptr[0] = 0
    r0 = z0 = 0
    for k, (folder, manifest, chunks, sizes) in enumerate(parts):
        start = r0
        for c, (nr, nz) in zip(chunks, sizes):
            cp = np.load(folder / f"{c}-ptr.npy")
            if len(cp) != nr + 1 or cp[0] != 0 or cp[-1] != nz:
                raise SystemExit(f"{folder}/{c}: its row pointers don't match its rows.")
            idx[z0:z0 + nz] = np.load(folder / f"{c}-idx.npy").view(np.int16)
            val[z0:z0 + nz] = np.load(folder / f"{c}-val.npy")
            ptr[r0 + 1:r0 + nr + 1] = cp[1:] + z0
            lab[r0:r0 + nr] = np.load(folder / f"{c}-lab.npy")
            r0, z0 = r0 + nr, z0 + nz
        if max_runs is not None and r0 > start:
            # Rows are in run order, so the first runs are a prefix; the next recording is written after it.
            r0 = start + int((lab[start:r0, RUN] < lab[start, RUN] + max_runs).sum())
            z0 = int(ptr[r0])
        source[start:r0] = k
        lab[start:r0, RUN] += RUN_OFFSET * (len(parts) - 1 - k)
    idx, val, ptr, lab, source = idx[:z0], val[:z0], ptr[:r0 + 1], lab[:r0], source[:r0]
    if len(lab) and (idx.min() < 0 or idx.max() >= first["inputs"]):
        raise SystemExit(f"{parts[-1][0]}: an input index is out of range.")
    return Data(idx, val, ptr, lab, first, source, [m for _, m, _, _ in parts])


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


def make_net(torch, inputs: int, hidden: list[int], dropout: float, input_dropout: float = 0.0):
    nn = torch.nn
    layers, width = [nn.Dropout(input_dropout)], inputs
    for h in hidden:
        layers += [nn.Linear(width, h), nn.ReLU(), nn.Dropout(dropout)]
        width = h
    layers.append(nn.Linear(width, 1 + STAGES))
    return nn.Sequential(*layers)


class Batches:
    """Dense batches built from the sparse rows on the training device."""

    def __init__(self, torch, data: Data, device):
        self.torch, self.device, self.inputs = torch, device, data.inputs
        t = torch.from_numpy
        self.idx = t(data.idx).to(device)
        self.val = t(data.val).to(device)
        self.ptr = t(data.ptr).to(device)
        self.lab = t(data.lab).to(device)

    def x(self, rows):
        """The dense inputs of these rows (a LongTensor of row numbers on the device)."""
        torch = self.torch
        start = self.ptr[rows]
        n = self.ptr[rows + 1] - start
        total = int(n.sum())
        out = torch.zeros((len(rows), self.inputs), dtype=torch.float32, device=self.device)
        if total == 0:
            return out
        row_of = torch.repeat_interleave(torch.arange(len(rows), device=self.device), n)
        first = torch.repeat_interleave(torch.cumsum(n, 0) - n, n)
        flat = torch.repeat_interleave(start, n) + torch.arange(total, device=self.device) - first
        out[row_of, self.idx[flat].long()] = self.val[flat].float()
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


def losses(torch, out, lab, aux: float):
    f = torch.nn.functional
    win = f.binary_cross_entropy_with_logits(out[:, 0], lab[:, WON], reduction="none")
    stage = f.cross_entropy(out[:, 1:], lab[:, END_STAGE].long(), reduction="none")
    return win, win + aux * stage


def predict(torch, net, b: Batches, rows, mean, scale, batch: int) -> np.ndarray:
    """The network's outputs for these rows (eval mode), as numpy."""
    net.eval()
    outs = []
    with torch.no_grad():
        for k in range(0, len(rows), batch):
            x = (b.x(rows[k:k + batch]) - mean) * scale
            outs.append(net(x).float().cpu())
    return torch.cat(outs).numpy() if outs else np.zeros((0, 1 + STAGES), np.float32)


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

def fit(torch, b: Batches, lab: np.ndarray, train_rows, judge_rows, mean, scale, cfg: dict, a, device):
    """Train one network; keep it as it was when the judging runs scored best."""
    torch.manual_seed(a.seed)
    in_drop = a.input_dropout if cfg.get("input_dropout") is None else cfg["input_dropout"]
    net = make_net(torch, b.inputs, cfg["hidden"], cfg["dropout"], in_drop).to(device)
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
    while step < total_steps and stale < a.patience:
        perm = tr[torch.randperm(len(tr), generator=gen).to(device)]
        for k in range(0, len(perm), a.batch):
            net.train()
            rows = perm[k:k + a.batch]
            _, loss = losses(torch, net((b.x(rows) - mean) * scale), b.lab[rows], a.aux)
            opt.zero_grad(set_to_none=True)
            loss.mean().backward()
            opt.step()
            sched.step()
            step += 1
            loss_sum += float(loss.detach().sum())
            seen += len(rows)
            if step % every == 0 or step == total_steps:
                out = predict(torch, net, b, ju, mean, scale, a.batch * 4)
                judged = bce(sigmoid(out[:, 0].astype(np.float64)), y)
                history.append({"step": step, "epoch": round(step / per_epoch, 2),
                                "train_loss": round(loss_sum / max(seen, 1), 5), "judging_bce": round(judged, 5)})
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
    data = load_data(folders, a.max_runs)
    man = data.manifest
    if data.rows == 0:
        raise SystemExit("The recording has no positions.")
    # The newest recording's runs are split to train on, judge training and measure; older recordings' runs are
    # all trained on (more positions to learn from), so the measures are the newest teacher's alone.
    newest = data.source == len(folders) - 1
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
    sample = torch.from_numpy(rng.permutation(train_rows)[:a.scale_rows]).to(device)
    mean, scale = input_scale(torch, b, sample, a.batch)
    rates = stage_rates(data.lab[train_rows])
    jl = data.lab[judge_rows]
    judge_base = bce(rates[jl[:, STAGE].astype(int).clip(0, 23)], jl[:, WON])
    print(f"Judging runs: BCE {judge_base:.4f} knowing only the stage.", flush=True)
    tried = []
    best = None
    for cfg in parse_configs(a.configs):
        net, judged, epoch, history = fit(torch, b, data.lab, train_rows, judge_rows, mean, scale, cfg, a, device)
        tried.append({**cfg, "judging_bce": round(judged, 5), "best_epoch": round(epoch, 2), "history": history})
        print(f"{cfg['hidden']} dropout {cfg['dropout']} decay {cfg['weight_decay']}: judging BCE {judged:.4f} "
              f"at epoch {epoch:.1f}", flush=True)
        if best is None or judged < best[1]:
            best = (net, judged, cfg)
    net, _, cfg = best

    # Measures on the runs kept apart, against what knowing only how far the run has got predicts.
    out = predict(torch, net, b, torch.from_numpy(test_rows).to(device), mean, scale, a.batch * 4)
    p = sigmoid(out[:, 0].astype(np.float64))
    measures = report_measures(p, data.lab[test_rows], rates)

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
    for r in m["by_ante"]:
        print(f"  ante {r['ante']}: {r['rows']:,} positions, won {r['won']:.3f}, predicted {r['predicted']:.3f}, "
              f"AUC within the ante {r['auc_within_ante']}")
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
    ap.add_argument("--split", type=int, default=SPLIT, help="one run in this many judges, another measures")
    ap.add_argument("--seed", type=int, default=1)
    ap.add_argument("--cpu", action="store_true", help="train on the CPU even with a GPU")
    a = ap.parse_args()
    train(a)


if __name__ == "__main__":
    sys.exit(main())
