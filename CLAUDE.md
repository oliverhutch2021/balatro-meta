# Balatro Meta: notes for Claude

Find the strategies that win Balatro most often. The plan (a document kept
outside the repo) is the source of truth; RUNNING.md records each step and
what it found.

## Non-negotiables

- **Fidelity.** The engine plays Balatro 1.0.1o exactly: the same seed gives the
  same deck order, shops, packs, chance rolls and scores as the real game.
  Every rule detail is checked against the real game (BalatroBot) before a
  study relies on it.
- **Fair play.** The bot sees only what the screen shows. It never gets the
  seed, the draw order, any random stream or future shops. Lookahead runs on
  copies whose hidden parts are reshuffled. Training uses fresh seeds; tests
  use seeds the bot never played.
- **Every move a player has**, at the moments the game allows it: reordering
  hand cards and jokers, using consumables, selling, mid-blind included.
- **Ownership.** Balatro belongs to LocalThunk (published by Playstack). We may
  read the game's scripts on the user's PC to confirm rules and random-stream
  names. Nothing from them goes into this repo: no code, comments, text, art
  or sound. Rules notes in `rules/` are in our own words; the engine is our own
  Rust. Only facts that exact seeds depend on (stream names, constants) are
  carried over.
- **Public repo.** The repo can be public: it is an unofficial fan project
  (README), holds only our own work, and names no person, account or
  computer: no real names, user names, e-mail addresses, home folders or
  host names in code, notes, results or commit messages. Reports show the
  home folder as `~` and the machine as `PC` (`jobs.public`, `jobs.HOST`).
  The user's own recorded games (`.sync/me/`) and the models stay off it.

## Layout

- `crates/bm-rng`: the game's RNG, bit for bit (seed hash, named streams,
  LuaJIT's generator, shuffles and picks).
- `crates/bm-core`: the engine. `state.rs`/`run.rs` (the run and its flow; a
  run is copied by the million in searches, so `Run::clone_from` reuses a kept
  copy's memory, as `bots::preview` does, and the streams share their names),
  `scoring.rs` (the evaluation and the joker routine's scoring contexts),
  `joker_events.rs` (blind start, discards, sales, shop and pack events),
  `blinds.rs`, `tags.rs`, `joker_data.rs` (all 150 jokers' numbers),
  `jokers.rs`, `content.rs` (consumables, vouchers, boosters), `create.rs`
  (card creation, pools, prices), `shop.rs`, `packs.rs`, `consumables.rs`,
  `observe.rs` (the fair-play wall: the only place the bot's input is
  built), `env.rs` (moves as kind + argument), `bots.rs` (greedy bot,
  projections), `teacher.rs` (the teacher bot, the network's warm start:
  every decision by value; each hand's joker order measured; what it
  remembers on a thread is forgotten at each run's start, `new_run`, so a
  run plays the same on any thread), `value.rs`
  (the value of a position: scoring power measured by playing the coming
  rounds out on fair copies, against the blinds ahead; the deck's power from
  many dealt hands, no hand type favoured; growth; money as scoring power for
  the shops still to come; with a learned value loaded, plus the log of
  its chance of winning; `Scales`: how many deals and sample contents back
  each decision, set by the teacher's options "steady", "looks", "light",
  "steady-rounds", "precise"; `PARAMS`: the formula's numbers an option
  can change, `name=value`, as `make teacher-tune` tries them),
  `learned.rs` (the learned value: a network read
  from the screen alone, `observe::encode_value`, trained by
  `scripts/train_value.py` on positions `baseline --record` keeps, each
  labelled with how its run ended; each chunk also keeps every run's moves,
  so `baseline --replay DIR --record NEW` encodes a recording's positions
  afresh in seconds, checking every run ends as recorded: a new value input
  needs no new recording, from round 11 on; input layouts,
  `observe::VALUE_LAYOUTS`, only ever add floats at the end, so a network
  of an older layout loads and reads the start of today's input, and
  `train_value.py --input-layout` reads a newer recording as an older
  layout; layout 3 adds the stake as its rules in force; teacher option
  `reach=K` rates positions by the networks' ending-stage head, the chance
  of getting K blinds further, for stakes where wins are too rare;
  `doubt=k`/`hope=k` move an ensemble's chance down/up by k standard
  deviations of its members' (their disagreement); `baseline --endless`
  plays each win on to ante 12 and labels its positions with the blinds
  beaten after it ("endless", 0-12; positions after the win are never
  recorded), which `train_value.py` learns as a depth head (13 outputs after
  the 25 stage logits, from those wins alone) and teacher option `margin=K`
  reads: the chance of winning with K blinds to spare (`make
  teacher-margin`); the learned term is off past ante 8;
  `late-x`/`late-from`: every count behind a decision scaled from that ante,
  `make teacher-late`; forks, `forks.rs`: a decision forked, the teacher's
  options each played out to the end on fair copies that share their seeds,
  `baseline --forks DIR`, what each option left and how its play-outs ended,
  which `train_value.py --forks` learns pairwise (the options' order) and
  reports as the judge test, `measured.forks`: the network, the champion's
  networks, the quick look and the teacher's own choice against the option
  that went best; `make forks-quick`, `make forks-night`; `baseline
  --diagnose` and `scripts/diagnose.py`, `make diagnose`: where the runs are
  lost and why, a share of the lost blinds played again with deeper search, a
  diagnosis, never play or training;
  `StakeMode`: `baseline --stake-net` corrects the value networks' log-odds,
  a residual pair trained with `train_value.py --offset` on recordings that
  keep their log-odds, `baseline --offsets`/`--offsets-into`, or a stake
  delta; `make boost-night`, `jobs.boost_round`, records each stake with its
  best teacher so far, `jobs.best_at_stake`), `explain.rs` (the teacher's plan in
  words, for the viewer),
  `planner.rs` (the teacher's rounds: which hand to build, tried on fair
  copies; digging; taking value from a round it can already win),
  `scenario.rs` (building test positions), `tally.rs` (what a run did and how it ended, counted as
  `examples/baseline.rs` counts it; `env::Env` hands each finished run's `RunEnd` to `VecEnv.take_endings`, so
  `scripts/train_policy.py`'s `RunBook` shows the network's runs like the teacher's: all, the latest 1,000 and
  block by block).
- `crates/bm-py`: Python bindings (`VecEnv`, `Game`); `python/balatro_meta`.
- Jobs (`scripts/jobs.py`, like monopoly-strategy's) leave live files in
  `.sync/watch/<job>/`: `progress.json` and `samples/` (runs to replay:
  `best-*.json`, the job's 30 best so far, and `typical-*.json`). Each step
  that measures a bot appends a line to `results/progress.jsonl` (the
  viewer's Progress panel; only the PC writes it after the first five).
  Reports name their seed base, value network and teacher options, and keep
  each run's ending in seed order (`run_stages`), so `scripts/compare_runs.py`
  sets two of them against each other run by run (`make teacher-precise`:
  `jobs.option_trial` tries an option on the best teacher there is;
  recordings use "precise" unless its trial found it no better).
  `make baseline` and `make teacher` are
  `crates/bm-core/examples/baseline.rs` (`--bot greedy|teacher` on all
  threads). `make teacher-data`, `make value-net` and `make
  teacher-learned` are the learned value's round: record (to
  `~/.cache/balatro-meta/data/`), train on the GPU (`models/`, not in git),
  play with it; `make learned-2`, then `learned-3`, `learned-4`... are the
  next rounds (expert iteration: `jobs.learned_round`, each recording with
  the best teacher so far; `--value-net a.bin,b.bin` is an ensemble, the
  mean of their chances, and each round also tries all rounds' networks
  together, `make teacher-ensemble` rounds 1 and 2). `make teacher-moves` and `make train` are the network that
  learns to play (`scripts/train_policy.py`: a transformer over the screen's
  objects, kind/slot/target/card heads with illegal moves masked, a value
  head; imitation of the teacher's recorded moves, then PPO on `VecEnv`,
  checkpoints in `models/policy/`, evaluation on the baseline job's seeds).
  `make watch` is `scripts/viewer.py` (replays samples with the engine, best
  first; serves `viewer/index.html`, which draws the table itself, the run's details under the ante: vouchers,
  the deck by card from the view's `deck_cards`, its upgrades, hand levels with `planet` keys) plus
  `scripts/checker.py` (the real-game checks: a hidden
  BalatroBot game on port 12350, compared after every move using
  `scripts/watch.py`'s BalatroBot link and comparison). The companion mod
  `mods/balatro_meta_watch` (idle signal, run-setup deck for `start`,
  optional lock and speed) keeps its files in
  `balatro_meta/<BalatroBot port>/` in the game's save folder. `make record-me`
  (`scripts/record_me.py`) records the user's own White Stake runs from a game
  in a window (BalatroBot's launcher, port 12351; the checks pause meanwhile):
  each settled state, without the game's text (`replay_me.scrub`), in
  `.sync/me/runs/` (local only). `scripts/replay_me.py` finds the engine's
  moves between those states (cards and jokers followed by the game's ids and
  lined up before each move; moves that fit equally well kept in reserve).
  `make compare-me` (`scripts/compare_me.py`, `examples/judge.rs`) replays
  them and asks the teacher, at each decision, for its own move and what each
  is worth (`teacher::judge_option`: a round's move by the chance of beating
  the blind, a shop's or pack's by the value once the teacher has played on to
  the end of the shop). The page's
  art comes from the user's own Balatro.exe at runtime (`scripts/art.py`
  parses the game's definitions for sheet cells then; served from memory to
  loopback clients only). No art, positions or text from the game are ever
  written to the repo or to disk.
- `docs/index.html`: the public project page (GitHub Pages), a standalone copy of the project page
  artifact: our own art and type only, no personal information.
- `reference/`: LuaJIT scripts that produce reference fixtures.
- `tests/fixtures/`: committed fixtures the Rust tests compare against.
- `rules/`: rules notes, one file per area, with sources.

## Engine status (phase 2)

Written to the notes: run start, blinds and bosses (all 28 effects), all 24
tags, rounds, scoring, all 150 jokers, end of round and cash-out, the shop
(rows, rerolls, vouchers, packs, prices), all 32 vouchers, all 52
consumables, card creation with its streams, and the seals that make
consumables. Per-frame joker values (Joker Stencil, Swashbuckler, Throwback,
the deck tallies) are refreshed just before each hand is scored
(`Run::refresh_frame_values`). The real-game check (phase 3) runs in the
background of `make watch` and saves each difference to `.sync/watch/mismatches/`
(60 since 29 October; `make recheck`, `scripts/recheck.py`, replays them on
today's engine: on 5 November 25 fixed, 35 open: 17 scores one chip apart, a
rounding difference to find; 2 larger scoring gaps; 9 card-debuff flags under
bosses; 3 hands dealt after an Arcana pack; 2 cash-outs a dollar or two apart;
2 rounds the engine ends and the game doesn't). The checks keep a tally of runs
and moves compared in `results/checks.json` (from 5 November). Open questions are the UNSURE sections
of the notes. Now: the teacher's learned value (expert
iteration: record, train, play with it, repeat) and `make train` (the network:
imitation of the teacher, then PPO on the GPU), shown by `make watch` like the
baseline.

## Commands

- `make test`: all tests. `make rng-fixtures`: regenerate the LuaJIT fixture.
  `make rng-stress`: a much larger LuaJIT comparison. `make lint`.

## Workflow

Claude writes code in the cloud; the user's PC (WSL2 Ubuntu, 16 threads, a
12 GB GPU) runs heavy jobs. The cloud cannot push to GitHub:
Claude leaves `.sync/claude.bundle`, the user runs `make sync` and pushes.
