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
.PHONY: help setup setup-gpu build test doctor sync status smoke rng-check rng-game rng-fixtures rng-stress fmt lint bench baseline teacher teacher-data value-net teacher-learned learned teacher-moves train eval-net teacher-data-2 value-net-2 teacher-learned-2 learned-2 teacher-ensemble teacher-stakes teacher-exam watch watch-mod record-me compare-me diagnose-control look pack-audit green-bench league rule1 recheck check-saved diagnose

help:  ## List the commands
	@grep -hE '^[a-z0-9%-]+:.*## ' $(MAKEFILE_LIST) | sed -E 's/:.*## /\t/; s/%/N/' | expand -t 19

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

test: build  ## Run all tests, Rust and Python (the engine module rebuilt first, so Python tests see today's engine)
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

baseline:  ## The number to beat: the greedy bot plays a million runs on all threads (about 20 minutes; make watch shows it)
	$(PY) scripts/jobs.py run baseline $(JOB_FLAGS)

teacher:  ## The teacher bot (every decision by value) plays 12,000 runs (about 50 minutes; make watch shows it)
	$(PY) scripts/jobs.py run teacher $(JOB_FLAGS)

teacher-data:  ## Learned value 1/3: the teacher plays 20,000 new runs, every position it values recorded (about 1.5 hours)
	$(PY) scripts/jobs.py run teacher-data $(JOB_FLAGS)

value-net:  ## Learned value 2/3: train the value network on the GPU from that recording (about 15 minutes)
	$(PY) scripts/jobs.py run value-net $(JOB_FLAGS)

teacher-learned:  ## Learned value 3/3: the teacher without the network and with it at three weights, 5,000 runs each (about 1.5 hours)
	$(PY) scripts/jobs.py run teacher-learned $(JOB_FLAGS)

teacher-moves:  ## The network's lessons: the teacher plays 10,000 new runs, every move recorded (about 45 minutes)
	$(PY) scripts/jobs.py run teacher-moves $(JOB_FLAGS)

train:  ## The network: copies the teacher, then PPO on the GPU for 8 hours (resumable; make watch shows it)
	$(PY) scripts/jobs.py run train $(JOB_FLAGS)

learned:  ## The three learned-value jobs in a row (about 3.5 hours)
	@$(MAKE) --no-print-directory teacher-data
	@$(MAKE) --no-print-directory value-net
	@$(MAKE) --no-print-directory teacher-learned

eval-net:  ## The trained network's best version plays the usual 5,000 seeds with its likeliest moves (about 5 minutes)
	$(PY) scripts/jobs.py run eval-net $(JOB_FLAGS)

teacher-data-2:  ## Learned value round 2, 1/3: the teacher with round 1's value plays 25,000 new runs, recorded (about 1.75 hours)
	$(PY) scripts/jobs.py run teacher-data-2 $(JOB_FLAGS)

value-net-2:  ## Learned value round 2, 2/3: train the round-2 value network on the GPU (about 15 minutes)
	$(PY) scripts/jobs.py run value-net-2 $(JOB_FLAGS)

teacher-learned-2:  ## Learned value round 2, 3/3: the teacher with the round-2 value at two weights, 5,000 runs each (about 45 minutes)
	$(PY) scripts/jobs.py run teacher-learned-2 $(JOB_FLAGS)

learned-2:  ## Round 2 of the learned value: the three jobs in a row (about 2.75 hours)
	@$(MAKE) --no-print-directory teacher-data-2
	@$(MAKE) --no-print-directory value-net-2
	@$(MAKE) --no-print-directory teacher-learned-2

teacher-data-%:  ## Learned value round N (3 and later), 1/3: the best teacher, precise, records 25,000 new runs (about 1.75 hours)
	$(PY) scripts/jobs.py run teacher-data-$* $(JOB_FLAGS)

value-net-%:  ## Learned value round N, 2/3: the round's value network, on its recording and the round before's (GPU; minutes)
	$(PY) scripts/jobs.py run value-net-$* $(JOB_FLAGS)

teacher-learned-%:  ## Learned value round N, 3/3: the teacher with the round's value, and all rounds' together at two weights (about 1 hour)
	$(PY) scripts/jobs.py run teacher-learned-$* $(JOB_FLAGS)

learned-%:  ## Round N of the learned value, the three jobs in a row: make learned-3, then learned-4... (round 11 on: recorded by the champion in 5 parts, about 19 hours, then 2 trainings and 3 screens, about 5.5)
	@$(MAKE) --no-print-directory teacher-data-$*
	@$(MAKE) --no-print-directory value-net-$*
	@$(MAKE) --no-print-directory teacher-learned-$*

teacher-ensemble:  ## The teacher with rounds 1 and 2's networks together, at weights 1 and 1.5, on the usual 5,000 seeds (about 40 minutes)
	$(PY) scripts/jobs.py run teacher-ensemble $(JOB_FLAGS)

diagnose-control:  ## Every lost blind of the newest recording played again 8 times at the usual samples and 8 at 4x: what deeper search adds over luck (minutes)
	$(PY) scripts/jobs.py run diagnose-control $(JOB_FLAGS)

look:  ## The gate before a long run: 400 Green and 400 White runs of the bot as it now plays, tallied, the bars printed pass or fail, the runs kept for the viewer (about 30 minutes)
	$(PY) scripts/jobs.py run look $(JOB_FLAGS)

pack-audit:  ## What the bot thinks each pack is worth where it could open one, in its own dollars (300 Green runs with the shop audit; about 20 minutes)
	$(PY) scripts/jobs.py run pack-audit $(JOB_FLAGS)

green-bench:  ## The build bench's validation: the base on 2,000 Green seeds with --bench (each build scored as each ante from 5 began; the final builds), read by bench_check.py (about 1.6 hours)
	$(PY) scripts/jobs.py run green-bench $(JOB_FLAGS)

rule1:  ## Rule 1 (lab #34): the -d5 base, a8e3 and 4579 replay generation 2's block A and W the wall's first 200 seeds; every run must end where it did (about 45 minutes)
	$(PY) scripts/rule1.py

league:  ## One league generation (lab #33): 40 strategies and the base on 200 fresh Green seeds, cut by bound, the best 12 to 1,000 seeds, the best 4 to 4,000; resumable (GEN=0, 1, ...; STOP=1 stops after block B)
	$(PY) scripts/league.py --gen $(or $(GEN),0) $(if $(STOP),--stop-after $(STOP))

check-saved:  ## Saved real-game differences played again through today's checks in the real game (FILES="a.json b.json"; seconds each)
	$(PY) scripts/check_saved.py $(FILES)

recheck:  ## Every difference the real-game checks saved, replayed on today's engine: fixed or still open (seconds)
	$(PY) scripts/recheck.py --out results/checks-recheck.json

diagnose:  ## Where the champion's runs are lost and why: the newest recording replayed, a quarter of the lost blinds played again with deeper search (about 15 minutes)
	$(PY) scripts/jobs.py run diagnose $(JOB_FLAGS)

teacher-stakes:  ## The best teacher at every stake, White to Gold, 2,000 seeds each (about 1.5 hours)
	$(PY) scripts/jobs.py run teacher-stakes $(JOB_FLAGS)

teacher-exam:  ## The best teacher there is on the exam's 8,000 seeds, used for nothing else: the headline number (about 4.5 hours at the champion's budget)
	$(PY) scripts/jobs.py run teacher-exam $(JOB_FLAGS)

SPEED ?= 1
PORT ?= 8790
CHECKS ?= 1
watch: build  ## Watch the running job live in your browser: its numbers and one run up close (setup: RUNNING.md)
	$(PY) scripts/watch.py --speed $(SPEED) --port $(PORT) $(if $(filter 0,$(CHECKS)),--no-checks,)

watch-mod:  ## (Re)install the companion mod into your Balatro Mods folder (make watch does it too)
	$(PY) scripts/watch.py --install-mod

record-me: build  ## Record your own Red Deck runs as you play them (STAKE=green: that stake, White by default; SPEED=fast: 10x; SEEDS=a,b: these seeds first)
	$(PY) scripts/record_me.py --speed $(if $(filter command line environment,$(origin SPEED)),$(SPEED),4) $(if $(SEEDS),--seeds $(SEEDS)) $(if $(STAKE),--stake $(STAKE))

compare-me: build  ## Set each decision of your recorded runs against the best teacher's (report in .sync/me/)
	$(PY) scripts/compare_me.py

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
