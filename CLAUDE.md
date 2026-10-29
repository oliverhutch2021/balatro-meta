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
  `scoring.rs` (the evaluation and the joker routine), `blinds.rs`,
  `tags.rs`, `joker_data.rs` (all 150 jokers' numbers), `jokers.rs`,
  `observe.rs` (the fair-play wall: the only place the bot's input is
  built), `env.rs` (moves as kind + argument), `bots.rs`, `scenario.rs`
  (building test positions).
- `crates/bm-py`: Python bindings (`VecEnv`, `Game`); `python/balatro_meta`.
- `scripts/watch.py` + `mods/balatro_meta_watch`: `make watch` replays runs
  in the user's own Balatro through BalatroBot and checks every move.
- `reference/`: LuaJIT scripts that produce reference fixtures.
- `tests/fixtures/`: committed fixtures the Rust tests compare against.
- `rules/`: rules notes, one file per area, with sources.

## Engine status (phase 1)

Exact so far: run start, blinds and bosses (all 28 effects), skip tags that
need no shop (Economy, Skip, Handy, Garbage, Orbital, Juggle, Investment,
Double, Boss), rounds, scoring, about 90 jokers (`jokers::is_implemented`),
end of round and cash-out. Not yet (phase 2): the shop, packs, vouchers,
consumables, Purple/Blue seal creations, the other jokers, Top-up and the
shop tags. Pack tags are consumed as if their pack were skipped.

## Commands

- `make test`: all tests. `make rng-fixtures`: regenerate the LuaJIT fixture.
  `make rng-stress`: a much larger LuaJIT comparison. `make lint`.

## Workflow

Claude writes code in the cloud; the user's PC (WSL2 Ubuntu, 16
threads, RTX 4070 Ti 12 GB) runs heavy jobs. The cloud cannot push to GitHub:
Claude leaves `.sync/claude.bundle`, the user runs `make sync` and pushes.
