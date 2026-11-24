"""The preflight battery's quick checks (lab #38): the cheap ones run here; the rest run in `make preflight-quick`."""
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]


def test_the_quick_checks_pass(tmp_path):
    out = tmp_path / "preflight.json"
    p = subprocess.run([sys.executable, "scripts/preflight.py", "--quick", "--cpu", "--only",
                        "returns,checkpoint,act-eval,candidates", "--out", str(out)],
                       cwd=ROOT, capture_output=True, text=True, timeout=900)
    assert p.returncode == 0, p.stdout + p.stderr
    assert "4 passed, 0 failed" in p.stdout
