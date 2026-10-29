# Balatro Meta: everything you run on the PC is a target here. See RUNNING.md.
#
# The Python environment and the Rust build live on the WSL disk (fast); the repo can stay on a Windows drive.

SHELL := /bin/bash
# Make cargo and uv reachable even from non-login shells (e.g. `wsl make smoke` in PowerShell).
export PATH := $(HOME)/.cargo/bin:$(HOME)/.local/bin:$(PATH)
VENV ?= $(HOME)/.venvs/balatro-meta
PY := $(VENV)/bin/python
export CARGO_TARGET_DIR ?= $(HOME)/.cache/balatro-meta/target
CARGO ?= cargo
LUAJIT ?= luajit
STRESS ?= 40
FORCE ?=
FORCE_FLAG := $(if $(FORCE),--force,)
WATCH ?= 1
JOB_FLAGS := $(FORCE_FLAG) $(if $(filter 0,$(WATCH)),--no-watch,)

.DEFAULT_GOAL := help
.PHONY: help setup setup-gpu build test doctor sync status smoke rng-check rng-game rng-fixtures rng-stress fmt lint bench watch watch-dry watch-mod

help:  ## List the commands
	@grep -hE '^[a-z0-9-]+:.*## ' $(MAKEFILE_LIST) | sed -E 's/:.*## /\t/' | expand -t 14

setup:  ## Create or update the Python environment and build the engine (Python 3.12, on the WSL disk)
	@command -v uv >/dev/null || { echo "uv is missing: run bash bootstrap.sh"; exit 1; }
	@command -v cargo >/dev/null || { echo "Rust is missing: run bash bootstrap.sh"; exit 1; }
	@test -x $(PY) || uv venv --quiet --python 3.12 $(VENV)
	uv pip install --quiet --python $(PY) -e ".[dev]"
	@echo "Python environment ready at $(VENV)"

setup-gpu:  ## Install PyTorch with CUDA for the RTX 4070 Ti (used from phase 4)
	uv pip install --quiet --python $(PY) torch --index-url https://download.pytorch.org/whl/cu128
	$(PY) -c "import torch; ok = torch.cuda.is_available(); print('PyTorch', torch.__version__, '| CUDA', ok, '|', torch.cuda.get_device_name(0) if ok else 'no GPU visible')"

build:  ## Rebuild the Rust engine module after a code change (jobs do this themselves)
	VIRTUAL_ENV=$(VENV) $(PY) -m maturin develop --release --uv

test:  ## Run all tests, Rust and Python
	$(CARGO) test --release
	$(PY) -m pytest -q -p no:cacheprovider

doctor:  ## Check this machine and print a pass/warn/fail table
	$(PY) scripts/doctor.py

sync:  ## Fast-forward to the code Claude left in .sync/claude.bundle
	$(PY) scripts/jobs.py sync

status:  ## Show which job steps are done
	$(PY) scripts/jobs.py status

smoke:  ## Two-minute check that the build, the tests and the job runner work end to end
	$(PY) scripts/jobs.py run smoke $(JOB_FLAGS)

rng-check:  ## Phase 1 gate: the Rust RNG against LuaJIT and the game's own lua51.dll, 2.4M cases each (a few minutes)
	$(PY) scripts/jobs.py run rng-check $(JOB_FLAGS)

bench:  ## Phase 1 gate: engine speed on all threads, in Rust and through Python (about two minutes)
	$(PY) scripts/jobs.py run bench $(JOB_FLAGS)

PAUSE ?= 0.8
SPEED ?= 1
watch:  ## Replay runs move by move in your own Balatro, through BalatroBot (setup: RUNNING.md)
	$(PY) scripts/watch.py --pause $(PAUSE) --speed $(SPEED)

watch-dry:  ## The same runs in the engine only, printed here (no game needed)
	$(PY) scripts/watch.py --dry --runs 3

watch-mod:  ## Install the in-game panel mod into your Balatro Mods folder
	$(PY) scripts/watch.py --install-mod

rng-game:  ## (dev) Only the game-runtime part of rng-check, outside the job runner
	$(PY) scripts/rng_game.py --scale 200

rng-fixtures:  ## (dev) Regenerate the committed LuaJIT reference fixture
	$(LUAJIT) reference/rng_reference.lua > tests/fixtures/rng/reference.tsv

rng-stress:  ## (dev) Compare the RNG with LuaJIT on STRESS x the committed cases
	$(LUAJIT) reference/rng_reference.lua $(STRESS) > /tmp/bm_rng_stress.tsv
	BM_RNG_FIXTURE=/tmp/bm_rng_stress.tsv $(CARGO) test --release -p bm-rng --test luajit_fixtures -- --nocapture
	@rm -f /tmp/bm_rng_stress.tsv

fmt:  ## (dev) Format the Rust code
	$(CARGO) fmt --all

lint:  ## (dev) Check formatting and clippy
	$(CARGO) fmt --all -- --check
	$(CARGO) clippy --all-targets -- -D warnings
