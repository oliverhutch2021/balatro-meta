"""Phase 1 gate, part 2: the Rust RNG against the game's own Lua runtime.

    make rng-game        (or: python scripts/rng_game.py [--scale 200])

Balatro runs on LuaJIT 2.0.5 (its lua51.dll) with the Windows C runtime, not the LuaJIT 2.1 our
cloud fixtures come from. This runs two of our scripts on that exact runtime, through Windows
Python and WSL's Windows interop:

1. reference/runtime_probe.lua: facts about the runtime (rounding of exact halves, NaN handling,
   the generator's first values, the iteration order of the poker-hand table). Saved to
   results/rng/game_runtime.txt.
2. reference/rng_reference.lua at the given scale: seeds, hashes, every kind of stream draw, the
   13-place rounding and shuffles. The Rust RNG must match every value bit for bit.

Needs a Windows Python (miniconda/anaconda/python.org are found automatically; BM_WIN_PYTHON
overrides, as a /mnt/... path to python.exe) and the Balatro install (found in the usual Steam
folders; BM_BALATRO_DIR overrides). The game's files are only loaded, never copied.
"""
from __future__ import annotations

import argparse
import glob
import json
import os
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "results" / "rng"
TMP = ROOT / ".sync" / "rng_game"


def sh(*cmd: str, cwd: str | None = None) -> str:
    r = subprocess.run(cmd, text=True, capture_output=True, cwd=cwd)
    if r.returncode != 0:
        raise RuntimeError(f"{' '.join(cmd)} failed: {(r.stderr or r.stdout).strip()}")
    return r.stdout.strip()


def win_path(p: Path) -> str:
    return sh("wslpath", "-w", str(p))


def windows_profile() -> Path | None:
    try:
        prof = sh("cmd.exe", "/c", "echo %USERPROFILE%", cwd="/mnt/c").replace("\r", "").strip()
        return Path(sh("wslpath", "-u", prof))
    except (RuntimeError, OSError):
        return None


def find_windows_python() -> Path:
    env = os.environ.get("BM_WIN_PYTHON")
    if env:
        return Path(env)
    cands: list[Path] = []
    prof = windows_profile()
    if prof:
        for sub in ("miniconda3", "anaconda3", "Miniconda3", "Anaconda3"):
            cands.append(prof / sub / "python.exe")
        cands += [Path(p) for p in sorted(glob.glob(str(prof / "AppData/Local/Programs/Python/Python3*/python.exe")),
                                          reverse=True)]
    try:
        for line in sh("cmd.exe", "/c", "where python", cwd="/mnt/c").replace("\r", "").splitlines():
            if line and "WindowsApps" not in line:
                cands.append(Path(sh("wslpath", "-u", line.strip())))
    except (RuntimeError, OSError):
        pass
    for c in cands:
        if c.exists():
            return c
    raise SystemExit("No Windows Python found. Set BM_WIN_PYTHON to a python.exe, e.g. "
                     "BM_WIN_PYTHON=/mnt/c/Users/<you>/miniconda3/python.exe make rng-game")


def find_balatro() -> Path:
    env = os.environ.get("BM_BALATRO_DIR")
    cands = [Path(env)] if env else []
    for drive in "hcdefg":
        for lib in ("Steam", "SteamLibrary", "Program Files (x86)/Steam", "Program Files/Steam"):
            cands.append(Path(f"/mnt/{drive}/{lib}/steamapps/common/Balatro"))
    for c in cands:
        if (c / "lua51.dll").exists():
            return c
    raise SystemExit("Balatro's folder (with lua51.dll) wasn't found. Set BM_BALATRO_DIR, e.g. "
                     "BM_BALATRO_DIR='/mnt/h/Steam/steamapps/common/Balatro' make rng-game")


def run_on_game(py: Path, balatro: Path, script: Path, out: Path, *args: str) -> None:
    runner = ROOT / "scripts" / "win_lua_runner.py"
    cmd = [str(py), win_path(runner), win_path(balatro), win_path(script), win_path(out), *args]
    r = subprocess.run(cmd, text=True, capture_output=True, cwd="/mnt/c")
    if r.returncode != 0:
        raise SystemExit(f"Running {script.name} on the game's runtime failed:\n{r.stderr or r.stdout}")


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--scale", type=int, default=200)
    a = ap.parse_args()
    py, balatro = find_windows_python(), find_balatro()
    print(f"Windows Python: {py}\nBalatro: {balatro}", flush=True)
    OUT.mkdir(parents=True, exist_ok=True)
    TMP.mkdir(parents=True, exist_ok=True)

    probe_out = TMP / "runtime_probe.txt"
    run_on_game(py, balatro, ROOT / "reference" / "runtime_probe.lua", probe_out)
    probe = probe_out.read_text()
    (OUT / "game_runtime.txt").write_text(probe)
    print("\nRuntime probe (saved to results/rng/game_runtime.txt):\n" + probe, flush=True)

    fixture = TMP / "reference_game.tsv"
    print(f"Running the reference on the game's runtime at scale {a.scale}...", flush=True)
    run_on_game(py, balatro, ROOT / "reference" / "rng_reference.lua", fixture, str(a.scale))
    with fixture.open() as fh:
        meta = fh.readline().rstrip("\n").split("\t")
        lines = 1 + sum(1 for _ in fh)
    print(f"{lines:,} lines from {meta[1] if len(meta) > 1 else '?'} ({meta[2] if len(meta) > 2 else '?'})", flush=True)
    r = subprocess.run(["cargo", "test", "--release", "-p", "bm-rng", "--test", "luajit_fixtures"], cwd=ROOT,
                       env={**os.environ, "BM_RNG_FIXTURE": str(fixture)}, text=True, capture_output=True)
    passed = r.returncode == 0
    tail = "\n".join((r.stdout + r.stderr).strip().splitlines()[-15:])
    print(tail, flush=True)
    summary = {"runtime": meta[1] if len(meta) > 1 else None, "mode": meta[2] if len(meta) > 2 else None,
               "scale": a.scale, "lines": lines, "rust_matches": passed, "balatro_dir": str(balatro),
               "windows_python": str(py)}
    (OUT / "game_check.json").write_text(json.dumps(summary, indent=1) + "\n")
    fixture.unlink(missing_ok=True)
    if not passed:
        raise SystemExit("The Rust RNG differs from the game's runtime; see the test output above.")
    print(f"\nThe Rust RNG matches the game's own runtime on all {lines:,} lines.")


if __name__ == "__main__":
    sys.exit(main())
