"""Check that this machine is ready for the Balatro Meta jobs, and print a pass/warn/fail table.

    make doctor        (or: python scripts/doctor.py [--quick])

Things later phases need (PyTorch on the GPU, BalatroBot) are reported as "LATER" until then.
Exit code 1 if anything fails.
"""
from __future__ import annotations

import argparse
import os
import platform
import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
FIXTURE = ROOT / "tests" / "fixtures" / "rng" / "reference.tsv"
rows: list[tuple[str, str, str]] = []


def add(status: str, check: str, detail: str) -> None:
    rows.append((status, check, detail))


def sh(*cmd: str, timeout: int = 600) -> tuple[int, str]:
    try:
        r = subprocess.run(cmd, cwd=ROOT, text=True, capture_output=True, timeout=timeout)
        return r.returncode, (r.stdout + r.stderr).strip()
    except (OSError, subprocess.TimeoutExpired) as e:
        return 1, str(e)


def meminfo_gb() -> float | None:
    try:
        for line in Path("/proc/meminfo").read_text().splitlines():
            if line.startswith("MemTotal:"):
                return int(line.split()[1]) / 1024 ** 2
    except OSError:
        return None
    return None


def free_gb(path: Path) -> float:
    return shutil.disk_usage(path).free / 1024 ** 3


def system() -> None:
    v = Path("/proc/version").read_text() if Path("/proc/version").exists() else platform.platform()
    wsl = "microsoft" in v.lower()
    add("PASS" if wsl else "WARN", "Running under WSL2", "yes" if wsl else f"no ({platform.platform()})")
    n = os.cpu_count() or 1
    add("PASS" if n >= 16 else "WARN", "CPU threads", f"{n} (the engine uses all of them)")
    mem = meminfo_gb()
    if mem is not None:
        add("PASS" if mem >= 12 else "WARN", "Memory available to Linux",
            f"{mem:.1f} GB" + ("" if mem >= 12 else "; raise it in %UserProfile%\\.wslconfig ([wsl2] memory=...)"))
    add("PASS" if free_gb(ROOT) >= 5 else "FAIL", "Free disk where the repo lives", f"{free_gb(ROOT):.0f} GB at {ROOT}")
    home = Path.home()
    add("PASS" if free_gb(home) >= 40 else "WARN", "Free disk on the WSL disk (builds, checkpoints)",
        f"{free_gb(home):.0f} GB at {home}")


def git_checks() -> None:
    code, out = sh("git", "rev-parse", "--abbrev-ref", "HEAD")
    add("PASS" if code == 0 and out == "main" else "WARN", "Git branch", out or "not a git repo")
    _, crlf = sh("git", "config", "--get", "core.autocrlf")
    _, mode = sh("git", "config", "--get", "core.fileMode")
    ok = crlf in ("", "false", "input") and mode == "false"
    add("PASS" if ok else "WARN", "Git settings for a Windows-drive checkout",
        f"core.autocrlf={crlf or 'unset'}, core.fileMode={mode or 'unset'} (bootstrap.sh sets these)")
    _, eol = sh("git", "ls-files", "--eol")
    n_crlf = sum("w/crlf" in line for line in eol.splitlines())
    add("PASS" if n_crlf == 0 else "WARN", "Line endings in the working tree",
        "all LF" if n_crlf == 0 else f"{n_crlf} files still CRLF; rerun bash bootstrap.sh")
    _, email = sh("git", "config", "--get", "user.email")
    add("PASS" if email else "FAIL", "Git identity for result commits", email or "missing; rerun bash bootstrap.sh")


def toolchain() -> None:
    rustc = shutil.which("rustc")
    _, ver = sh("rustc", "--version") if rustc else (1, "")
    add("PASS" if rustc else "FAIL", "Rust toolchain", ver or "missing; run bash bootstrap.sh")
    lj = shutil.which("luajit")
    _, ver = sh("luajit", "-v") if lj else (1, "")
    add("PASS" if lj else "WARN", "LuaJIT (for make rng-check)",
        ver.splitlines()[0] if ver else "missing; sudo apt install luajit")


def python_checks(quick: bool) -> None:
    ok = sys.version_info[:2] == (3, 12)
    add("PASS" if ok else "WARN", "Python", f"{platform.python_version()} at {sys.executable}")
    try:
        import numpy

        import balatro_meta as bm
    except ImportError as e:
        add("FAIL", "Engine module", f"{e}; run make setup")
        return
    add("PASS", "Engine module", f"balatro_meta {bm.__version__}, numpy {numpy.__version__}")
    # One fixture line as a quick end-to-end check of the compiled RNG.
    for line in FIXTURE.read_text().splitlines():
        kind, d, want = line.split("\t")[:3]
        if kind == "RAND":
            got = bm.lua_random(float(d), 12)
            same = got == [float(x) for x in want.split(",")]
            add("PASS" if same else "FAIL", "Compiled RNG matches LuaJIT (one fixture line)",
                "identical" if same else f"differs for seed value {d}")
            break
    if not quick:
        code, out = sh(sys.executable, "-m", "pytest", "-q", "-p", "no:cacheprovider")
        add("PASS" if code == 0 else "FAIL", "Python tests", out.splitlines()[-1] if out else "")


def gpu() -> None:
    smi = shutil.which("nvidia-smi") or ("/usr/lib/wsl/lib/nvidia-smi" if Path("/usr/lib/wsl/lib/nvidia-smi").exists()
                                          else None)
    if smi:
        code, out = sh(smi, "--query-gpu=name,memory.total,driver_version", "--format=csv,noheader")
        add("PASS" if code == 0 else "WARN", "GPU visible in WSL", out or "nvidia-smi failed")
    else:
        add("WARN", "GPU visible in WSL", "nvidia-smi not found; update the NVIDIA driver on Windows")
    try:
        import torch
    except ImportError:
        add("LATER", "PyTorch with CUDA (phase 4)", "make setup-gpu")
        return
    ok = torch.cuda.is_available()
    add("PASS" if ok else "WARN", "PyTorch with CUDA (phase 4)",
        f"torch {torch.__version__}, " + (torch.cuda.get_device_name(0) if ok else "CUDA not available"))


def game_runtime() -> None:
    """Windows Python and the game's lua51.dll, used by make rng-check to test the RNG on the game's own runtime."""
    sys.path.insert(0, str(ROOT / "scripts"))
    try:
        import rng_game
        py = rng_game.find_windows_python()
        add("PASS", "Windows Python (for the game-runtime RNG check)", str(py))
    except SystemExit as e:
        add("WARN", "Windows Python (for the game-runtime RNG check)", str(e))
    except Exception as e:  # noqa: BLE001 - interop missing etc.
        add("WARN", "Windows Python (for the game-runtime RNG check)", f"not reachable from WSL: {e}")
    try:
        import rng_game
        add("PASS", "Balatro's lua51.dll", str(rng_game.find_balatro()))
    except SystemExit as e:
        add("WARN", "Balatro's lua51.dll", str(e))


def later() -> None:
    add("LATER", "Balatro + Lovely + Steamodded + BalatroBot (phase 3)", "set up when validation starts")


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--quick", action="store_true", help="skip the full Python test suite")
    a = ap.parse_args()
    system()
    git_checks()
    toolchain()
    python_checks(a.quick)
    gpu()
    game_runtime()
    later()
    w = max(len(c) for _, c, _ in rows)
    print(f"\n{'':6}{'Check':{w}}  Detail")
    for status, check, detail in rows:
        print(f"{status:6}{check:{w}}  {detail}")
    fails = sum(s == "FAIL" for s, _, _ in rows)
    print(f"\n{'All checks passed.' if not fails else f'{fails} check(s) failed.'}")
    sys.exit(1 if fails else 0)


if __name__ == "__main__":
    main()
