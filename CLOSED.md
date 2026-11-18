# Closed experiments: what was tried, how it ended, where its code still is

Pruned code is not a dropped idea (lab #34). Every option, job or script taken out of the code is listed
here with the last commit that holds it. To bring one back, take its code from that commit
(`git show COMMIT:path`, or `git checkout COMMIT -- path`). The results stay where they were written
(`results/`, RUNNING.md, the lab issues).

## Options and value numbers (teacher.rs, value.rs, planner.rs, learned.rs)

Closed in the code audit (lab #34, 16 November; S's record). Each option's code is still on lab main until A's
subsystem prune takes it out; the last commit holding all of them is `49698c9`. The user's rule: a concept that fails
is not dropped, we find out why, so each line says why it closed.

| option | what it tried | last result | why closed | last code |
|---|---|---|---|---|
| `forecast`, `--forecast` | blinds after the next round by forecast v0's tables (S's centre and spread), in place of today's power plus the projection | −5.15 (−7.35 to −2.95), 2,000 Green, against `-d4` (lab #30) | it took the growth and money credits out of the value, so the bot stopped paying for them; forecast v1 is a different build (accepted on forks); the tables stay as a Python measuring tool | `49698c9` |
| `danger-x` | the long run scaled by the chance of living through the ante; a reroll in danger worth the best of those affordable | −4.05 (−6.34 to −1.76), 2,000 Green, on W (wall league) | ante 4 deaths fell and every later ante's rose; the bot is gloomy, not blind (lab #33) | `49698c9` |
| `danger-spend=T` | spending to raise the ante's survival chance when under T | pre-flight only: 11 → 2-3 wins of 150 (light bot) | read the value on two quick samples and emptied the purse on false alarms; the ceiling of spending in danger is nil (0.205 → 0.208 over 226 deaths) | `49698c9` |
| `--danger-bench` | the ceiling of a danger response on the bench | 0.205 → 0.208, 226 deaths, 1,000 Green | measured; the danger line is closed (lab #33) | `49698c9` |
| `commit-main` | the run's focus hand kept from ante 3 | −0.10 (−0.69 to +0.49), 2,000 Green, on W | changes 2% of seeds: the focus already serves that hand; a real commitment gene (round play favouring the hand) is a new build | `49698c9` |
| `commit=p` | a random hand committed to from ante 3 (recordings, whole-build exploration) | round 10, not the idea it was named for (lab #30) | withdrawn | `49698c9` |
| `explore`, `explore-within` | recordings that swap among near-best options | withdrawn (lab #30) | stays near the bot's habits; the league gives the spread instead | `49698c9` |
| `close-x`, `close-gap`, `close-beats` | the adaptive budget: close decisions judged again with more samples | measured cleanly, "stay closed" (lab #30) | not a concept | `49698c9` |
| `freeze-money` | the networks read the money a shop opened with, in every shop | −5.6 (all shops) | removed with nothing in its place; `freeze-money-last` (the last shop) stays in the base | `49698c9` |
| `xmult-bonus` | points added for x mult held by ante 6 | −2.5 | its premise is wrong: the bot holds more x-mult jokers than the player (lab #33) | `49698c9` |
| `final-hand` | playing a round down to its last hand for Dusk and Acrobat | Level | never promoted; the player calls it good practice, not critical | `49698c9` |
| `steady`, `looks`, `steady-rounds`, `flat-*`, `deals-x`, `quick-x`, `looks-x`, `tries-x`, `between-x`, `finalists` | the search budget's studies | measured, closed (steps 33 to 38) | not concepts | `49698c9` |
| `static-sells`, `no-juggle`, `classic` | old trials of joker sales, joker order, the classic teacher | superseded | `classic` also misled a reader into taking its code for the bot's | `49698c9` |
| stake nets: `StakeMode`, `--stake-net`, `--stake-mode`, `--offsets`, `--offsets-into`, `train_value --offset`, `stake-scale` | networks that know the stake | better calibrated, worse play (steps 25-26) | the wrong split (lab #30, Part 1 point 5) | `49698c9` |
| depth head: `--endless`, `margin`, `reach`, `doubt`, `hope` | winning with room to spare, the networks' depth head | learned almost nothing | no run of ours scaled; Endless depth is a gauge in the look, not a goal | `49698c9` |
| forced-buy forks: `--force-buy`, `--force-out`, `--force-copies`, `forced_buys.py` | a named joker taken where the run passed it, played out | leak-tally dropped; the league replaces it | | `49698c9` |

Kept as league genes for generation 2 (S, lab #34), each with its range: `thin`, `deck-fix` with `fix-dollars`,
`xmult-fit`, `reroll-best`. If generations 2 and 3 never promote them, they go then.

Dead code removed in prune 1 (no caller anywhere; `49698c9`): `learned::win_chance`, `shop::booster_kind`,
`poker::game_order`, `scenario::clear_hand`, `observe::joker_tokens`, `streams::stream_value`.

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
