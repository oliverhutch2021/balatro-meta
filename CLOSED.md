# Closed options

The user's rule: a concept that fails is not dropped, we find out why. Pruning its code is not dropping it, so long as it
stays findable. Each line: the option, what it tried, its last result (seeds, stake), why it closed, and the last
commit on lab main that holds its code (`git show <commit>:<file>` restores it).

Pruned in the code audit (lab #34, 16 November). The last commit holding all of these is `49698c9`.

| option | what it tried | last result | why closed |
|---|---|---|---|
| `forecast`, `--forecast` | blinds after the next round by forecast v0's tables (S's centre and spread), in place of today's power plus the projection | −5.15 (−7.35 to −2.95), 2,000 Green, against `-d4` (lab #30) | it took the growth and money credits out of the value, so the bot stopped paying for them; forecast v1 is a different build (accepted on forks); the tables stay as a Python measuring tool |
| `danger-x` | the long run scaled by the chance of living through the ante; a reroll in danger worth the best of those affordable | −4.05 (−6.34 to −1.76), 2,000 Green, on W (wall league) | ante 4 deaths fell and every later ante's rose; the bot is gloomy, not blind (lab #33) |
| `danger-spend=T` | spending to raise the ante's survival chance when under T | pre-flight only: 11 → 2-3 wins of 150 (light bot) | read the value on two quick samples and emptied the purse on false alarms; the ceiling of spending in danger is nil (0.205 → 0.208 over 226 deaths) |
| `--danger-bench` | the ceiling of a danger response on the bench | 0.205 → 0.208, 226 deaths, 1,000 Green | measured; the danger line is closed (lab #33) |
| `commit-main` | the run's focus hand kept from ante 3 | −0.10 (−0.69 to +0.49), 2,000 Green, on W | changes 2% of seeds: the focus already serves that hand; a real commitment gene (round play favouring the hand) is a new build |
| `commit=p` | a random hand committed to from ante 3 (recordings, whole-build exploration) | round 10, not the idea it was named for (lab #30) | withdrawn |
| `explore`, `explore-within` | recordings that swap among near-best options | withdrawn (lab #30) | stays near the bot's habits; the league gives the spread instead |
| `close-x`, `close-gap`, `close-beats` | the adaptive budget: close decisions judged again with more samples | measured cleanly, "stay closed" (lab #30) | not a concept |
| `freeze-money` | the networks read the money a shop opened with, in every shop | −5.6 (all shops) | removed with nothing in its place; `freeze-money-last` (the last shop) stays in the base |
| `xmult-bonus` | points added for x mult held by ante 6 | −2.5 | its premise is wrong: the bot holds more x-mult jokers than the player (lab #33) |
| `final-hand` | playing a round down to its last hand for Dusk and Acrobat | Level | never promoted; the player calls it good practice, not critical |
| `steady`, `looks`, `steady-rounds`, `flat-*`, `deals-x`, `quick-x`, `looks-x`, `tries-x`, `between-x`, `finalists` | the search budget's studies | measured, closed (steps 33 to 38) | not concepts |
| `static-sells`, `no-juggle`, `classic` | old trials of joker sales, joker order, the classic teacher | superseded | `classic` also misled a reader into taking its code for the bot's |
| stake nets: `StakeMode`, `--stake-net`, `--stake-mode`, `--offsets`, `--offsets-into`, `train_value --offset`, `stake-scale` | networks that know the stake | better calibrated, worse play (steps 25-26) | the wrong split (lab #30, Part 1 point 5) |
| depth head: `--endless`, `margin`, `reach`, `doubt`, `hope` | winning with room to spare, the networks' depth head | learned almost nothing | no run of ours scaled; Endless depth is a gauge in the look, not a goal |
| forced-buy forks: `--force-buy`, `--force-out`, `--force-copies`, `forced_buys.py` | a named joker taken where the run passed it, played out | leak-tally dropped; the league replaces it | |

Kept as league genes for generation 2 (S, lab #34), each with its range: `thin`, `deck-fix` with `fix-dollars`,
`xmult-fit`, `reroll-best`. If generations 2 and 3 never promote them, they go then.
