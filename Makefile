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
.PHONY: help setup setup-gpu build test doctor sync status smoke rng-check rng-game rng-fixtures rng-stress fmt lint bench baseline teacher teacher-data value-net teacher-learned learned teacher-moves train eval-net teacher-data-2 value-net-2 teacher-learned-2 learned-2 teacher-ensemble teacher-precise teacher-tune teacher-tune-2 teacher-ideas teacher-next teacher-ideas-2 teacher-focus teacher-control teacher-big teacher-weights teacher-weights-high teacher-inputs teacher-stakes stakes-trials stakes-night boost-quick boost-night teacher-exam watch watch-mod record-me compare-me teacher-doubt teacher-margin teacher-late teacher-boss teacher-budget teacher-budget-big teacher-budget-confirm teacher-skips skips-stakes teacher-budget-6 teacher-budget-confirm-6 recheck diagnose forks-quick forks-night forks-table forks-now

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

learned-%:  ## Round N of the learned value, the three jobs in a row: make learned-3, then learned-4... (about 3 hours)
	@$(MAKE) --no-print-directory teacher-data-$*
	@$(MAKE) --no-print-directory value-net-$*
	@$(MAKE) --no-print-directory teacher-learned-$*

teacher-ensemble:  ## The teacher with rounds 1 and 2's networks together, at weights 1 and 1.5, on the usual 5,000 seeds (about 40 minutes)
	$(PY) scripts/jobs.py run teacher-ensemble $(JOB_FLAGS)

teacher-precise:  ## The best teacher with more samples where its decisions are noisiest, against its own runs (about 10 minutes)
	$(PY) scripts/jobs.py run teacher-precise $(JOB_FLAGS)

teacher-tune:  ## The value formula's numbers changed one at a time on the best teacher, each against its own runs (about 1 hour)
	$(PY) scripts/jobs.py run teacher-tune $(JOB_FLAGS)

teacher-tune-2:  ## The tuning's winners together, and pushed further, against the same teacher's own runs (about 50 minutes)
	$(PY) scripts/jobs.py run teacher-tune-2 $(JOB_FLAGS)

teacher-ideas:  ## What your recorded runs showed the bot missing (Dusk, Five of a Kind, Overstock, Midas), each against the same runs (about 50 minutes)
	$(PY) scripts/jobs.py run teacher-ideas $(JOB_FLAGS)

teacher-next:  ## On the tuned formula: the quick look at twice its samples, your ideas together, both, and round 4's networks (about 1 hour)
	$(PY) scripts/jobs.py run teacher-next $(JOB_FLAGS)

teacher-ideas-2:  ## Your second batch: deck fixing, Glass, x mult that fits the hand, Card Sharp's x3, and all together (about 70 minutes)
	$(PY) scripts/jobs.py run teacher-ideas-2 $(JOB_FLAGS)

teacher-focus:  ## Focus on one hand: the most played hand's Planets always taken; pack picks judged against skipping; both (about 40 minutes)
	$(PY) scripts/jobs.py run teacher-focus $(JOB_FLAGS)

teacher-control:  ## The precise teacher on the newest round's networks, 2,000 seeds, against the control before it (about 15 minutes)
	$(PY) scripts/jobs.py run teacher-control $(JOB_FLAGS)

teacher-big:  ## focus + fair-skip on 8,000 seeds against the same teacher without them, to see a 1-point gain (about 1.5 hours)
	$(PY) scripts/jobs.py run teacher-big $(JOB_FLAGS)

teacher-weights:  ## The newest networks at weights 1.5, 2.5 and 4 on 8,000 seeds, each against 1.5 (about 3 hours)
	$(PY) scripts/jobs.py run teacher-weights $(JOB_FLAGS)

teacher-weights-high:  ## The newest networks at weights 8 and 16 on 8,000 seeds, each against 4 and 1.5 (about 1.75 hours)
	$(PY) scripts/jobs.py run teacher-weights-high $(JOB_FLAGS)

teacher-doubt:  ## The champion again, then with its networks' disagreement as the brake (doubt=k) at weights 2.5 and 4, 8,000 seeds each (about 3 hours)
	$(PY) scripts/jobs.py run teacher-doubt $(JOB_FLAGS)

teacher-margin:  ## After round 11: its networks at the champion's weight, aiming for wins with 3 or 6 blinds to spare (margin=K), 8,000 seeds each (about 2.5 hours)
	$(PY) scripts/jobs.py run teacher-margin $(JOB_FLAGS)

teacher-late:  ## The champion with three times the samples from ante 6 (late-x=3), and from ante 5, 8,000 seeds each against the champion (about 5 hours)
	$(PY) scripts/jobs.py run teacher-late $(JOB_FLAGS)

teacher-boss:  ## The champion with four times the samples in boss blinds (boss-x=4), 8,000 seeds against the champion (about 1 hour)
	$(PY) scripts/jobs.py run teacher-boss $(JOB_FLAGS)

teacher-budget:  ## How far the search budget goes: late-x=2, from ante 3, late-x=6, everywhere, screened on 2,000 seeds against the champion (about 3.5 hours)
	$(PY) scripts/jobs.py run teacher-budget $(JOB_FLAGS)

teacher-budget-big:  ## The same four on 8,000 seeds (the confirmation; about 14 hours)
	$(PY) scripts/jobs.py run teacher-budget-big $(JOB_FLAGS)

teacher-budget-confirm:  ## The screen's winner (three times the samples everywhere) on 8,000 seeds against the champion: what makes it the champion (about 5.5 hours)
	$(PY) scripts/jobs.py run teacher-budget-confirm $(JOB_FLAGS)

teacher-skips:  ## Skipping a blind for its tag, judged by value (option "skips"), screened on 2,000 seeds against the champion (about 1.75 hours)
	$(PY) scripts/jobs.py run teacher-skips $(JOB_FLAGS)

skips-stakes:  ## Skipping blinds for tags at Red, Green and Purple Stake, where a skip should be worth more: each stake's best teacher with "skips" against itself, 2,000 seeds a stake (about 45 minutes)
	$(PY) scripts/jobs.py run skips-stakes $(JOB_FLAGS)

teacher-budget-6:  ## Six times the samples everywhere, screened on 2,000 seeds against the champion (about 3.5 hours)
	$(PY) scripts/jobs.py run teacher-budget-6 $(JOB_FLAGS)

teacher-budget-confirm-6:  ## Six times the samples everywhere on 8,000 seeds against the champion: what would make it the champion (about 16 hours)
	$(PY) scripts/jobs.py run teacher-budget-confirm-6 $(JOB_FLAGS)

recheck:  ## Every difference the real-game checks saved, replayed on today's engine: fixed or still open (seconds)
	$(PY) scripts/recheck.py --out results/checks-recheck.json

diagnose:  ## Where the champion's runs are lost and why: the newest recording replayed, a quarter of the lost blinds played again with deeper search (about 15 minutes)
	$(PY) scripts/jobs.py run diagnose $(JOB_FLAGS)

forks-quick:  ## An hour of forks (the champion's options at recorded decisions, each played out), then the networks that learn from them: the judge test today (about 1.5 hours)
	$(PY) scripts/jobs.py run forks-quick $(JOB_FLAGS)
	$(PY) scripts/jobs.py run value-net-forks-quick $(JOB_FLAGS)

forks-now:  ## While the forks come in: the pair at three pair weights and its control on every fork so far, the judge test of each, one table (about 20 minutes on the GPU)
	$(PY) scripts/jobs.py run forks-now --force $(if $(filter 0,$(WATCH)),--no-watch,)

forks-table:  ## The judge test from every fork file so far (seconds; runs beside a job)
	@$(PY) -c 'import subprocess, sys; sys.path.insert(0, "scripts"); import jobs; s = jobs.fork_table_steps(); sys.exit(subprocess.call([sys.executable, *s[0][1]]) if s else print("No fork file yet (make forks-quick)."))'

forks-%:  ## Forks round N, 1/3: a night of forks (9 hours)
	$(PY) scripts/jobs.py run forks-$* $(JOB_FLAGS)

value-net-forks-%:  ## Forks round N, 2/3: the pair that learns the forks, and its control (about 40 minutes)
	$(PY) scripts/jobs.py run value-net-forks-$* $(JOB_FLAGS)

teacher-forks-%:  ## Forks round N, 3/3: the pair against its control, and the champion with the pair against the champion, 8,000 seeds each (about 5 hours)
	$(PY) scripts/jobs.py run teacher-forks-$* $(JOB_FLAGS)

forks-night:  ## Forks round 1, the three jobs in a row (about 15 hours)
	@$(MAKE) --no-print-directory forks-1
	@$(MAKE) --no-print-directory value-net-forks-1
	@$(MAKE) --no-print-directory teacher-forks-1

teacher-inputs:  ## Round 11's networks on the deck card by card against the totals, on the same runs, 8,000 seeds (about 2.5 hours)
	$(PY) scripts/jobs.py run teacher-inputs $(JOB_FLAGS)

teacher-stakes:  ## The best teacher at every stake, White to Gold, 2,000 seeds each (about 1.5 hours)
	$(PY) scripts/jobs.py run teacher-stakes $(JOB_FLAGS)

stakes-trials:  ## The best teacher of 3 November without eternal, perishable or rental jokers, and with "reach=6", each at its stake (about 35 minutes)
	$(PY) scripts/jobs.py run stakes-trials $(JOB_FLAGS)

stakes-data-%:  ## Stake-aware round N, 1/3: runs at every stake recorded (round 1: .sync/night.sh's, encoded in layout 3)
	$(PY) scripts/jobs.py run stakes-data-$* $(JOB_FLAGS)

stakes-net-%:  ## Stake-aware round N, 2/3: two networks learn layout 3 from the round's runs and the round before's
	$(PY) scripts/jobs.py run stakes-net-$* $(JOB_FLAGS)

stakes-play-%:  ## Stake-aware round N, 3/3: the pair at White on 8,000 seeds, then at every stake against 3 November (about 2 hours)
	$(PY) scripts/jobs.py run stakes-play-$* $(JOB_FLAGS)

stakes-round-%:  ## Stake-aware round N, the three jobs in a row
	@$(MAKE) --no-print-directory stakes-data-$*
	@$(MAKE) --no-print-directory stakes-net-$*
	@$(MAKE) --no-print-directory stakes-play-$*

stakes-night:  ## Round 1 of the stake-aware networks, the stake trials, then rounds 2, 3... until stopped (Ctrl-C; each finished step is kept)
	@$(MAKE) --no-print-directory stakes-round-1
	@$(MAKE) --no-print-directory stakes-trials
	@n=2; while [ $$n -le 12 ] && $(MAKE) --no-print-directory stakes-round-$$n; do git push || true; n=$$((n + 1)); done

boost-quick:  ## One quick turn of the correction on the night's runs: train one network, White on 1,000 seeds, 500 at each other stake (about 25 minutes)
	$(PY) scripts/jobs.py run boost-quick $(JOB_FLAGS)

boost-data-%:  ## Boost round N, 1/3: the champion's log-odds added to its runs (round 1), or each stake recorded with its best teacher so far
	$(PY) scripts/jobs.py run boost-data-$* $(JOB_FLAGS)

boost-net-%:  ## Boost round N, 2/3: a residual pair learns what the champion's networks get wrong, at every stake
	$(PY) scripts/jobs.py run boost-net-$* $(JOB_FLAGS)

boost-play-%:  ## Boost round N, 3/3: the champion with the correction at White (8,000 seeds), then every stake against 3 November
	$(PY) scripts/jobs.py run boost-play-$* $(JOB_FLAGS)

boost-round-%:  ## Boost round N, the three jobs in a row (round 1 about 3 hours, later rounds about 4)
	@$(MAKE) --no-print-directory boost-data-$*
	@$(MAKE) --no-print-directory boost-net-$*
	@$(MAKE) --no-print-directory boost-play-$*

boost-night:  ## Boost rounds 1, 2, 3... until stopped (Ctrl-C; each finished step is kept)
	@n=1; while [ $$n -le 12 ] && $(MAKE) --no-print-directory boost-round-$$n; do git push || true; n=$$((n + 1)); done

teacher-exam:  ## The best teacher there is on the exam's 8,000 seeds, used for nothing else: the headline number (about 4.5 hours at the champion's budget)
	$(PY) scripts/jobs.py run teacher-exam $(JOB_FLAGS)

SPEED ?= 1
PORT ?= 8790
CHECKS ?= 1
watch: build  ## Watch the running job live in your browser: its numbers and one run up close (setup: RUNNING.md)
	$(PY) scripts/watch.py --speed $(SPEED) --port $(PORT) $(if $(filter 0,$(CHECKS)),--no-checks,)

watch-mod:  ## (Re)install the companion mod into your Balatro Mods folder (make watch does it too)
	$(PY) scripts/watch.py --install-mod

record-me: build  ## Record your own White Stake, Red Deck runs as you play them (SPEED=fast: 10x; SEEDS=a,b: these seeds first)
	$(PY) scripts/record_me.py --speed $(if $(filter command line environment,$(origin SPEED)),$(SPEED),4) $(if $(SEEDS),--seeds $(SEEDS))

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
