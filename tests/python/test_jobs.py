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


def test_every_learned_value_step_reports_where_the_log_finds_it():
    for job in ("teacher-data", "teacher-learned"):
        for name, argv, *_ in jobs.JOBS[job]:
            if name != "build":
                assert "--out" in argv and argv[argv.index("--out") + 1].startswith("results/"), (job, name)
