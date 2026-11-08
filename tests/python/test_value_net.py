"""The learned value's pipeline: positions recorded by the baseline job (`--record`), read back by the trainer
(scripts/train_value.py) exactly as the engine encodes them, a network written in the engine's format and
read the same by both, and the teacher playing with one (`--value-net`).
"""
from __future__ import annotations

import json
import math
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


@pytest.fixture(scope="module")
def endless(tmp_path_factory):
    """Eight light-teacher runs whose wins play on into Endless (seed base 7: one win, four blinds beyond it)."""
    d = tmp_path_factory.mktemp("endless")
    baseline("--bot", "teacher", "--teacher-options", "light", "--runs", "8", "--threads", "4", "--seed", "7",
             "--endless", "--record", str(d / "on"), "--out", str(d / "on.json"))
    return d


def test_a_win_that_plays_on_labels_its_positions_with_the_blinds_beaten_after_it(endless, tmp_path):
    on = endless / "on"
    man = json.loads((on / "manifest.json").read_text())
    assert man["endless"] is True and man["labels"][tv.ENDLESS] == "endless" and len(man["labels"]) == 9
    report = json.loads((endless / "on.json").read_text())
    assert report["endless"] is True and report["wins"] >= 1
    data = tv.load_data(on)
    lab = data.lab
    for r in json.loads(next(on.glob("chunk-*-runs.json")).read_text()):
        rows = lab[lab[:, tv.RUN] == r["run"]]
        depth = set(rows[:, tv.ENDLESS].tolist())
        if rows[0, tv.WON] == 1:
            assert len(depth) == 1 and 0 <= depth.pop() <= tv.DEPTH - 1
            assert (rows[:, tv.END_STAGE] == 24).all(), "a win stays a win: ante 8's boss is where it ended"
        else:
            assert depth == {-1.0}
    hist = report["endless_hist"]
    won = lab[lab[:, tv.WON] == 1]
    assert sum(hist) == report["wins"] == len(np.unique(won[:, tv.RUN]))
    assert report["endless_blinds_per_win"] == pytest.approx(
        sum(k * n for k, n in enumerate(hist)) / sum(hist), abs=1e-6)
    # Playing on changes nothing before the win: the same seeds without it record the same positions.
    baseline("--bot", "teacher", "--teacher-options", "light", "--runs", "8", "--threads", "4", "--seed", "7",
             "--record", str(tmp_path / "off"))
    off = tv.load_data(tmp_path / "off")
    assert off.rows == data.rows and not off.depth and (off.lab[:, tv.ENDLESS] == -1).all()
    np.testing.assert_array_equal(off.lab[:, :tv.ENDLESS], lab[:, :tv.ENDLESS])
    rows = np.arange(0, data.rows, 7)
    np.testing.assert_array_equal(off.dense(rows), data.dense(rows))
    # A recording from before the column (round 10's: eight labels) mixes with it and reads "not known" there.
    older = tmp_path / "older"
    shutil.copytree(tmp_path / "off", older)
    for c in older.glob("chunk-*-lab.npy"):
        np.save(c, np.load(c)[:, :tv.BASE_LABELS])
    om = json.loads((older / "manifest.json").read_text())
    om["labels"] = om["labels"][:tv.BASE_LABELS]
    (older / "manifest.json").write_text(json.dumps(om))
    both = tv.load_data([older, on])
    assert both.depth and both.lab.shape[1] == 9 and (both.lab[both.source == 0, tv.ENDLESS] == -1).all()
    np.testing.assert_array_equal(both.lab[both.source == 1], lab)
    # A replay of its moves plays the wins on again and labels them the same.
    baseline("--replay", str(on), "--record", str(tmp_path / "again"), "--threads", "2")
    np.testing.assert_array_equal(tv.load_data(tmp_path / "again").lab, lab)


def test_the_depth_outputs_learn_only_from_wins_that_played_on():
    torch = pytest.importorskip("torch")
    n = 6
    out = torch.zeros(n, 1 + tv.STAGES + tv.DEPTH)
    lab = torch.full((n, 9), -1.0)
    lab[:, tv.WON] = torch.tensor([1.0, 1.0, 0.0, 1.0, 0.0, 1.0])
    lab[:, tv.END_STAGE] = torch.tensor([24.0, 24.0, 10.0, 24.0, 3.0, 24.0])
    lab[:, tv.ENDLESS] = torch.tensor([4.0, 0.0, -1.0, -1.0, -1.0, 12.0])  # row 3: a win from a recording that stopped
    _win, total = tv.losses(torch, out, lab, 1.0)
    out2 = out.clone()
    out2[:, 1 + tv.STAGES + 4] = 5.0  # sure of depth 4 everywhere
    _win, total2 = tv.losses(torch, out2, lab, 1.0)
    changed = (total2 - total).abs() > 1e-6
    assert changed.tolist() == [True, True, False, False, False, True]
    assert total2[0] < total[0] and total2[1] > total[1]
    # A network without the depth outputs, or labels without the column, trains as before.
    _w, plain = tv.losses(torch, out[:, :1 + tv.STAGES], lab, 1.0)
    _w, short = tv.losses(torch, out, lab[:, :tv.BASE_LABELS], 1.0)
    assert torch.allclose(plain, short) and torch.allclose(plain[[2, 3, 4]], total[[2, 3, 4]])


def test_the_depth_measures_set_the_outputs_against_the_depths_found():
    rng = np.random.default_rng(3)
    n = 400
    lab = np.full((n, 9), -1.0, np.float32)
    lab[:, tv.RUN] = np.arange(n) // 4
    lab[:, tv.WON] = (rng.random(n) < 0.6).astype(np.float32)
    d = rng.integers(0, 7, n)
    lab[:, tv.ENDLESS] = np.where(lab[:, tv.WON] == 1, d, -1)
    sure = np.zeros((n, 1 + tv.STAGES + tv.DEPTH), np.float32)
    sure[np.arange(n), 1 + tv.STAGES + d] = 20.0
    m = tv.depth_measures(sure, lab)
    wins = lab[:, tv.WON] == 1
    assert m["wins"] == int(wins.sum()) and m["ce"] < 1e-3 < m["ce_by_frequency_alone"]
    assert m["mean_expected"] == pytest.approx(d[wins].mean(), abs=1e-3) == m["mean_found"]
    assert m["three_or_more_found"] == pytest.approx((d[wins] >= 3).mean(), abs=1e-4)
    flat = np.zeros_like(sure)
    assert tv.depth_measures(flat, lab)["ce"] == pytest.approx(np.log(tv.DEPTH), abs=1e-4)
    assert tv.depth_measures(sure, lab[:, :tv.BASE_LABELS]) == {"wins": 0}


def test_training_on_a_recording_that_played_on_writes_the_depth_outputs(recording, endless, tmp_path):
    pytest.importorskip("torch")
    out, report = tmp_path / "value.bin", tmp_path / "value.json"
    r = subprocess.run([sys.executable, str(ROOT / "scripts" / "train_value.py"), "--data", str(recording),
                        "--data", str(endless / "on"), "--out", str(out), "--report", str(report), "--epochs", "1",
                        "--configs", "16/0.1/1e-4", "--scale-rows", "2000", "--cpu"],
                       cwd=ROOT, capture_output=True, text=True, check=False, timeout=900)
    assert r.returncode == 0, r.stdout + r.stderr
    rep = json.loads(report.read_text())
    assert rep["check"]["engine"] is not None and rep["check"]["engine"] < 1e-3
    assert "depth" in rep["measured"]
    _version, _inputs, layers = tv.read_bmvn(out.read_bytes())
    assert layers[-1][0].shape[0] == 1 + tv.STAGES + tv.DEPTH
    x = tv.load_data(recording).dense(np.arange(3))
    got = np.array([value_net_forward(out.read_bytes(), row.tolist()) for row in x], np.float32)
    np.testing.assert_allclose(got, tv.forward_np(layers, x), rtol=1e-4, atol=1e-4)


@pytest.fixture(scope="module")
def forked(tmp_path_factory, endless):
    """A light-teacher recording's first runs played again with forks and a diagnosis."""
    d = tmp_path_factory.mktemp("forked")
    baseline("--bot", "teacher", "--teacher-options", "light", "--replay", str(endless / "on"), "--runs", "4",
             "--forks", str(d / "forks"), "--fork-chance", "0.25", "--fork-copies", "2", "--fork-best", "2",
             "--diagnose", str(d / "diag.csv"), "--deep", "2", "--deep-share", "1", "--threads", "4",
             "--out", str(d / "forks.json"))
    return d


def test_a_fork_plays_the_teachers_options_out_on_shared_copies_and_keeps_what_each_left(forked):
    man = json.loads((forked / "forks" / "manifest.json").read_text())
    assert man["kind"] == "forks" and man["labels"] == tv.FORK_LABELS and man["fork_copies"] == 2
    report = json.loads((forked / "forks.json").read_text())
    f = report["forks"]
    assert f["forked"] >= 1 and f["copies"] == 2 and 0 <= f["chosen_best"] <= 1 and f["mean_regret"] >= 0
    data = tv.load_data(forked / "forks")
    lab = data.lab
    assert lab.shape == (f["playouts"], len(tv.FORK_LABELS)) and data.inputs == VALUE_LEN
    assert set(np.unique(lab[:, tv.F_PHASE])) <= {3.0, 5.0}, "forks are at shops and open packs"
    assert ((lab[:, tv.F_ANTE] >= 2) & (lab[:, tv.F_ANTE] <= 6)).all()
    assert ((lab[:, tv.F_WON] == 1) == (lab[:, tv.F_END_STAGE] == 24)).all()
    assert np.isnan(lab[:, tv.F_LOGIT]).all(), "no networks loaded: no log-odds"
    fs = tv.ForkSet(data)
    assert len(fs) == f["forked"]
    for fork in fs.forks:
        opts = fork["options"]
        assert 2 <= len(opts) <= 5 and sum(o["chosen"] for o in opts) == 1 and sum(o["made"] for o in opts) == 1
        assert all(len(o["rows"]) == 2 for o in opts) and len(fork["pairs"]) == len(opts) * (len(opts) - 1) // 2
        assert all(0 <= t <= 1 for _, _, t in fork["pairs"])
        # Every option's copies left different positions from the others' (another move was made).
        x = data.dense(np.concatenate([o["rows"] for o in opts]))
        assert len({tuple(np.flatnonzero(r)) + tuple(r[np.flatnonzero(r)]) for r in x}) >= 2
    # The report's judge test agrees with the labels.
    chosen_best = np.mean([int(next(k for k, o in enumerate(fk["options"]) if o["chosen"]) == np.lexsort(
        (-np.arange(len(fk["options"])), [o["key"].sum() for o in fk["options"]],
         [o["won"].mean() for o in fk["options"]]))[-1]) for fk in fs.forks])
    assert abs(chosen_best - f["chosen_best"]) < 1e-6
    rows, opt_of, n, pa, pb, pt = fs.batch(np.arange(len(fs)))
    assert len(rows) == len(opt_of) == lab.shape[0] and n == sum(len(fk["options"]) for fk in fs.forks)
    assert len(pa) == len(pb) == len(pt) == sum(len(fk["pairs"]) for fk in fs.forks)


def test_the_pairwise_loss_pulls_the_better_options_value_up():
    torch = pytest.importorskip("torch")
    opt_of = torch.tensor([0, 0, 1, 1, 2])  # three options, two copies each but the last
    pa, pb = torch.tensor([0, 0]), torch.tensor([1, 2])
    z = torch.zeros(5, requires_grad=True)
    loss = tv.pair_loss(torch, z, opt_of, 3, pa, pb, torch.tensor([1.0, 0.5]))
    assert abs(loss.item() - math.log(2)) < 1e-6, "no difference yet: log 2 a pair"
    loss.backward()
    g = z.grad
    assert g[0] < 0 and g[1] < 0 and g[2] > 0 and g[3] > 0 and abs(float(g[4])) < 1e-7, \
        "option 0 beat 1 on every copy: 0 up, 1 down; it tied with 2, which is left alone"
    assert abs(float(tv.option_values(torch, torch.tensor([1.0, 3.0, 5.0, 7.0, 9.0]), opt_of, 3)[1]) - 6.0) < 1e-6


def test_training_on_forks_reports_the_judge_test(recording, forked, tmp_path):
    pytest.importorskip("torch")
    out, report = tmp_path / "value.bin", tmp_path / "value.json"
    r = subprocess.run([sys.executable, str(ROOT / "scripts" / "train_value.py"), "--data", str(recording),
                        "--forks", str(forked / "forks"), "--out", str(out), "--report", str(report), "--epochs", "1",
                        "--configs", "16/0.1/1e-4", "--scale-rows", "2000", "--cpu", "--fork-batch", "2"],
                       cwd=ROOT, capture_output=True, text=True, check=False, timeout=900)
    assert r.returncode == 0, r.stdout + r.stderr
    rep = json.loads(report.read_text())
    assert rep["forks"]["forked"] >= 1 and rep["forks"]["pair_weight"] == 1.0
    judge = rep["measured"]["forks"]
    if judge["forks"]:
        for name in ("network", "chosen", "random"):
            assert 0 <= judge[name]["picked_best"] <= 1 and judge[name]["regret"] >= -1e-9
        assert "ensemble" not in judge, "no networks made these forks"
    assert rep["check"]["engine"] is not None and rep["check"]["engine"] < 1e-3
    assert any("judging_pairs" in h for h in rep["tried"][0]["history"])


def test_the_diagnosis_rows_say_where_each_run_ended_and_whether_deeper_play_beat_the_blind(forked):
    import csv
    rows = list(csv.DictReader((forked / "diag.csv").open()))
    assert len(rows) == 4 and list(rows[0]) == [c for c in (forked / "diag.csv").read_text().splitlines()[0].split(",")]
    report = json.loads((forked / "forks.json").read_text())
    assert report["deep"]["samples_x"] == 2
    replayed = [r for r in rows if r["deep_won"] != "-1"]
    assert len(replayed) == report["deep"]["lost_blinds_replayed"] == sum(r["won"] == "0" for r in rows)
    assert sum(r["deep_won"] == "1" for r in replayed) == report["deep"]["beaten"]
    for r in rows:
        if r["won"] == "1":
            assert r["lost_to"] == "" and r["closeness"] == "1.000" and r["deep_won"] == "-1"
            assert all(float(r[f"ante_best_{a}"]) > 0 for a in range(1, 9))
        else:
            assert 0 <= float(r["closeness"]) < 1 and float(r["needed"]) > 0 and r["lost_to"]
            assert float(r["deep_needed"]) == float(r["needed"])
            ante = int(r["ante"])
            assert float(r[f"ante_best_{ante}"]) >= float(r["blind_best"]) >= 0
            assert all(float(r[f"ante_best_{a}"]) == 0 for a in range(ante + 1, 9)), "antes never reached"
    import importlib
    sys.path.insert(0, str(ROOT / "scripts"))
    diagnose = importlib.import_module("diagnose")
    s = diagnose.summarise(rows)
    assert s["runs"] == 4 and s["wins"] + s["losses"] == 4
    if s["losses"]:
        assert abs(sum(s["by_blind"].values()) - 1) < 1e-3 and abs(sum(s["shortfall"].values()) - 1) < 1e-3
        assert s["deeper_play"]["replayed"] == len(replayed)
        text = "\n".join(diagnose.lines(s, forked / "diag.csv"))
        assert "Where the losses are" in text and "Could better play" in text


def test_a_run_that_replays_another_way_is_left_out_of_a_diagnosis_and_fails_a_plain_replay(recording, tmp_path):
    """One run of 300 said to have ended elsewhere: a diagnosis leaves it out of every count and names it; a plain
    replay (a check that the engine still plays the recording) fails."""
    import re
    bent = tmp_path / "bent"
    shutil.copytree(recording, bent)
    first = min(bent.glob("chunk-*-runs.json"))
    text = first.read_text()
    line = next(ln for ln in text.splitlines() if ln.lstrip().startswith('{"run": 0,'))
    stage = int(re.search(r'"end_stage": (\d+)', line).group(1))
    other = 0 if stage else 1
    won = "true" if other == 24 else "false"
    new = re.sub(r'"won": \w+', f'"won": {won}', re.sub(r'"end_stage": \d+', f'"end_stage": {other}', line))
    first.write_text(text.replace(line, new))
    report = tmp_path / "diag.json"
    baseline("--bot", "teacher", "--teacher-options", "light", "--replay", str(bent), "--diagnose",
             str(tmp_path / "diag.csv"), "--deep-share", "0",
             "--threads", "2", "--out", str(report))
    r = json.loads(report.read_text())
    assert r["runs"] == 299 and r["left_out"] == 1 and r["left_out_runs"] == [0] and r["refused_moves"] == 0
    assert sum(r["ante_hist"]) == 299, "the run left out is in no count"
    assert r["run_stages"][0] == chr(ord("A") + other) and len(r["run_stages"]) == 300, "the recording's ending"
    rows = (tmp_path / "diag.csv").read_text().splitlines()[1:]
    assert len(rows) == 299
    plain = subprocess.run(["cargo", "run", "--release", "-q", "-p", "bm-core", "--example", "baseline", "--",
                            "--replay", str(bent), "--threads", "2", "--out", str(tmp_path / "plain.json")],
                           cwd=ROOT, capture_output=True, text=True, check=False, timeout=900)
    assert plain.returncode == 1 and "replayed differently" in plain.stderr


def test_a_fork_recording_still_being_made_reads_the_chunks_its_manifest_counts(forked, tmp_path):
    partial = tmp_path / "partial"
    shutil.copytree(forked / "forks", partial)
    man = json.loads((partial / "manifest.json").read_text())
    whole = tv.load_data(partial)
    # As the recorder leaves it after its first chunk: the manifest counts the chunks whole so far; a chunk half
    # written after that is left alone.
    (partial / "manifest.json").write_text(json.dumps(dict(man, partial=True, chunks=1)))
    stem = min(partial.glob("chunk-*-ptr.npy")).name[: -len("-ptr.npy")]
    for part in ("idx", "val", "ptr"):
        shutil.copy(partial / f"{stem}-{part}.npy", partial / f"chunk-00009-{part}.npy")
    data = tv.load_data(partial)
    assert data.rows == whole.rows == man["rows"]
    fs = tv.ForkSet(data)
    j = tv.judge_forks(fs, np.arange(len(fs)))
    assert j["forks"] == len(fs) and set(j) >= {"chosen", "random", "spread", "options_mean"}
    assert j["random"]["gain"] == 0.0 and abs(j["random"]["picked_best"] - np.mean(
        [1 / len(f["options"]) for f in fs.forks])) < 1e-4
    if "oracle_half_gain" in j:
        assert -1 <= j["oracle_half_gain"] <= 1
        t = j["true_spread"]
        assert 0 <= t["sd_between_options"] <= t["sd_measured"] + 1e-9 and t["perfect_judge_gain"] >= 0
        # Options that differ by nothing: all noise. Copies that agree: none.
        flat = {"options": [{"won": np.array([1.0, 0.0, 1.0, 0.0]), "key": np.array([24.0, 3.0, 24.0, 3.0]),
                             "chosen": k == 0, "made": False, "quick": float("nan"), "logit": float("nan")}
                            for k in range(3)], "pairs": [], "run": 0, "ante": 2}
        sure = {"options": [{"won": np.full(4, float(k == 2)), "key": np.full(4, 24.0 if k == 2 else 3.0),
                             "chosen": k == 0, "made": False, "quick": float("nan"), "logit": float("nan")}
                            for k in range(3)], "pairs": [], "run": 1, "ante": 2}
        toy = tv.ForkSet.__new__(tv.ForkSet)
        toy.forks = [flat, flat]
        assert tv.judge_forks(toy, np.arange(2))["true_spread"]["sd_between_options"] == 0.0
        toy.forks = [sure, sure]
        ts = tv.judge_forks(toy, np.arange(2))["true_spread"]
        assert ts["sd_between_options"] == ts["sd_measured"] == pytest.approx(np.sqrt(1 / 3), abs=1e-4)
        assert ts["noise_share"] == 0.0
    # A network judge given as its picks: picking the best by the labels is a perfect judge.
    best = np.array([int(np.lexsort((-np.arange(len(f["options"])), [o["key"].sum() for o in f["options"]],
                                     [o["won"].mean() for o in f["options"]]))[-1]) for f in fs.forks])
    perfect = tv.judge_forks(fs, np.arange(len(fs)), {"network": best})["network"]
    assert perfect["picked_best"] == 1.0 and perfect["regret"] == 0.0 and perfect["gain"] >= 0
    assert tv.judge_lines(j)


def test_training_as_an_older_layout_still_checks_against_the_engine(recording, tmp_path):
    pytest.importorskip("torch")
    out, report = tmp_path / "v1.bin", tmp_path / "v1.json"
    r = subprocess.run([sys.executable, str(ROOT / "scripts" / "train_value.py"), "--data", str(recording),
                        "--input-layout", "1", "--out", str(out), "--report", str(report), "--epochs", "1",
                        "--configs", "16/0.1/1e-4", "--scale-rows", "2000", "--cpu"],
                       cwd=ROOT, capture_output=True, text=True, check=False, timeout=900)
    assert r.returncode == 0, r.stdout + r.stderr
    rep = json.loads(report.read_text())
    assert rep["inputs"] < VALUE_LEN and rep["input_version"] == 1
    assert rep["check"]["engine"] is not None and rep["check"]["engine"] < 1e-3
