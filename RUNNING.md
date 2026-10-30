# Running the jobs on your PC

Heavy jobs run on your PC (WSL2 Ubuntu), one command each. Claude writes and tests the code, then
says **Your turn** in the chat when a run is ready. This file holds the exact steps.

## How it works

- The repo lives in `H:\balatro-meta`; in Ubuntu that's `/mnt/h/balatro-meta`.
- New code from Claude arrives as `.sync/claude.bundle`. `make sync` applies it, and every job runs it first.
- The engine is Rust. Its build and the Python environment live on the WSL disk
  (`~/.cache/balatro-meta`, `~/.venvs/balatro-meta`), so the Windows drive only holds the code and results.
- Every job can be stopped with Ctrl+C and resumed by running the same command again. Jobs run at low
  priority, so the PC stays usable, just slower.
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

## Step 5 (when you like): watching the bots play in your own Balatro

`make watch` opens the viewer in your browser: your Balatro in the middle as live video (about 30 frames
a second, streamed by the game itself to your browser on this PC only), playing on its own, with live stats
around it (in the style of Monopoly Meta's viewer):

- **Your PC**: the running job and its steps, the engine's speed, and what this viewer has checked.
- **Run outlook**: after every move, quick playouts from that position (hidden cards reshuffled, fresh
  random streams) project where the run ends and how likely it is to clear the current blind. Click or
  drag the chart, or click a move, to see the game as it was after that move. Turning points list the
  moves that changed the outlook most, and what the shop or pack showed.
- **Play by play**, the round (ante, blind, chips, money, hands, discards), jokers with their values,
  consumables, vouchers, tags, the shop or open pack, and hand levels.
- **Engine and game**: every replay is also a check. After every move the game's money, hands, discards,
  cards in hand, round chips, blinds, tags, jokers and consumables (with their sell values), the shop's
  cards, vouchers, packs and prices, and an open pack's cards are compared with the engine's. A difference
  is shown, saved to `.sync/watch/mismatches/`, and the viewer moves on to the next run.

The buttons under the game pause it, set the speed (1x, the game's normal speed, to start with; 2x, 4x or
Max) and skip to the next run. The speed is the visible game's only: the projections run in the engine in
the background at full speed whatever you choose. While the viewer runs, Balatro ignores your mouse and keyboard so nothing disturbs the bots (a badge
in the game's corner says so); it takes input again a few seconds after you stop the viewer. For now the
runs are demo runs by a simple greedy bot that also shops; once jobs play real runs, it replays those.
BalatroBot has no "buy and use" and no boss reroll, so a run stops (without a mismatch) before either.

One-time setup, on Windows:

1. Install uv (PowerShell): `powershell -ExecutionPolicy ByPass -c "irm https://astral.sh/uv/install.ps1 | iex"`
2. Lovely 0.8.0 or newer (`winmm.dll` in `H:\Steam\steamapps\common\Balatro`;
   https://github.com/ethangreen-dev/lovely-injector/releases).
3. Steamodded 1.0.0-beta-1221a or newer (https://github.com/Steamodded/smods/releases) in
   `%AppData%\Balatro\Mods`.
4. BalatroBot (https://github.com/coder/balatrobot/releases) in `%AppData%\Balatro\Mods`.
5. Move every other mod out of `%AppData%\Balatro\Mods` while watching, so the rules stay vanilla.

Each time, in Ubuntu:

```
make watch
```

It rebuilds the engine if the code changed, and installs or updates the viewer's companion mod
(`mods/balatro_meta_watch`) in your Mods folder by itself. If Balatro is already open without it (or with an
older version), the page shows a **Restart Balatro** button; otherwise it opens Balatro through BalatroBot.
Ctrl+C stops the viewer; the game stays open. `make watch SPEED=4` starts at another speed; the page can
change it any time. The live video comes from a small server inside the game on `127.0.0.1:13346` (your PC
only); if your browser can't reach it, the page falls back to a few pictures a second by itself.

BalatroBot listens on Windows' own localhost, which Ubuntu can't reach by default, so `make watch` talks to
it through a small relay it runs with your Windows Python (the same miniconda the RNG check uses). If the
page keeps waiting for the game although Balatro is open, look in Balatro's Lovely log
(`%AppData%\Balatro\Mods\lovely\log`, newest file) for `BB.SERVER :: HTTP server listening`.
`make watch-dry` replays runs in the engine only, printed in the terminal.

## Later steps

Claude adds each one here when it's ready: checks against the real game with BalatroBot (phase 3), and
training (phase 4). Phase 2 (the full game: all jokers, consumables, vouchers, tags, the shop and packs) is
in the engine now; `make smoke` checks it.

## If something goes wrong

- `make status` shows which steps are done; logs are in `.sync/logs/`.
- A step failed: paste the last 20 lines of its log into the chat.
- "uncommitted changes": a tracked file was edited. `git stash` puts the edit aside, or ask Claude.
- "detected dubious ownership": run `git config --global --add safe.directory /mnt/h/balatro-meta`.
- "cargo: command not found" in an old terminal: run `source ~/.cargo/env`, or open a new terminal.
- Ubuntu can't see `H:`: check `ls /mnt/h`. If H: is a network drive, tell Claude.
