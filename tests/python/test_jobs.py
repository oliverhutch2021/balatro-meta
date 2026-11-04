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
    assert f"teacher-data-{jobs.LAST_ROUND + 1}" not in jobs.JOBS
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


def test_the_big_trial_is_named_so_a_clear_gain_is_recorded_with(tmp_path):
    folder = tmp_path / "results" / "value"
    folder.mkdir(parents=True)
    (tmp_path / "models").mkdir()
    (tmp_path / "models" / "value-1.bin").write_bytes(b"BMVN")
    (folder / "teacher-w1.json").write_text(json.dumps({"kind": "baseline", "runs": 5000, "win_rate": 0.4}))
    steps = {n: a for n, a, *_ in jobs.big_steps(tmp_path)}
    f = jobs.FORMULA
    assert steps["control"][steps["control"].index("--runs") + 1] == "8000"
    play = steps[jobs.BIG]
    assert play[play.index("--teacher-options") + 1] == f"precise,{jobs.BIG}"
    assert play[play.index("--out") + 1] == f"results/tune/f{f}-precise+focus+fair-skip-8k-on-teacher-w1.json"
    out = Path(steps["compare"][-1])
    assert out.match(f"f{f}-*-vs-*-on-teacher-w1.json"), "best_recording reads it"
    assert "teacher-big" in jobs.JOBS



def test_the_weight_trials_are_set_against_the_networks_own_weight_and_recorded_with(tmp_path):
    folder = tmp_path / "results" / "value"
    folder.mkdir(parents=True)
    (tmp_path / "models").mkdir()
    (tmp_path / "models" / "value-1.bin").write_bytes(b"BMVN")
    (folder / "teacher-w1.json").write_text(json.dumps({"kind": "baseline", "runs": 5000, "win_rate": 0.4}))
    steps = {n: a for n, a, *_ in jobs.weight_steps(tmp_path)}
    f = jobs.FORMULA
    control = steps["control"]
    assert control[control.index("--runs") + 1] == "8000" and control[control.index("--value-weight") + 1] == "1"
    assert control[control.index("--out") + 1] == f"results/value/f{f}-precise-8k-on-teacher-w1.json"
    for w in jobs.WEIGHT_TRIALS:
        play = steps[f"weight-{w}"]
        assert play[play.index("--value-weight") + 1] == w and play[play.index("--teacher-options") + 1] == "precise"
        assert play[play.index("--value-net") + 1] == "models/value-1.bin"
        assert Path(steps[f"compare-{w}"][-1]).match(f"f{f}-*-vs-*-on-teacher-w1.json"), "best_recording reads it"
    assert "vs-round-7" not in steps, "only once round 7's 8,000 runs are there"
    (tmp_path / jobs.ROUND_7_8K).parent.mkdir(parents=True)
    (tmp_path / jobs.ROUND_7_8K).write_text("{}")
    (folder / f"f{f}-precise-8k-on-teacher-w1.json").write_text("{}")
    steps = {n: a for n, a, *_ in jobs.weight_steps(tmp_path)}
    assert "control" not in steps and steps["vs-round-7"][1] == jobs.ROUND_7_8K
    (tmp_path / f"results/tune/f{f}-precise-8k-w2.5-on-teacher-w1.json").write_text("{}")
    steps = {n: a for n, a, *_ in jobs.weight_steps(tmp_path)}
    assert "weight-2.5" not in steps and "compare-2.5" in steps and "weight-4" in steps, "played once per networks"
    assert "teacher-weights" in jobs.JOBS


def test_the_high_weights_are_set_against_weight_4_as_well(tmp_path):
    folder = tmp_path / "results" / "value"
    folder.mkdir(parents=True)
    (tmp_path / "models").mkdir()
    (tmp_path / "models" / "value-1.bin").write_bytes(b"BMVN")
    (folder / "teacher-w1.json").write_text(json.dumps({"kind": "baseline", "runs": 5000, "win_rate": 0.4}))
    f = jobs.FORMULA
    steps = {n: a for n, a, *_ in jobs.weight_steps(tmp_path, weights=jobs.HIGH_WEIGHT_TRIALS,
                                                    against=jobs.HIGH_AGAINST)}
    for w in jobs.HIGH_WEIGHT_TRIALS:
        play = steps[f"weight-{w}"]
        assert play[play.index("--value-weight") + 1] == w and play[play.index("--runs") + 1] == "8000"
        assert play[play.index("--seed") + 1] == "1", "the usual seeds: the exam's stay for the exam"
        out = play[play.index("--out") + 1]
        against = steps[f"compare-{w}-vs-4"]
        assert against[1] == f"results/tune/f{f}-precise-8k-w4-on-teacher-w1.json" and against[2] == out
        assert against[-1] == f"results/tune/f{f}-precise-8k-w{w}-vs-w4-on-teacher-w1.json"
        assert f"compare-{w}" in steps, "and against 1.5, like the other weights"
    assert "weight-4" not in steps, "4 is already played: only set against"
    assert not any(n.startswith("compare-") and n.endswith("-vs-4") for n in
                   {n for n, *_ in jobs.weight_steps(tmp_path)}), "make teacher-weights is as before"
    assert "teacher-weights-high" in jobs.JOBS


def test_the_input_trial_trains_both_layouts_on_the_same_runs_and_sets_them_run_by_run(tmp_path):
    steps = {n: a for n, a, *_ in jobs.input_steps()}
    data = str(jobs.DATA / f"value-{jobs.INPUT_ROUND}")
    for layout in (1, 2):
        for name, seed in ((f"train-layout{layout}", "1"), (f"train-layout{layout}b", "2")):
            t = steps[name]
            assert t[t.index("--data") + 1] == data and t.count("--data") == 1, "round 11's runs alone"
            assert t[t.index("--input-layout") + 1] == str(layout) and t[t.index("--seed") + 1] == seed
        play = steps[f"play-layout{layout}"]
        nets = play[play.index("--value-net") + 1].split(",")
        assert nets == [steps[f"train-layout{layout}"][-3], steps[f"train-layout{layout}b"][-3]]
        assert play[play.index("--value-weight") + 1] == jobs.INPUT_WEIGHT
        assert play[play.index("--seed") + 1] == "1" and play[play.index("--runs") + 1] == "8000"
    compare = steps["compare"]
    assert compare[1] == steps["play-layout1"][-1] and compare[2] == steps["play-layout2"][-1]
    assert not any("value-11-layout" in m for m in jobs.ensemble(11).split(",")), "the ensemble keeps its own"
    assert "teacher-inputs" in jobs.JOBS
    # The Progress panel names each pair by its round and layout.
    out = tmp_path / play[play.index("--out") + 1]
    out.parent.mkdir(parents=True)
    out.write_text(json.dumps({"kind": "baseline", "runs": 8000, "bot": "teacher bot with a learned value"}))
    e = jobs.progress_entry("teacher-inputs", "play-layout2", play, "HEAD", root=tmp_path)
    assert e["name"].startswith(f"Learned value {jobs.INPUT_ROUND}, input layout 2 \u00d72.5")


def test_every_stake_is_measured_with_the_best_teacher_on_the_same_seeds(tmp_path):
    folder = tmp_path / "results" / "value"
    folder.mkdir(parents=True)
    (tmp_path / "models").mkdir()
    (tmp_path / "models" / "value-1.bin").write_bytes(b"BMVN")
    (folder / "teacher-w1.json").write_text(json.dumps({"kind": "baseline", "runs": 5000, "win_rate": 0.4}))
    steps = {n: a for n, a, *_ in jobs.stakes_steps(tmp_path)}
    best = jobs.best_recording(tmp_path)
    outs = set()
    for s in range(1, 9):
        play = steps[f"stake-{s}"]
        assert play[play.index("--stake") + 1] == str(s) and play[play.index("--seed") + 1] == "1"
        assert play[play.index("--runs") + 1] == str(jobs.STAKE_RUNS)
        assert play[play.index("--value-net") + 1] == best[best.index("--value-net") + 1], "the best teacher"
        out = play[play.index("--out") + 1]
        assert out.startswith(f"results/stakes/stake{s}-")
        outs.add(out)
    assert len(outs) == 8 and "teacher-stakes" in jobs.JOBS
    # The Progress panel names the stake, unless it is White's.
    p = tmp_path / out
    p.parent.mkdir(parents=True, exist_ok=True)
    p.write_text(json.dumps({"kind": "baseline", "runs": 2000, "bot": "teacher bot", "stake": "Gold Stake"}))
    assert jobs.progress_entry("teacher-stakes", "stake-8", play, "HEAD", root=tmp_path)["name"].endswith("Gold Stake")


def test_a_clear_gain_at_another_weight_is_what_the_next_round_records_with(tmp_path):
    folder = tmp_path / "results" / "value"
    tune = tmp_path / "results" / "tune"
    folder.mkdir(parents=True)
    tune.mkdir()
    (tmp_path / "models").mkdir()
    (tmp_path / "models" / "value-1.bin").write_bytes(b"BMVN")
    (folder / "teacher-w1.json").write_text(json.dumps({"kind": "baseline", "runs": 5000, "win_rate": 0.4}))
    f = jobs.FORMULA
    trial = f"results/tune/f{f}-precise-8k-w2.5-on-teacher-w1.json"
    (tmp_path / trial).write_text(json.dumps({"value_net": "models/value-1.bin", "value_weight": 2.5,
                                              "teacher_options": ["precise"]}))
    (tune / f"f{f}-precise-8k-w2.5-vs-f{f}-precise-8k-on-teacher-w1.json").write_text(json.dumps(
        {"win_rate_diff": 0.012, "p_value": 0.03, "b": {"report": trial}}))
    args = jobs.best_recording(tmp_path)
    assert args[args.index("--value-weight") + 1] == "2.5" and args[args.index("--teacher-options") + 1] == "precise"


def test_from_round_10_a_quarter_of_the_recorded_runs_commit_to_one_hand():
    assert jobs.exploring(["--teacher-options", "precise"], 9) == ["--teacher-options", "precise,explore=0.1"]
    assert jobs.exploring(["--teacher-options", "precise"], 10) == \
        ["--teacher-options", "precise,explore=0.1,commit=0.25"]
    rec = dict((n, a) for n, a, *_ in jobs.JOBS["teacher-data-10"])["record"]
    assert "commit=0.25" in rec[rec.index("--teacher-options") + 1]


def test_the_exam_measures_the_best_teacher_on_seeds_used_for_nothing_else(tmp_path):
    folder = tmp_path / "results" / "value"
    folder.mkdir(parents=True)
    (tmp_path / "models").mkdir()
    (tmp_path / "models" / "value-1.bin").write_bytes(b"BMVN")
    (folder / "teacher-w1.json").write_text(json.dumps({"kind": "baseline", "runs": 5000, "win_rate": 0.4}))
    steps = {n: a for n, a, *_ in jobs.exam_steps(tmp_path)}
    exam = steps["exam"]
    assert exam[exam.index("--seed") + 1] == str(jobs.EXAM_SEED) and exam[exam.index("--runs") + 1] == "8000"
    assert exam[exam.index("--value-net") + 1] == "models/value-1.bin"
    assert exam[exam.index("--teacher-options") + 1] == "precise" and "explore" not in " ".join(exam)
    assert exam[exam.index("--out") + 1] == f"results/exam/exam-8k-f{jobs.FORMULA}-precise-rounds1-w1.json"
    recording_bases = {int(a[a.index("--seed") + 1]) for job, steps in jobs.JOBS.items() if job.startswith("teacher-data")
                       for n, a, *_ in steps if n == "record"}
    assert jobs.EXAM_SEED not in recording_bases | {1}
    assert "teacher-exam" in jobs.JOBS


def test_the_weights_are_measured_on_the_newest_round_and_the_best_on_8000_seeds_is_recorded_with(tmp_path):
    f = jobs.FORMULA
    (tmp_path / "models").mkdir()
    for n in range(1, 10):
        for name in (f"value-{n}.bin", f"value-{n}b.bin"):
            (tmp_path / "models" / name).write_bytes(b"BMVN")
    value, tune = tmp_path / "results" / "value", tmp_path / "results" / "tune"
    value.mkdir(parents=True)
    tune.mkdir()
    # The 5,000-seed tests still favour round 8's networks; the newest round's are 9's.
    (value / "ensemble8-w1.5.json").write_text(json.dumps({"kind": "baseline", "runs": 5000, "win_rate": 0.57}))
    (value / "ensemble9-w1.5.json").write_text(json.dumps({"kind": "baseline", "runs": 5000, "win_rate": 0.55}))
    steps = {n: a for n, a, *_ in jobs.weight_steps(tmp_path)}
    play = steps["weight-2.5"]
    assert play[play.index("--value-net") + 1] == jobs.ensemble(9)
    assert play[play.index("--out") + 1] == f"results/tune/f{f}-precise-8k-w2.5-on-ensemble9-w1.5.json"

    def report(path, nets, w, rate, options=("precise",), runs=8000):
        (tmp_path / path).write_text(json.dumps({"kind": "baseline", "runs": runs, "win_rate": rate, "seed_base": 1,
                                                 "value_net": nets, "value_weight": w,
                                                 "teacher_options": list(options)}))
    assert jobs.best_8k(tmp_path) is None
    report(f"results/value/f{f}-precise-8k-on-ensemble8-w1.5.json", jobs.ensemble(8), 1.5, 0.6386)
    report(f"results/tune/f{f}-precise-8k-w2.5-on-ensemble8-w1.5.json", jobs.ensemble(8), 2.5, 0.6646)
    report(f"results/tune/f{f}-precise-8k-w2.5-on-ensemble9-w1.5.json", jobs.ensemble(9), 2.5, 0.6702)
    report(f"results/tune/f{f}-precise+explore-0.1-8k-on-ensemble9-w1.5.json", jobs.ensemble(9), 2.5, 0.70,
           options=("precise", "explore=0.1"))
    report(f"results/tune/f{f}-precise-2k-w4-on-ensemble9-w1.5.json", jobs.ensemble(9), 4, 0.71, runs=2000)
    rate, args = jobs.best_8k(tmp_path)
    assert rate == 0.6702 and args == ["--value-net", jobs.ensemble(9), "--value-weight", "2.5",
                                       "--teacher-options", "precise"], "exploring and 2,000-seed runs never count"
    assert jobs.best_recording(tmp_path) == args
    exam = dict((n, a) for n, a, *_ in jobs.exam_steps(tmp_path))["exam"]
    assert exam[exam.index("--out") + 1] == f"results/exam/exam-8k-f{f}-precise-rounds9-w2.5.json"


def test_committed_results_name_no_computer_and_no_home_folder(monkeypatch):
    home = str(Path.home())
    assert jobs.public(f"scripts/train_value.py --data {home}/.cache/balatro-meta/data/value-2") == \
        "scripts/train_value.py --data ~/.cache/balatro-meta/data/value-2"
    assert jobs.public(f"{home}/.venvs/x/bin/python -m pytest") == "~/.venvs/x/bin/python -m pytest"
    assert jobs.public("models/value-3.bin") == "models/value-3.bin"
    monkeypatch.delenv("BM_HOST", raising=False)
    import importlib
    assert importlib.reload(jobs).machine()["host"] == "PC"


def _stake_report(root, s, net="models/value-1.bin,models/value-1b.bin", w=2.5, options=("precise",)):
    p = root / jobs.stake_baseline(s)
    p.parent.mkdir(parents=True, exist_ok=True)
    p.write_text(json.dumps({"kind": "baseline", "runs": 2000, "win_rate": 0.1, "value_net": net,
                             "value_weight": w, "teacher_options": list(options)}))


def test_the_stake_trials_change_one_thing_on_the_teacher_of_3_november(tmp_path):
    for s in range(1, 9):
        _stake_report(tmp_path, s)
    steps = {n: a for n, a, *_ in jobs.stake_trials_steps(tmp_path)}
    for option, s in jobs.STAKE_TRIALS:
        play = steps[f"{jobs.file_safe(option)}-stake{s}"]
        assert play[play.index("--stake") + 1] == str(s) and play[play.index("--seed") + 1] == "1"
        assert play[play.index("--value-net") + 1] == "models/value-1.bin,models/value-1b.bin"
        assert play[play.index("--teacher-options") + 1] == f"precise,{option}", "that day's options, and the trial's"
        compare = steps[f"compare-{jobs.file_safe(option)}-stake{s}"]
        assert compare[1] == jobs.stake_baseline(s) and compare[2] == play[play.index("--out") + 1]
    assert "stakes-trials" in jobs.JOBS


def test_an_avoid_option_is_kept_only_when_its_stake_went_clearly_further(tmp_path):
    (tmp_path / "results" / "stakes").mkdir(parents=True)
    def trial(option, s, low):
        p = tmp_path / f"{jobs.stake_trial_name(option, s)}-vs.json"
        p.write_text(json.dumps({"blinds_beaten_diff_ci": [low, low + 0.4]}))
    trial("avoid-rental", 8, 0.05)
    trial("avoid-eternal", 4, -0.1)
    trial("reach=6", 3, 0.3)  # reach changes White's play too: never taken up from a trial
    assert jobs.stake_options(tmp_path) == ["avoid-rental"]
    t = jobs.stake_teacher(3, tmp_path, explore=True)
    assert t[t.index("--value-net") + 1] == "models/value-stakes-3.bin,models/value-stakes-3b.bin"
    assert t[t.index("--teacher-options") + 1] == f"precise,avoid-rental,{jobs.EXPLORE},{jobs.COMMIT}"


def test_a_stake_round_learns_every_stake_and_is_judged_at_white_first(tmp_path):
    for s in range(1, 9):
        _stake_report(tmp_path, s)
    folder = tmp_path / "results" / "value"
    folder.mkdir(parents=True, exist_ok=True)
    best = folder / f"f{jobs.FORMULA}-precise-8k-w2.5-on-ensemble8.json"
    best.write_text(json.dumps({"kind": "baseline", "runs": 8000, "seed_base": 1, "win_rate": 0.66,
                                "value_net": "models/value-8.bin", "value_weight": 2.5,
                                "teacher_options": ["precise"]}))
    r1, r2 = jobs.stake_round(1, tmp_path), jobs.stake_round(2, tmp_path)
    data1 = {n: a for n, a, *_ in r1["stakes-data-1"]}
    data2 = {n: a for n, a, *_ in r2["stakes-data-2"]}
    bases = set()
    for s in range(1, 9):
        # No night recording here: round 1 records at once with the best teacher, exploring.
        rec = data1[f"record-stake{s}"]
        assert rec[rec.index("--stake") + 1] == str(s)
        assert rec[rec.index("--record") + 1] == str(jobs.stake_data(1) / f"stake{s}")
        assert rec[rec.index("--runs") + 1] == str(jobs.STAKE_RUNS_1[s])
        assert jobs.EXPLORE in rec[rec.index("--teacher-options") + 1]
        rec2 = data2[f"record-stake{s}"]
        assert rec2[rec2.index("--value-net") + 1] == jobs.stake_nets(1), "round 2 records with round 1's pair"
        assert rec2[rec2.index("--runs") + 1] == str(jobs.STAKE_RUNS_N[s])
        assert not rec2[rec2.index("--out") + 1].startswith("/"), "reports stay in the repo, without the home"
        bases |= {rec[rec.index("--seed") + 1], rec2[rec2.index("--seed") + 1]}
    assert len(bases) == 16 and not bases & {"1", str(jobs.EXAM_SEED)}, "fresh seeds for every recording"
    assert sum(jobs.STAKE_RUNS_N[s] for s in (1, 2, 3, 4)) > 2 * sum(jobs.STAKE_RUNS_N[s] for s in (5, 6, 7, 8))
    train = {n: a for n, a, *_ in r2["stakes-net-2"]}["train"]
    folders = [train[i + 1] for i, a in enumerate(train) if a == "--data"]
    assert folders == [str(jobs.stake_data(m) / f"stake{s}") for m in (1, 2) for s in range(1, 9)]
    assert train[train.index("--newest") + 1] == "8" and train[train.index("--input-layout") + 1] == "3"
    play = r2["stakes-play-2"]
    names = [n for n, *_ in play]
    assert names[1:3] == ["white", "compare-white"], "White first: the round's test"
    white = {n: a for n, a, *_ in play}["white"]
    assert white[white.index("--runs") + 1] == "8000" and "--stake" not in white
    assert {n: a for n, a, *_ in play}["compare-white"][1] == str(best.relative_to(tmp_path))
    steps = {n: a for n, a, *_ in play}
    for s in range(1, 9):
        assert steps[f"compare-stake-{s}"][1] == jobs.stake_baseline(s)
        assert steps[f"compare-stake-{s}-round"][2] == steps[f"stake-{s}"][-1]
    assert "reach=6" in steps["reach-stake6"][steps["reach-stake6"].index("--teacher-options") + 1]
    # Round 1 also has a control for White: a pair that learned the round's White runs alone.
    net1 = {n: a for n, a, *_ in r1["stakes-net-1"]}
    assert net1["train-white"].count("--data") == 1
    assert net1["train-white"][net1["train-white"].index("--data") + 1] == str(jobs.stake_data(1) / "stake1")
    play1 = {n: a for n, a, *_ in r1["stakes-play-1"]}
    only = play1["white-only"]
    assert only[only.index("--value-net") + 1] == "models/value-stakes-1-white.bin,models/value-stakes-1-whiteb.bin"
    assert play1["compare-white-only"][1] == only[-1] and play1["compare-white-only"][2] == play1["white"][-1]
    assert "white-only" not in steps and "train-white" not in {n for n, *_ in r2["stakes-net-2"]}
    (tmp_path / only[-1]).write_text(json.dumps({"kind": "baseline", "runs": 8000, "bot": "teacher bot"}))
    e = jobs.progress_entry("stakes-play-1", "white-only", only, "HEAD", root=tmp_path)
    assert e["name"].startswith("White-only values 1 \u00d72.5")
    assert {f"stakes-{k}-{n}" for k in ("data", "net", "play") for n in (1, 2)} <= set(jobs.JOBS)
    # The Progress panel names the pair by its round, and leaves replays out.
    out = tmp_path / steps["stake-5"][-1]
    out.write_text(json.dumps({"kind": "baseline", "runs": 2000, "bot": "teacher bot with a learned value",
                               "stake": "Blue Stake"}))
    e = jobs.progress_entry("stakes-play-2", "stake-5", steps["stake-5"], "HEAD", root=tmp_path)
    assert e["name"].startswith("Stake-aware values 2 ×2.5") and e["name"].endswith("Blue Stake")
    rep = tmp_path / "results" / "stakes" / "replay-1-stake1.json"
    rep.write_text(json.dumps({"kind": "baseline", "runs": 3000, "bot": "replayed runs"}))
    assert jobs.progress_entry("stakes-data-1", "replay-stake1", ["--out", str(rep.relative_to(tmp_path))],
                               "HEAD", root=tmp_path) is None


def test_round_1_encodes_the_night_recording_afresh_when_it_is_there(tmp_path, monkeypatch):
    night = tmp_path / "night"
    for s in (3, 4):
        (night / f"stake{s}").mkdir(parents=True)
        (night / f"stake{s}" / "manifest.json").write_text("{}")
    monkeypatch.setattr(jobs, "NIGHT_RECORDING", night)
    steps = {n: a for n, a, *_ in jobs.stake_round(1, tmp_path)["stakes-data-1"]}
    assert steps["replay-stake3"][steps["replay-stake3"].index("--replay") + 1] == str(night / "stake3")
    assert steps["replay-stake3"][steps["replay-stake3"].index("--record") + 1] == str(jobs.stake_data(1) / "stake3")
    assert "record-stake3" not in steps and "record-stake5" in steps, "a stake not recorded yet is recorded here"
