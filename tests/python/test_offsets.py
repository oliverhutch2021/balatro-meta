"""The trainer's offsets (train_value.py --offset): recordings that keep the value networks' log-odds of each
position, read beside the positions, for a network that learns a correction to them. Synthetic recordings, so
these run without the engine or PyTorch."""
from __future__ import annotations

import json
import sys
from pathlib import Path

import numpy as np
import pytest

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))

import train_value as tv

INPUTS = 12
LABELS = ["run", "step", "phase", "ante", "stage", "won", "end_stage", "end_ante"]


def recording(folder: Path, runs: int, stake: int, net: str | None = "models/c.bin", fingerprint: str = "aa",
              rows_per_run: int = 3, per_chunk: int = 2, offsets: str = "manifest", seed: int = 0) -> np.ndarray:
    """A recording of `runs` runs in chunks of `per_chunk`, each run `rows_per_run` positions; its offsets (in
    the manifest's way, `baseline --offsets`, or offsets.json's, `--offsets-into`), returned in row order."""
    rng = np.random.default_rng(seed)
    folder.mkdir(parents=True)
    all_off = []
    chunks = 0
    for c, first in enumerate(range(0, runs, per_chunk)):
        ids = list(range(first, min(first + per_chunk, runs)))
        n = len(ids) * rows_per_run
        idx, val, ptr = [], [], [0]
        lab = np.zeros((n, len(LABELS)), np.float32)
        for r in range(n):
            on = sorted(rng.choice(INPUTS, 3, replace=False).tolist())
            idx += on
            val += [1.0] * 3
            ptr.append(len(idx))
            run = ids[r // rows_per_run]
            lab[r] = [run, r % rows_per_run, 0, 1, r % rows_per_run, run % 2, 5, 2]
        off = rng.normal(size=n).astype(np.float32)
        all_off.append(off)
        stem = folder / f"chunk-{c:05d}"
        np.save(f"{stem}-idx.npy", np.array(idx, "<u2"))
        np.save(f"{stem}-val.npy", np.array(val, "<f2"))
        np.save(f"{stem}-ptr.npy", np.array(ptr, "<i8"))
        np.save(f"{stem}-lab.npy", lab)
        np.save(f"{stem}-off.npy", off)
        chunks += 1
    manifest = {"format": 1, "input": "value", "input_version": 3, "inputs": INPUTS, "layouts": [[3, INPUTS]],
                "runs": runs, "rows": runs * rows_per_run, "chunks": chunks, "runs_per_chunk": per_chunk,
                "labels": LABELS, "moves_kept": True, "stake": stake}
    if offsets == "manifest":
        manifest.update({"offset_net": net, "offset_net_hash": fingerprint})
    else:
        manifest.update({"offset_net": None, "offset_net_hash": None})
        (folder / "offsets.json").write_text(json.dumps({"offset_net": net, "offset_net_hash": fingerprint}))
    (folder / "manifest.json").write_text(json.dumps(manifest))
    return np.concatenate(all_off)


def test_offsets_are_read_beside_their_positions_from_every_recording(tmp_path):
    a = recording(tmp_path / "a", 5, stake=3, seed=1)
    b = recording(tmp_path / "b", 4, stake=1, offsets="offsets.json", seed=2)
    data = tv.load_data([tmp_path / "a", tmp_path / "b"], offsets=True)
    assert data.rows == 27 and data.off.dtype == np.float32
    assert np.array_equal(data.off, np.concatenate([a, b]))
    assert data.offset_net == "models/c.bin"
    assert data.stakes.tolist() == [3] * 15 + [1] * 12
    # Without --offset, none are read.
    assert tv.load_data([tmp_path / "a"]).off is None
    # The first runs only: their offsets only.
    head = tv.load_data([tmp_path / "a"], max_runs=2, offsets=True)
    assert head.rows == 6 and np.array_equal(head.off, a[:6])


def test_a_recording_without_offsets_or_from_other_networks_is_refused(tmp_path):
    recording(tmp_path / "a", 4, stake=2)
    recording(tmp_path / "other", 4, stake=2, net="models/d.bin", fingerprint="bb")
    with pytest.raises(SystemExit, match="different networks"):
        tv.load_data([tmp_path / "a", tmp_path / "other"], offsets=True)
    recording(tmp_path / "gap", 4, stake=2)
    (tmp_path / "gap" / "chunk-00001-off.npy").unlink()
    with pytest.raises(SystemExit, match="no offsets for chunk-00001"):
        tv.load_data([tmp_path / "gap"], offsets=True)
    recording(tmp_path / "none", 2, stake=2, net=None)
    with pytest.raises(SystemExit, match="no offsets"):
        tv.load_data([tmp_path / "none"], offsets=True)
    recording(tmp_path / "short", 2, stake=2)
    np.save(tmp_path / "short" / "chunk-00000-off.npy", np.zeros(2, np.float32))
    with pytest.raises(SystemExit, match="2 offsets for 6 positions"):
        tv.load_data([tmp_path / "short"], offsets=True)


def test_a_correction_moves_the_log_odds_and_the_report_sets_it_beside_the_networks_alone():
    out = np.array([[0.0], [1.0], [-2.0]], np.float32)
    off = np.array([0.5, -1.0, 0.0], np.float32)
    assert np.allclose(tv.win_chance(out, off), tv.sigmoid(np.array([0.5, 0.0, -2.0])))
    assert np.allclose(tv.win_chance(out, None), tv.sigmoid(np.array([0.0, 1.0, -2.0])))
    lab = np.zeros((4, len(LABELS)), np.float32)
    lab[:, tv.RUN] = [0, 0, 1, 2]
    lab[:, tv.WON] = [1, 1, 0, 0]
    rows = tv.by_stake(np.array([0.9, 0.8, 0.2, 0.4]), lab, np.array([1, 1, 3, 3]), before=np.full(4, 0.5))
    assert [r["stake"] for r in rows] == [1, 3] and [r["runs"] for r in rows] == [1, 2]
    assert rows[0]["bce"] < rows[0]["bce_before"] and rows[1]["predicted_before"] == 0.5
