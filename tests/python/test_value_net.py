"""The learned value's pipeline: positions recorded by the baseline job (`--record`), read back by the trainer
(scripts/train_value.py) exactly as the engine encodes them, a network written in the engine's format and
read the same by both, and the teacher playing with one (`--value-net`).
"""
from __future__ import annotations

import json
import os
import shutil
import subprocess
import sys
from pathlib import Path

import numpy as np
import pytest

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))

import train_value as tv
from balatro_meta._engine import VALUE_INPUT_VERSION, VALUE_LEN, Game, value_net_forward


def baseline(*args: str, timeout: int = 900) -> subprocess.CompletedProcess:
    if not shutil.which("cargo"):
        pytest.skip("cargo is not installed")
    r = subprocess.run(["cargo", "run", "--release", "-q", "-p", "bm-core", "--example", "baseline", "--", *args],
                       cwd=ROOT, capture_output=True, text=True, check=False, timeout=timeout,
                       env={k: v for k, v in os.environ.items() if k != "BM_WATCH_DIR"})
    assert r.returncode == 0, r.stdout + r.stderr
    return r


@pytest.fixture(scope="module")
def recording(tmp_path_factory):
    d = tmp_path_factory.mktemp("record") / "greedy"
    baseline("--runs", "300", "--threads", "2", "--seed", "5", "--record", str(d))
    return d


def random_net(rng, sizes: list[int]) -> list[tuple[np.ndarray, np.ndarray, bool]]:
    layers, width = [], VALUE_LEN
    for k, o in enumerate(sizes):
        w = rng.normal(0, 1 / np.sqrt(width), (o, width)).astype(np.float32)
        b = rng.normal(0, 0.1, o).astype(np.float32)
        layers.append((w, b, k < len(sizes) - 1))
        width = o
    return layers


def test_the_recording_reads_back_as_the_engine_sees_it(recording):
    man = json.loads((recording / "manifest.json").read_text())
    assert man["runs"] == 300 and man["chunks"] == 2 and man["inputs"] == VALUE_LEN
    assert man["input_version"] == VALUE_INPUT_VERSION and man["bot"] == "greedy bot"
    data = tv.load_data(recording)
    assert data.rows == man["rows"] > 300 and data.inputs == VALUE_LEN
    lab = data.lab
    assert (np.diff(lab[:, tv.RUN]) >= 0).all(), "rows in run order"
    runs = [r for c in sorted(recording.glob("chunk-*-runs.json")) for r in json.loads(c.read_text())]
    assert [r["run"] for r in runs] == list(range(300))
    counts = np.bincount(lab[:, tv.RUN].astype(int), minlength=300)
    assert all(counts[r["run"]] == r["rows"] for r in runs)
    # Labels agree with the runs' ends; positions are blind choices, shops and packs.
    assert ((lab[:, tv.WON] == 1) == (lab[:, tv.END_STAGE] == 24)).all()
    assert set(np.unique(lab[:, tv.PHASE])) <= {0.0, 3.0, 5.0}
    assert (lab[:, tv.STAGE] <= lab[:, tv.END_STAGE]).all()
    # A run's first position is its first blind choice: the engine's own input for it, in half precision.
    starts = np.searchsorted(lab[:, tv.RUN], [r["run"] for r in runs[:20]])
    x = data.dense(starts)
    for k, r in enumerate(runs[:20]):
        want = np.array(Game(r["seed"]).value_input(), np.float32).astype(np.float16).astype(np.float32)
        assert lab[starts[k], tv.STEP] == 0
        np.testing.assert_array_equal(x[k], want, err_msg=r["seed"])
    # Runs are split whole: none is both trained on and kept apart.
    part = tv.split(lab[:, tv.RUN])
    assert set(np.unique(part)) == {0, 1, 2} and 0.02 < (part == 1).mean() < 0.1
    for k in (1, 2):
        assert not set(lab[part == k, tv.RUN]) & set(lab[part != k, tv.RUN])


def test_the_same_seeds_record_the_same_files(recording, tmp_path):
    again = tmp_path / "again"
    baseline("--runs", "300", "--threads", "2", "--seed", "5", "--record", str(again))
    for f in sorted(recording.glob("chunk-*")):
        assert (again / f.name).read_bytes() == f.read_bytes(), f.name


def test_a_network_written_for_the_engine_reads_the_same(tmp_path):
    rng = np.random.default_rng(3)
    layers = random_net(rng, [8, 4, 3])
    blob = tv.export_bmvn(tmp_path / "net.bin", layers, VALUE_INPUT_VERSION)
    assert (tmp_path / "net.bin").read_bytes() == blob
    version, inputs, back = tv.read_bmvn(blob)
    assert (version, inputs) == (VALUE_INPUT_VERSION, VALUE_LEN)
    for (w, b, relu), (w2, b2, relu2) in zip(layers, back):
        np.testing.assert_array_equal(w, w2)
        np.testing.assert_array_equal(b, b2)
        assert relu == relu2
    for seed in ["NETCHECK", "A1B2C3D4"]:
        x = np.array(Game(seed).value_input(), np.float32)
        want = tv.forward_np(layers, x[None])[0]
        np.testing.assert_allclose(value_net_forward(blob, x.tolist()), want, rtol=1e-4, atol=1e-5)
    # A network for another input layout is refused.
    with pytest.raises(ValueError, match="retrain"):
        value_net_forward(tv.export_bmvn(tmp_path / "old.bin", layers, VALUE_INPUT_VERSION + 1), [0.0] * VALUE_LEN)


def test_folding_the_input_scale_into_the_first_layer_changes_nothing():
    torch = pytest.importorskip("torch")
    torch.manual_seed(0)
    net = tv.make_net(torch, VALUE_LEN, [16, 8], 0.0).eval()
    mean = torch.rand(VALUE_LEN)
    scale = torch.rand(VALUE_LEN) * 2
    scale[:100] = 0  # inputs left out
    x = (torch.rand(5, VALUE_LEN) < 0.05).float() * torch.rand(5, VALUE_LEN) * 3
    with torch.no_grad():
        want = net((x - mean) * scale).numpy()
    got = tv.forward_np(tv.folded_layers(net, mean, scale), x.numpy())
    np.testing.assert_allclose(got, want, rtol=1e-4, atol=1e-4)


def test_training_writes_a_network_the_engine_reads(recording, tmp_path):
    pytest.importorskip("torch")
    out, report = tmp_path / "value.bin", tmp_path / "value.json"
    r = subprocess.run([sys.executable, str(ROOT / "scripts" / "train_value.py"), "--data", str(recording),
                        "--out", str(out), "--report", str(report), "--epochs", "2", "--configs", "16,8/0.1/1e-4",
                        "--scale-rows", "4000", "--cpu"],
                       cwd=ROOT, capture_output=True, text=True, check=False, timeout=900)
    assert r.returncode == 0, r.stdout + r.stderr
    rep = json.loads(report.read_text())
    assert rep["inputs"] == VALUE_LEN and rep["chosen"]["hidden"] == [16, 8] and len(rep["tried"]) == 1
    assert rep["check"]["engine"] is not None and rep["check"]["engine"] < 1e-3
    assert rep["measured"]["rows"] > 0 and rep["recording"]["runs"] == 300
    assert sum(rep["runs"].values()) == 300
    _version, inputs, layers = tv.read_bmvn(out.read_bytes())
    assert inputs == VALUE_LEN and [w.shape[0] for w, _, _ in layers] == [16, 8, 1 + tv.STAGES]


def test_the_teacher_plays_with_a_network(tmp_path):
    net = tmp_path / "net.bin"
    tv.export_bmvn(net, random_net(np.random.default_rng(5), [8, 1 + tv.STAGES]), VALUE_INPUT_VERSION)
    out = tmp_path / "teacher.json"
    r = baseline("--bot", "teacher", "--value-net", str(net), "--value-weight", "0.5", "--runs", "2",
                 "--threads", "2", "--seed", "3", "--out", str(out))
    assert "Value network:" in r.stdout
    final = json.loads(out.read_text())
    assert final["bot"] == "teacher bot with a learned value" and final["runs"] == 2
    assert final["refused_moves"] == 0
