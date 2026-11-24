#!/usr/bin/env bash
# A rented Linux GPU machine (Ubuntu 22.04 or 24.04 with NVIDIA drivers: RunPod, Vast.ai, Lambda and the like),
# from nothing to checked (lab #38). After cloning the public repo onto the machine:
#     cd balatro-meta && bash scripts/rent/setup.sh
# Installs what the PC's bootstrap.sh installs (system packages, uv, Rust, the Python environment with PyTorch for
# CUDA), builds the engine, then runs the preflight checks at training's own size. Safe to run again.
# Afterwards: start a run in tmux (so it survives a dropped connection), e.g.
#     tmux new -s run 'make smoke-learner-3'
# and fetch the results with scripts/rent/pack.sh.
set -euo pipefail
cd "$(dirname "$0")/../.."
say() { printf '\n== %s\n' "$*"; }

say "The GPU"
nvidia-smi --query-gpu=name,memory.total,driver_version --format=csv,noheader || {
  echo "No NVIDIA GPU visible: pick a GPU machine (or a CUDA image) and try again." >&2
  exit 1
}
echo "CPU threads: $(nproc)"

say "sudo, git, tmux (root-only images have no sudo)"
if ! command -v sudo >/dev/null; then
  if [ "$(id -u)" = 0 ]; then apt-get update -qq && apt-get install -y -qq sudo; else
    echo "No sudo and not root: install build-essential curl luajit pkg-config git tmux by hand." >&2; exit 1
  fi
fi
for c in git tmux; do command -v "$c" >/dev/null || sudo apt-get install -y -qq "$c"; done

say "The PC's setup (bootstrap.sh): system packages, uv, Rust, Python environment, PyTorch with CUDA"
bash bootstrap.sh
export PATH="$HOME/.local/bin:$HOME/.cargo/bin:$PATH"

say "The engine"
make --no-print-directory build

say "Preflight at training's own size (about 30 to 60 minutes)"
if make --no-print-directory preflight; then
  echo
  echo "Ready. Start a run in tmux, e.g.:  tmux new -s run 'make smoke-learner-3'"
else
  echo
  echo "Preflight failed: see results/preflight.json before spending GPU hours." >&2
  exit 1
fi
