# Balatro Meta

A search for the strategies that win [Balatro](https://www.playbalatro.com/) most often: a fast simulator of
the game in Rust, and a bot that plays it fairly and learns from its own runs.

> **Unofficial fan project.** Balatro is made by LocalThunk and published by Playstack. This project is not
> affiliated with, endorsed by or connected to either of them. The repo holds none of the game's code, text,
> art or sound: the engine is our own Rust, the rules notes in `rules/` are in our own words, and only facts
> that exact seeds depend on (random-stream names, constants) are carried over. To play, buy the game.

## What's here

- **The engine** (`crates/bm-core`, `crates/bm-rng`): Balatro 1.0.1o rule for rule. The same seed gives
  the same deck order, shops, packs, chance rolls and scores as the real game, and the rules are checked
  against the real game move by move through [BalatroBot](https://github.com/coder/balatrobot).
- **A fair bot.** It sees only what the screen shows: never the seed, the draw order or future shops.
  Lookahead runs on copies whose hidden parts are reshuffled.
- **The teacher bot** (`teacher.rs`, `value.rs`, `planner.rs`): every decision by the value of the
  position it leads to, measured by playing the coming rounds out on fair copies, plus a learned value
  network trained on its own runs (expert iteration with exploration).
- **Jobs and results** (`scripts/jobs.py`, `results/`): every measured bot, with its code, options and
  seeds, so any two can be compared run by run on the same seeds (`scripts/compare_runs.py`).
- **A viewer** (`make watch`): replays the bot's runs in the browser. Card art is read at runtime from your
  own copy of the game and is never written to disk.

## The project page

[`docs/index.html`](docs/index.html) tells the story so far: the climb, one of the bot's best runs, how it
plays and learns, and what's next. GitHub Pages serves it from `main`, folder `/docs`.

## Where it stands

White Stake, Red Deck: the teacher bot wins about 64% of runs on seeds it has never played (64.2% of 8,000).
The goal is 90%. `results/progress.jsonl` has every step on the way.

## Running it

Linux or WSL2 with Rust and Python 3.12. `bash bootstrap.sh` installs and builds everything, then
`make test`, `make smoke` and `make baseline`. [RUNNING.md](RUNNING.md) walks through every job.

## Licence

Our own code and notes are under the MIT licence ([LICENSE](LICENSE)). It covers nothing of Balatro's.
