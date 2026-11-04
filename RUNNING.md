# Running the jobs on your PC

Heavy jobs run on your PC (WSL2 Ubuntu), one command each. Claude writes and tests the code, then
says **Your turn** in the chat when a run is ready. This file holds the exact steps.

## How it works

- The repo lives in `H:\balatro-meta`; in Ubuntu that's `/mnt/h/balatro-meta`.
- New code from Claude arrives as `.sync/claude.bundle`. `make sync` applies it, and every job runs it first.
- The engine is Rust. Its build and the Python environment live on the WSL disk
  (`~/.cache/balatro-meta`, `~/.venvs/balatro-meta`), so the Windows drive only holds the code and results.
- Every job can be stopped with Ctrl+C and resumed by running the same command again: the steps already done
  are kept and the stopped step starts again, except the network's training (`make train`), which carries on
  from its last save (every 10 minutes, and when you stop it). For a short pause, Ctrl+Z pauses the job where
  it is and frees the PC; `fg` in the same window carries on (closing that window ends the job). Finish or
  resume a stopped job before starting a different one: starting a job applies Claude's newest code, and a
  job stopped on older code then has to start over (`make <job> FORCE=1`). Jobs run at low priority, so the
  PC stays usable, just slower.
- When a job finishes it commits its results, then writes `.sync/DONE-<job>.txt` and
  `.sync/from-pc.bundle`. Tell Claude in the chat; it reads both straight from the folder.
- For long runs, stop the PC from sleeping: Settings > System > Power > Screen and sleep > "Never" while
  plugged in.

## Step 1: one-time setup (about 20 minutes)

Run the commands in Ubuntu. From PowerShell, `bash bootstrap.sh` and `wsl make <target>` work too.

In Ubuntu:

```
cd /mnt/h/balatro-meta
git init -q -b main
git pull -q --ff-only .sync/claude.bundle main
bash bootstrap.sh
```

It installs make, LuaJIT, uv, Rust and Python 3.12, builds the engine, and installs PyTorch with CUDA for
the 4070 Ti (about 3 GB; `SKIP_GPU=1 bash bootstrap.sh` leaves that for later). It asks for your Ubuntu
password once and ends with a check table. If any line says FAIL, paste the table into the chat.

Result commits are made under your name. If the check table says the Git identity is missing:

```
git config user.name "Your Name"
git config user.email "you@example.com"
```

Once the empty GitHub repo exists, connect it and push:

```
git remote add origin https://github.com/<you>/balatro-meta.git
git push -u origin main
```

## Step 2: smoke test (about 2 minutes)

```
make smoke
```

Builds the engine, runs every Rust and Python test, and checks the machine. When it prints "All done",
tell Claude in the chat.

## Step 3: the RNG gate (a few minutes)

```
make rng-check
```

Compares the engine's random numbers with LuaJIT on about 2.4 million cases (seed hashes, every kind
of stream draw, the 13-place rounding and deck shuffles), twice: first with Ubuntu's LuaJIT, then on the
game's own `lua51.dll` (LuaJIT 2.0.5 with the Windows C runtime), which it runs through your Windows
Python (miniconda is found automatically). Every value must match bit for bit. It also saves a few facts
about the game's runtime to `results/rng/game_runtime.txt`. This is the RNG part of the phase 1 gate.

If it can't find Windows Python or Balatro, it says which setting to give, for example
`BM_WIN_PYTHON=/mnt/c/Users/<you>/miniconda3/python.exe make rng-check`.

## Step 4: the speed gate (about two minutes)

```
make bench
```

Plays random legal moves on all 16 threads for 20 seconds in Rust, then 20 seconds through Python with
the bot's input built after every move. The phase 1 target is 200,000 moves a second; results go to
`results/bench/`. The leak test (same screen, different seed: identical input) runs in every
`make smoke`, with the rest of the tests.

## Step 5: the baseline, and watching the search

Open two Ubuntu windows in `/mnt/h/balatro-meta`. In the first:

```
make baseline
```

The greedy bot (the simple bot the search has to beat) plays a million fresh-seed runs, Red Deck on
White Stake, on all 16 threads: about 850 runs a second, so about 20 minutes. Its numbers go to
`results/baseline/greedy.json` and are committed like every job's.

Then, the same way:

```
make teacher
```

The teacher bot is the bot that plays before the network can, and the network's warm start. It decides by
value: one number for a position (`crates/bm-core/src/value.rs`) that counts its scoring power against the
blinds ahead (measured by playing the coming rounds out on copies with the hidden cards reshuffled, using
the run's own deck, jokers, levels, enhancements and seals), the deck's power (24 hands dealt from the whole
deck and scored by the engine: how often it deals each hand, what each scores at its level, and what seals,
retriggers and jokers add to it, with no hand type favoured), what the run gains by itself (growing jokers,
levels, income, cards made; a joker that pays for one kind of hand counts by how often the run plays it;
DNA's copies, and any card added, changed or lost, by what the deck then deals) and money as scoring power
for the shops still to come (none at the last one, so it doesn't die rich; cash beyond the interest cap at
half, since it earns nothing held). A held card that pays money (Hermit, Temperance) is worth what the
engine pays for it, used now or after the next round, so it buys them when that beats the price and keeps
Hermit until the money is there; a reroll is judged as a search that can go on while the money lasts. Every
shop, pack, Tarot, skip and round decision tries its options and keeps the one that raises that number most,
each option and doing nothing judged on the same deals; nothing about what to buy or build is a fixed rule.
In a round it plans which hand to build (and digs for it), and once it holds a winning hand it first takes
what the round still offers (throwing Mail-In Rebate's rank or Purple Seals, four-card hands for Square
Joker, a lone 6 for Sixth Sense, Gold Seals played, Steel and Blue Seals kept...) as long as the win stays
certain. Before every hand it tries every order of its jokers (up to six) and plays the one that scores most
(+mult before x-mult and Polychrome jokers, copiers on the best joker for that hand); for an extra hand
played only to dig for value it takes the opposite order, so the hand scores little and the round goes on.
It moves Blueprint and Brainstorm onto whatever pays at each moment (Perkeo or Cartomancer as the shop
closes, Mail-In Rebate before a throw). The plan's "teacher's playbook" section lists every idea, where it
came from and what it did. It plays 12,000 runs (about an hour; it thinks far more per move than the greedy
bot) into `results/baseline/teacher.json`. On 400 fresh test runs (seeds used by neither version while
building) it won 26% (95% range 22 to 31%), with a mean final ante of 6.4; the version before these changes
won 21.5% on the same seeds (mean final ante 6.1), the earlier rule-based teacher about 4% and the greedy
bot 0.07% (mean final ante 3.1).

In the second:

```
make watch
```

Your browser opens the viewer, in the style of Monopoly Meta's:

- **In the middle**, one run drawn by the page with the art from your own Balatro (the viewer reads it from
  your installed game each time it starts and keeps it in memory; it is never stored or copied): the blind
  and its target, the score, your hand, the jokers, consumables, shop and packs. Under the ante and round, the
  left panel has the run in detail: the vouchers bought, every card in the deck by suit and rank (the whole
  deck at all times, as the shop shows it; a gold number is a duplicate), the deck's enhancements, seals and editions
  with their counts, and the hands levelled or played with their planets. It's a run the job has just played, replayed move by move with
  the same engine at a speed you can follow. **Best runs** (the default) shows the job's best runs so far:
  wins first, ranked by their biggest single hand, then losses by how far they got (the ante, which
  blind, how close to its target). **Typical runs** shows runs picked at random, as the job plays them.
- **Your PC and the search so far**: runs played and runs a second (live), win rate with its 95% range,
  the average final ante, where runs end, what ends them, the jokers the bot takes and the ones in its
  wins, the hands it plays, the best runs so far (click one to watch it), and the job's latest output.
  While the network trains or plays, a switch shows these for **Now** (its latest 1,000 runs, how it plays
  at this point) or **All runs**, and **Through the training** draws, block by block of 1,000 runs, its win
  rate, final ante, jokers taken, packs, rerolls, money at the end, moves, refused moves and runs cut short.
- **Progress**: every version of the bot measured on this PC, oldest to newest, against the 90% goal: its win
  rate with its 95% range, how far its runs got and what changed in it, with the running job's win rate so far
  as a live point. Each job step that measures a bot adds a line to `results/progress.jsonl` as it finishes,
  committed with the job's results.
- **Run outlook**: for the run on screen, quick playouts by the greedy bot after every move (hidden cards
  reshuffled, fresh random streams) project where it ends and how likely it is to clear the current blind,
  and the teacher's own view of the position: its chance of beating each blind ahead, the hand it scores
  most with and a round's typical total (the value model it decides by), and the teacher's plan in a few
  plain sentences made from the same model: the risk ahead, the hands its deck deals, what its jokers pay
  for, its money plan and what it is looking for. Click or drag the chart, or click a move in the play by
  play, to see the table as it was after that move. Turning points list the moves that changed the
  outlook most.
- **Engine and your game**: a hidden copy of your Balatro (no window) replays the job's runs at full
  speed through BalatroBot, and after every move compares money, hands, discards, the cards in hand,
  round chips, blinds, tags, jokers and consumables (with their sell values), the shop's cards,
  vouchers, packs and prices, and an open pack's cards with the engine's. A difference is saved to
  `.sync/watch/mismatches/` and the checks move on. BalatroBot has no "buy and use" and no boss reroll,
  so a check stops (without a difference) before either.

With no job running, the middle shows demo runs by the greedy bot. Ctrl+C stops the viewer and closes the
hidden game; a running job carries on. `make watch SPEED=4` starts the replay faster (the page can change
it any time), and `make watch CHECKS=0` leaves the real-game checks out. The viewer never touches a job: it
only reads the files the job leaves in `.sync/watch/`.

One-time setup for the checks, on Windows:

1. Install uv (PowerShell): `powershell -ExecutionPolicy ByPass -c "irm https://astral.sh/uv/install.ps1 | iex"`
2. Lovely 0.8.0 or newer (`winmm.dll` in `H:\Steam\steamapps\common\Balatro`;
   https://github.com/ethangreen-dev/lovely-injector/releases).
3. Steamodded 1.0.0-beta-1221a or newer (https://github.com/Steamodded/smods/releases) in
   `%AppData%\Balatro\Mods`.
4. BalatroBot (https://github.com/coder/balatrobot/releases) in `%AppData%\Balatro\Mods`.
5. Move every other mod out of `%AppData%\Balatro\Mods` while the checks run, so the rules stay vanilla.

`make watch` installs or updates the companion mod (`mods/balatro_meta_watch`) itself and starts the hidden
game through BalatroBot's launcher on its own port (12350), so it never touches a Balatro you have open.
Balatro opens a window while it loads (full screen and black if that's how you play); the checks hide it
as soon as it appears, so a start or restart no longer covers your screen, and the companion mod keeps its
sound off (only that game's, while the checks run; your own Balatro and settings are untouched). It can run overnight: Balatro's memory grows over a long session, so the hidden game gets a fresh start
every hour (sooner if it passes 3 GB, and at once if it crashes or stops answering), and only the newest
two of its log files are kept (it logs every call, about 100 MB an hour); a Balatro you play yourself keeps
all of its logs.
BalatroBot listens on Windows' own localhost, which Ubuntu can't reach by default, so the checks talk to it
through a small relay run with your Windows Python (the same miniconda the RNG check uses). If the panel
keeps waiting for the hidden game, look in `%AppData%\Balatro\balatro_meta\logs` (BalatroBot's log for
port 12350) and in Balatro's Lovely log (`%AppData%\Balatro\Mods\lovely\log`, newest file).

## Step 6: the learned value (about 3.5 hours)

The teacher's value is a formula: it sees the next few blinds clearly, but not what wins at ante 8. The
learned value is a network that does. It is trained on how the teacher's own runs ended, gives the chance of
winning from a position, and the teacher adds it to its formula (as the log of that chance) for every
decision between rounds. Three jobs, one after the other:

```
make teacher-data
make value-net
make teacher-learned
```

or `make learned` for all three in a row.

- `make teacher-data` (about 1.5 hours): the teacher plays 20,000 runs on fresh seeds (none of the 12,000
  `make teacher` plays), and every position it values between rounds (blind choices, shops, open packs:
  about 130 a run) is recorded as the network will read it, with how its run ended. The recording (about
  2 GB) goes to `~/.cache/balatro-meta/data/value-1` on the WSL disk. The runs' numbers go to
  `results/value/record-1.json`: a measure of today's teacher on seeds it has never played.
- `make value-net` (about 15 minutes): installs PyTorch with CUDA if it isn't there yet, then trains the
  network on the 4070 Ti. It reads a position as the screen shows it (the same rule as everything the bot
  sees) and learns the chance of winning, and how far the run gets. It tries four network sizes and keeps
  the best, judged on runs it never trained on, and measures it on others again. The network goes to
  `models/value-1.bin` (not committed; the report holds its checksum), and the report, with how well it
  predicts those runs at each ante, to `results/value/value-1.json`.
- `make teacher-learned` (about 1.5 hours): on the same 5,000 seeds (the first of `make teacher`'s), the
  teacher plays without the network, then with it at three weights (how much its chance counts against the
  formula), into `results/value/`. Each report lists how every run ended, so they compare run by run.

`make watch` shows the first and third like `make teacher`. Then tell Claude. Each round of this learns from
a better player: the next recording is made by the teacher with the network.

## Step 7: the network that learns to play

```
make teacher-moves
make train
```

- `make teacher-moves` (about 45 minutes): the teacher plays 10,000 new runs and every move it makes is
  recorded with what the screen showed, in `~/.cache/balatro-meta/data/moves-1` (about 3 GB). It plays with
  the learned value at the weight that won most often in `make teacher-learned`, so the network copies the
  strongest teacher; its report names the weight.
- `make train`: the network. It reads the screen as a set of objects (the cards in hand, the jokers in
  order, the consumables, the shop's cards, vouchers and packs, an open pack's cards) with a transformer,
  picks a kind of move the game allows, then which slot, where to move it and which cards; a second head
  estimates what the run will still earn. Its steps:
  1. **imitate** (about 15 minutes on the GPU): it copies the teacher's moves. `results/train/imitate.json`
     says how often it makes the teacher's exact move on runs it didn't learn from.
  2. **eval-imitate** (a few minutes): it plays 2,000 of the usual seeds with its likeliest moves
     (`results/train/eval-imitate.json`), to compare with the teacher on the same seeds.
  3. **ppo** (8 hours): thousands of runs at once on all 16 threads, the network on the GPU, rewarded for
     every blind beaten and for winning. It starts by having PyTorch compile the network for your GPU (a minute
     or two; if that fails it says so and trains without it), and its first update says how long it took and how
     much GPU memory it used. Its first 20 updates (about 10 minutes) train only its judgement of
     positions, so how it plays stays exactly the copy's; after that, an update stops early once it has changed
     the play as far as one update should, and the learning rate falls to a tenth over the 8 hours. It keeps its
     best network so far in `models/policy/best.pt` (judged on its last 2,000 runs); if its play collapses (final
     ante a whole ante below the best, or under half the best win rate), it goes back to that network and learns
     at half the rate, and after six collapses it stops. `make watch` shows it live: runs played, the win rate of its last
     2,000 runs, how it plays now against all its runs so far, how it changes block by block, and its runs
     to replay. Ctrl+C stops it after saving; `make train` again carries on where
     it stopped. The network is saved in `models/policy/` (not committed).
  4. **eval**: its best network plays 5,000 of the usual seeds, the same ones as the teacher's in
     `results/value/teacher-plain.json` and `teacher-w1.json`, and the Progress panel adds it to the chart.

## Step 8: round 2 of the learned value

The teacher is still the strongest player (39.3% with the round-1 value), so it gets better first. If `make train`
has been stopped early, `make eval-net` tests its best network the way `make train` would have (5,000 of the usual
seeds, its likeliest moves) and adds it to the Progress panel. Then:

```
make learned-2
```

- `make teacher-data-2` (about 1.75 hours): the teacher with the round-1 value at its best weight plays 25,000 new
  runs, and every position it values is recorded with how its run ended, in `~/.cache/balatro-meta/data/value-2`.
  These are a better player's positions, so the next value learns what wins when playing that well.
- `make value-net-2` (about 15 minutes): the round-2 value network, `models/value-2.bin`.
- `make teacher-learned-2` (about 45 minutes): the teacher with the round-2 value at weights 1 and 2 on the usual
  5,000 seeds (`results/value/teacher2-w*.json`), to set against round 1's 39.3% on the same seeds.

Whichever round and weight wins most is what later jobs use (`make teacher-moves` records that teacher).

## Step 9: rounds 1 and 2 together (about 40 minutes)

Round 2's network alone won no more than round 1's, but the two together (the chance of winning taken as the
mean of theirs) did better in the cloud: with "precise" on, 49.75% of 400 seeds against 45.0% with round 1's
alone. On your PC:

```
make sync && make teacher-ensemble
```

The teacher with both networks plays the usual 5,000 seeds at weights 1 and 1.5
(`results/value/ensemble2-w*.json`; in the cloud, 1.5 did a little better), to set against each alone (39.3% and
39.0%). If one wins most, every job after it (steps 10 to 12) uses it. From round 3 on, each round also tries
the networks of all its rounds together.

## Step 10: a more precise teacher (about 10 minutes)

The teacher judges each shop and pack decision on a handful of dealt rounds and sample contents (what a pack
might hold, what a reroll might bring), and each plan for a round on a few tries, so the same position can get
a different decision on different samples. Cloud tests, 400 runs each with round 1's value, against 38.5%:

| Samples | Won | Time |
| --- | --- | --- |
| half of everything between rounds | 35.5% | 0.4x |
| twice everything between rounds ("steady") | 43.0% | 3x |
| twice the pack and reroll samples ("looks") | 43.75% | 1.3x |
| twice the tries per plan in a round ("steady-rounds") | 42.0% | 1.1x |
| "looks" and "steady-rounds" together ("precise") | 45.0% | 1.45x |

400 runs can only just tell "precise" from luck (p = 0.03), so on your PC, once round 2 has finished:

```
make teacher-precise
```

The best teacher there is (the learned value and weight that won most) with "precise" plays the first 2,000 of
the usual seeds, and `scripts/compare_runs.py` sets it run by run against that teacher's own runs on those
seeds: `results/value/precise-vs-*.json` holds the difference with its 95% interval. The next recordings use
"precise" unless this trial finds it no better.

## Step 11: round 3 of the learned value (about 3 hours)

```
make learned-3
```

- `make teacher-data-3` (about 1.75 hours): the best teacher there is (the round and weight that won most, with
  "precise" unless step 10 found it no better) plays 25,000 new runs, recorded in
  `~/.cache/balatro-meta/data/value-3`.
- `make value-net-3` (a few minutes): two round-3 networks, `models/value-3.bin` and `value-3b.bin` (another
  seed), trained on round 3's recording and round 2's (more positions to learn from); only round 3's runs judge
  and measure them. Networks that learned apart do better together than alone.
- `make teacher-learned-3` (about 1 hour): the teacher with it at weight 1 on the usual 5,000 seeds
  (`results/value/teacher3-w1.json`), and with the networks of rounds 1 to 3 together at weights 1 and 1.5
  (`results/value/ensemble3-w*.json`), without options, to set against rounds 1 and 2 on the same footing.

Each later round is the same three jobs: `make learned-4`, then `make learned-5`, and so on (up to 9), each
recording with the best teacher there is so far. From round 4 that includes the trials of steps 12 and 13: the
recording uses the change that beat the precise teacher by the most on the same seeds, if one did clearly
(p below 0.1; `scripts/jobs.py`: `best_recording`).

## Step 12: tuning the value's formula (about 1 hour)

Round 2 of the learned value won no more than round 1, so the hand-made part of the value is the next lever:
how sharply a blind's chance turns with the margin, how much the long run counts, how money is weighed. Each is
a number in `crates/bm-core/src/value.rs` (`PARAMS`) that a teacher option can change (`long-run=0.25`). After
step 10:

```
make teacher-tune
```

The best teacher (with "precise") plays the first 2,000 of the usual seeds with each number changed in turn
(`scripts/jobs.py`: `TUNE`), each set run by run against its own runs from step 10:
`results/tune/*-vs-*.json` hold the differences. A change that wins clearly goes into the formula.

It found two winners: the planner's edge at 0.3 (52.05% against 49.4%, p = 0.02) and a softer turn of a
blind's chance, sharpness 2.5 (51.6%, p = 0.07). After `make learned-3` (step 11), with no job running:

```
make teacher-tune-2
```

tries them together (and with the long run at 0.35), and each pushed further (`TUNE_2`), the same way (about 50
minutes). What wins becomes the formula's numbers for round 4.

## Step 13: your own runs, recorded move by move (as many as you like to play)

Where a good player and the bot disagree is where the bot has the most to learn. You play White Stake, Red Deck
runs as you normally would; every move is recorded, and the engine later sets each of your decisions against
the bot's.

1. Stop `make watch` if it is running (Ctrl+C in its window): its hidden copy of Balatro shares your save
   profile with the game you'll play. The jobs can keep running.
2. In Ubuntu: `make sync`, then

   ```
   make record-me
   ```

   Balatro opens (at 4x speed; `make record-me SPEED=fast` for 10x) with BalatroBot and the companion mod, and a
   White Stake, Red Deck run starts by itself with a fresh random seed. To play given seeds first (seeds the bot
   keeps losing, say): `make record-me SEEDS=MV3B41CL,XR7SBBFC`.
3. Play. The recorder only watches: it never clicks or changes anything. When a run ends, win or lose, choose
   Main Menu (after a win: Main Menu, not Endless; beating ante 8 is the win we count) and the next run starts by
   itself a few seconds later. You can change the game speed in the game's own options as usual.
4. Ctrl+C in the Ubuntu window stops recording and closes the game. Every run is kept as you go, in
   `.sync/me/runs/` on this PC only (never in git): the states the game showed, without any of the game's text.

The game counts these runs as seeded (that is what lets the engine replay them exactly): no unlocks,
achievements or streaks come from them. While you play, the engine follows each move; a run it couldn't follow to
the end is still kept whole, and a fix replays it later. Even 5 to 10 runs help: a run is a couple of hundred
decisions.

Then, with no job running (it uses all threads for a few minutes):

```
make compare-me
```

Each of your decisions is set against the best teacher's: what it would have done, and what it thinks each choice
is worth (in a round, the chance of beating the blind after it; in a shop or pack, the value of the position once
it has played on to the end of the shop). `.sync/me/report.txt` lists how often you agree by kind of decision, the
swaps you make most, and the decisions with the largest gaps either way. Claude reads it from there, and turns the
patterns that recur into changes to try on thousands of seeds.

The first two games (two wins) showed the bot selling Dusk where you played rounds down to it, and undervaluing
Overstock and Midas Mask; the second was built on Five of a Kind (copies of Kings), a hand the bot's quick search
never played. After `make teacher-tune-2`, with no job running:

```
make teacher-ideas
```

tries each as a change to the bot ("final-hand", "five-kind", `overstock=0.4`, `midas=3`), on the same 2,000 seeds
against its own runs (about 50 minutes). Then round 4 (step 11's `make learned-4`, about 3.5 hours) records with
the change that won most clearly. All three in a row, overnight:

```
make teacher-tune-2 && make teacher-ideas && make learned-4
```

## Step 14: the tuned formula, and the next trials (about 1 hour, then round 5)

`make teacher-tune-2` found the formula's numbers that win most: the planner's edge 0.3, sharpness 2.5 and the
long run 0.35 together won 59.85% of the 2,000 seeds against 53.4% (round 4 recorded with them: 59.34% of 25,000
fresh seeds). They are now the formula's own numbers (formula 2), so the teacher plays that way without options.
Trials on it are set only against a control measured with the same numbers (`f2-` in their reports). With no job
running:

```
make teacher-next && make learned-5
```

`make teacher-next` tries, each on the same 2,000 seeds against the precise teacher: twice the samples behind its
first look at each option (`quick-x=2`), the four ideas from your runs together, two of your rules (x mult held
counts more as the run goes on, `xmult-bonus`; a clipped deck counts more, `thin`), and round 4's networks with the
tuned teacher (about 1 hour). `make learned-5` then records round 5 with the change that won most clearly, if one did. Rounds
measured from here (`teacher-learned-5`...) play with the new numbers, so they are not on the same footing as
rounds 1 to 4.

## Step 15: your second batch of ideas (about 70 minutes)

From your runs and what you said about them, each tried on the same 2,000 seeds against the tuned teacher:

- `deck-fix`: a held Tarot that changes cards is priced by what it would do to the deck used on the cards the build
  wants (copying a Lucky, Red-sealed Jack with Photograph held is worth about $40, not $6); with suit Tarots for
  Blackboard's Spades and Clubs too. With `glass`: a card's x mult (Glass x2, Polychrome x1.5) counts by the times
  it scores, retriggers included.
- `xmult-fit`: held x mult counts more by ante 6 only as far as it fires with the hands the run plays (The Duo with
  Pairs, not The Order), at 0.3 and 0.6.
- `card-sharp`: with Card Sharp, the teacher also tries playing each hand type first, so the next hand of that type
  gets the x3.
- All of them together.

After `make learned-5` (or whenever no job is running):

```
make teacher-ideas-2
```

A clear winner is what the next round records with.

## Step 16: focus on one hand (about 40 minutes)

Your runs pile the Planets on one hand (Straight to level 19 in 4C3GL8EU, Pair to 27 before); the bot's main hand
ends its wins at a median level 5, with a third of its Planets. Each tried on the same 2,000 seeds against the
precise teacher on all five rounds' networks:

- `focus`: the hand the run has played most (High Card aside) is its focus once played four times; a Celestial
  pack's Black Hole, else the focus's Planet, is always taken, and the focus's Planet is bought in the shop when it
  leaves $4 (any money from ante 5).
- `fair-skip`: a pack's picks are judged against the run once the pack is skipped, as each pick is judged once the
  pack has closed (the learned value rated a position inside a Buffoon pack above the same run after skipping it,
  so a joker had to beat that too).
- Both together.

After `make teacher-ideas-2` (or whenever no job is running):

```
make sync && make teacher-focus
```

## Step 17: the next control, then round 7 exploring (about 4 hours)

Since the formula was tuned, the bot has stayed near 60% (62.0% with round 5's networks): each learned-value round
gains less, because the network only learns from the choices the bot already makes. From round 7 the recording
explores: one shop or pack decision in ten takes another option the bot rates nearly as high, so the network also
sees where those lead. Measuring never explores. After `make learned-6`, with no job running:

```
make sync && make teacher-control && make learned-7
```

`make teacher-control` plays the precise teacher on the newest networks on the 2,000 seeds the trials use, against
the control before it (about 15 minutes); `make learned-7` is round 7 (about 3.5 hours).

With a longer night: `make teacher-control && make teacher-big` after it. The second control measures round 7's
networks; `make teacher-big` tries `focus` and `fair-skip` together (+1.05 points on 2,000 seeds, not clear) on 8,000
seeds, where a 1-point gain shows (about 1.5 hours).

## Step 18: focus and fair skipping become the bot's own play (about 5 hours)

`make teacher-big` found the first clear gain since the formula was tuned: `focus` and `fair-skip` together won
64.24% of 8,000 seeds against 62.60% (+1.64 points, p = 0.001). They are now how the bot plays (`no-focus` and
`no-fair-skip` switch them off), and trials from here are set against controls that play that way (`f3-` in their
reports). Its comparison step failed on that many seeds (a number too large for the test); Claude ran the fixed
test, so `make teacher-big` only has to commit the results. With no job running:

```
make teacher-big && make sync && make teacher-control && make learned-8 && make teacher-control
```

The first `make teacher-control` measures the new play on the 2,000 trial seeds (the new control); `make learned-8`
is round 8, exploring, and the last `make teacher-control` sets round 8's networks against round 7's.

## Step 19: how much say the networks get, then round 9 (about 7 hours)

Round 8 (the second round that explores) gave no gain where it counts: with the precise teacher, round 8's
networks won 64.0% of the 2,000 trial seeds against round 7's 65.75% (p = 0.12, within the noise). Yet on their
own they won more (47.9% against 43.3%), so stronger networks may need more say against the formula.
`make teacher-weights` plays the newest networks at weights 1.5, 2.5 and 4 on the first 8,000 usual seeds (half the
noise of 2,000), each set run by run against 1.5, and 1.5 against round 7's networks on the same seeds. A clear gain
(p below 0.1) is what round 9 records with. Overnight, with no job running:

```
make teacher-weights && make learned-9 && make teacher-control
```

## Step 20: round 9's networks at each weight, while you sleep (about 4 hours after step 19)

`make teacher-weights` found a clear gain: at weight 2.5 the networks won 66.46% of the 8,000 seeds against 63.86% at
1.5 (+2.6 points, p = 0.00001; weight 4: +2.4), so round 9 records at 2.5. Run again after round 9, the job measures
round 9's networks the same way (runs already played are only compared again). To start it on its own once step 19
has finished, in a second Ubuntu tab:

```
cd /mnt/h/balatro-meta && touch /tmp/bm-mark && while [ ! .sync/DONE-teacher-control.txt -nt /tmp/bm-mark ]; do sleep 60; done && make teacher-weights
```

## Step 21: the exam, then round 10 with whole builds (about 6 hours after step 20)

`make teacher-exam` plays the best teacher there is on 8,000 seeds kept for that alone (seed base 5003): ideas are
compared and chosen on the usual seeds, which flatters them a little, so the headline number comes from seeds that
never chose anything. `make learned-10` is the first round whose recording explores whole builds: a quarter of its
runs commit from ante 3 (the shop after ante 2's boss; before that they play as usual) to one hand type, picked at
random by the run's number (never from the game), its Planets taken and bought as the focus's, so the learned value sees how builds the teacher wouldn't choose end. To
start them on their own once step 20 has finished, in a third Ubuntu tab:

```
cd /mnt/h/balatro-meta && touch /tmp/bm-mark2 && while [ ! .sync/DONE-teacher-weights.txt -nt /tmp/bm-mark2 ]; do sleep 60; done && make teacher-exam && make learned-10 && make teacher-control
```

Ctrl+C stops it whenever you need the PC; the same line (or `make learned-10`) carries on where it stopped.

## Step 22: the exam on the fair-play fixes, then the networks nearly alone (about 2.75 hours)

The fair-play fixes (Luchador in sight only, face-down jokers and the ante's voucher in fair copies) change how the
teacher plays a little, so the headline is measured again: `make teacher-exam` plays the same teacher on the same
exam seeds, and set run by run against the last exam it shows what the fixes cost or gave. Then
`make teacher-weights-high` plays round 10's networks at weights 8 and 16 on the first 8,000 usual seeds, each set
run by run against weight 4 and 1.5. At those weights the formula's numbers barely move a decision: if the rate
holds or climbs, the strength is in the networks, and another stake is a matter of training them there.

If the exam is already running in another tab, this waits for it, takes the new code and starts the trial:

```
cd /mnt/h/balatro-meta && touch /tmp/bm-mark3 && while [ ! .sync/DONE-teacher-exam.txt -nt /tmp/bm-mark3 ]; do sleep 60; done && make sync && make teacher-weights-high
```

## Step 23: round 11, then the deck card by card (about 5.5 hours)

Round 11's recording is the first to keep every move (a run's seed and moves play it again exactly, so its
positions can be encoded again for any new input in seconds: `baseline --replay`) and the first in input layout 2.
Layout 2 adds the deck counted by whole cards (which ranks and suits carry which enhancement, seal and edition) after
layout 1's floats, which stay as they were: the networks of rounds 1 to 10 read layout 1's part and play as before,
and round 11's own networks (`make value-net-11`: rounds 10 and 11, read as layout 1, the layout they share) join
the ensemble as usual. `make teacher-inputs` then asks whether the new input helps: two networks of each layout learn
from round 11's runs alone and play at weight 2.5 on the first 8,000 usual seeds, set run by run, so a difference is
the input's.

```
cd /mnt/h/balatro-meta && make learned-11 && make teacher-inputs
```

## Step 24: every stake, White to Gold (about 1.5 hours)

`make teacher-stakes` plays the best teacher there is (the exam's) at every stake on the first 2,000 usual seeds.
Stakes add their rules one by one, so the steps from one to the next show what each costs. On 60 runs each, the
teacher without networks went from 18 wins (White) to 15 (Red), 4 (Green: blinds grow faster), 1, 0..., and from
19.4 blinds beaten a run to 8.8 at Gold: the biggest falls were Green's faster blinds (3.6 blinds), Blue's lost
discard (2.1) and Gold's rentals (2.3); eternal jokers cost nothing. From Green on most runs end at an ante's Small or
Big Blind, not its boss. The formula's own numbers turned at Green (the long run's weight and rounds, what money is
worth) changed nothing clearly: the gap needs networks that learned the stake, not another hand-tuned formula.

It plays Gold first and White last. `make watch` in another tab replays each stake's runs as they come, the stake
in the run's name, and its real-game checks start the hidden game at the run's stake: the first check of every
stake's own rules against the game (a difference it finds is a finding about the engine, saved like any other).

```
cd /mnt/h/balatro-meta && make teacher-stakes
```

## Step 25: networks that know the stake, all night (until you stop it)

The best teacher's results of 3 November at every stake (2,000 seeds each): 66.8% won at White, 52.0% Red, 19.7%
Green, 11.35% Black, 8.8% Blue, 1.85% Purple, 0.9% Orange, 0.15% Gold. Blinds beaten fell most at Green (3.2: blinds
grow faster), Gold (2.4: rentals), Purple (1.7) and Red (1.5). Its networks learned White's runs alone: they never
saw a stake, and the trainer leaves out inputs that never vary, so they never read a joker's eternal, perishable or
rental mark either (always off at White). Eternals cost it 1.15 blinds; the teacher without networks lost none.

So the networks now read the stake as its rules in force (input layout 3: one float for each stake's rule, Red's to
Gold's, so Gold is Orange with rentals and a rule learned at one stake holds above it) and learn runs at every
stake, most where a run's result is least certain (White and Red, Green, Black), where a run teaches the most.
Each round is judged at White first, on the usual 8,000 seeds against the best teacher there: harder stakes are
worth learning from only if they make the play better all round. Round 1 also trains a control pair on its White
runs alone and sets the two pairs against each other at White, run by run: the direct test of whether learning the
other stakes too helps White's play or hurts it. Then every stake on the 2,000 seeds of 3
November, run by run. `make stakes-trials` (after round 1) tries, on the teacher of 3 November, never buying
eternal (at Black), perishable (Orange) or rental (Gold) jokers: one its stake does clearly better without is one
the value overrates, and the rounds after keep that option. It also tries "reach=6" at Green and Purple: the learned
term from the networks' chance of getting 6 blinds further, not of winning (a network that knows the stake rates
nearly every Purple-to-Gold position under the 1% floor of its win chance, where that term tells the choices apart
by nothing).

`.sync/night.sh` records round 1 at once on the code you have (the best teacher, exploring; about 3 hours), every
move kept, then takes up the code that makes the rest when `.sync/night-ready` appears and runs `make stakes-night`:
round 1 (replayed into layout 3 in minutes, two networks, then White and every stake: about 2.5 hours), the trials
(35 minutes), then rounds 2, 3... (about 4.5 hours each) until you stop it. Each finished step is committed.

```
cd /mnt/h/balatro-meta && bash .sync/night.sh 2>&1 | tee -a .sync/logs/night.log
```

## Step 26: the champion, corrected where the stakes show it wrong (round 1 about 5 hours, then 4 a round)

What step 25 found (rounds 1 to 3, stopped in round 4): the stake-aware pairs lost to the champion at every stake but
Gold (level), and at White (54.2%, 58.8%, 55.9% against its 66.5% on the usual 8,000 seeds). A pair that learns
40,000 to 75,000 runs can't match fourteen networks that learned about 250,000 White runs, and from round 2 each
round recorded with the round before's pair, weaker than the champion, so the rounds went in circles. It also found:
the other stakes' runs teach White's play too (on the same White runs, the pair that learned all eight stakes won
54.2% at White, a pair that learned White alone 41.6%); only rentals are overrated (never buying them: +0.56 blinds
beaten at Gold; never buying eternals -1.16 at Black, perishables -0.66 at Orange); a stake-aware network wants less
say (weight 1.5 beat 2.5 at Green every round); and "reach=6" took a stake-aware pair 0.7 to 1.8 blinds further at
Green and Purple, but won less at Green.

So the champion keeps its networks, and a residual pair learns only what they get wrong. Each recorded position
gets the champion's log-odds of winning (`baseline --offsets-into`: its runs played again by their moves, nothing
else rewritten), and the pair learns the gap between those and how the runs ended (`train_value.py --offset`),
starting at no gap and keeping one only as far as its judging runs bear out; its report sets the corrected chance
beside the champion's alone, stake by stake. The engine adds it (`baseline --stake-net`). Round 1 learns the
champion's own runs: the night's, and about 31,000 more at every stake, most at White to Black, that it records first
(unless `.sync/record-champion.sh` already did). Then it plays at White against the champion and at every stake
against 3 November, and tries the other way too: the stake-aware pairs' delta (their chance at the stake less at
White) on the champion; and at Green, the champion alone and corrected at weight 1.5. From round 2 each stake records with the best teacher measured there so far: the
one that went clearly furthest against 3 November's on the same seeds, so no round learns from a teacher worse than
the last.

Start with quick turns, to build on while you watch: `make boost-quick` (about 25 minutes) adds the champion's
log-odds to the night's runs (once), trains one correction network on them, and plays the champion with it at White
on the first 1,000 seeds and at every other stake on the first 500, run by run against the champion's own games;
its last step prints the turn's table (`scripts/quick_table.py`). 500 seeds tell a stake's blinds beaten to about
+-0.55: big gains and losses show, small ones need the 2,000 of `make boost-night`.

```
cd /mnt/h/balatro-meta && make sync && make test && make boost-quick
```

Each new turn Claude sends is `make sync && make boost-quick FORCE=1`. When a turn wins, `make boost-night` confirms it
on the full sizes and goes on with the ratchet.

## Later steps

Claude adds each one here when it's ready. Next: more rounds of the learned value and of the network, and a
search guided by the network at card-play time.

## If something goes wrong

- `make status` shows which steps are done; logs are in `.sync/logs/`.
- A step failed: paste the last 20 lines of its log into the chat.
- "uncommitted changes": a tracked file was edited. `git stash` puts the edit aside, or ask Claude.
- "detected dubious ownership": run `git config --global --add safe.directory /mnt/h/balatro-meta`.
- "cargo: command not found" in an old terminal: run `source ~/.cargo/env`, or open a new terminal.
- Ubuntu can't see `H:`: check `ls /mnt/h`. If H: is a network drive, tell Claude.
