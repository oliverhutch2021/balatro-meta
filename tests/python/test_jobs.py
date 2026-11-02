"""The job runner (scripts/jobs.py): a step that measures a bot leaves a line in results/progress.jsonl."""
from __future__ import annotations

import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))

import jobs


def test_a_measuring_step_is_logged_with_its_code_and_options(tmp_path):
    out = tmp_path / "results" / "value" / "teacher-w1.json"
    out.parent.mkdir(parents=True)
    out.write_text(json.dumps({"kind": "baseline", "bot": "teacher bot with a learned value", "done": True,
                               "runs": 5000, "wins": 1400, "win_rate": 0.28, "win_ci": [0.27, 0.29], "mean_ante": 6.6}))
    argv = [*jobs.BASELINE, "--bot", "teacher", "--value-net", "models/value-1.bin", "--value-weight", "2",
            "--runs", "5000", "--seed", "1", "--out", "results/value/teacher-w1.json"]
    e = jobs.progress_entry("teacher-learned", "weight-2", argv, "HEAD", root=tmp_path)
    assert e["runs"] == 5000 and e["wins"] == 1400 and e["win_rate"] == 0.28 and e["win_ci"] == [0.27, 0.29]
    assert e["value_net"] == "models/value-1.bin" and e["value_weight"] == 2.0 and e["seed"] == 1
    assert e["job"] == "teacher-learned" and e["step"] == "weight-2" and e["out"] == "results/value/teacher-w1.json"
    assert e["name"] == "Learned value \u00d72"
    assert e["code"] and not e["title"].startswith("results("), "the code commit, not a results commit"
    log = tmp_path / "progress.jsonl"
    jobs.record_progress(e, log)
    jobs.record_progress(None, log)  # a step that measured nothing adds nothing
    jobs.record_progress(e, log)
    assert [json.loads(line)["step"] for line in log.read_text().splitlines()] == ["weight-2", "weight-2"]


def test_steps_that_measure_nothing_are_not_logged(tmp_path):
    assert jobs.progress_entry("value-net", "train", ["scripts/train_value.py", "--out", "models/x.bin"], "HEAD",
                               root=tmp_path) is None
    report = tmp_path / "results" / "value" / "value-1.json"
    report.parent.mkdir(parents=True)
    report.write_text(json.dumps({"kind": "value-net"}))
    assert jobs.progress_entry("value-net", "train", ["x.py", "--report", "r", "--out", "results/value/value-1.json"],
                               "HEAD", root=tmp_path) is None
    assert jobs.progress_entry("smoke", "build", ["make", "build"], "HEAD", root=tmp_path) is None
    plain = tmp_path / "results" / "value" / "teacher-plain.json"
    plain.write_text(json.dumps({"kind": "baseline", "bot": "teacher bot", "runs": 10, "wins": 3, "win_rate": 0.3}))
    e = jobs.progress_entry("teacher-learned", "plain", ["x", "--out", "results/value/teacher-plain.json"], "HEAD",
                            root=tmp_path)
    assert e["name"] == "Teacher" and e["value_net"] is None and e["value_weight"] is None and e["seed"] == 1


def test_the_teacher_s_moves_are_recorded_with_its_best_learned_value(tmp_path):
    assert jobs.learned_value_args(tmp_path) == [], "before make teacher-learned: the plain teacher"
    folder = tmp_path / "results" / "value"
    folder.mkdir(parents=True)
    for w, rate in (("0.5", 0.37), ("1", 0.39), ("2", 0.39)):
        (folder / f"teacher-w{w}.json").write_text(json.dumps({"kind": "baseline", "runs": 5000, "win_rate": rate}))
    assert jobs.learned_value_args(tmp_path) == [], "without the network: the plain teacher"
    (tmp_path / "models").mkdir()
    (tmp_path / "models" / "value-1.bin").write_bytes(b"BMVN")
    assert jobs.learned_value_args(tmp_path) == ["--value-net", "models/value-1.bin", "--value-weight", "1"], \
        "the weight that won most; the lower one on a tie"
    (folder / "teacher-w2.json").write_text(json.dumps({"kind": "baseline", "runs": 5000, "win_rate": 0.41}))
    assert jobs.learned_value_args(tmp_path)[-1] == "2"
    record = {n: a for n, a, *_ in jobs.JOBS["teacher-moves"]}["record"]
    assert "--record-moves" in record and record[record.index("--bot") + 1] == "teacher"


def test_the_best_learned_value_wins_across_rounds(tmp_path):
    folder = tmp_path / "results" / "value"
    folder.mkdir(parents=True)
    (tmp_path / "models").mkdir()
    for name, rate in (("teacher-w0.5", 0.37), ("teacher-w1", 0.393), ("teacher-w2", 0.367)):
        (folder / f"{name}.json").write_text(json.dumps({"kind": "baseline", "runs": 5000, "win_rate": rate}))
    (tmp_path / "models" / "value-1.bin").write_bytes(b"BMVN")
    assert jobs.learned_value_args(tmp_path) == ["--value-net", "models/value-1.bin", "--value-weight", "1"]
    (folder / "teacher2-w1.json").write_text(json.dumps({"kind": "baseline", "runs": 5000, "win_rate": 0.42}))
    assert jobs.learned_value_args(tmp_path)[1] == "models/value-1.bin", "round 2 counts once its network is there"
    (tmp_path / "models" / "value-2.bin").write_bytes(b"BMVN")
    assert jobs.learned_value_args(tmp_path) == ["--value-net", "models/value-2.bin", "--value-weight", "1"]
    (folder / "teacher2-w1.json").write_text(json.dumps({"kind": "baseline", "runs": 5000, "win_rate": 0.38}))
    assert jobs.learned_value_args(tmp_path)[1] == "models/value-1.bin", "a worse round 2 doesn't replace round 1"


def test_round_two_and_the_network_s_test_are_named_in_the_progress_log(tmp_path):
    out = tmp_path / "results" / "x.json"
    out.parent.mkdir(parents=True)
    out.write_text(json.dumps({"kind": "baseline", "bot": "teacher bot with a learned value", "runs": 10, "wins": 4,
                               "win_rate": 0.4}))
    e = jobs.progress_entry("teacher-learned-2", "weight-1", ["x", "--value-net", "models/value-2.bin",
                                                             "--value-weight", "1", "--out", "results/x.json"],
                            "HEAD", root=tmp_path)
    assert e["name"] == "Learned value 2 \u00d71"
    out.write_text(json.dumps({"kind": "baseline", "bot": "network", "runs": 10, "wins": 1, "win_rate": 0.1}))
    assert jobs.progress_entry("eval-net", "eval", ["x", "--out", "results/x.json"], "HEAD",
                               root=tmp_path)["name"] == "Network after PPO"
    assert jobs.progress_entry("train", "eval-imitate", ["x", "--out", "results/x.json"], "HEAD",
                               root=tmp_path)["name"] == "Network, copying the teacher"
    for job in ("eval-net", "teacher-data-2", "value-net-2", "teacher-learned-2"):
        for name, argv, *_ in jobs.JOBS[job]:
            if name not in ("build", "setup-gpu"):
                outs = [argv[argv.index(f) + 1] for f in ("--out", "--report") if f in argv]
                assert outs and all(o.startswith(("results/", "models/")) for o in outs), (job, name)


def test_the_teacher_s_options_are_tried_on_the_best_teacher_against_its_own_runs(tmp_path):
    def trial(opt):
        steps = {name: argv for name, argv, *_ in jobs.option_trial(opt, tmp_path)}
        play = steps[opt]
        assert play[play.index("--teacher-options") + 1] == opt and play[play.index("--seed") + 1] == "1"
        out = play[play.index("--out") + 1]
        assert steps["compare"][2] == out and out.startswith("results/value/")
        return play, steps["compare"][1]

    play, control = trial("precise")
    assert "--value-net" not in play and control == "results/value/teacher-plain.json", "no learned value yet"
    (tmp_path / "models").mkdir()
    (tmp_path / "models" / "value-1.bin").write_bytes(b"BMVN")
    (tmp_path / "results" / "value").mkdir(parents=True)
    for w, rate in (("0.5", 0.37), ("1", 0.393), ("2", 0.367)):
        (tmp_path / "results" / "value" / f"teacher-w{w}.json").write_text(json.dumps({"win_rate": rate}))
    play, control = trial("precise")
    assert play[play.index("--value-net") + 1] == "models/value-1.bin" and play[play.index("--value-weight") + 1] == "1"
    assert control == "results/value/teacher-w1.json"
    assert play[play.index("--out") + 1] == "results/value/precise-on-teacher-w1.json"
    (tmp_path / "models" / "value-2.bin").write_bytes(b"BMVN")
    (tmp_path / "results" / "value" / "teacher2-w2.json").write_text(json.dumps({"win_rate": 0.45}))
    play, control = trial("steady")
    assert play[play.index("--value-net") + 1] == "models/value-2.bin" and play[play.index("--value-weight") + 1] == "2"
    assert control == "results/value/teacher2-w2.json", "round 2's teacher, set against its own runs"
    assert "teacher-precise" in jobs.JOBS


def test_round_three_records_with_precise_unless_its_trial_found_it_no_better(tmp_path):
    assert jobs.recording_options(tmp_path) == ["--teacher-options", "precise"], "the cloud tests' finding"
    (tmp_path / "results" / "value").mkdir(parents=True)
    trial = tmp_path / "results" / "value" / "precise-vs-teacher2-w1.json"
    trial.write_text(json.dumps({"kind": "comparison", "win_rate_diff": 0.031}))
    assert jobs.recording_options(tmp_path) == ["--teacher-options", "precise"]
    trial.write_text(json.dumps({"kind": "comparison", "win_rate_diff": -0.004}))
    assert jobs.recording_options(tmp_path) == []
    for n, seed in ((3, "404"), (4, "505"), (9, "1010")):
        steps = {name: argv for name, argv, *_ in jobs.JOBS[f"teacher-data-{n}"]}
        rec = steps["record"]
        assert rec[rec.index("--seed") + 1] == seed and rec[rec.index("--runs") + 1] == "25000"
        assert Path(rec[rec.index("--record") + 1]).name == f"value-{n}"
        trains = dict(jobs.JOBS[f"value-net-{n}"])
        for name, suffix, seed in (("train", "", "1"), ("trainb", "b", "2")):
            train = trains[name]
            datas = [train[i + 1] for i, a in enumerate(train) if a == "--data"]
            assert [Path(d).name for d in datas] == [f"value-{n - 1}", f"value-{n}"], "the round before's, newest last"
            assert train[train.index("--out") + 1] == f"models/value-{n}{suffix}.bin"
            assert train[train.index("--seed") + 1] == seed
        for name, argv, *_ in jobs.JOBS[f"teacher-learned-{n}"]:
            if name != "build":
                assert "--teacher-options" not in argv, "tested on the same footing as the earlier rounds"
                want = f"results/value/ensemble{n}-w" if name.startswith("ensemble") else f"results/value/teacher{n}-w"
                assert argv[argv.index("--out") + 1].startswith(want)
        assert jobs.LEARNED_ROUNDS[n - 1] == (f"models/value-{n}.bin", f"results/value/teacher{n}-w{{w}}.json")
    assert "teacher-data-10" not in jobs.JOBS
    assert len(jobs.LEARNED_ROUNDS) == 2 * jobs.LAST_ROUND - 1, "each round's network, and from round 2 all so far"
    # A round's recording uses the best network so far, whichever round it came from.
    (tmp_path / "models").mkdir()
    for n, rate in ((1, 0.393), (2, 0.43), (3, 0.41)):
        (tmp_path / "models" / f"value-{n}.bin").write_bytes(b"BMVN")
        name = "teacher-w1.json" if n == 1 else f"teacher{n}-w1.json"
        (tmp_path / "results" / "value" / name).write_text(json.dumps({"win_rate": rate}))
    rec = dict(jobs.learned_round(4, tmp_path)["teacher-data-4"])["record"]
    assert rec[rec.index("--value-net") + 1] == "models/value-2.bin"


def test_the_formulas_numbers_are_tuned_on_the_best_teacher_against_its_own_runs(tmp_path):
    steps = jobs.tune_steps(tmp_path)
    names = [name for name, *_ in steps]
    assert names[:2] == ["build", "precise"], "the precise teacher's own runs first, when the trial hasn't made them"
    plays = {name: argv for name, argv, *_ in steps}
    control = plays["precise"][plays["precise"].index("--out") + 1]
    assert control == "results/value/precise-on-teacher-plain.json"
    for change in jobs.TUNE:
        play = plays[change]
        assert play[play.index("--teacher-options") + 1] == f"precise,{change}"
        assert play[play.index("--runs") + 1] == "2000" and play[play.index("--seed") + 1] == "1"
        out = play[play.index("--out") + 1]
        assert out.startswith("results/tune/") and "=" not in out and "," not in out
        assert plays[f"compare {change}"][1:3] == [control, out]
    # With the trial's runs there, they are the control and aren't played again.
    (tmp_path / "results" / "value").mkdir(parents=True)
    (tmp_path / "results" / "value" / "precise-on-teacher-plain.json").write_text("{}")
    assert [name for name, *_ in jobs.tune_steps(tmp_path)][1] == jobs.TUNE[0]
    # Without precise (its trial found it no better), the teacher's own report is the control.
    (tmp_path / "results" / "value" / "precise-vs-teacher-plain.json").write_text(json.dumps({"win_rate_diff": -0.01}))
    steps = {name: argv for name, argv, *_ in jobs.tune_steps(tmp_path)}
    first = steps[jobs.TUNE[0]]
    assert first[first.index("--teacher-options") + 1] == jobs.TUNE[0]
    assert steps[f"compare {jobs.TUNE[0]}"][1] == "results/value/teacher-plain.json"
    assert "teacher-tune" in jobs.JOBS


def test_the_tunings_winners_are_tried_together_against_the_same_runs(tmp_path):
    steps = jobs.tune_steps(tmp_path, jobs.TUNE_2)
    plays = {name: argv for name, argv, *_ in steps}
    both = jobs.TUNE_2[0]
    assert "planner-edge=0.3" in both and "sharpness=2.5" in both
    play = plays[both]
    assert play[play.index("--teacher-options") + 1] == f"precise,{both}"
    out = play[play.index("--out") + 1]
    assert out == "results/tune/precise+planner-edge-0.3+sharpness-2.5-on-teacher-plain.json"
    assert plays[f"compare {both}"][1:3] == ["results/value/precise-on-teacher-plain.json", out]
    assert "teacher-tune-2" in jobs.JOBS


def test_the_ideas_from_the_players_runs_are_tried_as_options_against_the_same_runs(tmp_path):
    plays = {name: argv for name, argv, *_ in jobs.tune_steps(tmp_path, jobs.IDEAS)}
    play = plays["final-hand"]
    assert play[play.index("--teacher-options") + 1] == "precise,final-hand"
    assert plays["compare final-hand"][1] == "results/value/precise-on-teacher-plain.json"
    assert {"five-kind", "overstock=0.4", "midas=3"} <= set(plays)
    assert "teacher-ideas" in jobs.JOBS


def test_the_rounds_networks_together_are_tried_and_used_when_they_win_most(tmp_path):
    assert jobs.ensemble(2) == "models/value-1.bin,models/value-2.bin"
    assert jobs.ensemble(4) == ("models/value-1.bin,models/value-2.bin,models/value-3.bin,models/value-4.bin,"
                                "models/value-3b.bin,models/value-4b.bin"), "two networks a round from round 3"
    steps = {n: a for n, a, *_ in jobs.JOBS["teacher-ensemble"]}
    for w in ("1", "1.5"):
        argv = steps[f"ensemble-w{w}"]
        assert argv[argv.index("--value-net") + 1] == "models/value-1.bin,models/value-2.bin"
        assert argv[argv.index("--value-weight") + 1] == w and argv[argv.index("--seed") + 1] == "1"
        assert argv[argv.index("--out") + 1] == f"results/value/ensemble2-w{w}.json"
    steps = {n: a for n, a, *_ in jobs.JOBS["teacher-learned-4"]}
    assert list(steps) == ["build", "weight-1", "ensemble-w1", "ensemble-w1.5"]
    assert steps["ensemble-w1"][steps["ensemble-w1"].index("--value-net") + 1] == jobs.ensemble(4)
    (tmp_path / "models").mkdir()
    (tmp_path / "results" / "value").mkdir(parents=True)
    for n in (1, 2):
        (tmp_path / "models" / f"value-{n}.bin").write_bytes(b"BMVN")
    for report, rate in (("teacher-w1.json", 0.393), ("teacher2-w1.json", 0.39), ("ensemble2-w1.json", 0.41),
                         ("ensemble2-w1.5.json", 0.405)):
        (tmp_path / "results" / "value" / report).write_text(json.dumps({"win_rate": rate}))
    assert jobs.best_learned(tmp_path) == ("models/value-1.bin,models/value-2.bin", "1",
                                           "results/value/ensemble2-w1.json")
    play, = [a for n, a, *_ in jobs.option_trial("precise", tmp_path) if n == "precise"]
    assert play[play.index("--value-net") + 1] == "models/value-1.bin,models/value-2.bin"
    assert play[play.index("--out") + 1] == "results/value/precise-on-ensemble2-w1.json"
    (tmp_path / "models" / "value-2.bin").unlink()
    assert jobs.best_learned(tmp_path)[0] == "models/value-1.bin", "an ensemble needs all its networks"
    out = tmp_path / "results" / "x.json"
    out.write_text(json.dumps({"kind": "baseline", "bot": "teacher bot with a learned value", "runs": 10, "wins": 4,
                               "win_rate": 0.4}))
    e = jobs.progress_entry("teacher-ensemble", "ensemble", ["x", "--value-net", jobs.ensemble(2), "--value-weight",
                                                             "1", "--out", "results/x.json"], "HEAD", root=tmp_path)
    assert e["name"] == "Learned values 1+2 \u00d71"


def test_a_step_with_teacher_options_is_named_with_them(tmp_path):
    out = tmp_path / "results" / "x.json"
    out.parent.mkdir(parents=True)
    out.write_text(json.dumps({"kind": "baseline", "bot": "teacher bot with a learned value", "runs": 10, "wins": 4,
                               "win_rate": 0.4}))
    argv = ["x", "--value-net", "models/value-1.bin", "--value-weight", "1", "--teacher-options", "steady,no-juggle",
            "--out", "results/x.json"]
    e = jobs.progress_entry("teacher-steady", "steady", argv, "HEAD", root=tmp_path)
    assert e["name"] == "Learned value \u00d71, steady, no-juggle" and e["teacher_options"] == "steady,no-juggle"
    out.write_text(json.dumps({"kind": "comparison", "seeds": 10}))
    assert jobs.progress_entry("teacher-steady", "compare", ["scripts/compare_runs.py", "a", "b", "--out",
                                                             "results/x.json"], "HEAD", root=tmp_path) is None


def test_every_learned_value_step_reports_where_the_log_finds_it():
    for job in ("teacher-data", "teacher-learned"):
        for name, argv, *_ in jobs.JOBS[job]:
            if name != "build":
                assert "--out" in argv and argv[argv.index("--out") + 1].startswith("results/"), (job, name)


def test_round_four_records_with_the_trial_that_beat_the_precise_teacher_clearly(tmp_path):
    folder = tmp_path / "results" / "value"
    folder.mkdir(parents=True)
    (tmp_path / "models").mkdir()
    (tmp_path / "models" / "value-1.bin").write_bytes(b"BMVN")
    (folder / "teacher-w1.json").write_text(json.dumps({"kind": "baseline", "runs": 5000, "win_rate": 0.4}))
    record = lambda: {n: a for n, a, *_ in jobs.learned_round(4, tmp_path)["teacher-data-4"]}["record"]
    opts = lambda a: a[a.index("--teacher-options") + 1]
    assert opts(record()) == "precise", "no trials yet: the precise teacher"
    tune = tmp_path / "results" / "tune"
    tune.mkdir(parents=True)
    f = f"f{jobs.FORMULA}-"
    for pre, change, diff, p in (("", "long-run=0.35", 0.06, 0.001), (f, "final-hand", 0.012, 0.3),
                                 (f, "planner-edge=0.3,sharpness=2.5", 0.02, 0.05), (f, "midas=3", 0.03, 0.2)):
        name = pre + jobs.file_safe(f"precise,{change}")
        report = f"results/tune/{name}-on-teacher-w1.json"
        (tmp_path / report).write_text(json.dumps({"teacher_options": ["precise", *change.split(",")]}))
        (tune / f"{name}-vs-{pre}precise-on-teacher-w1.json").write_text(json.dumps(
            {"kind": "comparison", "b": {"report": report}, "win_rate_diff": diff, "p_value": p}))
    assert opts(record()) == "precise,planner-edge=0.3,sharpness=2.5", \
        "the largest clear gain on the formula in force, not the largest, nor one on an older formula"
    nets = lambda a: (a[a.index("--value-net") + 1], a[a.index("--value-weight") + 1])
    assert nets(record()) == ("models/value-1.bin", "1"), "with the networks the trial played with"
    assert opts({n: a for n, a, *_ in jobs.learned_round(3, tmp_path)["teacher-data-3"]}["record"]) == "precise", \
        "round 3 recorded as it did"
    # A trial of other networks (make teacher-next's "values 1-4") that gains most clearly records with them.
    report = f"results/tune/{f}precise-on-ensemble4-w1.5.json"
    (tmp_path / report).write_text(json.dumps(
        {"teacher_options": ["precise"], "value_net": "models/a.bin,models/b.bin", "value_weight": 1.5}))
    (tune / f"{f}precise-on-ensemble4-w1.5-vs-{f}precise-on-teacher-w1.json").write_text(json.dumps(
        {"kind": "comparison", "b": {"report": report}, "win_rate_diff": 0.025, "p_value": 0.01}))
    rec = record()
    assert nets(rec) == ("models/a.bin,models/b.bin", "1.5") and opts(rec) == "precise"
    assert rec.count("--value-net") == 1 and rec.count("--teacher-options") == 1


def test_the_next_trials_are_set_against_a_control_with_the_formula_in_force(tmp_path):
    folder = tmp_path / "results" / "value"
    folder.mkdir(parents=True)
    (tmp_path / "models").mkdir()
    (tmp_path / "models" / "value-1.bin").write_bytes(b"BMVN")
    (folder / "teacher-w1.json").write_text(json.dumps({"kind": "baseline", "runs": 5000, "win_rate": 0.4}))
    (folder / "precise-on-teacher-w1.json").write_text(json.dumps({"runs": 2000}))  # measured with older numbers
    steps = {n: a for n, a, *_ in jobs.next_steps(tmp_path)}
    control = steps["precise"][steps["precise"].index("--out") + 1]
    assert control == f"results/value/f{jobs.FORMULA}-precise-on-teacher-w1.json"
    for change in jobs.NEXT:
        assert steps[f"compare {change}"][1] == control
    assert steps["compare values 1-4"][1] == control
    nets = steps["values 1-4"][steps["values 1-4"].index("--value-net") + 1]
    assert nets == jobs.ensemble(4)
    assert "teacher-next" in jobs.JOBS


def test_the_second_ideas_are_set_against_the_control_on_the_formula_in_force(tmp_path):
    folder = tmp_path / "results" / "value"
    folder.mkdir(parents=True)
    (tmp_path / "models").mkdir()
    (tmp_path / "models" / "value-1.bin").write_bytes(b"BMVN")
    (folder / "teacher-w1.json").write_text(json.dumps({"kind": "baseline", "runs": 5000, "win_rate": 0.4}))
    (folder / f"f{jobs.FORMULA}-precise-on-teacher-w1.json").write_text(json.dumps({"runs": 2000}))
    steps = {n: a for n, a, *_ in jobs.tune_steps(tmp_path, jobs.IDEAS_2, tag=f"f{jobs.FORMULA}")}
    assert "precise" not in steps, "the control's runs are there already"
    for change in jobs.IDEAS_2:
        play = steps[change]
        assert play[play.index("--teacher-options") + 1] == f"precise,{change}"
        assert steps[f"compare {change}"][1] == f"results/value/f{jobs.FORMULA}-precise-on-teacher-w1.json"
        assert Path(steps[f"compare {change}"][-1]).name.startswith(f"f{jobs.FORMULA}-")
    assert "teacher-ideas-2" in jobs.JOBS


def test_the_focus_trials_are_set_against_the_control_on_the_formula_in_force(tmp_path):
    folder = tmp_path / "results" / "value"
    folder.mkdir(parents=True)
    (tmp_path / "models").mkdir()
    (tmp_path / "models" / "value-1.bin").write_bytes(b"BMVN")
    (folder / "teacher-w1.json").write_text(json.dumps({"kind": "baseline", "runs": 5000, "win_rate": 0.4}))
    (folder / f"f{jobs.FORMULA}-precise-on-teacher-w1.json").write_text(json.dumps({"runs": 2000}))
    steps = {n: a for n, a, *_ in jobs.tune_steps(tmp_path, jobs.FOCUS, tag=f"f{jobs.FORMULA}")}
    assert "precise" not in steps
    for change in jobs.FOCUS:
        play = steps[change]
        assert play[play.index("--teacher-options") + 1] == f"precise,{change}"
        assert steps[f"compare {change}"][1] == f"results/value/f{jobs.FORMULA}-precise-on-teacher-w1.json"
    assert "teacher-focus" in jobs.JOBS


def test_recordings_explore_from_round_7_and_measuring_never_does(tmp_path):
    args = ["--value-net", "models/value-1.bin", "--value-weight", "1.5", "--teacher-options", "precise"]
    assert jobs.exploring(args, 6) == args
    assert jobs.exploring(args, 7)[-1] == f"precise,{jobs.EXPLORE}"
    assert jobs.exploring([], 8) == ["--teacher-options", jobs.EXPLORE]
    record = jobs.learned_round(7, tmp_path)["teacher-data-7"][1][1]
    assert jobs.EXPLORE in record[record.index("--teacher-options") + 1]
    for name, steps in jobs.learned_round(7, tmp_path).items():
        if name != "teacher-data-7":
            assert not any(jobs.EXPLORE in " ".join(map(str, s[1])) for s in steps), name


def test_a_new_control_is_set_against_the_one_before(tmp_path):
    folder = tmp_path / "results" / "value"
    folder.mkdir(parents=True)
    (tmp_path / "models").mkdir()
    nets = jobs.ensemble(6).split(",")
    for n in nets:
        (tmp_path / n).write_bytes(b"BMVN")
    (folder / "ensemble6-w1.5.json").write_text(json.dumps({"kind": "baseline", "runs": 5000, "win_rate": 0.56}))
    (folder / "ensemble5-w1.5.json").write_text(json.dumps({"kind": "baseline", "runs": 5000, "win_rate": 0.54}))
    (folder / f"f{jobs.FORMULA}-precise-on-ensemble5-w1.5.json").write_text(json.dumps({"runs": 2000}))
    steps = {n: a for n, a, *_ in jobs.control_steps(tmp_path)}
    play = steps["precise"]
    assert play[play.index("--out") + 1] == f"results/value/f{jobs.FORMULA}-precise-on-ensemble6-w1.5.json"
    assert steps["compare"][1:3] == [f"results/value/f{jobs.FORMULA}-precise-on-ensemble5-w1.5.json",
                                     f"results/value/f{jobs.FORMULA}-precise-on-ensemble6-w1.5.json"]
    assert "teacher-control" in jobs.JOBS
