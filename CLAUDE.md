# Balatro Meta: notes for Claude

Find the strategies that win Balatro most often. The plan is the source of
truth: https://claude.ai/artifact/GiEZz9CFZnAtdoEPBLkSFA

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
  carried over. The repo stays private.

## Layout

- `crates/bm-rng`: the game's RNG, bit for bit (seed hash, named streams,
  LuaJIT's generator, shuffles and picks).
- `crates/bm-core`: the engine. `state.rs`/`run.rs` (the run and its flow),
  `scoring.rs` (the evaluation and the joker routine's scoring contexts),
  `joker_events.rs` (blind start, discards, sales, shop and pack events),
  `blinds.rs`, `tags.rs`, `joker_data.rs` (all 150 jokers' numbers),
  `jokers.rs`, `content.rs` (consumables, vouchers, boosters), `create.rs`
  (card creation, pools, prices), `shop.rs`, `packs.rs`, `consumables.rs`,
  `observe.rs` (the fair-play wall: the only place the bot's input is
  built), `env.rs` (moves as kind + argument), `bots.rs` (greedy bot,
  projections), `teacher.rs` (the teacher bot, the network's warm start:
  every decision by value), `value.rs` (the value of a position: scoring
  power measured by playing the coming rounds out on fair copies, against
  the blinds ahead; growth; money as scoring power for later shops),
  `planner.rs` (the teacher's rounds: which hand to build, tried on fair
  copies; digging; taking value from a round it can already win),
  `scenario.rs` (building test positions).
- `crates/bm-py`: Python bindings (`VecEnv`, `Game`); `python/balatro_meta`.
- Jobs (`scripts/jobs.py`, like monopoly-strategy's) leave live files in
  `.sync/watch/<job>/`: `progress.json` and `samples/` (runs to replay:
  `best-*.json`, the job's 30 best so far, and `typical-*.json`).
  `make baseline` and `make teacher` are
  `crates/bm-core/examples/baseline.rs` (`--bot greedy|teacher` on all
  threads). `make watch` is `scripts/viewer.py` (replays samples
  with the engine, best first; serves `viewer/index.html`, which draws the
  table itself) plus `scripts/checker.py` (the real-game checks: a hidden
  BalatroBot game on port 12350, compared after every move using
  `scripts/watch.py`'s BalatroBot link and comparison). The companion mod
  `mods/balatro_meta_watch` (idle signal, run-setup deck for `start`,
  optional lock and speed) keeps its files in
  `balatro_meta/<BalatroBot port>/` in the game's save folder. The page's
  art comes from the user's own Balatro.exe at runtime (`scripts/art.py`
  parses the game's definitions for sheet cells then; served from memory to
  loopback clients only). No art, positions or text from the game are ever
  written to the repo or to disk.
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
background of `make watch`; no rule difference found so far. Open questions are
the UNSURE sections of the notes. Next: `make train` (a neural network, PPO,
on the GPU), shown by `make watch` like the baseline.

## Commands

- `make test`: all tests. `make rng-fixtures`: regenerate the LuaJIT fixture.
  `make rng-stress`: a much larger LuaJIT comparison. `make lint`.

## Workflow

Claude writes code in the cloud; the user's PC (WSL2 Ubuntu, 16
threads, RTX 4070 Ti 12 GB) runs heavy jobs. The cloud cannot push to GitHub:
Claude leaves `.sync/claude.bundle`, the user runs `make sync` and pushes.
