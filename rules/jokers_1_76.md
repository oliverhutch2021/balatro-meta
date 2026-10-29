# Jokers in game order 1 to 76 (Joker … Midas Mask)

This note gives the complete behaviour, in Balatro 1.0.1o, of the jokers that open
the game's joker list, from Joker to Midas Mask. For each one it covers every moment
it acts, the exact condition and numbers, every random stream it draws (key string,
moment, comparison), whether Blueprint and Brainstorm copy each part, when its state
changes land, and how it meets debuffs, Stone cards, Wild cards, Smeared Joker and
Pareidolia. The text is in our own words. Only numbers, keys, stream names and the
names of the game's functions and contexts are taken from the scripts.
It uses the terms of `scoring.md` (sections 3 to 6: 'before' pass, played-card loop,
held-card loop, joker loop, 'after' pass) and refers to `hands.md`, `rng_keys.md`,
`shop.md`, `round.md`, `timing.md` and `blinds.md` rather than repeating them.

**Scope and numbering.** The brief listed 75 jokers ending with "75 Midas Mask" and
had no Obelisk. In the game's data, Obelisk (`j_obelisk`) has order 75 and Midas
Mask has order 76; every other entry of the brief matches the game. This file covers
all 76 and numbers them by the game's `order` field. That field is also the pool
order used when cards are created (`shop.md` 2.7), so the numbering matters for
seeds.

## 0. Conventions

- **p** = `G.GAME.probabilities.normal`: 1 at run start, doubled by each
  non-debuffed Oops! All 6s (and halved again when one leaves or is debuffed). A
  roll "1 in n on stream `s`" draws `s` once and succeeds when the value is
  **strictly below p / n**. p is read at the moment of the roll.
- **Copyable** means a Blueprint or Brainstorm copying this joker also produces that
  effect. The game decides this per effect inside the joker routine (an effect is
  skipped when the call is a copy). The data table's `blueprint_compat` flag only
  sets the "compatible / incompatible" label on Blueprint and Brainstorm and changes
  nothing else (section 1.3).
- **Debuffed joker**: the joker routine returns at once for a debuffed joker, so it
  does nothing in any context, and a copy of it does nothing either. Exceptions:
  per-frame values are still refreshed, and debuffing undoes its add-to-deck
  effects (section 1.4).
- **Scoring card / kicker / held card / play order**: as in `hands.md` and
  `scoring.md`. Debuffed scoring cards are skipped by the played-card loop
  (`scoring.md` 4.1), so a per-card joker effect never sees a debuffed card.
- **Pending money** = `G.GAME.dollar_buffer`, the running total of money earned
  during the current evaluation but not yet added (`scoring.md` 13.1).
- **Consumable room** = held consumables + pending creations
  (`G.GAME.consumeable_buffer`) < consumable slots (`scoring.md` 13.2).
- **A** = the current ante written as an integer, used in stream keys (`rng_keys.md`).
- Context names in brackets are the game's own (`before`, `individual`,
  `repetition`, `joker_main`, `other_joker`, `after`, `debuffed_hand`,
  `setting_blind`, `end_of_round`, ...).

## 1. Shared mechanics

### 1.1 How one call is dispatched

Each call of the joker routine (`Card:calculate_joker`) handles exactly one context
family. After the Blueprint/Brainstorm step, the flags are tested in this order and
only the first match is handled: `open_booster`, `buying_card`, `selling_self`,
`selling_card`, `reroll_shop`, `ending_shop`, `skip_blind`, `skipping_booster`,
`playing_card_added`, `first_hand_drawn`, `setting_blind`, `destroying_card`,
`cards_destroyed`, `remove_playing_cards`, `using_consumeable`, `debuffed_hand`,
`pre_discard`, `discard`, `end_of_round`, `individual`, `repetition`, `other_joker`,
and finally the joker-area contexts: `before`, `after`, otherwise the main context
(`joker_main`).

Consequence: the held-card calls made at the end of a round carry both the
end-of-round flag and the "individual" flag. They fall into the end-of-round family,
whose individual branch is empty, so Baron and Raised Fist never act at the end of a
round. Only Mime acts there, in the repetition branch.

### 1.2 Generic branches at the top of the main context

In the main context the routine first runs Loyalty Card's own check, then three
generic branches that apply to every joker, then the named checks. The first branch
that returns ends the call, so a joker gives at most one main result.

1. **Generic X-mult.** If the joker is not Seeing Double, its stored `x_mult` is
   greater than 1, and its stored hand type is empty or contained in the played
   hand, it returns X(`x_mult`). Jokers in this file that rely on it: Joker Stencil
   (17), Constellation (55), Madness (64), Vampire (68), Hologram (70) and Obelisk
   (75). All six have an empty type, so only "x_mult > 1" matters. A joker that
   passes this branch never reaches its own named check (this is why Joker Stencil's
   own branch is never used; see Joker Stencil, number 17, in section 3).
2. **Generic type-mult.** If its stored `t_mult` > 0 and the played hand contains its
   type: +`t_mult` mult. Used by Jolly, Zany, Mad, Crazy and Droll (6 to 10).
3. **Generic type-chips.** If its stored `t_chips` > 0 and the hand contains its
   type: +`t_chips` chips. Used by Sly, Wily, Clever, Devious and Crafty (11 to 15).

"Contains" means the hand table entry for that type is non-empty (`hands.md`
section 4), computed from all played cards, not only the scoring ones.

### 1.3 Blueprint and Brainstorm

- Blueprint copies the joker directly to its right; Brainstorm copies the leftmost
  joker. Neither copies itself. A copy calls the copied joker's routine with a copy
  marker. The result is credited to the copying card and applied at the copying
  card's place in the joker order (for per-card, held-card and repetition contexts:
  at its place among the jokers for that card and trigger). Chains (Blueprint →
  Blueprint → X) work; a depth guard stops loops.
- Every effect written as "not copyable" below checks the copy marker. A copy of a
  debuffed joker gives nothing, since the copied joker's routine returns at once.
- Copies that roll draw the same stream again, at the copying card's own place in
  joker order. A Blueprint always sits directly to the **left** of the joker it
  copies, so the Blueprint's draw comes **before** the original's. A Brainstorm
  copies the leftmost joker and sits to its right, so its draw comes after. (Note:
  `rng_keys.md` 3.7 says a copy always draws "right after the original"; that holds
  for Brainstorm, not for Blueprint.) Room checks for creations are made at each
  one's turn, so whichever acts later sees the earlier one's pending creation.
- At blind start (`setting_blind`) two marks are checked: a copied joker that is
  already marked for destruction does nothing, and Burglar, Marble Joker and
  Riff-raff also do nothing when the copying card itself is marked (section 1.8).

### 1.4 Debuffed jokers

- Debuffed jokers do nothing in any context (scoring, discards, blind start, end of
  round, shop, cash-out bonus). The passive jokers in this file (Four Fingers,
  Pareidolia, Splash, Shortcut) are found through the game's joker search, which
  skips debuffed jokers, so they also stop working.
- When a joker in the joker area becomes debuffed, the game runs its
  remove-from-deck effects; when the debuff ends, its add-to-deck effects run again.
  In this file that matters for Credit Card (bankruptcy floor) and Chaos the Clown
  (free rerolls).
- Every time a joker enters or leaves the joker area, or is debuffed or un-debuffed
  there, the game also queues a fresh boss debuff test of every playing card
  (`Blind:set_blind` in reset mode). A Negative joker's extra slot is added or
  removed only when it really enters or leaves, not when it is debuffed.
- Per-frame values (Joker Stencil, Steel Joker, Cloud 9) are still refreshed while
  debuffed, but are only used when the joker is not debuffed.
- Sources of joker debuffs: Crimson Heart (one joker, re-picked after each draw; the
  last pick lasts through the end-of-round effects and the cash-out rows, because
  the blind is only cleared after the cash-out rows are computed), and Perishable
  running out (permanent).
- A Perishable joker's countdown runs in the end-of-round step, right after that
  joker's own end-of-round effect. On its last round it still does its end-of-round
  effect, then becomes debuffed, so it pays no cash-out bonus that same round
  (relevant for Delayed Gratification, Cloud 9 and Rocket).

### 1.5 Values refreshed every frame

`Card:update` recomputes these every frame during a run, for every copy of the card
wherever it sits (debuffed or not):

| Joker | Stored value |
|---|---|
| Joker Stencil | `x_mult` = (joker slots − jokers in the joker area) + number of Joker Stencils in the joker area |
| Steel Joker | `steel_tally` = cards in the full deck (`G.playing_cards`) whose enhancement is Steel |
| Cloud 9 | `nine_tally` = cards in the full deck whose rank id is 9 (Stone cards never count) |

The score evaluation runs inside one queued step, so these hold the value of the
frame before the evaluation. Changes made during the 'before' pass (Vampire and
Midas Mask changing enhancements, DNA adding a card) reach them only from the next
hand. The engine may compute them on demand from the state before the step.

### 1.6 Floating point: keep running totals

Growing X-mult values are stored as doubles and grow by repeated addition:
Constellation (+0.1 each Planet), Obelisk (+0.2), Vampire (+0.1 × cards stripped, the
product computed first, then added), Madness (+0.5), Hologram (+0.25 × cards added).
The engine must store the running value and add each increment in the same order,
never recompute 1 + 0.1 × n: repeated binary additions of 0.1 or 0.2 differ from the
product in the last bits, and that reaches the final floor(chips × mult). Steel
Joker's factor is computed fresh each time as 1 + 0.2 × tally.

### 1.7 Initial ability values (set when the card is created)

Every card gets its stored values from the data table's `config` when it is created
(`Card:set_ability`): `mult` from config mult (else 0), `x_mult` from config Xmult
(else 1), `t_mult`, `t_chips` (else 0), `extra` = a copy of config extra, `type`
from config type (else empty), sell bonus `extra_value` = 0, and
`hands_played_at_create` = the run's hands-played counter at that moment. Hand size
and discard size are 0 for all 76 jokers here. Two jokers in this file get more at
creation: Loyalty Card (counter state) and To Do List (its target hand, a random
draw). The per-joker sections list each starting state.

Copies made by `copy_card` (Ankh, Invisible Joker) first run creation as a new card
(including To Do List's random draw), then overwrite every stored value with the
original's. A copy therefore keeps the original's Loyalty phase, growth, sell bonus
and target hand.

All 76 jokers here are unlocked from the start.

### 1.8 Blind-start marks and the joker buffer

- When Ceremonial Dagger or Madness chooses a joker to destroy at blind start, that
  joker is marked at once (`getting_sliced`). For the rest of that blind start a
  marked joker does nothing: every `setting_blind` effect is skipped for it. Dagger
  will not target a marked joker and Madness will not pick one. Its removal is
  queued: it dissolves and is removed about 1.2 s (game time) later, and its
  remove-from-deck effects run then. A marked Hologram also ignores playing cards
  added (`playing_card_added`).
- `G.GAME.joker_buffer` is read only by Riff-raff: "free slots" = slots − (jokers in
  the area + buffer). Dagger lowers the buffer by 1 when it marks its victim (so a
  Riff-raff to its right sees the slot as free). Riff-raff raises it by the number of
  jokers it will create. Their queued steps set it back to 0. Madness does not touch
  the buffer, so its victim still counts as occupying a slot for Riff-raff.

### 1.9 Money, creations and cards added (timing)

| Joker | What | When it is decided | When it lands |
|---|---|---|---|
| Business Card | +$2 | played-card loop, per trigger | queued; pending money +2 at once |
| To Do List | +$4 | 'before' pass | queued; pending money +4 at once |
| Faceless Joker | +$5 | discard, on the last discarded card | two queued hops after the discard |
| Delayed Gratification, Cloud 9, Rocket | cash-out rows | when the cash-out screen computes its rows | paid with Cash Out |
| 8 Ball | Tarot `8ba` | played-card loop (room check, roll) | creation queued when the effect is applied; runs after the evaluation |
| Vagabond, Superposition, Seance | Tarot `vag`, Tarot `sup`, Spectral `sea` | joker loop | creation queued at once; runs after the evaluation |
| Sixth Sense | Spectral `sixth` | destruction step | queued; runs after the evaluation |
| Riff-raff | up to 2 Common jokers `rif` | blind start | queued step |
| Marble Joker | 1 Stone card | blind start | queued steps, before the round's deck shuffle |
| DNA | copy of the played card | 'before' pass | at once, into the hand |

Creations run in the order they were queued (`scoring.md` 13.2). Consumables created
by these jokers never become The Soul or Black Hole: they are created without the
Soul check, so no `soul_` stream is drawn, and Spectral picks that land on the Soul
or Black Hole placeholders resample. Pool keys: `Tarot8ba` + A, `Tarotvag` + A,
`Tarotsup` + A, `Spectralsea` + A, `Spectralsixth` + A, then `_resample` + n
(`rng_keys.md` 1). Consumables get no edition or sticker draws.

### 1.10 Card tests used in this file

- **Scoring suit test** (suit jokers 2 to 5): see `scoring.md` 12.8. A debuffed card
  has no suit (never reached here), a Stone card has none, a Wild card has every
  suit, with a non-debuffed Smeared Joker a card has both suits of its colour,
  otherwise its printed suit.
- **Flush-style suit test** (Blackboard only), first rule that applies: Stone → no;
  Wild and not debuffed → yes; Smeared Joker active → same colour as the asked suit;
  otherwise printed suit. Debuff is otherwise ignored (a debuffed Spade still counts
  as a Spade).
- **Face test** (Scary Face, Business Card, Ride the Bus, Faceless Joker, Midas
  Mask): a debuffed card is never a face. Otherwise a card is a face if it is a
  Jack, Queen or King, or if a non-debuffed Pareidolia is held (then every card,
  Stone cards included). Boss checks use the same test with debuff ignored.
- **Rank id** (Fibonacci, Even Steven, Odd Todd, Scholar, 8 Ball, Hack, Baron,
  Superposition, Sixth Sense, Cloud 9): 2 to 10 as printed, J 11, Q 12, K 13, A 14.
  Debuff is ignored. A Stone card returns a fresh large negative number each time,
  so it never matches any rank. Raised Fist reads the printed rank directly and
  skips Stone cards explicitly.

## 2. Overview

Rarity: C = Common, U = Uncommon, R = Rare. Cost = base cost (`shop.md` 5.1 for
prices). E / P = whether the card can carry the Eternal / Perishable sticker (data
flags; any joker can be Rental). Contexts: B = 'before', PC = played-card loop
(per scoring card, per trigger), HC = held-card loop, RP = repetition of played
cards, RH = repetition of held cards (scoring), M = main (joker loop), A = 'after',
SB = blind start (`setting_blind`), D = discard, EoR = end of round, CO = cash-out
row, other = named.

| # | Key | Name | R | $ | E | P | Acts in (copyable parts in bold) | Random streams |
|---|---|---|---|---|---|---|---|---|
| 1 | `j_joker` | Joker | C | 2 | y | y | **M** | — |
| 2 | `j_greedy_joker` | Greedy Joker | C | 5 | y | y | **PC** | — |
| 3 | `j_lusty_joker` | Lusty Joker | C | 5 | y | y | **PC** | — |
| 4 | `j_wrathful_joker` | Wrathful Joker | C | 5 | y | y | **PC** | — |
| 5 | `j_gluttenous_joker` | Gluttonous Joker | C | 5 | y | y | **PC** | — |
| 6 | `j_jolly` | Jolly Joker | C | 3 | y | y | **M** (generic) | — |
| 7 | `j_zany` | Zany Joker | C | 4 | y | y | **M** (generic) | — |
| 8 | `j_mad` | Mad Joker | C | 4 | y | y | **M** (generic) | — |
| 9 | `j_crazy` | Crazy Joker | C | 4 | y | y | **M** (generic) | — |
| 10 | `j_droll` | Droll Joker | C | 4 | y | y | **M** (generic) | — |
| 11 | `j_sly` | Sly Joker | C | 3 | y | y | **M** (generic) | — |
| 12 | `j_wily` | Wily Joker | C | 4 | y | y | **M** (generic) | — |
| 13 | `j_clever` | Clever Joker | C | 4 | y | y | **M** (generic) | — |
| 14 | `j_devious` | Devious Joker | C | 4 | y | y | **M** (generic) | — |
| 15 | `j_crafty` | Crafty Joker | C | 4 | y | y | **M** (generic) | — |
| 16 | `j_half` | Half Joker | C | 5 | y | y | **M** | — |
| 17 | `j_stencil` | Joker Stencil | U | 8 | y | y | per frame; **M** (generic X) | — |
| 18 | `j_four_fingers` | Four Fingers | U | 7 | y | y | passive (detection) | — |
| 19 | `j_mime` | Mime | U | 5 | y | y | **RH**, **EoR held repetition** | — |
| 20 | `j_credit_card` | Credit Card | C | 1 | y | y | add/remove (floor) | — |
| 21 | `j_ceremonial` | Ceremonial Dagger | U | 6 | y | n | SB; **M** | — |
| 22 | `j_banner` | Banner | C | 5 | y | y | **M** | — |
| 23 | `j_mystic_summit` | Mystic Summit | C | 5 | y | y | **M** | — |
| 24 | `j_marble` | Marble Joker | U | 6 | y | y | **SB** | `marb_fr` |
| 25 | `j_loyalty_card` | Loyalty Card | U | 5 | y | y | **M** | — |
| 26 | `j_8_ball` | 8 Ball | C | 5 | y | y | **PC** | `8ball`, creation `Tarot8ba`+A |
| 27 | `j_misprint` | Misprint | C | 4 | y | y | **M** | `misprint` |
| 28 | `j_dusk` | Dusk | U | 5 | y | y | **RP** | — |
| 29 | `j_raised_fist` | Raised Fist | C | 5 | y | y | **HC** | — |
| 30 | `j_chaos` | Chaos the Clown | C | 4 | y | y | add/remove, round start (free rerolls) | — |
| 31 | `j_fibonacci` | Fibonacci | U | 8 | y | y | **PC** | — |
| 32 | `j_steel_joker` | Steel Joker | U | 7 | y | y | per frame; **M** | — |
| 33 | `j_scary_face` | Scary Face | C | 4 | y | y | **PC** | — |
| 34 | `j_abstract` | Abstract Joker | C | 4 | y | y | **M** | — |
| 35 | `j_delayed_grat` | Delayed Gratification | C | 4 | y | y | CO | — |
| 36 | `j_hack` | Hack | U | 6 | y | y | **RP** | — |
| 37 | `j_pareidolia` | Pareidolia | U | 5 | y | y | passive (face test) | — |
| 38 | `j_gros_michel` | Gros Michel | C | 5 | n | y | **M**; EoR | `gros_michel` |
| 39 | `j_even_steven` | Even Steven | C | 4 | y | y | **PC** | — |
| 40 | `j_odd_todd` | Odd Todd | C | 4 | y | y | **PC** | — |
| 41 | `j_scholar` | Scholar | C | 4 | y | y | **PC** | — |
| 42 | `j_business` | Business Card | C | 4 | y | y | **PC** | `business` |
| 43 | `j_supernova` | Supernova | C | 5 | y | y | **M** | — |
| 44 | `j_ride_the_bus` | Ride the Bus | C | 6 | y | n | B; **M** | — |
| 45 | `j_space` | Space Joker | U | 5 | y | y | **B** | `space` |
| 46 | `j_egg` | Egg | C | 4 | y | y | EoR | — |
| 47 | `j_burglar` | Burglar | U | 6 | y | y | **SB** | — |
| 48 | `j_blackboard` | Blackboard | U | 6 | y | y | **M** | — |
| 49 | `j_runner` | Runner | C | 5 | y | n | B; **M** | — |
| 50 | `j_ice_cream` | Ice Cream | C | 5 | n | y | **M**; A | — |
| 51 | `j_dna` | DNA | R | 8 | y | y | **B** | — |
| 52 | `j_splash` | Splash | C | 3 | y | y | passive (scoring cards) | — |
| 53 | `j_blue_joker` | Blue Joker | C | 5 | y | y | **M** | — |
| 54 | `j_sixth_sense` | Sixth Sense | U | 6 | y | y | destruction step | creation `Spectralsixth`+A |
| 55 | `j_constellation` | Constellation | U | 6 | y | n | consumable used; **M** (generic X) | — |
| 56 | `j_hiker` | Hiker | U | 5 | y | y | **PC** | — |
| 57 | `j_faceless` | Faceless Joker | C | 4 | y | y | **D** | — |
| 58 | `j_green_joker` | Green Joker | C | 4 | y | n | B; D; **M** | — |
| 59 | `j_superposition` | Superposition | C | 4 | y | y | **M** | creation `Tarotsup`+A |
| 60 | `j_todo_list` | To Do List | C | 4 | y | y | creation; **B**; EoR | `to_do` |
| 61 | `j_cavendish` | Cavendish | C | 4 | n | y | **M**; EoR | `cavendish` |
| 62 | `j_card_sharp` | Card Sharp | U | 6 | y | y | **M** | — |
| 63 | `j_red_card` | Red Card | C | 5 | y | n | pack skipped; **M** | — |
| 64 | `j_madness` | Madness | U | 7 | y | n | SB; **M** (generic X) | `madness` |
| 65 | `j_square` | Square Joker | C | 4 | y | n | B; **M** | — |
| 66 | `j_seance` | Seance | U | 6 | y | y | **M** | creation `Spectralsea`+A |
| 67 | `j_riff_raff` | Riff-raff | C | 6 | y | y | **SB** | creation `Joker1rif`+A, `edirif`+A |
| 68 | `j_vampire` | Vampire | U | 7 | y | n | B; **M** (generic X) | — |
| 69 | `j_shortcut` | Shortcut | U | 7 | y | y | passive (detection) | — |
| 70 | `j_hologram` | Hologram | U | 7 | y | n | card added; **M** (generic X) | — |
| 71 | `j_vagabond` | Vagabond | R | 8 | y | y | **M** | creation `Tarotvag`+A |
| 72 | `j_baron` | Baron | R | 8 | y | y | **HC** | — |
| 73 | `j_cloud_9` | Cloud 9 | U | 7 | y | y | per frame; CO | — |
| 74 | `j_rocket` | Rocket | U | 6 | y | n | EoR; CO | — |
| 75 | `j_obelisk` | Obelisk | R | 8 | y | n | B; **M** (generic X) | — |
| 76 | `j_midas_mask` | Midas Mask | U | 7 | y | y | B | — |

Pool notes (`shop.md` 2.3): Steel Joker is offered only while at least one Steel
card is in the full deck. Gros Michel is no longer offered once it has gone extinct
in this run. Cavendish is offered only after that.

Contexts where **no** joker of this file acts: `open_booster`, `buying_card`,
`selling_self`, `selling_card`, `reroll_shop`, `ending_shop`, `skip_blind`,
`cards_destroyed`, `remove_playing_cards`, `debuffed_hand`, `pre_discard`,
joker-on-joker (`other_joker`). In `first_hand_drawn` only DNA reacts, with a
cosmetic flash.

### 2.1 By context (for the engine's dispatch)

When the boss debuffs the whole hand (The Psychic, The Eye, The Mouth), the 'before'
pass, the played-card, held-card and repetition loops, the joker loop and the
destruction step are all skipped. Only the 'after' pass runs (`scoring.md` 10).

| Context | Jokers of this file, with the copyable ones marked * |
|---|---|
| 'before' (non-debuffed hands only) | Ride the Bus, Space Joker*, Runner, DNA*, Green Joker, To Do List*, Square Joker, Vampire, Obelisk, Midas Mask |
| played-card loop (per trigger) | Greedy*, Lusty*, Wrathful*, Gluttonous*, 8 Ball*, Fibonacci*, Scary Face*, Even Steven*, Odd Todd*, Scholar*, Business Card*, Hiker* |
| held-card loop (per pass) | Raised Fist*, Baron* |
| repetition, played cards | Dusk*, Hack* |
| repetition, held cards (scoring and end of round) | Mime* |
| main (joker loop) | Joker*, Jolly…Crafty*, Half*, Stencil*, Ceremonial Dagger*, Banner*, Mystic Summit*, Loyalty Card*, Misprint*, Steel Joker*, Abstract*, Gros Michel*, Supernova*, Ride the Bus*, Blackboard*, Runner*, Ice Cream*, Blue Joker*, Constellation*, Green Joker*, Superposition*, Cavendish*, Card Sharp*, Red Card*, Madness*, Square*, Seance*, Vampire*, Hologram*, Vagabond*, Obelisk* |
| destruction step | Sixth Sense |
| 'after' (every hand, debuffed ones too) | Ice Cream |
| blind start | Ceremonial Dagger, Marble Joker*, Burglar*, Madness, Riff-raff* |
| discard (each discard action, The Hook's too) | Faceless Joker*, Green Joker |
| end of round (joker loop) | Gros Michel, Egg, To Do List, Cavendish, Rocket (none copyable) |
| consumable used | Constellation |
| playing card added | Hologram |
| pack skipped | Red Card |
| cash-out rows | Delayed Gratification, Cloud 9, Rocket |
| add to / remove from the joker area | Credit Card, Chaos the Clown |
| round start (not a joker context) | Chaos the Clown (free rerolls reset) |
| hand detection (passive) | Four Fingers, Shortcut, Splash; face test: Pareidolia |

## 3. The jokers

### 1. Joker (`j_joker`)

- Common, $2. Start: mult 4.
- **Main:** +4 mult on every non-debuffed hand, no condition. Copyable.
- Acts nowhere else.
- Joker is also what card creation falls back to when a joker pool has no available
  entry (`shop.md` 2.3, "Fallback").

### 2. Greedy Joker (`j_greedy_joker`)

- Common, $5. Start: extra = {per-card mult 3, suit Diamonds}.
- **Played-card loop:** on each trigger of each non-debuffed scoring card that passes
  the scoring suit test for **Diamonds**, +3 mult (the per-card result's mult, applied
  after the card's own effects, in joker order). Copyable. No randomness, no state.
- Stone: never. Wild: always (also counts for the other three suit jokers). Smeared
  Joker: Hearts also count. Kickers do not count, except with Splash (all played
  cards score). Retriggers repeat it.
- The game matches the four suit jokers (2 to 5) by their shared effect label
  "Suit Mult", not by name; the suit comes from the stored extra.

### 3. Lusty Joker (`j_lusty_joker`)

- Common, $5. Same as Greedy Joker with suit **Hearts**: +3 mult per trigger of each
  scoring Heart. With Smeared Joker, Diamonds also count. Wild cards count. Stone
  cards never. Copyable.

### 4. Wrathful Joker (`j_wrathful_joker`)

- Common, $5. Same as Greedy Joker with suit **Spades**: +3 mult per trigger of each
  scoring Spade. With Smeared Joker, Clubs also count. Wild yes, Stone never.
  Copyable.

### 5. Gluttonous Joker (`j_gluttenous_joker`)

- Common, $5. Same as Greedy Joker with suit **Clubs**: +3 mult per trigger of each
  scoring Club. With Smeared Joker, Spades also count. Wild yes, Stone never.
  Copyable. (The key is spelled `gluttenous`.)

### Shared rules for 6 to 15 (type jokers)

These ten have no handling of their own in the joker routine. They use the generic
main branches (section 1.2) with their stored type. "The hand contains T" is decided from all played cards
(`hands.md` 4). With r = number of non-Stone played cards of one rank:

| Type | Contained when |
|---|---|
| Pair | some rank has r ≥ 2 (Pair, Two Pair, Three/Four/Five of a Kind, Full House, Flush House, Flush Five, and any Flush or Four-Fingers Straight that also repeats a rank) |
| Three of a Kind | some rank has r ≥ 3 |
| Two Pair | two different ranks with r = 2, or one rank with r = 3 and another with r = 2 (never Four or Five of a Kind) |
| Straight | a straight exists (Straight, Straight Flush), with Four Fingers and Shortcut as in `hands.md` 3.3 |
| Flush | a flush exists (Flush, Straight Flush, Flush House, Flush Five), with Four Fingers, Wild cards and Smeared Joker as in `hands.md` 3.2 |

All ten are copyable, act only in the main context of non-debuffed hands, and have
no state or randomness. Stone cards never help form a type. Debuffed cards still do,
except that a debuffed Wild card loses its wildness for flushes.

### 6. Jolly Joker (`j_jolly`)

- Common, $3. Start: type-mult 8, type Pair.
- **Main:** +8 mult if the hand contains a Pair. Copyable.

### 7. Zany Joker (`j_zany`)

- Common, $4. Start: type-mult 12, type Three of a Kind.
- **Main:** +12 mult if the hand contains Three of a Kind. Copyable.

### 8. Mad Joker (`j_mad`)

- Common, $4. Start: type-mult 10, type Two Pair.
- **Main:** +10 mult if the hand contains Two Pair (a Full House does; Four of a
  Kind does not). Copyable.

### 9. Crazy Joker (`j_crazy`)

- Common, $4. Start: type-mult 12, type Straight.
- **Main:** +12 mult if the hand contains a Straight. Copyable.

### 10. Droll Joker (`j_droll`)

- Common, $4. Start: type-mult 10, type Flush.
- **Main:** +10 mult if the hand contains a Flush. Copyable.

### 11. Sly Joker (`j_sly`)

- Common, $3. Start: type-chips 50, type Pair.
- **Main:** +50 chips if the hand contains a Pair. Copyable.

### 12. Wily Joker (`j_wily`)

- Common, $4. Start: type-chips 100, type Three of a Kind.
- **Main:** +100 chips if the hand contains Three of a Kind. Copyable.

### 13. Clever Joker (`j_clever`)

- Common, $4. Start: type-chips 80, type Two Pair.
- **Main:** +80 chips if the hand contains Two Pair. Copyable.

### 14. Devious Joker (`j_devious`)

- Common, $4. Start: type-chips 100, type Straight.
- **Main:** +100 chips if the hand contains a Straight. Copyable.

### 15. Crafty Joker (`j_crafty`)

- Common, $4. Start: type-chips 80, type Flush.
- **Main:** +80 chips if the hand contains a Flush. Copyable.

### 16. Half Joker (`j_half`)

- Common, $5. Start: extra = {mult 20, size 3}.
- **Main:** +20 mult if the number of played cards (everything in the play area:
  scoring cards, kickers, Stone cards) is at most 3. Copyable.

### 17. Joker Stencil (`j_stencil`)

- Uncommon, $8. Start: x_mult 1 until the first frame refresh.
- **Every frame:** x_mult = (joker slots − number of cards in the joker area) +
  number of Joker Stencils in the joker area. Slots are the joker area's limit (5 at
  base; +1 for each Negative joker held, Black Deck, Antimatter, and so on). Every
  joker counts as one card, Negative ones and Blueprint/Brainstorm included.
  Debuffed Stencils count as Stencils. Copies (Blueprint, Brainstorm) do not count
  as Stencils.
- **Main:** through the generic X-mult branch, X(x_mult) if x_mult > 1. Its own
  named branch (X x_mult when free slots > 0) is never reached: free slots > 0
  forces x_mult ≥ 2. Net rule: **X(free slots + Stencils held) when that number is
  at least 2, else nothing.** Examples with 5 slots: Stencil alone gives X5; 3
  jokers including one Stencil give X3; 5 jokers with one Stencil give nothing; 5
  jokers with two Stencils give X2 from each. More jokers than slots (possible only
  through odd slot changes) gives a value ≤ 1, hence nothing.
- Copyable: a copy returns the copied Stencil's x_mult.
- Uses the previous frame's value. Nothing changes the joker area during an
  evaluation, so this is the value at the moment of the hand.

### 18. Four Fingers (`j_four_fingers`)

- Uncommon, $7. No state.
- **Passive:** while at least one non-debuffed Four Fingers is held, flushes and
  straights need 4 cards instead of 5 (`hands.md` 3.2, 3.3). More copies change
  nothing. It also applies to the hand preview and to Burnt Joker's check of
  discarded cards. Played hands are still at most 5 cards.
- Not copyable (no routine effect). Debuffed: inactive.

### 19. Mime (`j_mime`)

- Uncommon, $5. Start: extra 1 (one retrigger).
- **Held-card repetition (scoring):** for each held card, after its first pass, +1
  extra pass if the first pass produced anything: the card's own held effect
  (a non-debuffed Steel card's X1.5) or any joker's held result for that card,
  including a "debuffed" message from Baron, Raised Fist or Shoot the Moon and a
  successful Reserved Parking roll. Retriggers are decided once, from the first pass
  only (`scoring.md` 5). Copyable: each Mime or copy adds 1.
- **End-of-round held repetition:** in the end-of-round held-card step
  (`round.md` 8.4), for each held card whose own end-of-round effect is non-empty,
  +1 evaluation. That means a non-debuffed Gold card (h_dollars), or a non-debuffed
  Blue-seal card whose evaluation actually queued a Planet (consumable room
  available). No joker gives held end-of-round effects, so jokers never qualify a
  card here. Each extra evaluation recomputes the effect: Gold pays $3 again; a Blue
  seal makes another Planet only if room is still left. Copyable.
- Mime never affects played cards.

### 20. Credit Card (`j_credit_card`)

- Common, $1. Start: extra 20.
- **Added to the joker area** (bought, taken from a pack, created, copied, or
  un-debuffed): bankruptcy floor (`G.GAME.bankrupt_at`) −20. **Removed** (sold or
  destroyed, when its removal completes; or debuffed): floor +20. Several Credit
  Cards stack.
- The floor is read by the shop and blind-select checks: buy a joker-row card or
  open a pack if cost ≤ money − floor (or cost is 0), buy-and-use likewise, redeem a
  voucher if cost ≤ money − floor, reroll if money − floor − price ≥ 0 (or price
  0), reroll the boss if money − floor − 10 ≥ 0 (`shop.md` 5.2, 5.6). Money may then
  go negative. Negative money earns no interest (`round.md` 9) and no joker in this
  file reads it, except Vagabond (≤ $4 holds).
- When sold, the sale money is queued first and the floor goes back up when the
  card's dissolve finishes (≈0.7 s), so the floor change follows the sale.
- While debuffed (Crimson Heart, Perishable ran out), the floor is back at $0.
- Not copyable, no scoring effect.

### 21. Ceremonial Dagger (`j_ceremonial`)

- Uncommon, $6. Cannot be Perishable. Start: mult 0.
- **Blind start** (any selected blind: Small, Big or Boss; never on skipped
  blinds), in joker order, not copyable. Condition: the Dagger is not marked for
  destruction itself, and the joker immediately to its right exists, is not Eternal,
  and is not already marked. Then, at once: that joker is marked (section 1.8) and
  the joker buffer drops by 1. Queued: the buffer is set to 0; Dagger's mult
  increases by 2 × that joker's current sell value (read when the queued step runs;
  nothing in vanilla changes it in between); the victim starts dissolving and is
  removed about 1.2 s later (its remove-from-deck effects then).
- The sell value includes Egg/Gift Card bonuses, rental ($1 base) and edition
  surcharges (`shop.md` 5.4).
- The victim does nothing at this blind start, even if it is to the right and has
  its own blind-start effect (Burglar, Marble, Riff-raff, Madness, another Dagger).
  Two adjacent Daggers: the left one eats the right one.
- **Main:** +mult if mult > 0. Copyable (a copy adds the Dagger's current mult).
- Under Amber Acorn the victim is chosen from the order before Amber Acorn's joker
  shuffles, which are queued (see UNSURE).

### 22. Banner (`j_banner`)

- Common, $5. Start: extra 30.
- **Main:** if discards left (`current_round.discards_left`, read during the
  evaluation) > 0: +30 × discards left chips. Copyable.
- Discards left already reflects discards used earlier this round, Burglar (0) and
  The Water (0). The Hook's forced discard does not use a discard.

### 23. Mystic Summit (`j_mystic_summit`)

- Common, $5. Start: extra = {mult 15, required discards left 0}.
- **Main:** +15 mult if discards left is exactly 0 during the evaluation. Copyable.
  Burglar and The Water make it active for the whole round.

### 24. Marble Joker (`j_marble`)

- Uncommon, $6. Start: extra 1.
- **Blind start** (every selected blind), copyable: a copy adds another card.
  Skipped if Marble is marked for destruction or, for a copy, if the copying card is
  marked. Steps, all before the round's deck shuffle (`nr` + A) because they are
  queued ahead of it:
  1. Queued: pick a rank and suit with stream `marb_fr` from the 52 card keys in
     sorted order (`rng_keys.md` 2), create a **Stone Card** with that hidden
     rank/suit (no edition, no seal), add it to the full deck list and put it in the
     play area.
  2. Queued: the deck's card limit +1.
  3. Queued: the card moves from the play area into the deck (inserted at the
     bottom; the shuffle that follows reorders by creation first anyway).
  4. Immediately, at the time of the blind-start call: "playing card added" is
     raised with one card (Hologram +0.25 per Marble trigger, even before the card
     exists).
- The stream is drawn when step 1 runs, one draw per Marble or copy, in joker order.
- The Stone card is a permanent deck card. Its hidden rank and suit matter if it
  ever stops being Stone (Vampire, Midas Mask with Pareidolia, Tarots), for the hand
  sort, and for nothing else while it is Stone. Rank-based counts (Cloud 9, rank
  jokers) never see it, and "card of the round" picks skip Stone cards.
- The new card is not passed through the boss's debuff check when created. It is
  only checked at the next debuff refresh (a joker added or removed, the boss
  disabled, the next blind). Under Verdant Leaf, or The Plant with Pareidolia, it can
  therefore stay undebuffed (see UNSURE).

### 25. Loyalty Card (`j_loyalty_card`)

- Uncommon, $5. Start: extra = {X 4, every 5}; `hands_played_at_create` = run
  hands-played counter when the card object is created (in the shop: when the shop
  slot is generated, not when bought; in a Buffoon pack: when the pack is opened);
  `loyalty_remaining` 5.
- **Main** (non-debuffed hands), first check of the main context: with
  d = (run hands-played counter, which does **not** yet include this hand) −
  `hands_played_at_create`, the card stores remaining = (4 − d) mod 6, with the
  result always in 0..5. **X4 when remaining = 5, i.e. when d ≡ 5 (mod 6):** the
  6th, 12th, 18th, ... hand played after the card was created. Otherwise it returns
  nothing from this branch and falls through to the generic branches, none of which
  apply.
- Copyable: a copy computes the same value (and stores it on the original) and
  gives X4 on the same hands.
- Every played hand advances the cycle, debuffed hands included (the run counter
  always rises), but the X4 is only given in the main context. A trigger hand that
  is debuffed, or played while Loyalty Card is debuffed, is lost and the cycle goes
  on.
- Copies made by Ankh or Invisible Joker keep the original's creation counter, so
  they stay in phase with it.
- The countdown shown on the card is display only.

### 26. 8 Ball (`j_8_ball`)

- Common, $5. Start: extra 4.
- **Played-card loop**, per trigger of each non-debuffed scoring card, per 8 Ball or
  copy in joker order:
  1. If there is no consumable room (held + pending ≥ slots): nothing, **no draw**.
  2. Else, if the card's rank id is 8: roll `8ball`, success if < p/4. Non-8s do
     not draw.
  3. On success: pending creations +1 at once; when this per-card result is
     applied, a Tarot creation is queued (append `8ba`, pool `Tarot8ba` + A, no
     Soul check). It is created after the evaluation.
- Copyable: a copy runs the same steps at its own place in joker order (a Blueprint
  copy, being left of its target, goes first). Whichever acts later sees the
  earlier one's pending Tarot in its room check.
- Stone: never (no rank). Retriggers roll again.

### 27. Misprint (`j_misprint`)

- Common, $4. Start: extra = {min 0, max 23}.
- **Main** (non-debuffed hands): draw stream `misprint` as a whole number from 0 to
  23 inclusive (one reseeded draw: floor(r × 24)), then + that many mult. A 0 adds
  nothing, but the draw is used. Copyable: each copy draws again, in joker order.
- UI fact: hovering Misprint shows, among its cycling description strings, the rank
  id and suit initial of the deck's top card (the next card to be drawn). A human
  can therefore see the next draw on screen; the fair-play wall should decide
  whether the bot gets this.

### 28. Dusk (`j_dusk`)

- Uncommon, $5. Start: extra 1.
- **Repetition, played cards:** if hands left is 0 during the evaluation (hands left
  drops at Play, so this is the round's last available hand), +1 retrigger for every
  non-debuffed scoring card. Copyable (+1 per copy).
- Hands left includes Burglar's +3 and The Needle's reduction.

### 29. Raised Fist (`j_raised_fist`)

- Common, $5. No state.
- **Held-card loop**, each pass of each held card. Raised Fist scans all cards
  currently held, left to right, skipping Stone cards. It keeps the card with the
  lowest printed rank (2 lowest, Ace 14 highest); a tie replaces the kept card, so
  the **rightmost** of the lowest wins. Debuffed cards are not skipped.
  - When the loop is on that exact card: if the card is debuffed, a "debuffed"
    message only (no value, but it counts as an effect, so Mime still retriggers
    the card; the card's own Red seal never works while it is debuffed); else
    +2 × its rank chips (2 to 10 face value, J/Q/K 10, Ace 11) as held mult.
  - Every other held card: nothing.
- If every held card is Stone, or no card is held, nothing.
- Copyable: a copy adds the same amount again on the same card. Mime and Red seal
  retriggers repeat it.
- Held cards include a DNA copy made this hand and exclude The Hook's discards.

### 30. Chaos the Clown (`j_chaos`)

- Common, $4. Start: extra 1 (the code always adds exactly 1).
- **Round start** (`new_round`, when a blind is selected): free rerolls are set to
  the number of non-debuffed Chaos the Clown held at that moment (unused free
  rerolls never carry over).
- **Added to the joker area** (bought, created, copied, un-debuffed): free rerolls
  +1, and the reroll price is recomputed (0 while free rerolls > 0).
- **Removed** (sold, destroyed, debuffed, Perishable ran out): free rerolls −1, then
  the price is recomputed. The recompute clamps negative free rerolls to 0, so
  selling a Chaos after using its free reroll costs nothing.
- Result: one free reroll per Chaos per shop, and a Chaos bought in a shop is usable
  at once. A Chaos whose Perishable runs out at a round end loses the free reroll of
  the shop that follows. Reroll pricing is in `shop.md` 5.5.
- Not copyable.

### 31. Fibonacci (`j_fibonacci`)

- Uncommon, $8. Start: extra 8.
- **Played-card loop:** +8 mult per trigger of a scoring card with rank id 14, 2, 3,
  5 or 8 (Ace, 2, 3, 5, 8). Copyable. Stone never.

### 32. Steel Joker (`j_steel_joker`)

- Uncommon, $7. Start: extra 0.2.
- **Every frame:** steel tally = number of Steel cards in the full deck (draw pile,
  hand, play area, discard pile), debuffed ones included.
- **Main:** if tally > 0: X(1 + 0.2 × tally). Copyable.
- Uses the tally from before the evaluation. A Steel card stripped by Vampire, or a
  Steel face card turned Gold by Midas Mask, still counts this hand. A DNA copy of a
  Steel card counts only from the next hand.
- Pool: offered only while the full deck contains a Steel card.

### 33. Scary Face (`j_scary_face`)

- Common, $4. Start: extra 30.
- **Played-card loop:** +30 chips per trigger of each scoring face card. Copyable.
- Pareidolia: every scoring card is a face, Stone cards included.

### 34. Abstract Joker (`j_abstract`)

- Common, $4. Start: extra 3.
- **Main:** +3 × (number of jokers in the joker area during the evaluation) mult.
  Itself, debuffed jokers, Negative jokers, Blueprint/Brainstorm and jokers waiting
  for removal all count. Copyable (the copy adds the same amount).

### 35. Delayed Gratification (`j_delayed_grat`)

- Common, $4. Start: extra 2.
- **Cash-out row** (computed once when the cash-out screen builds its rows, in joker
  order): if not debuffed, discards used this round is 0, and discards left > 0:
  +$2 × discards left.
- The Hook's forced discards do not count as used. Burglar or The Water (0 left)
  means $0.
- Not copyable (cash-out rows are asked of each joker itself; Blueprint has none).

### 36. Hack (`j_hack`)

- Uncommon, $6. Start: extra 1.
- **Repetition, played cards:** +1 retrigger for each non-debuffed scoring card with
  rank id 2, 3, 4 or 5. Copyable. Stone never.

### 37. Pareidolia (`j_pareidolia`)

- Uncommon, $5. No state.
- **Passive:** while a non-debuffed Pareidolia is held, every card passes the face
  test (section 1.10), Stone cards included. Debuffed cards still fail it for
  jokers. For bosses debuff is ignored, so **every** card counts as a face: The
  Plant debuffs every card, The Mark draws every card face down.
- In this file it affects Scary Face, Business Card, Ride the Bus (resets on any
  hand with a non-debuffed scoring card), Faceless Joker (any 3 non-debuffed
  discarded cards) and Midas Mask (every non-debuffed scoring card becomes Gold). It
  does not change rank ids (Baron still wants Kings).
- Not copyable.

### 38. Gros Michel (`j_gros_michel`)

- Common, $5. Cannot be Eternal. Start: extra = {mult 15, odds 6}.
- **Main:** +15 mult. Copyable.
- **End of round** (every round end, in joker order, before this joker's rental and
  Perishable steps; not copyable; skipped while debuffed): roll `gros_michel`. If
  < p/6, it is destroyed (removal queued) and the run's flag `gros_michel_extinct`
  is set at once. From then on Gros Michel leaves the pools and Cavendish enters
  them. Otherwise it survives.
- Two Gros Michels draw `gros_michel` in joker order.

### 39. Even Steven (`j_even_steven`)

- Common, $4. Start: extra 4.
- **Played-card loop:** +4 mult per trigger of a scoring card with rank id 2, 4, 6, 8
  or 10 (even ids from 0 to 10; faces and Aces never). Copyable. Stone never.

### 40. Odd Todd (`j_odd_todd`)

- Common, $4. Start: extra 31.
- **Played-card loop:** +31 chips per trigger of a scoring card with rank id 3, 5, 7,
  9 or 14 (Ace). Copyable. Stone never.

### 41. Scholar (`j_scholar`)

- Common, $4. Start: extra = {chips 20, mult 4}.
- **Played-card loop:** per trigger of a scoring Ace (id 14): +20 chips, then +4 mult
  (in that order inside the result). Copyable. Stone never.

### 42. Business Card (`j_business`)

- Common, $4. Start: extra 2 (the odds; the payout is a fixed $2 in the code).
- **Played-card loop**, per trigger, per Business Card or copy: only if the card is a
  face (face test), roll `business`; success if < p/2. Non-faces do not draw. On
  success: pending money +2 at once, and +$2 is queued when the result is applied.
- Copyable (each copy rolls). Pareidolia: every scoring card draws.

### 43. Supernova (`j_supernova`)

- Common, $5. Start: extra 1 (unused).
- **Main:** + (number of times this hand type was played this run, including this
  play) mult. The counter rises at the start of the evaluation and counts debuffed
  plays too (`hands.md` 6.3). Copyable.

### 44. Ride the Bus (`j_ride_the_bus`)

- Common, $6. Cannot be Perishable. Start: mult 0, extra 1.
- **'Before'** (non-debuffed hands; not copyable): if any scoring card passes the face
  test, mult is set to 0; otherwise mult +1. The change is immediate, so the main
  effect of the same hand uses it.
- **Main:** +mult if mult > 0. Copyable.
- A debuffed face card does not reset it. With Pareidolia it resets on every hand
  that has a non-debuffed scoring card. With Splash, kicker faces count. A debuffed
  hand changes nothing.

### 45. Space Joker (`j_space`)

- Uncommon, $5. Start: extra 4.
- **'Before'** (non-debuffed hands), per Space Joker or copy in joker order: roll
  `space`; if < p/4, the played hand type gains 1 level at once (`hands.md` 6.2).
  The base chips and mult are read after the 'before' pass, so the level counts for
  this hand. Copyable (each copy rolls again).

### 46. Egg (`j_egg`)

- Common, $4. Start: extra 3, sell bonus 0.
- **End of round** (not copyable; skipped while debuffed): sell bonus +3, then its
  price and sell value are recomputed (`shop.md` 5.4): sell = max(1, floor(price/2))
  + bonus. Without discounts that is $2 + $3 per round held.
- The higher sell value also feeds Ceremonial Dagger and Swashbuckler.

### 47. Burglar (`j_burglar`)

- Uncommon, $6. Start: extra 3.
- **Blind start**, copyable (a copy repeats it): skipped if the acting card (copy or
  original) or the copied Burglar is marked for destruction. Queued step: discards
  left are reduced by the number of discards left at that moment (to 0; the change
  is itself queued and cannot go below 0), and hands left +3. Both are for this
  round only (round base values are untouched).
- Two Burglars (or a copy): +6 hands, discards 0.
- With The Water the discards are already 0. With The Needle: 1 + 3 = 4 hands.
- If The Water is disabled later in the round (Chicot at the same blind start, or
  selling Luchador), the boss gives back the discards it removed, and Burglar does
  not take them again (see UNSURE).
- Effects elsewhere: Banner gets 0, Mystic Summit is active, Delayed Gratification
  pays 0, and the Garbage Tag's unused discards gain 0.

### 48. Blackboard (`j_blackboard`)

- Uncommon, $6. Start: extra 3.
- **Main:** X3 if every card held in hand during the evaluation passes the
  flush-style test (section 1.10) for Spades or for Clubs. Copyable.
- A held Stone card breaks it. A non-debuffed Wild card passes. A debuffed Spade or
  Club passes (debuff ignored). A debuffed Wild card counts only by its printed suit.
  Smeared Joker changes nothing (Spades and Clubs are already the whole colour).
- **No cards held** (all played, or the hand is empty): X3.
- Held cards are those left after The Hook's discards, plus a DNA copy made this
  hand.

### 49. Runner (`j_runner`)

- Common, $5. Cannot be Perishable. Start: extra = {chips 0, step 15}.
- **'Before'** (not copyable): if the hand contains a Straight, chips +15 at once.
- **Main:** + chips (it always returns its value; 0 adds nothing). Copyable.

### 50. Ice Cream (`j_ice_cream`)

- Common, $5. Cannot be Eternal. Start: extra = {chips 100, step 5}.
- **Main:** + chips. Copyable.
- **'After'** (every played hand, debuffed hands included; not copyable; skipped
  while debuffed): if chips − 5 ≤ 0, it melts (removal queued); else chips −5. It
  gives 100, 95, ..., 5 over 20 hands and melts after the 20th.

### 51. DNA (`j_dna`)

- Rare, $8. No stored values.
- **'Before'** (non-debuffed hands only), copyable: if hands played this round is 0
  (first hand of the round) and exactly 1 card was played:
  - A full copy of that played card is made: rank, suit, enhancement, edition, seal
    and every stored value, including perma bonus, the "played this ante" mark (set
    at the click) and its current debuff flag. It is added to the full deck list
    and placed **directly into the hand at the right end**; the deck's card limit
    +1.
  - "Playing card added" is raised at once with one card (Hologram +0.25 before
    Hologram's main effect, so it counts this hand).
  - The copy takes part in this hand's held-card loop (Steel, Baron, Raised Fist,
    Mime, Blackboard). It is not in the draw pile, so Blue Joker is unchanged. The
    frame-refreshed tallies (Steel Joker, Cloud 9) see it from the next frame.
- The played card's state at that moment is what is copied: changes made by 'before'
  jokers to the left of DNA (Midas Mask, Vampire) are included, changes by jokers to
  its right are not.
- Debuffed played card: still copied, and the copy starts debuffed. Stone card:
  copied as Stone.
- Copies: each Blueprint/Brainstorm of DNA makes one more copy.
- The Psychic debuffs one-card hands, so DNA cannot trigger under it.
- **First hand drawn:** DNA only flashes (cosmetic).

### 52. Splash (`j_splash`)

- Common, $3. No state.
- **Passive:** while a non-debuffed Splash is held, every played card is a scoring
  card (`hands.md` 5.4). The hand type is unchanged.
- Consequences here: per-card jokers, Hack, Dusk, 8 Ball, Business Card and Hiker
  reach kickers; Ride the Bus, Midas Mask, Vampire and Superposition look at
  kickers too.
- Not copyable.

### 53. Blue Joker (`j_blue_joker`)

- Common, $5. Start: extra 2.
- **Main:** if the draw pile (`G.deck`, cards not yet drawn) holds at least 1 card:
  +2 × that count chips. Read live during the evaluation. Cards in hand, in play and
  in the discard pile do not count. Copyable.

### 54. Sixth Sense (`j_sixth_sense`)

- Uncommon, $6. No state.
- **Destruction step** (non-debuffed hands, after the joker loop and Plasma): for each
  scoring card, the jokers are asked left to right whether to destroy it, stopping at
  the first yes. Sixth Sense says yes when exactly 1 card was played, that card's
  rank id is 6, and hands played this round is 0. It does not look at debuff, so a
  debuffed 6 is destroyed too. A Stone card never qualifies.
- When it says yes: if there is consumable room, pending +1 and a Spectral creation
  is queued (append `sixth`, pool `Spectralsixth` + A, no Soul/Black Hole). The 6 is
  destroyed even without room.
- Not copyable: copies are skipped entirely in this step, and a Blueprint on its left
  answers "no", so asking moves on. With two Sixth Senses only the leftmost acts
  (asking stops at the first yes), so only one Spectral is made.
- A Glass 6 still makes its own Glass roll (`scoring.md` 8).

### 55. Constellation (`j_constellation`)

- Uncommon, $6. Cannot be Perishable. Start: x_mult 1, extra 0.1.
- **Consumable used** (any use through the use button: from the consumable slots, a
  Celestial pack, or shop buy-and-use), not copyable, skipped while debuffed: if the
  used card is a Planet, x_mult +0.1 at once. Black Hole is a Spectral and does not
  count.
- **Main:** generic X-mult: X(x_mult) once x_mult > 1. Copyable.

### 56. Hiker (`j_hiker`)

- Uncommon, $5. Start: extra 5.
- **Played-card loop:** per trigger of each scoring card, the card's permanent bonus
  chips +5 (stored on the card). The card's chips for this trigger were already
  computed, so the +5 counts from its next trigger (a retrigger in this hand, or
  later hands). Copyable (+5 per copy). It applies to Stone cards and every
  enhancement, and survives enhancement changes and copying (`scoring.md` 12.2).

### 57. Faceless Joker (`j_faceless`)

- Common, $4. Start: extra = {$5, 3 faces}.
- **Discard** (each discard action): when the jokers are asked about the last
  discarded card (the rightmost; cards are processed left to right), count the face
  cards among all cards of this discard (face test: debuffed never; with Pareidolia
  every non-debuffed card). If at least 3: queue +$5 (lands two queued hops later).
- Copyable (each copy pays again).
- The Hook's forced discard runs it too, but has at most 2 cards, so it never pays.

### 58. Green Joker (`j_green_joker`)

- Common, $4. Cannot be Perishable. Start: mult 0, extra = {+1 per hand, −1 per
  discard}.
- **'Before'** (non-debuffed hands; not copyable): mult +1 at once (counts this hand).
- **Discard** (not copyable): when the jokers are asked about the last discarded
  card, mult = max(0, mult − 1). This happens once per discard action, **including
  The Hook's forced discard**. That discard runs before that hand's scoring, so under
  The Hook each hand is −1 then +1.
- **Main:** +mult if mult > 0. Copyable.
- A debuffed hand gives no +1.

### 59. Superposition (`j_superposition`)

- Common, $4. No state.
- **Main** (non-debuffed hands), copyable: if there is consumable room, at least one
  scoring card has rank id 14, and the hand contains a Straight: pending +1 and a
  Tarot creation is queued (append `sup`, pool `Tarotsup` + A).
- A debuffed Ace among the scoring cards still counts (rank ids ignore debuff); Stone
  never. With Splash, kicker Aces count.

### 60. To Do List (`j_todo_list`)

- Common, $4. Start: extra = {$4, placeholder hand High Card (never used)}; target
  hand = a random draw at creation.
- **Creation:** every time a To Do List card object is created (shop slot, Buffoon
  pack, Judgement, Riff-raff, Top-up Tag, Ankh or Invisible Joker copies, and the
  collection screen if opened during a run), it draws `to_do` once, inside card
  creation after the pool pick and before any sticker or edition draws
  (`rng_keys.md` 1, step 5). The pick is among the **visible**
  poker hands, listed in the runtime walk order of the hands table (`rng_keys.md` 2).
  A copy draws, then takes the original's target.
- **'Before'** (non-debuffed hands only), copyable: if the played hand type equals the
  target: +$4 (queued; pending money +4 at once). A copy pays again.
- **End of round** (not copyable; skipped while debuffed): a new target is drawn with
  `to_do` from the visible hands **except the current target**. Several To Do Lists
  draw in joker order.

### 61. Cavendish (`j_cavendish`)

- Common, $4. Cannot be Eternal. Start: extra = {X 3, odds 1000}.
- **Main:** X3. Copyable.
- **End of round** (not copyable; skipped while debuffed): roll `cavendish`; if
  < p/1000 it is destroyed (removal queued). No flag is set, so it can be offered
  again.
- Pool: offered only after Gros Michel has gone extinct this run.

### 62. Card Sharp (`j_card_sharp`)

- Uncommon, $6. Start: extra = {X 3}.
- **Main:** X3 if this hand type's "played this round" counter, which already
  includes this play, is greater than 1 (the type was played earlier this round,
  debuffed plays included). Copyable.

### 63. Red Card (`j_red_card`)

- Common, $5. Cannot be Perishable. Start: mult 0, extra 3.
- **Pack skipped** (`skipping_booster`): each press of a pack's Skip button (any pack
  type, shop or tag pack, also after some choices were already taken) gives +3 mult
  at once. Closing a pack by using up its choices does not count. Not copyable.
- **Main:** +mult if mult > 0. Copyable.

### 64. Madness (`j_madness`)

- Uncommon, $7. Cannot be Perishable. Start: x_mult 1, extra 0.5.
- **Blind start**, Small and Big blinds only (not Boss; not on skips), not copyable,
  skipped if Madness itself is marked:
  1. x_mult +0.5 at once (even if nothing can be destroyed).
  2. Candidates: every other joker in the area that is not Eternal and not already
     marked. If there is at least one, pick one with stream `madness` (candidates in
     creation order, `rng_keys.md` 2). With no candidate there is no draw.
  3. The pick is marked at once and its dissolve is queued (removal ≈1.2 s later;
     its remove-from-deck effects then). Madness does not change the joker buffer.
- A victim to the right of Madness does nothing at this blind start.
- **Main:** generic X-mult once x_mult > 1. Copyable.

### 65. Square Joker (`j_square`)

- Common, $4. Cannot be Perishable. Start: extra = {chips 0, step 4}.
- **'Before'** (not copyable): if exactly 4 cards were played (all cards in the play
  area), chips +4 at once.
- **Main:** + chips (always returns; 0 adds nothing). Copyable.

### 66. Seance (`j_seance`)

- Uncommon, $6. Start: extra = {hand Straight Flush}.
- **Main** (non-debuffed hands), copyable: if there is consumable room and the hand
  contains a Straight Flush (a straight and a flush both exist; with Four Fingers
  they may use different cards, `hands.md` 4): pending +1 and a Spectral creation is
  queued (append `sea`, pool `Spectralsea` + A, no Soul/Black Hole).

### 67. Riff-raff (`j_riff_raff`)

- Common, $6. Start: extra 2 (the code uses a fixed 2).
- **Blind start** (every selected blind), copyable; skipped if Riff-raff or the
  acting card is marked. If (jokers in the area + joker buffer) < joker slots:
  n = min(2, slots − (jokers + buffer)); buffer +n at once; a step is queued that
  creates n jokers one by one, each added to the right end of the joker area.
- Each created joker: rarity fixed to Common (no `rarity` draw); pool pick
  `Joker1rif` + A (resamples `Joker1rif` + A + `_resample` + n); no Eternal,
  Perishable or Rental draws (made straight into the joker area); edition roll
  `edirif` + A (can be Negative). The second pick excludes the first (owned jokers
  leave the pool unless Showman is held).
- Created jokers do not get this blind start's event (the loop is over). Their
  add-to-deck effects apply at once (a created Chaos the Clown gives a free reroll
  for the next shop; a Credit Card lowers the floor).
- A Ceremonial Dagger to the left frees one slot through the buffer; a Madness victim
  does not.

### 68. Vampire (`j_vampire`)

- Uncommon, $7. Cannot be Perishable. Start: x_mult 1, extra 0.1.
- **'Before'** (non-debuffed hands; not copyable): every scoring card, in play order,
  that has an enhancement (any of Bonus, Mult, Wild, Glass, Steel, Stone, Gold,
  Lucky), is not debuffed, and was not already stripped by a Vampire this hand:
  - at once it becomes a plain card. Edition, seal and perma bonus stay. The
    "played this ante" mark is lost, and the boss's debuff test is re-run on the
    card;
  - after the evaluation a queued step clears its "already stripped" mark.

  Then x_mult += 0.1 × (number stripped), at once, so this hand's main effect
  includes it.
- Effects on this hand: a stripped Stone card scores its printed rank (and may
  become debuffed: a Stone Club under The Club becomes a Club and is then debuffed,
  so it does not score). A stripped Lucky card draws no `lucky_mult`/`lucky_money`.
  A stripped Glass card gives no X2 and makes no Glass roll. A stripped Wild card is
  no longer wild for per-card suit jokers (detection is already done). A stripped
  Gold card gives no end-of-round money later.
- Order with Midas Mask: Midas to the left of Vampire makes faces Gold, then Vampire
  strips them (and gains). The other way round, Vampire strips first and Midas then
  gilds the faces.
- **Main:** generic X-mult once x_mult > 1. Copyable.
- Steel Joker's tally still counts a stripped Steel card this hand (section 1.5).

### 69. Shortcut (`j_shortcut`)

- Uncommon, $7. No state.
- **Passive:** while a non-debuffed Shortcut is held, straights may step over single
  missing ranks, never two adjacent missing ranks. Five present ranks are still
  needed, 4 with Four Fingers (`hands.md` 3.3). Not copyable.

### 70. Hologram (`j_hologram`)

- Uncommon, $7. Cannot be Perishable. Start: x_mult 1, extra 0.25.
- **Playing card added** (not copyable; skipped while debuffed or marked): x_mult +=
  0.25 × (number of cards in the event), at once. Sources: Marble Joker (1, at blind
  start), DNA (1, during 'before'), Certificate (1, first hand drawn), Cryptid (2),
  Familiar (3), Grim (2), Incantation (4), a playing card taken from a Standard pack
  (1), a playing card bought in the shop (1).
- **Main:** generic X-mult once x_mult > 1. Copyable.

### 71. Vagabond (`j_vagabond`)

- Rare, $8. Start: extra 4.
- **Main** (non-debuffed hands), copyable: if there is consumable room and money is
  at most $4 (`G.GAME.dollars`, without pending money, so it is the money at the
  start of the evaluation, after The Tooth and after The Ox's reset): pending +1 and
  a Tarot creation is queued (append `vag`, pool `Tarotvag` + A). Negative money
  qualifies.

### 72. Baron (`j_baron`)

- Rare, $8. Start: extra 1.5.
- **Held-card loop**, each pass of each held King (rank id 13; Stone never): if the
  King is debuffed, a "debuffed" message only (it counts as an effect, so Mime still
  retriggers the card, again with no value; a debuffed card's own Red seal never
  works); else X1.5 mult. Copyable.
- Does nothing at end of round (section 1.1).

### 73. Cloud 9 (`j_cloud_9`)

- Uncommon, $7. Start: extra 1.
- **Every frame:** nine tally = cards in the full deck with rank id 9. Debuffed 9s
  count. Stone cards never count, whatever their hidden rank.
- **Cash-out row:** if not debuffed and tally > 0: +$1 × tally. The tally is the one
  at cash-out time (cards destroyed during the round are already gone).
- Not copyable.

### 74. Rocket (`j_rocket`)

- Uncommon, $6. Cannot be Perishable. Start: extra = {payout 1, step 2}.
- **End of round** (not copyable; skipped while debuffed): if the blind was a Boss,
  payout +2 at once (also when Mr. Bones saved the round).
- **Cash-out row:** if not debuffed: +$payout. A boss win therefore pays the raised
  amount (first boss: $3).
- Not copyable.

### 75. Obelisk (`j_obelisk`)

- Rare, $8. Cannot be Perishable. Start: x_mult 1, extra 0.2.
- **'Before'** (non-debuffed hands; not copyable): let c = this hand type's run play
  count, including this play. If no **other visible** hand type has a play count
  ≥ c (this type is strictly the most played), it resets: x_mult back to 1 (only if
  above 1). Otherwise x_mult +0.2 at once. A tie counts as "not most played", so it
  gains. Play counts include debuffed plays. The first hand of a run always resets.
- **Main:** generic X-mult once x_mult > 1. Copyable.

### 76. Midas Mask (`j_midas_mask`)

- Uncommon, $7. No state.
- **'Before'** (non-debuffed hands; not copyable): every scoring card that passes the
  face test becomes a **Gold Card** at once, in play order. Its old enhancement is
  replaced (Glass, Lucky, Stone and Steel effects are lost for this hand); edition,
  seal and perma bonus stay; the "played this ante" mark is lost and the boss's
  debuff test is re-run on the card.
- Debuffed faces are not faces, so they are untouched. With Pareidolia every
  non-debuffed scoring card turns Gold, Stone cards included. They then score their
  printed rank, may become debuffed under a suit boss (`scoring.md` 3.2), and count
  for rank jokers this hand. With Splash, kickers are included.
- Gold Cards pay at the end of the round while held (`scoring.md` 12.3).
- Order with Vampire: see Vampire (68).

## UNSURE

- **Numbering.** The brief had no Obelisk and called Midas Mask number 75. The game's
  data has Obelisk at order 75 and Midas Mask at 76. Both are covered here. Check
  that the companion note for the later jokers starts at order 77 (Luchador), so
  that Obelisk is not covered twice.
- **Riff-raff after a Dagger or Madness kill at the same blind start.** The doomed
  joker stays in the joker area, and its name stays out of the pools, until its
  dissolve ends (≈1.2 s). Riff-raff's creation step waits behind the blocking
  message steps queued before it. Normally that is about 0.9 s, so the doomed name
  is probably still excluded from Riff-raff's pick. With more blind-start messages in
  between it might not be. Timing-dependent; not tested in game.
- **Amber Acorn and blind-start joker changes.** Amber Acorn's three `aajk` shuffles
  are queued, while Dagger, Madness and Riff-raff act before them. Which jokers are
  in the area when each shuffle runs (new Riff-raff jokers, doomed jokers not yet
  removed) is timing-dependent and changes the final order. Belongs with
  `blinds.md`.
- **Marble Joker's Stone card and boss debuffs.** The card skips the debuff check at
  creation (Certificate's card gets one; Marble's does not). Under Verdant Leaf, or
  The Plant with Pareidolia, it should stay undebuffed until a later debuff refresh.
  From the code; not verified in game.
- **Burglar and a later disable of The Water** (Chicot, Luchador): from the code the
  discards come back and Burglar does not remove them again. Not verified in game.
- **DNA copies inherit the debuff flag and the "played this ante" mark.** From the
  code; not verified in game. It matters under The Pillar for the rest of the ante,
  and when the copied card was debuffed.
- **Midas Mask and Vampire clear "played this ante".** From the code, a card changed
  by them is no longer debuffed by The Pillar later in the ante. Not verified in
  game.
- **Rocket, Cloud 9 and Delayed Gratification on a Perishable's last round:** the
  end-of-round effect happens (Rocket +2), then the joker is debuffed, so it pays no
  cash-out row that round. From the code; not verified in game.

## Where in the game

- card.lua: `Card:calculate_joker` (all joker contexts, Blueprint/Brainstorm
  handling), `Card:set_ability` (initial values, To Do List draw, Loyalty Card
  counters), `Card:add_to_deck`, `Card:remove_from_deck`, `Card:set_debuff`,
  `Card:update` (Joker Stencil, Steel Joker, Cloud 9 tallies; Blueprint label),
  `Card:calculate_dollar_bonus`, `Card:set_cost`, `Card:sell_card`, `Card:remove`,
  `Card:start_dissolve`, `Card:shatter`, `Card:get_id`, `Card:is_face`,
  `Card:is_suit`, `Card:get_chip_bonus`, `Card:get_end_of_round_effect`,
  `Card:calculate_seal`, `Card:calculate_perishable`, `Card:use_consumeable`
  (Cryptid, Familiar, Grim, Incantation card additions), `Card:init`,
  `Card:generate_UIBox_ability_table` (meaning only; Misprint's tooltip)
- functions/state_events.lua: `G.FUNCS.evaluate_play` (before, per-card, held,
  repetition, joker loop, destruction, after, debuffed hand; Splash),
  `G.FUNCS.play_cards_from_highlighted` (counters), `G.FUNCS.discard_cards_from_highlighted`
  (discard contexts, The Hook's flag), `new_round` (Chaos reset, blind-start
  contexts), `end_round` (end-of-round contexts, held repetition),
  `G.FUNCS.evaluate_round` (cash-out rows), `G.FUNCS.get_poker_hand_info`
- functions/common_events.lua: `eval_card`, `card_eval_status_text` (DNA's
  playing-card-added hand-off), `ease_dollars`, `ease_discard`, `ease_hands_played`,
  `create_card`, `get_current_pool` (pool flags, enhancement gate), `copy_card`,
  `draw_card`, `calculate_reroll_cost`, `level_up_hand`
- functions/misc_functions.lua: `find_joker`, `pseudorandom`, `pseudoseed`,
  `pseudorandom_element`, `playing_card_joker_effects`, `evaluate_poker_hand`,
  `get_flush`, `get_straight`, `get_X_same`
- functions/button_callbacks.lua: `G.FUNCS.skip_booster`, `G.FUNCS.can_skip_booster`,
  `G.FUNCS.use_card`, `G.FUNCS.buy_from_shop`, `G.FUNCS.sell_card`,
  `G.FUNCS.reroll_shop`, `G.FUNCS.can_buy`, `G.FUNCS.can_buy_and_use`,
  `G.FUNCS.can_redeem`, `G.FUNCS.can_open`, `G.FUNCS.can_reroll`,
  `G.FUNCS.reroll_boss_button`, `G.FUNCS.toggle_shop`
- blind.lua: `Blind:set_blind`, `Blind:debuff_card`, `Blind:debuff_hand`,
  `Blind:press_play`, `Blind:disable`, `Blind:defeat`, `Blind:drawn_to_hand`,
  `Blind:stay_flipped`
- cardarea.lua: `CardArea:emplace`, `CardArea:align_cards`, `CardArea:draw_card_from`
- game.lua: joker data table (`P_CENTERS`, orders 1 to 76), `P_CARDS`, `P_BLINDS`,
  `Game:init_game_object` (probabilities, bankruptcy floor, joker buffer, pool
  flags), `Game:update_draw_to_hand` (first-hand-drawn context),
  `Game:update_round_eval`
- engine/event.lua: `Event:handle`, `EventManager:add_event` (queued-step timing)
- localization/en-us.lua: joker descriptions, read for meaning only
