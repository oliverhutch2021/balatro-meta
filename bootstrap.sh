#!/usr/bin/env bash
# One-time setup on the PC, run inside Ubuntu (WSL2) from the repo folder:
#     cd /mnt/h/balatro-meta && bash bootstrap.sh
# Safe to run again: every step checks before it changes anything.
# SKIP_GPU=1 skips the PyTorch download (about 3 GB); `make setup-gpu` does it later.
set -euo pipefail
cd "$(dirname "$0")"
say() { printf '\n== %s\n' "$*"; }

say "Git settings for a Windows-drive checkout used from Linux"
git config core.autocrlf false
git config core.fileMode false
if [ -z "$(git config user.email || true)" ]; then
  # results are committed as the repo's owner: the global identity if set, else the latest non-Claude
  # author of this repo, else of the sibling monopoly-strategy repo
  who=$(git log --format='%an|%ae' | awk -F'|' '$2 != "noreply@anthropic.com" {print; exit}' || true)
  if [ -z "$who" ] && [ -d ../monopoly-strategy/.git ]; then
    who=$(git -C ../monopoly-strategy log --format='%an|%ae' | awk -F'|' '$2 != "noreply@anthropic.com" {print; exit}' || true)
  fi
  if [ -n "$who" ]; then
    name=${who%|*}; email=${who##*|}
    git config user.name "$name"; git config user.email "$email"
    echo "Result commits will be made as: $name <$email> (this repo only)"
  else
    echo "No identity found yet. Set one with: git config --global user.name 'Your Name' && git config --global user.email you@example.com"
  fi
fi

say "Line endings"
n_crlf=$(git ls-files --eol | grep -c 'w/crlf' || true)
if [ "$n_crlf" -gt 0 ]; then
  if git diff --quiet --ignore-cr-at-eol HEAD; then
    echo "Switching $n_crlf files to LF line endings (one-time; no content changes)..."
    git rm -r --cached --quiet . && git reset --quiet --hard
  else
    echo "You have uncommitted changes, so line endings were left alone. Commit or stash them, then rerun." >&2
  fi
else
  echo "Already LF."
fi

say "System packages (make, compilers, curl, LuaJIT)"
missing=()
for c in make gcc curl luajit pkg-config; do command -v "$c" >/dev/null || missing+=("$c"); done
if [ ${#missing[@]} -gt 0 ]; then
  echo "Installing: build-essential curl luajit pkg-config (needs your Ubuntu password)"
  sudo apt-get update -qq && sudo apt-get install -y -qq build-essential curl luajit pkg-config
else
  echo "Present."
fi

say "uv (Python package manager)"
if ! command -v uv >/dev/null; then
  curl -LsSf https://astral.sh/uv/install.sh | sh
fi
export PATH="$HOME/.local/bin:$PATH"
uv --version

say "Rust (rustup, stable)"
if ! command -v cargo >/dev/null && [ ! -x "$HOME/.cargo/bin/cargo" ]; then
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --component clippy,rustfmt
fi
# shellcheck disable=SC1091
source "$HOME/.cargo/env"
rustc --version

say "Python environment and the engine module (first build takes a few minutes)"
make --no-print-directory setup

if [ "${SKIP_GPU:-0}" != "1" ]; then
  say "PyTorch with CUDA for the GPU (about 3 GB, one-time)"
  make --no-print-directory setup-gpu || echo "GPU setup didn't finish; rerun later with: make setup-gpu"
fi

say "Checks"
make --no-print-directory doctor || true

cat <<'EOF'

Setup finished. Next (step 2 in RUNNING.md), from Ubuntu in this folder (or `wsl make smoke` in PowerShell):
    make smoke
EOF
