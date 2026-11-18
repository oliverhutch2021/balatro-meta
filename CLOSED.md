# Closed experiments: what was tried, how it ended, where its code still is

Pruned code is not a dropped idea (lab #34). Every option, job or script taken out of the code is listed
here with the last commit that holds it. To bring one back, take its code from that commit
(`git show COMMIT:path`, or `git checkout COMMIT -- path`). The results stay where they were written
(`results/`, RUNNING.md, the lab issues).

## Options and value numbers (teacher.rs, value.rs, planner.rs, learned.rs)

Added as each Rust prune lands (lab #34, A's half): the option, what it tried, its last result with seeds
and stake, why it closed, and the last commit that holds its code.

| option | what it tried | last result | why closed | last code |
|---|---|---|---|---|

## Jobs, make targets and scripts (scripts/jobs.py, Makefile, scripts/)

Removed in B's prune (lab #34). All of them were one-off runs whose results are written. Each one's
code is in **49698c9** (lab main as the prune began): `scripts/jobs.py`, the `Makefile` target of the
same name, and `tests/python/test_jobs.py`.

| job (make target) | what it ran | results |
|---|---|---|
| `teacher-precise` | "precise" (more samples where decisions are noisiest) against the best teacher's own runs | adopted; `results/value/precise-vs-*.json` |
| `teacher-tune`, `teacher-tune-2` | the value formula's numbers changed one at a time, then the winners together | `results/tune/`, formula version 2 |
| `teacher-ideas`, `teacher-ideas-2`, `teacher-next`, `teacher-focus` | ideas from the user's recorded runs (Dusk, Five of a Kind, Overstock, Midas; deck fixing, Glass, x mult that fits, Card Sharp's x3; one hand's focus) | `results/tune/` |
| `teacher-control`, `teacher-big` | a new round's control; `focus` + `fair-skip` on 8,000 seeds (+1.64, formula version 3) | `results/tune/` |
| `teacher-weights`, `teacher-weights-high` | the networks' weight 1.5 to 16 on 8,000 seeds | `results/tune/`; the champion's weight 2.5 |
| `teacher-doubt`, `teacher-margin` | the networks' disagreement as a brake (`doubt=k`); wins with blinds to spare (`margin=K`) | `results/tune/` |
| `teacher-late`, `teacher-boss`, `teacher-budget*`, `teacher-budget-parts`, `teacher-budget-away`, `teacher-looks`, `teacher-close`, `teacher-close-2` | the search budget: more samples late, at bosses, everywhere, by part, and the adaptive budget | `late-x=3`, `boss-x=4`, `late-from=1` kept in the base; the rest closed (lab #30: "not concepts") |
| `teacher-skips`, `skips-stakes` | skipping blinds for tags, judged by value | Level then; `skips` is a league gene now |
| `diagnose-fresh` | the champion's own runs recorded and diagnosed | `results/diagnose/`; `make diagnose` and `diagnose-control` stay |
| `teacher-bundle` | the champion against round 11's pair with the reroll fix, 8,000 White seeds | `results/tune/` |
| `shop-audit`, `teacher-money-belief` | why late shops are left with money; the networks' belief about money | step 38-40 in RUNNING.md; `freeze-money` −5.6 |
| `leak-tally` | the leak tally and the forced-buy forks | dropped by S (13 Nov); the tally lives on in `make look` |
| `teacher-boss-reroll`, `reroll-demo`, `teacher-held-cards`, `teacher-defaults`, `teacher-batch1` | batch 1 of the master list against step 37's controls | in the base (lab #27) |
| `night`, `night-green`, `green-next`, `green-calib`, `night-d4` | the 13-14 Nov chains: look and gate, batch 1, E1 (`ahead`), Card Sharp, deck-fix, calibration recordings | lab #27, #30 |
| `e1-white-ahead2/3/8` | E1's White guard | White dropped for now (S, 13 Nov) |
| `green-packs`, `green-forecast`, `green-spend` | `open-packs` (Level, base), `open-packs-all` (−2.50), `forecast` (−5.15, −6.80), `spend-down` (−0.45, −2.85) | lab #27, #30 |
| `danger-look`, `league-wall` | antes 4-6: what the bot saw, the spending ceiling (0.205 → 0.208), the wall league on W | lab #33; ledger `results/league/gen-wall.json` |
| `teacher-money`, `teacher-long` | the money numbers and the long run retuned one at a time | superseded by the league's price genes |
| `forks-quick`, `forks-now`, `forks-table`, `forks-N`, `forks-night`, `value-net-forks-N`, `teacher-forks-N` | forks at recorded decisions and the networks that learn from them (the judge test) | `results/forks/`; forks.rs stays for the shop forks |
| `teacher-inputs` | round 11's networks on the deck card by card against the totals | `results/tune/` |
| `stakes-trials`, `stakes-N`, `stakes-night`, `boost-quick`, `boost-N`, `boost-night` | the stake-aware networks and the residual correction | "better calibrated, worse play" (#30); `results/stakes/` |

Scripts removed with them: `forced_buys.py` (the forced-buy forks' summary), `fork_table.py`,
`forks_report.py` (fork judge tables), `quick_table.py` (the boost turn's table), `night_gate.py` and
`gated.py` (the night's gate), `audit_shops.py` (step 38's shop audit; `pack_audit.py` replaced it).
