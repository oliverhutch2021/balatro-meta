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
It can run overnight: Balatro's memory grows over a long session, so the hidden game gets a fresh start
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
     the play as far as one update should. `make watch` shows it live: runs played, the win rate of its last
     2,000 runs, how it plays now against all its runs so far, how it changes block by block, and its runs
     to replay. Ctrl+C stops it after saving; `make train` again carries on where
     it stopped. The network is saved in `models/policy/` (not committed).
  4. **eval**: it plays 5,000 of the usual seeds, the same ones as the teacher's in
     `results/value/teacher-plain.json` and `teacher-w1.json`, and the Progress panel adds it to the chart.

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
