# Balatro Meta

What's the best way to play [Balatro](https://www.playbalatro.com/)? A computer plays it hundreds of thousands
of times on a faithful copy of the game, sees only what a player sees, and learns from its own runs. The goal is
a bot that wins 90% of runs at White Stake and finds strategies its human teacher doesn't know.

> **Unofficial fan project.** Balatro is made by LocalThunk and published by Playstack. This project is not
> affiliated with, endorsed by or connected to either of them, and the repo holds none of the game's code, text,
> art or sound. To play, buy the game.

![Runs won at White Stake by each version of the bot, from 0.07% on 30 October to 71.8% on 5 November](docs/climb.svg)

## The idea

1. **A faithful game.** Our own Rust engine plays Balatro 1.0.1o rule for rule: all 150 jokers, 52 consumables,
   32 vouchers, 24 tags and 28 boss blinds. The same seed deals the same cards, shops and packs as the real game,
   checked against the real game move by move.
2. **A fair player.** The bot sees only what the screen shows: never the seed, the draw order or the next shop.
   When it looks ahead it plays on copies whose hidden parts are reshuffled, the way a player imagines what might
   come. Nothing it learns comes from replaying a seed it has already seen.
3. **Deciding by value, learning from play.** Every move, from a purchase to the cards in a hand, is tried on
   copies and judged by one number: the chance of beating the blinds ahead, the run's scoring power later, money as
   buying power, plus a network's estimate of the chance of winning, trained on the bot's own runs.

## What it found

1. **Search at the decisive moments is worth more than a better judge.** Three times the usual samples from
   ante 5 on took the bot from 66.4% to 70.7% on the same 8,000 seeds (p = 2 × 10⁻²⁰); four times the samples in
   boss blinds added another point, to 71.8%. The price is a bot about four times slower per run.
2. **Its losses are near misses.** Of 1,160 lost runs, 44% scored 80% or more of the target they fell to and
   another 43% between half and 80%; only 13% were hopeless. 82% die at a boss, 70% at antes 5 to 8, and the ante 8
   finisher Violet Vessel (six times the base target) alone takes 12%.
3. **A third of lost blinds were winnable with the cards it had.** Played again from the blind's start with four
   times the samples, the bot beat 36% of the blinds it had lost (49% of the near misses). By boss: Violet Vessel
   10% (a build problem), The Mouth 51% and Amber Acorn 44% (play problems).
4. **The build gap compounds from ante 4.** Runs that go on to lose at antes 5 to 8 trail the winners' biggest hand
   by 11% at ante 4, 19% at 5, 24% at 6 and 34% at 7, and are poorer at every ante ($16 against $20 at ante 4). It
   isn't one bad decision; it is a weaker economy feeding a weaker engine from ante 3 on.

   ![The biggest hand each ante, winners against late losers, with money below](docs/build-gap.svg)

5. **The shop choices it weighs are close calls.** 6,750 forked decisions (each of the bot's best options played
   out to the end of the run on shared fair copies) found its choice, its networks and picking at random all within
   a point of win rate of each other; a network trained on those forks played worse. The gains were not in choosing
   between its top options.
6. **Learning from outcomes alone plateaued.** Ten rounds of recording 25,000 runs and training a network on
   how each position's run ended gained 1.75 points at round 4, 0.4 at round 5 and little after round 6, with or
   without exploration. More say for the networks helped up to a point (66.1% at weight 4) and collapsed past it
   (53.8% at weight 16): picking the best of many options with a judge that errs picks its errors.
7. **A human player's rules mostly hurt as rules.** Of eight lessons from a player who wins three runs in four,
   only "focus on one hand" held up (+1.6 points on 8,000 seeds). Extra points for x mult jokers or a thin deck
   made the bot worse (−1 to −2.75): they skew everything else it weighs.

## Current performance

| Table | Bot | Seeds | Won |
| --- | --- | --- | --- |
| White Stake, Red Deck | champion of 5 November (3× samples from ante 5, 4× in boss blinds) | 8,000 usual seeds | **71.8%** |
| White Stake, Red Deck | champion of 3 November, the exam | 8,000 seeds kept apart, never used to choose | 66.5% |
| Red Stake | champion of 3 November | 2,000 | 52.0% |
| Green · Black · Blue | champion of 3 November | 2,000 each | 19.7% · 11.4% · 8.8% |
| Purple · Orange · Gold | champion of 3 November | 2,000 each | 1.9% · 0.9% · 0.2% |

The usual seeds are where ideas are compared, so they flatter the bot a little; the exam's seeds only ever measure
the best bot there is. The current champion has not sat the exam yet. Every measured version, with its code,
options and seeds, is in `results/progress.jsonl`.

## How we know it's right

- The engine is checked against the real game through [BalatroBot](https://github.com/coder/balatrobot): a hidden
  copy of the game plays the same seeds move by move and every number is compared. No rule difference has been
  found.
- Two versions of the bot are always compared on the same seeds, run by run, with a paired test; a version's own
  win rate on 8,000 seeds is known to about ±1 point.
- Training uses fresh seeds; tests use seeds the bot never played; the exam's seeds choose nothing.
- The fair-play wall is one module (`observe.rs`) and is fuzzed: a fair copy and the real run give the bot the
  same screen.

## How it does it

- **Engine** (`crates/bm-core`, `crates/bm-rng`): the game's random streams bit for bit, scoring, jokers, blinds,
  shop, packs and consumables, each with a note in `rules/` in our own words.
- **The teacher bot** (`teacher.rs`, `value.rs`, `planner.rs`): every decision by the value of the position it
  leads to, measured by playing the coming rounds out on fair copies; a planner that chooses which hand to build in
  each round; a search budget that grows where runs are decided.
- **The learned value** (`learned.rs`, `scripts/train_value.py`): an ensemble of networks that read the screen and
  give the chance of winning, trained by expert iteration on the bot's recordings.
- **Forks and diagnosis** (`forks.rs`, `baseline --diagnose`): decisions played out to the end on shared fair
  copies, and lost blinds played again with deeper search, to find where the win rate is hiding before tuning.
- **Jobs and results** (`scripts/jobs.py`, `results/`): every measurement reproducible from its report; a viewer
  (`make watch`) that replays the bot's runs and charts the climb.

## Read more

- [The project page](docs/index.html) (GitHub Pages serves it from `main`, folder `/docs`): the climb, one of the
  bot's best runs, how it plays and learns, where it loses, what's next.
- [RUNNING.md](RUNNING.md): every job, in order, with what it found.
- [`rules/`](rules/): the game's rules as the engine plays them, with sources.
- [`results/`](results/): every measured bot.

```
bash bootstrap.sh      # Linux or WSL2 with Rust and Python 3.12: installs and builds everything
make test              # the engine's tests, the fixtures from the game's own random streams, the jobs
make baseline          # the bot on fresh seeds, reporting as it goes
```

---

Balatro Meta is an independent, non-commercial fan project, not affiliated with LocalThunk or Playstack. Our own
code and notes are under the MIT licence ([LICENSE](LICENSE)), which covers nothing of Balatro's. The results are a
strong guide to good play at White Stake, not proof of best play: a bot that wins 72% still loses more than one run
in four.
