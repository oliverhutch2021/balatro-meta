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
the GPU (about 3 GB; `SKIP_GPU=1 bash bootstrap.sh` leaves that for later). It asks for your Ubuntu
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
  network on the GPU. It reads a position as the screen shows it (the same rule as everything the bot
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
37,000 to 68,000 runs can't match fourteen networks that learned about 195,000 White runs, and from round 2 each
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

## Step 27: the networks' doubt, then round 11 plays its wins on into Endless (about 12 hours)

What `make boost-quick` found: the correction made the champion's chance far better calibrated at the hard stakes
(BCE 0.37 against 0.66 on runs kept apart) but no better at telling positions apart (AUC lower at six stakes of
eight), and it played worse at every stake (blinds beaten clearly fewer from Red to Gold). A lower chance makes the
learned term, w ln(p + 0.01), steeper, so the corrected networks get more say just where they know least: the same
fall as weight 16 at White. So no `make boost-night` for now.

Three jobs in a row, each a fresh idea against the same plateau:

1. `make teacher-doubt` (about 3 hours): the champion's networks disagree most where they know least; "doubt=k"
   counts an option down by k standard deviations of their chances, the brake that may let them take more say.
2. `make learned-11` (about 4 hours): round 11, its runs recorded by the best teacher there is (the doubt trial's,
   if it won). Each won run plays on into Endless to ante 12, and its positions learn how many blinds it beat after
   the win: one result for about 200 decisions says little about which were good; how strong the win was says more.
   Its networks learn that as a second head.
3. `make teacher-weights` then `make teacher-margin` (about 5 hours): round 11's networks at weights 1.5, 2.5 and
   4, then at the champion's weight aiming for wins with 3 or 6 blinds to spare ("margin=K"). 82% of losses come
   at a boss: a teacher that wants room to spare may build stronger before the bosses arrive.

```
cd /mnt/h/balatro-meta && make sync && make test && make teacher-doubt && make learned-11 && make teacher-weights && make teacher-margin
```

## Step 28: a rethink: diagnose first, then train on decisions (today 1.5 hours, then a night)

Where step 27's thinking went wrong, and what changes. The bot takes the best of many options by a judge that
errs, so a more accurate judge (the stake correction: far better calibrated) still played worse: the maximum picks
the judge's mistakes. Its networks learn from one result per run, two hundred decisions back, so they learn what a
winning position looks like, never which option here was better. And 82% of its losses come at bosses, 70% at antes
5 to 8, without anyone having asked whether those are builds that could never have scored it or play that fell
short. So, before any more three-hour trials:

1. `make diagnose` (about 10 minutes): the champion's 3,000 White runs of the night of 4 November (the newest
   recording that kept its moves: round 10's didn't) played again by their moves, one row each: where it ended and
   by how much it fell short, its build then, and ante by ante its biggest hand, money and x mult jokers.
   Every lost blind is played again from its start by the champion with four times the samples: a blind that
   deeper play beats was lost to play, not to the build (that replay sees the run's own cards: a diagnosis, never
   the bot's play). `scripts/diagnose.py` prints the tables; `results/diagnose/night-summary.json`
   keeps them. (9 November: that reading was wrong. A second try at the usual samples beats as many lost blinds by
   luck alone, so a single deeper try measures luck, not play; step 34.)
2. `make forks-quick` (about 1.5 hours): forks (`forks.rs`). At a share of the shops entered and packs opened in
   antes 2 to 6 of those runs, the champion's three best options and doing nothing (and its own choice, if
   another) are each played out to the end by the champion on six fair copies that share their seeds across the
   options. Each fork says which option went best here; how often the champion's own choice was that option, and
   what its choices gave away, is the judge test (`results/forks/forks-quick.json`, `forks`): a measure of the judge
   that takes minutes instead of three hours of play. Then a pair of networks learns from the forks as well as the
   positions (`train_value.py --forks`: for each pair of options the difference of the network's win logits is
   trained towards the share of copies where the first went better), beside a control pair that learns the same
   positions without them; each report's `measured.forks` sets the network, the champion's networks, the quick look
   and the champion's own choice against the option that went best, on forks kept apart.

```
cd /mnt/h/balatro-meta && make sync && make test && make diagnose && make forks-quick
```

What the night of 5 November found. `make diagnose` (on the champion's 3,000 White runs of 4 November): 44% of the
losses scored 80% or more of the lost blind's target and another 43% half to 80%; the champion with four times the
samples beat 36% of the lost blinds from their start (half of the near misses; Violet Vessel 10%, The Mouth 51%);
the late losers' biggest hand trails the winners' by 11% at ante 4 and 34% at ante 7, and they are poorer at every
ante. `make forks-night` (6,192 forks at four copies, 78,000 play-outs, three hours: the recording's 3,000 runs ran
out) found every judge, the champion's full choice included, under one point of win rate better than picking at
random among the champion's two best options and leaving, and an oracle that knew half the play-outs no better: at
four copies the labels are mostly noise, and a pair trained on them ranked no better than its control. So the search
budget at the decisive moments is tried first (`make teacher-late`, `make teacher-boss`: three times the samples from
ante 6 or 5, four times in boss blinds, 8,000 seeds each against the champion), and the forks wait for a wider set
of options and more copies (`jobs.fork_chance` now sets a night's share of decisions from its time). `make teacher-late` (about 5 hours) tries the other half of the thought: three times the samples from
ante 6 (and from ante 5), where the runs are lost; it can run after the forks or on another night.

```
cd /mnt/h/balatro-meta && make forks-night && make teacher-late
```

## Step 29: how far the search budget goes (3.5 hours, then a confirmation)

5 November: three times the samples from ante 6 won 69.4% against the champion's 66.4% on the same 8,000 seeds
(p = 4 x 10^-14); from ante 5, 70.7% (+4.3 points, p = 2 x 10^-20), at 3.9 times the time a run takes. The jobs take
the best 8,000-seed report as the champion, so the ante-5 budget is the champion now (`best_8k_report`), and every
trial from here is set against it played again. `make teacher-budget` screens four settings on 2,000 seeds each
(a 3-point gain shows there; the comparison pairs the first 2,000 seeds of the 8,000-seed control): twice the samples
from ante 5, three times from ante 3, six times from ante 5, and three times everywhere. The winner is then confirmed
on 8,000 seeds, which is what makes it the champion (`make teacher-budget-big` plays all four there, a night's work;
or Claude queues the winner alone).

```
cd /mnt/h/balatro-meta && make teacher-budget
```

The screen (2,000 seeds, against the champion's first 2,000): twice the samples from ante 5 71.6% (no better,
p = 0.77), three times from ante 3 73.2% (p = 0.08), six times from ante 5 73.8% (p = 0.004), three times everywhere
77.05% (p < 10^-4, +0.55 blinds a run). The confirmation, `make teacher-budget-confirm` (5.1 hours): three times
everywhere won 75.6% against 71.8% on the 8,000 seeds (+3.8 points, 95% range 2.7 to 4.9, p = 4 x 10^-11; 1,205 runs
won that the champion lost against 901 the other way), so it is the champion, and the screen's 77% was about a
point and a half of flattery from the seeds it was chosen on. The exam, `make teacher-exam` (4.6 hours), 8,000 seeds
never used to choose anything: 75.6% (74.7 to 76.6), against 66.5% for the champion of 3 November. The tuning and
exam numbers agree, as they did on 3 November.

## Step 30: skipping a blind for its tag, judged by value (about 1.75 hours)

The bot has never skipped a blind (0 a run in every report); its human teacher skips an ante 1 blind for an
Investment tag, holds Double tags, and takes a few others when the moment is right. The diagnosis of 5 November put
the outcome of a run in its economy at antes 2 and 3, which is what a tag buys. Teacher option "skips" makes the skip
a move judged like any other: at the blind choice, the teacher plays both ways out on fair copies of the run (fresh
hidden futures), the blind skipped and the blind played, to the same place with the tag's work done (the Boss
Blind's choice, or the next ante's first for Investment, Double and a Big Blind's shop tag), and compares the
positions reached: the lost shop and reward and the tag's gift all counted by having happened. The networks are
shown the skipped blind as beaten and the tags as spent (no recording holds either, so they would misjudge them). A
skip is taken when its mean gain over four copies clears the copies' own noise by 2.5 standard errors (one copy
first; most lean against the skip and stop there), so only the clear cases go: on 16 seeds without networks, 0.19
skips a run, among them the ante 1 Big Blind for an Investment tag. A run is about 30% slower.

```
cd /mnt/h/balatro-meta && make teacher-skips
```

2,000 seeds against the champion of 6 November. A gain of a point or more shows; if it is level, the skips it takes
are still worth reading in the viewer before the option is dropped, since they are the first the bot has chosen.

6 November, 107 minutes: level. 76.85% against the champion's 77.05% on the same 2,000 seeds (-0.2 points, 95% range
-1.4 to +1.0, p = 0.81; 76 seeds won only with skips, 80 only without), 0.34 skips a run, a run 39% slower. The skips
it took, read from the 70 saved runs (18 skips): Economy when rich ($67, $91, $142), Polychrome, Holographic and
Negative tags for an edition joker, an Investment tag at ante 1 with $4, Charm packs late with money to spare, one
Rare at ante 3 in a run that then lost at ante 5. They are the skips a player takes, and on 2,000 seeds they bought
nothing: the blind and its shop are worth about what the tag is. That is White Stake's answer, where every blind pays
and the blinds are small; it is not the mechanic's. From Red Stake up the Small Blind pays nothing, so skipping it
costs only its shop, and at Green and Purple the blinds grow faster, so what a tag buys is a larger share of what the
run needs. `make skips-stakes` (about 45 minutes) tries the option at Red, Green and Purple with each stake's best
teacher against itself on the stake's 2,000 seeds; a stake where the skips take the runs clearly further is adopted
by the ratchet (`best_at_stake`) by itself.

```
cd /mnt/h/balatro-meta && make skips-stakes
```

## Step 31: the adaptive budget (3.5 hours)

6 November, `make teacher-budget-6`: six times the samples everywhere won 79.2% against the champion's 77.05% on
2,000 seeds (+2.15, 95% range +0.03 to +4.3, p = 0.053), at four hours for 2,000 runs: the budget still pays, less
each time, and a 6x champion would make every screen a four-hour job. The confirm was not run. Teacher option
"close-x=K" spends the care where it tells: when a decision's two best options are close (a shop's or pack's within
"close-gap" = 0.1 in value; a round's two best plans within "close-beats" = 10% of the tries in beats), those two are
judged again with every count scaled by K, and the better taken; every other decision costs nothing more. On the
champion's 3x budget, close-x=2 judges close calls at 6x for about 30% more time a run and close-x=3 at 9x for about
45%. The job screens both on 2,000 seeds against the champion.

```
cd /mnt/h/balatro-meta && make teacher-close
```

The screen (6 November, 77 and 85 minutes): close-x=2 75.3% and close-x=3 74.6% against 77.05%, -1.75 (p = 0.11) and
-2.45 points (p = 0.024), blinds beaten level. Worse, not better, and for a reason found in the morning: having judged
the two close options again, the code let a third option, or doing nothing, win at its first, unchecked estimate. The
two judged again come down on average (they led partly by luck), so the bot bought less: packs 17.2 and 16.9 a run
against the champion's 18.0, money at the end $58 and $60 against $55, where 6x everywhere bought more (18.7 packs).
Fixed on 7 November: the choice is between the two options judged again, alone. `make teacher-close-2` screens the
fixed option the same way (about 2.75 hours).

```
cd /mnt/h/balatro-meta && make teacher-close-2
```

7 November, the fixed screen: close-x=2 76.25% (-0.8, p = 0.47) and close-x=3 77.45% (+0.4, p = 0.74; +0.18 blinds,
95% range -0.01 to +0.36) against 77.05%. Level: re-judging the close calls among the finalists is not where six
times everywhere found its 2 points. `make teacher-budget-parts` doubles each part of the budget on its own (the
looks, the deals, the planner's tries) to find where it is.

Also 6 November, `make teacher-stakes` with the champion (2,000 seeds a stake): Red 61.4% (the 3 November bot: 52.0),
Green 29.0% (19.7), Black 16.1% (11.4), Blue 11.4% (8.8), Purple 2.9% (1.9), Orange 1.5% (0.9), Gold 0.15% (0.2).

## Step 32: the money numbers, retuned on the champion (about 7 hours)

`make diagnose-fresh` (7 November): the champion's own 3,000 runs recorded (75.1% won) and diagnosed; 8 of them
play another way on today's engine (7 met The Pillar, whose fix came after the recording) and are left out, so
the diagnosis is of 2,992 runs, 75.0% won (`results/diagnose/champion-summary.json`; `champion.json`'s 74.8% still
counted those 8 as losses, fixed in code since: a diagnosis now leaves them out of the report too and names them
in `left_out_runs`). 88% of the
losses at a boss, 83% at antes 5 to 8; Violet Vessel 18% and The Wall 11% of them, and deeper play beat only 7% and
14% of those (the build, not the play: deeper play beat 25% of all lost blinds, 41% at The Mouth). At the loss the
median run held $23 unspent (59% held $20 or more), 60% had no x mult joker, the main hand stood at level 3. Runs
that never held an x mult joker (42%) won 62%; a main hand ending at level 1 or 2, 52%; no voucher, 48%. The bot
dies at the big targets with a weak engine and money in its pocket: a valuation problem, how much a dollar held is
worth against what it buys now. Those numbers were last tuned on 1 November (+6.5 points then, on a bot at 53%).
`make teacher-money` tries four on 2,000 seeds against the champion: three lean towards spending, and `long-run=0.5`
weighs the whole long view more (the power some rounds on and the held money's shopping alike, so it is not a
spending test). The cheaper budget (`make teacher-budget-parts`) waits: win rate first, the processing later.

Results (8 November; the champion won 77.05% of the same 2,000 seeds): `buying-power=0.02` -4.45 points
(p = 0.0001), `money-scale=15` -2.6 (p = 0.016), `long-run=0.5` -1.55 (p = 0.15), `above-cap=0.25` level (-0.3,
p = 0.75). None beat the champion. Counting money for less loses, so the $23 held at a loss is not the
teacher spending too little; the trials worth trying next go the other way (money counted for more), or change
what the money buys rather than its worth.

```
cd /mnt/h/balatro-meta && make teacher-money
```

## Step 33: the long run, one part at a time (about 8.5 hours)

The value formula's long run is the run's scoring power some rounds on, its growth, and what the held money will buy
in the shops still to come, all under one weight (`long-run`, 0.35), so `long-run=0.5` moved them all at once. Two
new teacher options scale its parts apart: `long-power-x` (the power and growth) and `long-money-x` (the held
money), both 1 as usual. `make teacher-long` tries each at 1.4 (about `long-run=0.5` on that part alone), and how far
the long run looks: `long-rounds` is 9 (three antes), so before ante 6 it never sees the eighth ante's boss, where
most runs are lost; `long-rounds=15` and `=24` (the rest of the run). Then the money numbers the other way, since
counting money for less lost: `buying-power=0.04` and `money-scale=40` (is the usual 0.03 and 25 the best, or is
money still worth more?). Six trials, 2,000 seeds each against the champion at its own weight, about 85 minutes
each. Run it after `make teacher-money` has finished.

```
cd /mnt/h/balatro-meta && make teacher-long
```

## Step 34: is the boss play worth more search, then round 11 recorded by the champion (minutes, then about 25 hours)

Steps 32 and 33 settled the formula: ten trials either side of its numbers, none better, so the $23 the losing runs
hold is a symptom. The judge's other half is the networks, and every one of them learned from teachers that won 50%
to 66%. Round 11 was planned twice (steps 23 and 27) and never recorded, so no network has yet learned from the
Endless labels, and none from a 75% player. Two jobs:

1. `make diagnose-control` (a few minutes). The diagnosis of 7 November replayed each lost blind once with four
   times the samples and called a blind it beat "lost to play" (41% at The Mouth, 38% Amber Acorn, 25% The Eye,
   20% The Needle; 25% of all lost blinds). But the same blind played again at the champion's own budget is beaten
   some of the time by luck alone, and a single try a blind leaves that luck in the number. This plays every lost
   blind of the same 2,992 runs again 8 times at the usual samples (the control) and 8 times at 4x, each try with
   its own samples, so each blind's chance at each budget is measured and what deeper search adds is the
   difference (`baseline --deep 1`, `--deep-tries`; `diagnose.py --control`). The summary,
   `results/diagnose/champion-deep4-summary.json`, gives it by boss and as points of win rate at stake, an upper
   bound. Before it, those four bosses add up to at most 1.3 points; a 2,000-seed screen resolves about 2.5. If the
   four together still show a point or more over the control, a per-boss budget gets a screen; if not, the boss
   play is not where the points are and the budget stays as it is.
2. `make learned-11` (about 19 hours of recording, then about 5.5 hours of training and screens). Changed from the
   earlier rounds (`jobs.screened_round`, from round 11 on):
   - Recorded by the champion as it plays: its 14 networks, weight 2.5 and 3x/4x budget, exploring single moves
     (one shop or pack decision in ten takes a near-equal option) but no whole builds (`commit` gave nothing in
     round 10). Each win plays on into Endless to ante 12 and its positions learn how many blinds it beat after
     (in a 4-run test the wins went 3, 5 and 6 blinds on; it costs about 22% more time).
   - In five parts of 5,000 runs (`teacher-data-11`, steps `record-1` to `record-5`, seed bases 1212 to 1216,
     `~/.cache/balatro-meta/data/value-11/part-K`), about 3.8 hours each. A stopped recording starts again at the
     part it was in, not from nothing.
   - The pair (`models/value-11.bin`, `-11b`) learns from round 11's runs alone: the point is labels from the
     strongest player, not more positions from weaker ones.
   - `teacher-learned-11`: three screens on the first 2,000 usual seeds, each set run by run against the
     champion's own runs (77.05% there): the pair alone at weight 2.5, the pair alone at 4, and the champion's 14
     networks with the pair at 2.5. The pair alone level with the champion's 14 would already say the labels
     carry more; a gain of two points or more is what gets stacked. The margin trials (`margin=K`, reading the
     depth head) follow on whichever networks win.

```
cd /mnt/h/balatro-meta && make sync && make test && make diagnose-control && make learned-11
```

Ctrl+C stops it whenever you need the PC; the same line carries on (`make diagnose-control` is then skipped as done,
and the recording starts the part it was in again: at most about 3.8 hours lost).

What it found. The recording took 19 h 15 min and the pair trained in two minutes. `make diagnose-control`: a
second try at the usual budget beats 26.6% of the lost blinds by luck alone, and four times the samples beats 25.3%
(-1.3 points blind by blind, 95% -2.4 to -0.1; at ante 8 level). At The Mouth, Amber Acorn, The Eye and The Needle
together: 28.6% against 30.0% (95% -4.3 to +1.4), at most 0.06 points of win rate. The "lost to play" shares of
7 November were a second roll of the dice: deeper search does not beat the lost blind, so a per-boss budget has
nothing to gain. Where the 3x budget's gain comes from (shops and packs, or the play of blinds it wins) is not
settled by this.

## Step 35: round 11's screens against a fresh-salt control (about 6 hours)

Every 2,000-seed screen so far was set against the champion's own runs on seeds 1 to 2,000. Those are the runs that
made it champion, and they sit about a point above its level there: by block of 2,000 seeds the champion won 77.05%,
74.35%, 75.85% and 75.00%, and beyond what each block's difficulty explains (from 25 other 8,000-seed reports) that is
+0.9, -1.2, +1.1 and -0.8 points. So a screen against them reads a challenger about a point low, and moving to the
next block would read one about a point high. Playing the champion again does not help either: a run is fixed by its
seed, its index, the code and the options, so the `again` reports replay the same runs (8,000 of 8,000 endings).

`baseline --salt K` keeps the deals and draws the bot's own samples afresh (run i's salt becomes `i | K << 24`;
K = 0 replays today's runs exactly). From round 11 a round's screens (`jobs.champion_trials(..., salt=SCREEN_SALT)`)
play the champion once at salt 1 on seeds 1 to 2,000 and every trial at the same salt, so control and trials share
their samples as before but the control's luck is a new draw.

`make teacher-learned-11`: the control (the champion at salt 1), then round 11's pair alone at weight 2.5, the pair
alone at 4, and the champion's 14 networks with the pair at 2.5, each set run by run against the control.

The decision, stated before the run (each arm's paired difference with its 95% range; three arms screened together):

- **Stack** the best arm if it is +2 points or more and its range excludes zero: it joins the champion. The best of
  three equal arms reads about a point high (one standard error), so +2 here can be about +1 underneath; that is
  accepted because a stacked arm goes through 8,000 seeds and the exam before any headline. Its salt-1 runs are the
  ones that chose it, so screens on it next use a new salt (2), with it played at that salt as their control.
- **Worse** if every arm's range is below zero: round 11's pair does not help.
- **Level** otherwise: nothing is stacked. The pair alone level with the 14 means the two can't be told apart at
  about two points, not that one round of a 75% teacher's labels carries as much as eight rounds of weaker ones.

What runs next is chosen when the result lands. The margin trials (`margin=K`) were the plan, but they read the depth
head, which beats knowing only how often each depth comes by 3% (cross-entropy 1.751 against 1.805,
`results/value/value-11.json`), against 13% for the win head: a low-prior screen, to be weighed against the others.

The fresh control against the champion's stored runs on the same seeds is information only (one draw carries about a
point of noise of its own, so it shows the selection's direction at best); Claude runs it from the committed reports,
`scripts/compare_runs.py <stored report> <salt-1 control>`, with no PC step.

The job was first started on 9 November on the code before this step and stopped within minutes, so this first run
needs FORCE=1 (it restarts the job on the new code):

```
cd /mnt/h/balatro-meta && make sync && make test && make teacher-learned-11 FORCE=1
```

Ctrl+C stops it whenever you need the PC; the same line carries on from the step it was in.

What it found (all at salt 1 on seeds 1 to 2,000, against the champion at salt 1, 75.35%): the 14 networks with
round 11's pair at 2.5 won 77.05%, +1.70 (95% -0.28 to +3.68); the pair alone 61.70% at 2.5 (-13.65) and 60.50%
at 4 (-14.85). Level by the rule: nothing stacked. The pair alone can't stand in for the 14 at any weight tried.
The fresh control against the stored runs on the same seeds: 75.35% against 77.05%, the direction the stored
runs' luck predicted. The +1.70 arm stays a candidate for the final combined bot, with no re-screen.

## Step 36: where the budget's gain sits, by taking it away (about 3.5 to 4 hours)

From the usual counts to the champion's 3x at every ante (4x in boss rounds) was worth about 9 points (66.4% to
75.6% on the 8,000 tuning seeds, step 29). Doubling one part on top (`make teacher-budget-parts`, parked) could show
a point or so, under a 2,000-seed screen's reach. This takes the 3x/4x away instead, from one phase of the run or
one part of the budget at a time (`teacher::move_scales`): teacher option `flat-between` keeps every count at its
own between rounds (the blind choice, shops, packs) and `flat-round` every count in a round (the planner's tries
and the values of held cards), the other phase scaled as the champion; `flat-deals` and `flat-looks` keep only the
deals behind each full value or the sample contents behind packs and rerolls (both also value held cards in a
round). "Its own" is the champion's base ("precise": deals 1, quick look 1, sample contents 2, tries 2).

`make teacher-budget-away`: four arms, each on seeds 1 to 2,000 at salt 1, set run by run against the champion at
salt 1 (round 11's control, already played: no control step):

- (a) every count between rounds flat (`flat-between`);
- (b) every count in a round flat (`flat-round`);
- (c) the deals alone flat, (d) the sample contents alone flat (each including their in-round valuations);
- and (a) against (b) run by run (`compare-between-vs-round`).

Each arm thinks less than the control, so each runs faster (in a small cloud run without networks, (a) took about
a fifth of (b)'s time). (a) and (b) together are every count flat, the 66.4% bot, so their two losses can be set
against the 9 points.

The decision, stated before the run (each difference with its 95% range; four arms and one comparison of two):

- **Between rounds carries it** if (a) is 3 points or more below the control with its range below zero, and (a) is
  2 points or more below (b) with that range below zero. Next: one screen raising only the counts between rounds
  to 6x (rounds stay at 3x/4x), on salt 1 against the same control; (c) against (d) says whether the work on
  valuing shops aims at the deals or at the sample contents.
- **Round play carries it** in the mirror case: (b) 3 or more below the control and 2 or more below (a), both
  ranges below zero. Next: one screen raising only the counts in a round to 6x, and a look at what changes in the
  play of the blinds it wins.
- **An even split** otherwise: 6x everywhere stays for the final bot and neither phase gets work of its own. That
  includes both halves under 3 points each while their sum falls well short of the 9: then the phases stand in for
  each other, which is not "neither matters".

For information: (a) and (b) together against the 9 points (much less than 9: they stand in for each other; much
more: they need each other). Where 1x to 3x sits need not be where 3x to 6x sits; the screen that raises the
carrying phase checks that. No arm here is adopted, so salt 1's control stays unselected and the next screen can
use it.

```
cd /mnt/h/balatro-meta && make sync && make test && make teacher-budget-away
```

Ctrl+C stops it whenever you need the PC; the same line carries on from the step it was in.

## Step 37: overnight, the bundle on 8,000 seeds (about 12.5 hours)

Step 36's first two arms (information only until all four are in): every count between rounds flat lost 5.45 points
(95% -7.75 to -3.15) and ran in 19 minutes; every count in a round flat lost 0.80 (-2.96 to +1.36) in 77 against
the control's 74. The budget's gain and its time are between rounds, which is where the losses to Violet Vessel
point too: those runs start ante 8 with a median $39.50, see the 6x from its start, and lose holding $32.50.

One reason, found in the code: in the last shop the long run's money term is zero (value.rs `spend`), so a
reroll's cost in value is about zero, and `search` (teacher.rs) values a reroll as one sample shop's average
whenever that cost is zero or below, dropping the chance to reroll again exactly where rerolling is free. The sign
of the noise in the cost picks between "best of many shops" and "one shop". Teacher option `reroll-best` values it
instead as the best of the sample shops the money can still buy (`free_search`, `rerolls_affordable`); it changes
nothing elsewhere.

`make teacher-bundle`, on seeds 1 to 8,000 at a fresh salt (2), each pair compared as soon as its second arm ends:

- the champion (the control; also the control of the longer chain to come), about 5.1 hours;
- one combined candidate at the champion's budget: its 14 networks with round 11's pair (+1.70 on 2,000 seeds,
  95% -0.28 to +3.68) and `reroll-best`, about 5.1 to 5.5 hours;
- then both on 2,000 Green seeds at the same salt (`--stake 3`), about 0.85 hours each, a second gauge;
- and, for information only, the fresh control against the champion's stored 8,000 runs (`compare-stored`): its
  luck and the code's drift since 6 November.

A salted report never becomes the champion by a higher rate (`best_8k_report` leaves them out): a champion changes
on this rule. The decision, stated before the run (the White paired difference with its 95% range, about +/-1):

- **Gain** if the candidate is +1 point or more with its range above zero: it becomes the base the longer chain
  builds on. Later candidates are paired against this same salt-2 champion control, which chose nothing, so their
  difference is the total over the champion, which is what the exam decision needs (a salt-3 control of the
  bundle is needed only to read a step on top of it). The exam follows only for a combined bot 3 points or more
  above its control with the range above zero.
- **Worse** if the range is below zero: the combination is dropped and the parts split to find the one that costs
  (the only case that spends runs on the parts).
- **Level** otherwise: nothing is adopted, the champion doesn't change, and no split runs follow (5 hours an arm
  for parts worth under a point each). The reroll fix is decided by the shop audit's spend-down fork, where it
  acts; the pair is carried in later candidates, free to run, without being called a gain. A Level bundle does not
  say the reroll fix failed: it bites where a reroll's cost comes out at zero or below (always in the last shop, by
  noise elsewhere), its ceiling is the ante 8 losses (about 8% of runs), and its likely share is under what +/-1
  can see here.
- Green is information for this decision. A candidate level at White and clearly up at Green (its paired 95%
  range above zero, about +/-2 on 2,000 seeds) stays a candidate for the stakes phase, not the White champion.

It starts once step 36 has finished (the sync needs its results committed); this line waits for it:

```
cd /mnt/h/balatro-meta && while grep -q '"state": "running"' .sync/watch/current.json; do sleep 60; done; sleep 120; make sync && make test && make teacher-bundle
```

Ctrl+C stops it whenever you need the PC; `make teacher-bundle` carries on from the arm it was in.

What steps 36 and 37 found.

- **Step 36 (salt 1, seeds 1 to 2,000, against the champion's 75.35%):** every count between rounds flat -5.45
  (95% -7.75 to -3.15); every count in a round flat -0.80 (-2.96 to +1.36); between rounds against in a round
  -4.65 (-6.93 to -2.37). **Between rounds carries it.** Within it, the deals flat -0.25 (-2.36 to +1.86) and the
  sample contents flat -4.65 (-6.85 to -2.45): the gain is in the looks behind packs and rerolls, not the deals.
  Time for 2,000 runs: the control 74 min; between flat 19; in a round flat 77; deals flat 67; looks flat 44.
- **Step 37 (salt 2, seeds 1 to 8,000):** the champion 75.98%; the bundle (round 11's pair and `reroll-best`)
  76.05%, +0.07 (-0.93 to +1.08). **Level:** nothing adopted, no split runs; the pair is carried in later
  candidates, the reroll fix waits for the shop audit. Green: 28.20% against 29.00%, +0.80 (-1.14 to +2.74),
  information. The fresh control against the stored 8,000 runs: +0.41 (-0.69 to +1.51), information.

## Step 38: the long run, buying looks (about 40 hours)

Step 36 says the budget worth buying is more looks, not more of everything, and the looks are also where the time
goes (looks flat saved 41% of the control's time, deals flat 10%). Teacher option `looks-x=K` sets the sample
contents' own count (the champion's "precise" is 2, so 3x makes 6); `flat-deals` keeps the deals at their own;
`between-x=K` raises every count between rounds K times more on top of `late-x` (`teacher::move_scales`).

`make teacher-looks`, on seeds 1 to 8,000 at salt 2 with the champion's own 14 networks (round 11's pair is out:
step 37's bundle was Level, so its +1.70 on 2,000 seeds was noise), each arm set run by run against step 37's
champion control (salt 2, chose nothing: no control step), cheapest first, each compared as soon as it ends:

- `looks12`: twice the looks (12), the deals at their own count, about 7.5 hours;
- `looks24`: four times the looks (24), the deals at their own, about 13.6 hours; also set against `looks12`;
- the two looks arms on 2,000 Green seeds against step 37's Green control (28.20%), about 1.1 and 2.1 hours,
  information as step 37 defines it;
- `between6`: 6x every count between rounds, about 16 hours, last and the first to cut: with the looks arms in, it
  asks only whether anything between rounds other than the looks pays (it differs from `looks12` by the quick
  look and the deals at 6).

The decision, stated before the run (each White arm's difference against the control with its 95% range, about
+/-1; three White arms, so the best reads about half a point high):

- **An arm is void** if its time is outside 0.6 to 1.6 times its estimate above: the check that the option bit.
- **A new champion** if the best arm is +1.5 or more with its range above zero. If `looks24` against `looks12`
  spans zero and both clear the bar, the cheaper `looks12` is the one. If `between6` is best but not clearly
  above the better looks arm, the looks arm is the one. The adoption is its own change, after C's review: salted
  reports never become the champion by rate (`best_8k_report`).
- **Level** if no arm reaches it: the champion stays, the budget is near its plateau for this judge, and the
  work turns to what the judge believes (the shop audit, the reroll fix, Track 3's labels).
- **The exam** on the user's return only for an adopted arm +3 or more with its range above zero.
- For information: the looks curve at 2 (-4.65 on 2,000 seeds), 6 (the control), 12 and 24 says how much more the
  looks can buy; money at the end against the control says whether more looks spend the money runs now keep.

```
cd /mnt/h/balatro-meta && make sync && make test && make teacher-looks
```

Ctrl+C stops it whenever you need the PC; `make teacher-looks` carries on from the arm it was in.

What step 38 found (salt 2, 8,000 seeds, against the champion's 75.98%): `looks12` 75.86%, -0.11 (-1.19 to
+0.97); `looks24` 76.46%, +0.49 (-0.59 to +1.56); **`between6` 78.00%, +2.02 (+0.99 to +3.06)**, and +1.54 (+0.48
to +2.60) above `looks24`. Green: `looks12` -1.25, `looks24` -0.90, both spanning zero. Times 0.71 to 0.88 of the
estimates (none void). By the rule `between6` is the arm: **6x every count between rounds is the final bot's
setting** (B's review, 11 November); the research base stays the 3x champion; no exam (under +3). The looks arms
also dropped the deals to 1, so they show only that more looks with fewer deals is level; `between6` ends runs
richer ($58.2 against $54.9), so it wins by choosing better, not by spending more.

## Step 39: the shop audit (about 6 hours)

Runs lost at the last ante start it with a median $39.50, see the 6x Violet Vessel from its start, and lose holding
$32.50; `reroll-best` barely moved what the bot does (step 37). So: what does the teacher see and decide in the late
shops? `baseline --audit-shops FILE --audit-antes LO-HI` writes every shop decision of those antes as a JSON line:
the shop's row and packs with prices, the money, the interest cap, the reroll's price, the jokers and slots, and
what the teacher noted (`teacher::audit_begin`): every option's quick value against staying, the reroll's sample
shops (their mean and best), its cost in value (where `search` drops rerolling again when it is zero or below), the
finalists' full values against leaving, and the move. Play doesn't change: a unit test and a cloud run (8 runs, the
same endings and moves with and without it) check that.

`make shop-audit`: the champion plays step 37's control again with the audit on, so the runs are the control's own
move for move (`white-same-runs` and `green-same-runs` compare them: no seed may differ): 8,000 White seeds at
salt 2 with antes 6 to 8 audited, then 2,000 Green seeds with antes 3 to 5 (where Green's losses gather). The lines
stay in `.sync/audit` (gzipped; too large for git); `scripts/audit_shops.py` writes a summary into results/audit:
for the runs won, lost at the win ante and lost earlier, how often a shop was left with $20 or more while a reroll
was on offer, the reroll's value against leaving, how often its cost came out at zero or below, and what was chosen.

What it decides: if the runs lost at the last ante leave rich shops with a reroll on offer that the teacher rates
near or above leaving, the reroll's valuation (or its gate) is where the fix goes, and the spend-down fork (each
such shop played out both ways on fair copies) is built next to measure it; if they leave because nothing rated
helps, preparation has to start earlier and `blinds_ahead`'s mixture gets its screen. The 12x reference and the
spend-down fork are the audit's second part, after this one is read.

```
cd /mnt/h/balatro-meta && make sync && make test && make shop-audit
```

Ctrl+C stops it whenever you need the PC; `make shop-audit` carries on from the step it was in.

What step 39 found. Both audit runs reproduced their controls exactly (no seed differs). Over all of antes 6 to 8
winners and the runs lost at the last ante shop alike. The last shop before the ante-8 boss is where they part
(6,078 won, 559 lost there; medians): money on leaving $37 against $24; the reroll's sample shops average -0.004
against -0.029 below staying, their best +0.048 against +0.155; and **a $5 reroll costs 0.0135 in value for the
winners and 0.060 for the runs about to lose**, where the formula prices money at nothing (value.rs `spend`). The
cost is zero or below in 12% and 2% (so `reroll-best`'s gate hardly mattered). The likeliest source of the charge is
the networks, which read the money from the screen (`observe::encode_value`) and learned that rich runs win.

## Step 40: the judge's belief about money (about 11 hours)

Teacher option `freeze-money` has the networks read the money as it stood when the shop opened while the shop's
decisions are weighed (`teacher::frozen_money`, `learned::with_frozen_money`), so spending inside a shop isn't read
as becoming poorer and only the formula's own money terms price it; `freeze-money-last` does so only in the run's
last shop, where money can't matter afterwards. The shop audit's reroll note now also splits the cost: the
formula's part (`cost_formula`) and the rest (the networks').

`make teacher-money-belief`, on step 37's salt-2 seeds against its controls, each compared as it ends:

- `last`: `freeze-money-last`, 8,000 White seeds, ante 8 audited, about 5 hours;
- `all`: `freeze-money`, 8,000 White seeds, ante 8 audited, about 5 hours;
- `green-all`: `freeze-money`, 2,000 Green seeds (Green dies at antes 4 and 5, where `last` does nothing), about
  0.8 hours.

The decision, stated before the run (S, #12; each difference with its 95% range):

- `last` is adopted at +1 or more with its range above zero; its ceiling is the ante-8 boss losses (about 7% of
  runs), so a Level doesn't close it: part 2's fork measures it more sharply on the same states.
- `all` at +1.5 or more with its range above zero; then Green and a retune of `buying-power` (tuned while the
  networks charged too) decide the next step. Clearly down: outside the last shop the charge does real work.
  Level: the money belief is not where the under-building comes from, and that question closes.
- Worse is a range below zero. A Level is a Level: nothing adopted on logic alone.
- For information: the audit's `cost_formula` against the cost says where the charge comes from.

```
cd /mnt/h/balatro-meta && make sync && make test && make teacher-money-belief
```

Ctrl+C stops it whenever you need the PC; `make teacher-money-belief` carries on from the arm it was in.

**What it found (12 November; salt 2, paired against step 37's controls):**

- `last`: 75.98% → 76.38%, **+0.40 (95% +0.09 to +0.71)**, p 0.015. Real but under the +1 bar stated before the run:
  not adopted as the research base; B to rule on adding it to the final bot's settings beside 6x between rounds.
- `all`: 75.98% → 70.36%, **-5.61 (-6.71 to -4.52)**. Worse.
- `green-all`: 28.20% → 23.10%, **-5.10 (-7.14 to -3.06)**. Worse.
- So outside the last shop the networks' charge for money does real work, and spending more freely without knowing
  what the money is for costs about five points: the problem is not the willingness to spend (lab #20).

## Step 41: the leak tally and the forced-buy forks (about 11 hours)

Two diagnoses in one night, neither changing a move (the lab's direction 2: the bot plays the position in front of it
and builds towards nothing). `make leak-tally` plays the champion on the first 3,000 of step 37's salt-2 seeds,
move for move the same runs as its 8,000-seed control (`same-runs` checks it; from step 44 with `no-boss-reroll` and
`no-held-cards`, which that control predates), with two things noted:

- **The leak tally** (`baseline --leaks`, `leaks.rs`), a line per run: Blue and Purple Seals drawn and what became of
  each (thrown while digging or as a spare, played, held at the end, the Planet or Tarot made or not made for want of
  a slot); won rounds that had a winning hand in hand with two or more hands or a discard to spare, and whether the
  round ended on that very move; what each way of finishing fared in `planner::finish_choice` (a new note: not
  offered, failed a try, passed every try but gained too little, kept); consumables held through rounds by kind;
  rounds ended with every slot full; which card each targeted consumable went on (its rank by `card_value`, already
  sealed or not); Spectral packs' kinds offered and taken; shops left with money at the interest cap after the
  cheapest pack still on sale; the last shop's money with and without Bull or Bootstraps; jokers bought and sold by
  ante; vouchers offered, affordable and redeemed; hands by type. `scripts/leaks.py` sums it.
- **The forced-buy forks** (`baseline --force-buy`, `forks::forced_fork`). Where the run passes Blueprint,
  Brainstorm, Bloodstone, Campfire, Photograph, Sock and Buskin or Lucky Cat (antes 2-8), or Mr. Bones or Luchador
  (antes 7-8), on sale and affordable (leaving the shop or rerolling with it in the row, or skipping it in a Buffoon
  pack), one such position per joker and run is picked by a hash and forked once the run has ended, three lines on
  the same fair copies, each played to the end by the champion as it is: the run's own move; taking the joker
  (selling first the held joker the value says it would miss least, if the row is full:
  `teacher::take_joker_anyway`) and keeping it, the game's own Eternal sticker put on it in that line so it can't be
  sold again (the primary line: a value that rated the joker rightly would buy it and keep it; the bot sees the
  sticker); and taking it with the bot free to sell it again (information: in the cloud test it sold a Bloodstone
  straight back). Two copies each, four for the rarer jokers (Blueprint, Brainstorm, Lucky Cat, Mr. Bones);
  Photograph is forked in 60% of the runs that pass it (about 600 forks). `scripts/forced_buys.py` gives per joker
  each forced line less the run's own in win rate and in blinds, with 95% ranges over the forks, and how often the
  bought joker was still held as that shop closed and at the end.
  From step 39's audit, about 5,700 such (run, joker) pairs come up in antes 6-8 of 8,000 runs; with the earlier
  antes, about 1.5 forks a run (a small test in the cloud: 13 in 8 runs), so roughly 4,000 to 4,500 forks on 3,000
  runs before Photograph's cap, Blueprint the fewest (under 100). The forks take most of the time (in the cloud
  test, five times the runs themselves).

The decision, stated before the run (S, #19 and #20):

- Leaks: one present in a fifth of runs or more and collected less than half the time gets a fix. The fixes (seals,
  held cards, targets: #19's spec) go in as one bundle on one 8,000-seed arm; this tally run again on the fixed bot
  is the proof the behaviour changed, and the win rate must not fall: the lower end of that arm's paired 95% range
  above -1.0 point.
- Forks, on the held line: a joker worth 3 points of win rate or more from the states where it was passed, its range
  above zero, is valued wrong by the judge (class A: fixed in the value). A joker the user calls strong that shows
  nothing needs a way of playing the bot lacks (class B: Plan v0, #20). A range wider than +-3 points is unmeasured,
  never class B. Below 3 points with the range above zero, or Level: no change from this alone.

```
cd /mnt/h/balatro-meta && make sync && make test && make leak-tally
```

Ctrl+C stops it whenever you need the PC; `make leak-tally` starts the tally again from its first run (the lines are
written as it goes, but a run's forks only once it ends, so a stopped tally is played again whole).

## Step 42: the boss reroll (about 6.5 hours)

The bot never rerolled a boss: the real-game checks (BalatroBot) have no call for it, so the move was kept out. The
user: "the boss reroll can be game changing, since that is where we struggle most". Teacher option `boss-reroll`:

- At each blind choice where the Reroll Boss button works (Director's Cut once an ante, Retcon any time, $10),
  `teacher::boss_reroll_gains` values the run with and without a reroll on six fair copies (a fresh hidden future
  each, so every new boss is a fresh draw from the bosses still possible, never the real next one), and rerolls when
  the mean gain beats `boss-reroll-margin` (0.02). Within the skip option's look-ahead it never rerolls.
- In a shop, a Director's Cut or Retcon on offer is worth, on top of the position, what rerolling the bosses still
  to come would gain (`reroll_voucher_worth`): on a fair copy the run leaves the shop for its next blind choice, and
  there the run is valued facing six bosses drawn as the game would draw them (this ante's, the last regular ante's,
  the showdown's: from the bosses still possible by the use counts a player can follow), each only the boss
  changed. This boss's reroll counts its own gain; each later regular ante the mean gain over the regular draws, the
  last ante the mean over the showdown draws, each gain the one the bot would take (`reroll_gain`: the mean of a
  fresh draw less $10, priced as the value of $10 less here; above the margin only). Retcon, bought on top of
  Director's Cut, is worth what rerolling as often as needed adds (the reservation value of drawing at $10 a draw).
  In 64 light runs in the cloud Director's Cut was bought in 12 of the 59 shops that could afford it (1 of 49 when
  only this ante's boss counted), and 5 bosses were rerolled in 4 runs.
- The viewer shows the Reroll Boss button under the boss (red when it works, "rerolled" once used), names each
  reroll in the move log ("Violet Vessel → Cerulean Bell, -$10"), and its "Boss rerolls" button replays the runs
  that rerolled one (a job keeps the first 30 as `reroll-*.json`). The real-game checks still stop before a reroll.
- `make reroll-demo` (about 15 minutes) plays the champion with the option on 400 seeds, only to look at: its reroll
  runs in the viewer and its tally of vouchers bought and bosses rerolled.
- The leak tally counts the bosses rerolled away.

`make teacher-boss-reroll` on step 37's salt-2 seeds against its controls: 8,000 White seeds with the leak tally on
(about 5.5 hours), then 2,000 Green (about 1 hour).

The decision, stated before the run:

- Adopted at +1 point or more on White with its range above zero. Worse (a range below zero): dropped. Level: read
  how often the voucher was bought and which bosses were rerolled into what before deciding anything. Bought in under
  a tenth of the runs that could afford it: the valuation is still too low and the arm is unmeasured, not Level (S).
- Green is for information (its bosses come at antes 4 and 5).

```
cd /mnt/h/balatro-meta && make sync && make test && make reroll-demo
cd /mnt/h/balatro-meta && make teacher-boss-reroll
```

## Step 43: held Tarots and Spectrals, the leak bundle's first part (about 6.5 hours)

The user, watching `make reroll-demo`'s runs: "it is holding Death and Devil much longer than it should... trying to
find the perfect hand and taking up slots it doesn't need to". In those 400 runs a Tarot or Spectral held as a round
began was still held at its end 61% of the time (S's count, lab #19; The Sun 96%, The Devil 82%, Death 39%), 573
rounds ended with every slot full, and 95 Purple-Seal Tarots and 52 Blue-Seal Planets were never made for want of a
slot. Teacher option `held-cards`:

- In a round a held card is used whenever using it beats losing it (`value_consumable_in_round`: no longer only when
  that realises half its held worth, and no margin to beat), so it goes in the first round that offers a use.
- The cards the user uses at once whenever they can be are: The High Priestess, The Emperor, The Wheel of Fortune,
  Temperance, and The Hermit at $20 or more.
- A card held through a whole round unused is worth nothing held from then on (`teacher::stale_card`,
  `value::consumable_dollars`), so the next shop sells it.
- With every consumable slot full, the Planets and Tarots the run's seals and jokers would make count for nothing in
  the value (`flows_for`), so a full slot costs what it blocks.

In 96 light runs in the cloud (no networks): Tarots and Spectrals still held at a round's end 130 of 236 rounds
(55%) before, 9 of 111 (8%) with the option; rounds ended slots-full 19 before, 2 after.

`make teacher-held-cards` on step 37's salt-2 seeds against its controls, both with the leak tally on: 2,000 Green
seeds first (the development ground, #20), then 8,000 White (the guard).

The decision, stated before the run (S and the user, #19):

- Behaviour: each Tarot and Spectral kind still held at a round's end in under 5% of the rounds it was held at the
  start (`leaks.py`'s `held_through_share`), any kind above it named with its reason; rounds ended slots-full and
  cards not made for want of a slot falling towards zero.
- Win rate: White's paired 95% range's lower end above -1.0 point; Green for direction.
- Both met: it goes into the base with the rest of the leak bundle. Behaviour met and the win rate below the bar: what
  the freed slots and early uses cost is read from the tally before anything else.

```
cd /mnt/h/balatro-meta && make sync && make test && make teacher-held-cards
```

## Step 44: the boss reroll and the held cards on by default (about 6.5 hours)

The user: "why is it off, turn it on". From 13 November the teacher plays with `boss-reroll` (step 42) and
`held-cards` (step 43) on by default (`teacher::ON_BY_DEFAULT`); `no-boss-reroll` and `no-held-cards` turn them off.
What changes with them:

- Every job that plays the champion now plays it with both, the recordings included. A report says so in its
  `teacher_defaults`; a report without that line was played before them.
- Screens never reuse a control played before them: once the champion's report predates the defaults, a screen's
  control and trials carry `-d1` in their names and the control is played again on today's code (`champion_trials`).
- `make leak-tally` (step 41) turns both off, so it replays step 37's control move for move as before and stays the
  "before" for the leak bundle; its forks measure the bot without them.
- The real-game checks stop before a boss reroll, so they follow fewer runs to the end.
- `make teacher-defaults`: the bot as it now plays, both on, against step 37's controls (played without either):
  2,000 Green seeds first, then 8,000 White, both with the tally on. `make teacher-boss-reroll` and
  `make teacher-held-cards` measure one without the other.

The decision, stated before the run: White's paired lower end above -1.0 point (the leak bundle's bar) keeps them on;
below it, the single-option arms say which part costs it, and that one goes back off until it is fixed. Green is for
direction. The held cards' behaviour test is step 43's (each kind under 5% held through). B's masking rule (#25): if
White's point estimate is under +1.0, `make teacher-held-cards` runs on White too before the held cards are taken as
costless (a reroll gain could hide a held-card loss).

```
cd /mnt/h/balatro-meta && make sync && make test && make teacher-defaults
```

## Step 45: the master list's batch 1, on by default (lab #27)

The user: "Let's co-ordinate every fix now, leave no stone unturned", and "why do we keep delaying the fixes, just push
them now". Issue #27 is the one list of known faults with a bar each; batch 1 is in, each part on by default
(`teacher::ON_BY_DEFAULT`) and off with its `no-` option:

- **Held cards (C1-C3, C5-C7, `held-cards`).** A held card a round never used, or one with no use for the run as it
  stands (`useless_card`: The Tower without Stone or Marble Joker, a suit Tarot off the suit being built, Judgement
  with a full row, a no-target card the engine won't let be used), is sold before anything else in the next shop
  (its sale alone was too small for the value's screen, so stale cards sat in their slots: the cause of The Sun's
  78%, which a probe showed is used at the first round as often as the other three). The cards the user uses rather
  than holds are used when a use costs at most 0.01 in value. The seal Spectrals, Cryptid and Aura go on the most
  valuable card that can take them (Trance: a Steel card first). A full slot sells the least useful held card, not
  the last. The Spectrals the user skips (Grim, Incantation, Ouija, Ectoplasm, Ankh) are never taken from a pack.
  `glass` on: a card's worth counts its retriggers.
- **Rounds (P1-P3, P5, P8, `round-fixes`).** Digging throws by worth: a Purple Seal first while a slot is free,
  then the cards worth least, never a card with a seal, enhancement or edition while anything else can go. A winning
  hand leaves in hand the cards that pay held (a Blue Seal, Steel, Gold, a Purple Seal still to throw) when one that
  avoids them still wins. Throwing for value and extra hands never spend those cards. The first move of a won round
  acts on the hand the teacher's own search found. `five-kind` on (P7).
- **Shops (S1-S4, S6, `shop-fixes`, `freeze-money-last`).** A joker the money covers only once a held joker is sold
  is offered; a pack's joker with a full row is taken after a sale; any held joker may be the one sold; no $3 kept
  back from rerolls in the last shop; `freeze-money-last` on (+0.40, step 40). The reroll voucher's worth draws each
  later boss from the others (B's note).
- **Jokers the value couldn't see (J1-J3, `joker-fixes`).** Mr. Bones: the blind where it gains most counts its
  chance at a quarter of the target. Luchador: this ante's boss judged without its effect while it is held.
  Vampire's growth capped by the enhanced cards there are.
- Not in batch 1 yet: C4 (digging for a Tarot's target), P6 (the steer away from Two Pair), S5 (a sample shop's
  second buy, a Mega pack's second pick).

Every tally now prints the bars pass or fail (`leaks.py`, lab #27). The gate before any long run: `make look`
(400 Green and 400 White runs of the bot as it now plays, tallied, about 30 minutes; the runs kept for the viewer),
the bars read and ten runs watched. Then `make teacher-batch1` (every default on, named so its reports never meet
`teacher-defaults`', against step 37's controls: 2,000 Green, then 8,000 White, both tallied; White's paired lower
end above -1.0). `make leak-tally` turns every default off and stays the "before".

```
cd /mnt/h/balatro-meta && make sync && make test && make look
```

## Step 46: the night (about 9.5 hours; S's order, lab #27)

B's two findings on batch 1 are fixed:
- with Luchador held, The Wall's target is halved and Violet Vessel's divided by 3 in that blind's term
  (rules/blinds.md section 11.4);
- a Stone card is thrown while digging unless Stone Joker pays for it.

The tally adds B's bar (rounds lost after a winning hand was in hand, per such round) and splits Purple Seals played
with discards left by whether a slot was free.

E1, teacher option `ahead=K` (lab #27, the user's go): `blinds_ahead` also counts the three blinds of each of the K
antes after the next, at their real targets (the stake's table of base scores, Small 1, Big 1.5, a boss 2: the same in
every run), each with the growth and shopping the value projects to that round, weighted 0.7 per ante further off.
It reads nothing of the seed: not which boss a later ante holds, no later shop, pack or card.

`make night`, one resumable job (about 9.5 hours):
1. **The look:** 400 Green and 400 White runs of the bot as it now plays, and the same 400 White with every default
   off, all tallied, the bars printed, the runs kept for the viewer.
2. **The gate** (`scripts/night_gate.py`). It stops the long arms when White's look is more than 8 points under the
   control on the same seeds, or when rounds lost after a winning hand run at more than twice all-off's rate. On a
   stop, `make leak-tally` runs instead (`scripts/gated.py`). A crash stops the job.
3. **`teacher-batch1`:** 2,000 Green, then 8,000 White, against step 37's controls.
4. **E1 on Green:** `ahead=2`, `ahead=3` and every ante left (`ahead=8`: the antes past the winning one are never
   counted; the user: "a human player would also know the scoring requirements for every ante"), 2,000 seeds each,
   paired against batch1's Green arm.
5. In the morning, **E1's guard** for the setting Green favours: `make e1-white-ahead<K>` (2,000 White seeds against
   the first 2,000 of batch1's White arm).

The rules, stated before the run:
- **Batch 1:** White's paired lower end above -1.0. If White's point estimate is below 0, leave-one-group-out arms
  run on 2,000 Green each before any group is called costless (B). Green is information.
- **E1:** the best K is adopted into the next base if Green gains 3 points or more with its range above zero (best
  of three: a little over half a point high) and White's point estimate on 2,000 is above -1.0. The 8,000-seed White pair
  follows before it is final.
- **The early gauge:** if the best hand's growth from ante 2 to 5, or the cushion at ante 5, moves and the win rate
  doesn't, that goes to P1.
- Nothing from the night is a headline. The look's runs are watched in the morning.

```
cd /mnt/h/balatro-meta && make sync && make test && make night
```

## Step 47: Card Sharp, deck fixing and the calibration recording (about 3.5 hours; S's order, lab #27 and #30)

E1 read Worse on Green (`ahead=2` -5.05, `ahead=3` -7.05 points against batch 1's 31.75%); `ahead` stays at 0 while
its cause is found (lab #27: the value's projection of growth, and the shape of the far blinds' term). The user's rule
from 13 November: a core concept that fails is not dropped; we find why and fix it. White runs are off for now.

New in the tally (`leaks.rs`, `scripts/leaks.py`): Card Sharp held, the rounds in which its x3 fired and the hands it
fired on; Tarots used a run (the user's own runs use 47 to 69).

New in `baseline`: `--calibrate FILE` (E1 step 1, the forecast's dataset): at each round's start the value's measured
power (the mean log total of the next round played out with no target, the measure each later round repeats) and its
projections three, six and nine rounds on, split by source (the jokers' flat chips and mult, hand levels, growing x
mult, the deck, drain, money), with what the round then scored. Play is unchanged by it.

`make green-next`, one resumable job:
1. **`card-sharp`** on top of the base (batch 1's defaults), 2,000 Green seeds at salt 2, against batch 1's Green arm,
   tallied.
2. **`deck-fix`**, the same.
3. **The base again with the calibration recording** on the same seeds, tallied. Neither recording changes play, so
   its compare against batch 1's Green arm is a check: 0.00 points and no seed won by one side only.

```
cd /mnt/h/balatro-meta && make sync && make green-next
```

## Step 48: the look with the new counters, and the calibration to 8,000 Green seeds (about 3.8 hours; S, lab #30)

Read from step 47: `card-sharp` Level (+0.60, -0.09 to +1.29), all of it in the runs holding Card Sharp, and on by default
from 13 November by the user's judgement, not as a measured gain (`no-card-sharp` turns it off; screens' controls are
marked `-d3`); `deck-fix` Level, leaning down (-1.30), off. The calibration reproduced batch 1 seed for seed and shows the
value's growth projection wrong in level (at ante 1 it expects x1.3 over six rounds; the runs grew x7.3) and nearly
useless in ranking (lab #30).

The tally now also counts Tarots, Planets and Spectrals used the moment they were taken (picked from a pack, bought and
used), which it had missed, and packs by kind: offered, affordable, opened, by ante.

`make green-calib`: the look on Green (400 seeds) with those counters, then the calibration recording and the tally on
Green seeds 2,001 to 8,000 at salt 2, the base as it now is.

```
cd /mnt/h/balatro-meta && make sync && make green-calib
```

## Step 49: C8 and C10 on by default; what the bot thinks a pack is worth (lab #30)

- **C8** (`card-tiers`, on by default): `card_worth` ranks the enhancements in the user's order, Glass, then Lucky (more
  with Lucky Cat), then Steel and Gold, then Mult and Bonus. It ranks targets (what Death copies, what a seal goes on,
  what the digging keeps); it adds nothing to the value.
- **C10** (`fool-copies`, on by default): The Fool is used at once when the card it would make is The Hanged Man,
  Death, The Hermit, or Temperance paying over $20. The tally counts what each Fool made.
- C9 as first framed is not in: the deck measure already credits a thinned deck (a test holds it to that).
- `make pack-audit` (about 20 minutes): 300 Green runs with the shop audit at every ante, read by
  `scripts/pack_audit.py`: each pack's worth beyond its price in the value's own dollars, by kind and ante, and what
  the bot did where an Arcana pack beat leaving the shop.

```
cd /mnt/h/balatro-meta && make sync && make pack-audit
```

## Step 50: the pack audit, the new base, and the no-death recording (Green, about 3 hours; S's ruling, lab #27)

`green-calib` stopped at 03:47 on 14 November with 5,690 of its 6,000 calibration runs written (the PC stopped
mid-run); the 5,693 lines in `.sync/calib/base-green-2001-8000-salt2.jsonl` are kept as the forecast's data (the last
may be cut short). It is not run again: the base has changed since (C8, C10).

`make night-d4`:
1. **The pack audit** (step 49), the real bot with its networks.
2. **The new base** (Card Sharp's play, C8's card tiers, C10's Fool, on by default) on batch 1's 2,000 Green seeds,
   tallied, against batch 1's Green arm. S's rule, stated before: Level or Better and it is the base; Worse and the new
   defaults are split to find which.
3. **The no-death recording** (B, lab #30): `baseline --play-on --calibrate` on the same seeds, lost blinds played on
   as if beaten, so every position has its power at every later blind whatever the stake asked. Recording only: the
   report says `"play_on": true`, `compare_runs.py` refuses it and no progress line is written. Each round's record
   carries `lost_before`, the blinds lost before it, and the deck as counts.

```
cd /mnt/h/balatro-meta && git add results && (git commit -qm "results(green-calib): look" || true) && make sync && make night-d4
```

## Step 51: opening packs, and forecast v0 (S, lab #27 and #30)

Read from step 50: the `-d4` base against batch 1, +0.35 points (-0.39 to +1.09), Level: `-d4` is the base. The pack
audit: where the bot left an affordable Arcana pack, the value priced it under its cost (the shop-ordering suspect is
dead); the user opens 86-100% of affordable Arcana packs from ante 2, the bot 19-67%.

- `open-packs` (teacher option): from ante 2, where the value would reroll or leave the shop, an affordable Arcana or
  Spectral pack is opened instead; `open-packs-all` adds Buffoon and Standard packs. Purchases and vouchers the value
  chose come first; picks inside the pack are the value's.
- `forecast` (teacher option, with `baseline --forecast FILE`): every blind after the next round is judged by forecast
  v0's tables (centre and spread of ln power at a later blind, from today's power, by ante and rounds on), in place of
  today's power plus the projection.
- The shop audit carries each pack option's formula part, so `pack_audit.py` splits an Arcana pack's worth into the
  formula's part and the networks' part by ante; it groups shops by key (the audit's lines interleave across threads).
- Batch 1's jobs keep batch 1's defaults in their file names (`BATCH1_DEFAULTS`), which fixes night-d4's compare.

`make green-packs` (about 2.5 hours) and `make green-forecast` (about 3.5 hours), each against the `-d4` base on 2,000
Green seeds.

## Step 52: open-packs on by default, and forecast v0 against it (lab #27, #30)

Read from step 51: `open-packs` +0.80 points (-1.40 to +3.00) against `-d4`, Level, with Arcana packs opened 46% to 87%
of those affordable and Tarots used 7.6 to 11.6 a run: the base by S's rule, on by default (`no-open-packs`; screens
marked `-d5`). `open-packs-all` -2.50 (-4.79 to -0.21), Worse: off.

`forecast` now turns its centre and spread into a chance through S's link (offset, round spread, boss shift, from the
file's `link`). `make green-forecast` plays `forecast`, `forecast,ahead=3` and `forecast,ahead=8` against green-packs'
`open-packs` arm (the `-d5` base on the same 2,000 Green seeds).

```
cd /mnt/h/balatro-meta && git add results && (git commit -qm "results(green-packs)" || true) && make sync && make green-forecast
```

## Later steps

Claude adds each one here when it's ready. Next: a screen that can be extended (`baseline --skip`, so seeds 2,001
to 4,000 add to a screen that came out in between instead of replaying the first 2,000), ready for round 11's
screens. Next: more rounds of the learned value and of the network, and a
search guided by the network at card-play time.

## If something goes wrong

- `make status` shows which steps are done; logs are in `.sync/logs/`.
- `git push` says "non-fast-forward": `make sync` replays your results commits on top of the new code, which rewrites
  their hashes, so GitHub has to be moved to your branch: `git push --force-with-lease origin main` (it refuses if
  GitHub has anything you haven't seen).
- A step failed: paste the last 20 lines of its log into the chat.
- "uncommitted changes": a tracked file was edited. `git stash` puts the edit aside, or ask Claude.
- "detected dubious ownership": run `git config --global --add safe.directory /mnt/h/balatro-meta`.
- "cargo: command not found" in an old terminal: run `source ~/.cargo/env`, or open a new terminal.
- Ubuntu can't see `H:`: check `ls /mnt/h`. If H: is a network drive, tell Claude.
