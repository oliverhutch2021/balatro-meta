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


def test_a_recording_plays_again_move_by_move_to_the_same_files(recording, tmp_path):
    # Every move is kept, so the runs play again without the bot and their positions are encoded afresh: with the
    # same code, to the very same files. A new value input then needs no new recording.
    man = json.loads((recording / "manifest.json").read_text())
    assert man["moves_kept"] is True
    runs = [r for c in sorted(recording.glob("chunk-*-runs.json")) for r in json.loads(c.read_text())]
    kinds = np.concatenate([np.load(c) for c in sorted(recording.glob("chunk-*-mkind.npy"))])
    assert len(kinds) == sum(r["moves"] for r in runs) and len(np.concatenate(
        [np.load(c) for c in sorted(recording.glob("chunk-*-marg.npy"))])) == len(kinds)
    again = tmp_path / "again"
    baseline("--replay", str(recording), "--threads", "2", "--record", str(again))
    for f in sorted(recording.glob("chunk-*")):
        assert (again / f.name).read_bytes() == f.read_bytes(), f.name
    new = json.loads((again / "manifest.json").read_text())
    assert new["replayed_from"] == recording.name and new["bot"] == man["bot"] and new["seed_base"] == man["seed_base"]
    assert new["input_version"] == VALUE_INPUT_VERSION and new["rows"] == man["rows"]
    assert tv.load_data(again).rows == man["rows"]


def test_a_run_that_plays_again_differently_leaves_no_recording(recording, tmp_path):
    if not shutil.which("cargo"):
        pytest.skip("cargo is not installed")
    changed = tmp_path / "changed"
    shutil.copytree(recording, changed)
    kind = min(changed.glob("chunk-*-mkind.npy"))
    kinds = np.load(kind)
    assert kinds[0] == 0, "a run starts at its first blind choice"
    kinds[0] = 2  # the first run's first move becomes a cash-out, which no blind choice allows
    np.save(kind, kinds)
    out = tmp_path / "out"
    r = subprocess.run(["cargo", "run", "--release", "-q", "-p", "bm-core", "--example", "baseline", "--",
                        "--replay", str(changed), "--threads", "2", "--record", str(out)],
                       cwd=ROOT, capture_output=True, text=True, check=False, timeout=900,
                       env={k: v for k, v in os.environ.items() if k != "BM_WATCH_DIR"})
    assert r.returncode == 1 and "replayed differently" in r.stderr, r.stderr
    assert not (out / "manifest.json").exists(), "the trainer refuses a recording without a manifest"


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


def test_a_network_of_an_older_layout_reads_the_start_of_this_engines_input(recording, tmp_path):
    # Rounds 1 to 10's networks read layout 1: they load, and read layout 1's floats of today's longer input.
    n1 = tv.layouts(json.loads((recording / "manifest.json").read_text()))[1]
    assert n1 < VALUE_LEN
    rng = np.random.default_rng(5)
    layers, width = [], n1
    for k, o in enumerate([6, 3]):
        layers.append((rng.normal(0, 1 / np.sqrt(width), (o, width)).astype(np.float32),
                       rng.normal(0, 0.1, o).astype(np.float32), k == 0))
        width = o
    blob = tv.export_bmvn(tmp_path / "v1.bin", layers, 1)
    for seed in ["OLDLAYT1", "A1B2C3D4"]:
        x = np.array(Game(seed).value_input(), np.float32)
        assert len(x) == VALUE_LEN
        want = tv.forward_np(layers, x[None, :n1])[0]
        np.testing.assert_allclose(value_net_forward(blob, x.tolist()), want, rtol=1e-4, atol=1e-5)


def test_a_recording_of_a_newer_layout_trains_as_the_older_one_too(recording, tmp_path):
    man = json.loads((recording / "manifest.json").read_text())
    held = tv.layouts(man)
    assert held[VALUE_INPUT_VERSION] == VALUE_LEN and 1 in held and held[1] < VALUE_LEN
    n1 = held[1]
    new, old_view = tv.load_data(recording), tv.load_data(recording, layout=1)
    assert (new.inputs, new.manifest["input_version"]) == (VALUE_LEN, VALUE_INPUT_VERSION)
    assert (old_view.inputs, old_view.manifest["input_version"]) == (n1, 1) and old_view.rows == new.rows
    rows = np.arange(0, new.rows, 97)
    np.testing.assert_array_equal(old_view.dense(rows), new.dense(rows)[:, :n1])
    # A recording made before the new layout (its manifest names no layouts, its rows layout 1's floats): trained
    # with a newer one, both are read as layout 1, the layout they share; layout 2 can't be asked of them.
    older = tmp_path / "older"
    shutil.copytree(recording, older)
    for c in sorted(older.glob("chunk-*-ptr.npy")):
        stem = str(c)[: -len("-ptr.npy")]
        ci, cv = np.load(f"{stem}-idx.npy"), np.load(f"{stem}-val.npy")
        keep = ci.view(np.int16) < n1
        np.save(f"{stem}-ptr.npy", np.concatenate([[0], np.cumsum(keep)])[np.load(c)].astype(np.int64))
        np.save(f"{stem}-idx.npy", ci[keep])
        np.save(f"{stem}-val.npy", cv[keep])
    om = dict(man, inputs=n1, input_version=1)
    om.pop("layouts")
    (older / "manifest.json").write_text(json.dumps(om))
    both = tv.load_data([older, recording])
    assert (both.inputs, both.manifest["input_version"], both.rows) == (n1, 1, 2 * new.rows)
    np.testing.assert_array_equal(both.dense(rows), old_view.dense(rows))
    with pytest.raises(SystemExit, match="layout 2"):
        tv.load_data([older, recording], layout=2)


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


def test_the_auc_counts_a_tie_as_half_whatever_order_the_rows_come_in():
    # Two positions rated alike, one won and one lost: no ranking either way, in either row order.
    assert tv.auc(np.array([0.5, 0.5]), np.array([0.0, 1.0])) == 0.5
    assert tv.auc(np.array([0.5, 0.5]), np.array([1.0, 0.0])) == 0.5
    # Against the definition (every won-lost pair: 1 if the won one is rated higher, 1/2 if alike), many ties.
    rng = np.random.default_rng(0)
    for _ in range(200):
        n = int(rng.integers(2, 30))
        p, y = rng.integers(0, 4, n) / 3, rng.integers(0, 2, n).astype(float)
        if y.min() == y.max():
            assert tv.auc(p, y) is None
            continue
        won, lost = p[y == 1], p[y == 0]
        want = ((won[:, None] > lost[None]).sum() + 0.5 * (won[:, None] == lost[None]).sum()) / (len(won) * len(lost))
        assert abs(tv.auc(p, y) - want) < 1e-12


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


def test_older_recordings_are_trained_on_and_the_newest_alone_measured(recording, tmp_path):
    older = tmp_path / "older"
    baseline("--runs", "120", "--threads", "2", "--seed", "6", "--record", str(older))
    data = tv.load_data([older, recording])
    one = tv.load_data(recording)
    old_alone = tv.load_data(older)
    assert data.rows == one.rows + old_alone.rows
    assert (data.source[:old_alone.rows] == 0).all() and (data.source[old_alone.rows:] == 1).all()
    # The newest keeps its run numbers (and so its split); the older one's are moved clear of them.
    assert (data.lab[data.source == 1] == one.lab).all()
    assert (data.lab[data.source == 0, tv.RUN] == old_alone.lab[:, tv.RUN] + tv.RUN_OFFSET).all()
    row = old_alone.rows + 7
    assert (data.dense(np.array([row])) == one.dense(np.array([7]))).all()
    assert (data.dense(np.array([3])) == old_alone.dense(np.array([3]))).all()
    short = tv.load_data([older, recording], max_runs=50)
    assert len(np.unique(short.lab[:, tv.RUN])) == 100 and short.source[-1] == 1
    assert (short.dense(np.arange(short.rows)[-3:]) == one.dense(np.arange(3) + int((one.lab[:, tv.RUN] < 50).sum()) - 3)).all()

    pytest.importorskip("torch")
    out, report = tmp_path / "value.bin", tmp_path / "value.json"
    r = subprocess.run([sys.executable, str(ROOT / "scripts" / "train_value.py"), "--data", str(older),
                        "--data", str(recording), "--out", str(out), "--report", str(report), "--epochs", "1",
                        "--configs", "16/0.1/1e-4", "--scale-rows", "2000", "--cpu"],
                       cwd=ROOT, capture_output=True, text=True, check=False, timeout=900)
    assert r.returncode == 0, r.stdout + r.stderr
    rep = json.loads(report.read_text())
    assert rep["recording"]["runs"] == 300 and rep["data"] == str(recording)
    assert [o["runs"] for o in rep["older_recordings"]] == [120]
    assert rep["runs"]["train"] == 120 + (300 - rep["runs"]["judge"] - rep["runs"]["measure"])
    assert rep["measured"]["runs"] == rep["runs"]["measure"]


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


def test_a_correction_learns_on_the_value_networks_log_odds_and_the_engine_adds_it(recording, tmp_path):
    # Offsets two ways: kept while recording, and added to a recording made without them: the same.
    c = tmp_path / "c.bin"
    tv.export_bmvn(c, random_net(np.random.default_rng(8), [8, 1 + tv.STAGES]), VALUE_INPUT_VERSION)
    kept = tmp_path / "kept"
    baseline("--runs", "300", "--threads", "2", "--seed", "5", "--record", str(kept), "--value-net", str(c),
             "--offsets")
    added = tmp_path / "added"
    shutil.copytree(recording, added)
    baseline("--offsets-into", str(added), "--value-net", str(c), "--threads", "2")
    for chunk in sorted(kept.glob("chunk-*-off.npy")):
        assert (np.load(chunk) == np.load(added / chunk.name)).all()
    assert json.loads((added / "offsets.json").read_text())["rows"] == json.loads(
        (recording / "manifest.json").read_text())["rows"]
    data = tv.load_data(added, offsets=True)
    x = data.dense(np.arange(20))
    _v, _i, layers = tv.read_bmvn(c.read_bytes())
    assert np.abs(data.off[:20] - tv.forward_np(layers, x)[:, 0]).max() < 2e-3, "the network's log-odds"

    pytest.importorskip("torch")
    out, report = tmp_path / "residual.bin", tmp_path / "residual.json"
    r = subprocess.run([sys.executable, str(ROOT / "scripts" / "train_value.py"), "--data", str(added),
                        "--offset", "--out", str(out), "--report", str(report), "--epochs", "2",
                        "--configs", "16/0.1/1e-4", "--scale-rows", "2000", "--cpu"],
                       cwd=ROOT, capture_output=True, text=True, check=False, timeout=900)
    assert r.returncode == 0, r.stdout + r.stderr
    rep = json.loads(report.read_text())
    assert rep["residual"]["offset_net"].endswith("c.bin")
    # No correction is the one to beat: the kept one judges no worse.
    assert rep["tried"][0]["history"][0]["step"] == 0
    assert rep["tried"][0]["judging_bce"] <= rep["residual"]["judging_bce_before"] + 1e-9
    assert rep["measured"]["before"]["rows"] == rep["measured"]["rows"]
    assert [s["stake"] for s in rep["measured"]["by_stake"]] == [1]
    # The engine adds it to the value network's chance.
    played = tmp_path / "played.json"
    baseline("--bot", "teacher", "--value-net", str(c), "--stake-net", str(out), "--runs", "2", "--threads", "2",
             "--stake", "3", "--seed", "3", "--out", str(played))
    final = json.loads(played.read_text())
    assert final["stake_net"] == str(out) and final["stake_mode"] == "residual" and final["refused_moves"] == 0
