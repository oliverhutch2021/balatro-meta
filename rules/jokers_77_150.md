# Jokers in game order 77 to 150 (Luchador … Perkeo)

This note gives the complete behaviour, in Balatro 1.0.1o, of the second half of
the game's joker list: the 74 jokers from Luchador (order 77) to Perkeo (order
150), the five legendaries included. For each joker it covers every moment it
acts, the exact condition and numbers, every random stream it draws (key string,
moment, comparison), which parts Blueprint and Brainstorm copy, when its state
changes land, and how it meets debuffs, Stone cards, Wild cards, Smeared Joker,
Pareidolia and Showman. The text is in our own words. Only numbers, keys, stream
names and the names of the game's functions and contexts come from the scripts.

**Read `jokers_1_76.md` first.** Its sections 0 and 1 hold the conventions and
shared mechanics this file relies on and does not repeat: the probability
numerator p and the "strictly below p/n" rule (0), the order in which one call of
the joker routine picks its context family (1.1), the generic main branches
(1.2), Blueprint and Brainstorm (1.3), debuffed jokers and add/remove effects on
debuff (1.4), values refreshed every frame (1.5), floating point (1.6), initial
values and `copy_card` (1.7), blind-start marks and the joker buffer (1.8), money
and creation timing (1.9), and the suit, face and rank tests (1.10). Scoring
terms are those of `scoring.md`; streams are in `rng_keys.md`; prices, pools and
sell values in `shop.md`; the round flow in `round.md`; bosses in `blinds.md`;
what the player may do and when in `timing.md`.

**Scope check.** The data table in `game.lua` numbers the 150 jokers 1 to 150
with no gaps. Order 77 is `j_luchador` and order 150 is `j_perkeo`. Obelisk (75)
and Midas Mask (76) are in `jokers_1_76.md`, so nothing is covered twice. The
numbering below is the `order` field, which is also the pool order
(`shop.md` 2.7).

**Locks.** Orders 77 to 105 are unlocked from the start. Orders 106 to 145 start
locked in a fresh profile (research runs assume a fully unlocked profile,
`shop.md` 2.3). The legendaries 146 to 150 ignore locks and come only from The
Soul's `Joker4` pick (`rng_keys.md` 3.10), never from a shop or Buffoon pack.

## 0. Conventions (additions)

- Everything in `jokers_1_76.md` section 0 applies: p, "copyable", "debuffed
  joker", scoring card / kicker / held card / play order, pending money,
  consumable room, A, context names in brackets.
- **Discard events.** During a discard the jokers are asked once per discarded
  card: cards left to right, and for each card the jokers left to right
  (`round.md` 6). Before that, every joker gets one "before discard" event
  (`pre_discard`) for the whole set. The Hook's forced discard runs the same
  events with a "hook" flag set; only Burnt Joker looks at that flag.
- **Eaten / removal queued.** Ramen, Popcorn, Turtle Bean and Seltzer remove
  themselves through an animation: a queued step, then about 0.3 s later the card
  leaves the joker area and its remove-from-deck effects run. Mr. Bones dissolves
  (about 0.7 s). Until then the card is still in the joker area. No player action
  can happen in between, so the engine may remove them right after the step that
  decided it.
- **Removal report**: the event that tells every joker which playing cards were
  destroyed (`remove_playing_cards`), section 1.6.
- **Card of the round**: the four per-round targets (Idol card, Mail-In rank,
  Ancient suit, Castle suit), section 1.5.
- **Scoring suit test**, **face test**, **rank id**: as in `jokers_1_76.md` 1.10.
  In short: a debuffed card has no suit and is never a face; a Stone card has no
  suit and a rank id that never matches; a Wild card has every suit; with a
  non-debuffed Smeared Joker a card has both suits of its colour; with a
  non-debuffed Pareidolia every non-debuffed card is a face.

## 1. Shared mechanics added by this range

### 1.1 Dispatch notes

- The order of context families in one call is the one in `jokers_1_76.md` 1.1.
  The blind-start family and the "playing card added" family are skipped for a
  joker marked for destruction at that blind start; the call then falls through
  to the joker-area branches, which need the joker-area marker, so it does
  nothing. The destruction question (`destroying_card`) is skipped for copies.
- The routine has a `cards_destroyed` family (Caino and Glass Joker have
  branches in it), but nothing in 1.0.1o ever raises that event. The live path
  is the removal report (`remove_playing_cards`, section 1.6).
- `buying_card` is raised (shop purchases and voucher redemptions) but no joker
  reacts to it. No joker of this range acts in `skipping_booster`,
  `playing_card_added`, `destroying_card`, or the held-card sub-events of the
  end of round (individual and repetition; only Mime, in `jokers_1_76.md`, acts
  there).

### 1.2 Users of the generic X-mult branch

These jokers have no main branch of their own. They give X(`x_mult`) through the
generic branch (`jokers_1_76.md` 1.2) when `x_mult` > 1 and the played hand
contains their type (empty type: always). All are copyable: a copy returns the
copied joker's current value.

| Joker | `x_mult` when created | Type | How it changes |
|---|---|---|---|
| Lucky Cat | 1 | — | +0.25 per trigger with a Lucky hit |
| Ramen | 2 | — | −0.01 per discarded card |
| Campfire | 1 | — | +0.25 per card sold; back to 1 after a Boss round |
| Throwback | 1 | — | every frame: 1 + 0.25 × skips |
| Glass Joker | 1 | — | +0.75 per Glass card destroyed |
| Hit the Road | 1 | — | +0.5 per Jack discarded; back to 1 at every round end |
| Yorick | 1 | — | +1 per 23 cards discarded |
| The Duo | 2 | Pair | fixed |
| The Trio | 3 | Three of a Kind | fixed |
| The Family | 4 | Four of a Kind | fixed |
| The Order | 3 | Straight | fixed |
| The Tribe | 2 | Flush | fixed |

Seeing Double is excluded from the generic branch by name; its `x_mult` is 1
anyway, so nothing follows from it.

### 1.3 Selling a Blueprint or Brainstorm

The "this card is sold" event (`selling_self`) goes only to the card being sold.
When that card is a Blueprint or Brainstorm, it first runs its copy step as for
any event, so the sale effect of the joker it copies runs:

- copying Luchador: the current Boss blind is disabled (if not already);
- copying Diet Cola: a Double Tag is gained;
- copying Invisible Joker: nothing (Invisible Joker refuses copies).

The copied joker stays and keeps its own sale effect for later. A Blueprint at
the right end, or a Brainstorm that is itself the leftmost joker, copies
nothing. A debuffed copier or a debuffed target gives nothing. From the code;
see UNSURE.

### 1.4 Values refreshed every frame (additions to `jokers_1_76.md` 1.5)

| Joker | Stored value |
|---|---|
| Throwback | `x_mult` = 1 + 0.25 × `G.GAME.skips` (blinds skipped this run) |
| Driver's License | `driver_tally` = cards in the full deck (`G.playing_cards`) with any enhancement (Bonus, Mult, Wild, Glass, Steel, Stone, Gold or Lucky), debuffed ones included |
| Stone Joker | `stone_tally` = cards in the full deck that are Stone cards |
| Swashbuckler | `mult` = sum of the sell values of every other card in the joker area |
| Blueprint, Brainstorm | the "compatible / incompatible" label only (display) |

During an evaluation these hold the previous frame's value: changes made in the
'before' pass (Vampire, Midas Mask, DNA) count from the next hand.

### 1.5 Card of the round (The Idol, Mail-In Rebate, Ancient Joker, Castle)

Four values in `G.GAME.current_round` are drawn at run start (right after the
`shuffle` deck shuffle, in the order below) and again at the end of every won
round (Mr. Bones saves included), in the last end-of-round step, after the ante
has risen for a Boss round (`round.md` 8). They are drawn **whether or not** any
of these jokers is owned.

| Target | Stream | Picked from (creation order) | If there is nothing to pick |
|---|---|---|---|
| Idol card: rank id and suit | `idol` + A | every non-Stone card in the full deck | the shown target resets to Ace of Spades, but the stored rank id keeps its previous value (it starts empty) |
| Mail-In rank: rank id | `mail` + A | same | shown as Ace; the stored rank id keeps its previous value |
| Ancient suit | `anc` + A | the list Spades, Hearts, Clubs, Diamonds without the current suit (all four at run start, where the current suit is cleared first) | — |
| Castle suit | `cas` + A | same as the Idol card | Spades |

The target is the picked card's printed rank and printed suit. Its enhancement,
edition, seal and debuff do not matter (a Wild card is picked under its printed
suit). All copies of one of these jokers share the same target. Cards anywhere
(draw pile, hand, discard pile) can be picked.

### 1.6 Removal reports (Caino, Glass Joker)

When playing cards are destroyed every joker, left to right, gets the list of
them. The sources:

| Source | When the report is sent | Are destroyed Glass cards marked "shattered" when it is sent? |
|---|---|---|
| Destruction step of a scored hand (Glass rolls, Sixth Sense) | inside the evaluation, after the destruction step; sent after every non-debuffed hand, with an empty list if nothing was destroyed | yes: every destroyed card that is a Glass card is marked, including a Glass 6 taken by Sixth Sense (even a debuffed one) |
| Trading Card destroying a discarded card | at the end of that discard, after all its cards were processed | yes (a Glass card is shattered at once) |
| The Hanged Man, Familiar, Grim, Incantation, Immolate | at the moment of use, while the destruction itself is still queued | **no**: the shatter is queued, so the mark is not set yet |

Caino applies the face test to each listed card at that moment. Glass Joker
counts the "shattered" marks, and has a separate rule for The Hanged Man (it
counts the selected Glass cards when the card is used), so Hanged Man glass is
counted exactly once. Glass cards destroyed by Familiar, Grim, Incantation or
Immolate are not counted at all (see UNSURE).

### 1.7 Hand size, hands and discards changed by jokers

- Hand-size changes (Juggler, Turtle Bean, Troubadour, Merry Andy, Stuntman)
  go through the hand area's size change, which is queued. An increase while the
  game is drawing or selecting a hand, with at least one card in hand, draws that
  many extra cards at once; a decrease never discards (`round.md` 4, item 8). The
  shown limit never goes below 0, but the true running value is kept, so later
  changes start from it.
- Base discards (Drunkard, Merry Andy): the base value per round changes at
  once; discards left change through a queued step that can never take them
  below 0.
- Base hands (Troubadour): only the base value changes. Hands left this round
  are untouched; the next round starts with max(1, base + bonus).
- All of these are add/remove effects, so they also run when the joker is
  debuffed or un-debuffed in the joker area (`jokers_1_76.md` 1.4). Under Crimson
  Heart, the re-pick after each refill first un-debuffs every joker and then
  debuffs one: a hand-size joker that comes back during hand selection draws its
  extra cards at once (see UNSURE).

### 1.8 Floating point

Ramen is the only joker of this range whose growth is inexact in binary: it
loses 0.01 per discarded card from 2. Keep the running value and subtract 0.01
each time; never compute 2 − 0.01 × n. After 99 cards the stored value is
1.0099999999999991, and the 100th card eats it (section 3, Ramen). Every other
change here (+0.25, +0.5, +0.75 × n, +1, 1 + 0.25 × skips) is exact.

### 1.9 Money, creations and copies (timing)

| Joker | What | Decided | Lands |
|---|---|---|---|
| Reserved Parking | +$1 | held-card loop, per pass | queued; pending money +1 at once |
| Golden Ticket | +$4 | played-card loop, per trigger | queued; pending +4 at once |
| Rough Gem | +$1 | played-card loop, per trigger | queued; pending +1 at once |
| Matador | +$8 | joker loop, or the debuffed-hand pass | queued; pending +8 at once |
| Mail-In Rebate | +$5 | discard, per matching card | queued (one step) |
| Trading Card | +$3 | discard | queued (one step) |
| Golden Joker, Satellite | cash-out rows | when the cash-out screen computes its rows | paid with Cash Out |
| To the Moon | interest | same | paid with Cash Out |
| Hallucination | Tarot, append `hal` | pack opened | after the pack's contents are made |
| Cartomancer | Tarot, append `car` | blind start | two queued hops; before the opening draw |
| Certificate | a playing card | first hand drawn | one queued hop; after the drawn cards, before the blind's after-draw effect |
| Perkeo | Negative copy of a held consumable | leaving the shop | one queued hop per Perkeo or copy |
| Invisible Joker | copy of a joker | at the sale click | at once, before the sale money |
| Diet Cola | Double Tag | at the sale click | one queued hop |

Hallucination's and Cartomancer's Tarots are created without the Soul check (no
`soul_` draw). Like every consumable creation of this kind (`jokers_1_76.md`
1.9), the creation step sets the pending-creation count back to 0 when it runs;
it does not subtract 1.

### 1.10 Disabling the boss (Chicot, Luchador)

What a disable does is in `blinds.md` 11 (discards, hands and hand size given
back, face-down cards turned up, targets of The Wall and Violet Vessel reduced,
debuffs recomputed, and an immediate win if the round total already meets the
new target). Two more facts matter here:

- The disable step itself has no "already disabled" guard. Luchador checks
  first, and so does Chicot's add-to-deck path, but Chicot's blind-start path
  does not: every Chicot queues an unconditional disable. With two Chicots the
  restoring effects run twice (section 3, Chicot; UNSURE).
- A beaten blind is cleared on the cash-out screen (`Blind:defeat`, then a reset
  about 1.2 s later), after which it is no longer a Boss blind for Luchador or
  Chicot.

## 2. Overview

Rarity: C Common, U Uncommon, R Rare, L Legendary. $ = base cost (`shop.md` 5.1).
E / P = may carry the Eternal / Perishable sticker (data flags; any joker can be
Rental). Contexts (copyable parts in **bold**): B 'before', PC played-card loop,
HC held-card loop, RP repetition of played cards, M main (joker loop), OJ
joker-on-joker, A 'after', DH debuffed hand, SB blind start, FHD first hand drawn,
PD before discard, D discard (per card), EoR end of round, CO cash-out row, RPT
removal report, UC consumable used, OB pack opened, SS this card sold, SC another
card sold, RR shop reroll, ES leaving the shop, SK blind skipped, add/remove,
frame (per-frame value), passive.

| # | Key | Name | R | $ | E | P | Acts in (copyable in bold) | Random streams |
|---|---|---|---|---|---|---|---|---|
| 77 | `j_luchador` | Luchador | U | 5 | n | y | SS | — |
| 78 | `j_photograph` | Photograph | C | 5 | y | y | **PC** | — |
| 79 | `j_gift` | Gift Card | U | 6 | y | y | EoR | — |
| 80 | `j_turtle_bean` | Turtle Bean | U | 6 | n | y | add/remove; EoR | — |
| 81 | `j_erosion` | Erosion | U | 6 | y | y | **M** | — |
| 82 | `j_reserved_parking` | Reserved Parking | C | 6 | y | y | **HC** | `parking` |
| 83 | `j_mail` | Mail-In Rebate | C | 4 | y | y | **D** | target `mail`+A |
| 84 | `j_to_the_moon` | To the Moon | U | 5 | y | y | add/remove (interest) | — |
| 85 | `j_hallucination` | Hallucination | C | 4 | y | y | **OB** | `halu`+A; creation `Tarothal`+A |
| 86 | `j_fortune_teller` | Fortune Teller | C | 6 | y | y | **M**; UC (message only) | — |
| 87 | `j_juggler` | Juggler | C | 4 | y | y | add/remove (hand size) | — |
| 88 | `j_drunkard` | Drunkard | C | 4 | y | y | add/remove (discards) | — |
| 89 | `j_stone` | Stone Joker | U | 6 | y | y | frame; **M** | — |
| 90 | `j_golden` | Golden Joker | C | 6 | y | y | CO | — |
| 91 | `j_lucky_cat` | Lucky Cat | U | 6 | y | n | PC (growth); **M** (generic X) | — |
| 92 | `j_baseball` | Baseball Card | R | 8 | y | y | **OJ** | — |
| 93 | `j_bull` | Bull | U | 6 | y | y | **M** | — |
| 94 | `j_diet_cola` | Diet Cola | U | 6 | n | y | SS | — |
| 95 | `j_trading` | Trading Card | U | 6 | y | y | D | — |
| 96 | `j_flash` | Flash Card | U | 5 | y | n | RR; **M** | — |
| 97 | `j_popcorn` | Popcorn | C | 5 | n | y | **M**; EoR | — |
| 98 | `j_trousers` | Spare Trousers | U | 6 | y | n | B; **M** | — |
| 99 | `j_ancient` | Ancient Joker | R | 8 | y | y | **PC** | target `anc`+A |
| 100 | `j_ramen` | Ramen | U | 6 | n | y | D; **M** (generic X) | — |
| 101 | `j_walkie_talkie` | Walkie Talkie | C | 4 | y | y | **PC** | — |
| 102 | `j_selzer` | Seltzer | U | 6 | n | y | **RP**; A | — |
| 103 | `j_castle` | Castle | U | 6 | y | n | D; **M** | target `cas`+A |
| 104 | `j_smiley` | Smiley Face | C | 4 | y | y | **PC** | — |
| 105 | `j_campfire` | Campfire | R | 9 | y | y | SC; EoR; **M** (generic X) | — |
| 106 | `j_ticket` | Golden Ticket | C | 5 | y | y | **PC** | — |
| 107 | `j_mr_bones` | Mr. Bones | U | 5 | n | y | EoR (loss only) | — |
| 108 | `j_acrobat` | Acrobat | U | 6 | y | y | **M** | — |
| 109 | `j_sock_and_buskin` | Sock and Buskin | U | 6 | y | y | **RP** | — |
| 110 | `j_swashbuckler` | Swashbuckler | C | 4 | y | y | frame; **M** | — |
| 111 | `j_troubadour` | Troubadour | U | 6 | y | y | add/remove (hand size, hands) | — |
| 112 | `j_certificate` | Certificate | U | 6 | y | y | **FHD** | `cert_fr`, `certsl` |
| 113 | `j_smeared` | Smeared Joker | U | 7 | y | y | passive (suit tests) | — |
| 114 | `j_throwback` | Throwback | U | 6 | y | y | frame; **M** (generic X); SK (message only) | — |
| 115 | `j_hanging_chad` | Hanging Chad | C | 4 | y | y | **RP** | — |
| 116 | `j_rough_gem` | Rough Gem | U | 7 | y | y | **PC** | — |
| 117 | `j_bloodstone` | Bloodstone | U | 7 | y | y | **PC** | `bloodstone` |
| 118 | `j_arrowhead` | Arrowhead | U | 7 | y | y | **PC** | — |
| 119 | `j_onyx_agate` | Onyx Agate | U | 7 | y | y | **PC** | — |
| 120 | `j_glass` | Glass Joker | U | 6 | y | n | RPT; UC (The Hanged Man); **M** (generic X) | — |
| 121 | `j_ring_master` | Showman | U | 5 | y | y | passive (pools) | — |
| 122 | `j_flower_pot` | Flower Pot | U | 6 | y | y | **M** | — |
| 123 | `j_blueprint` | Blueprint | R | 10 | y | y | copies the joker to its right | — |
| 124 | `j_wee` | Wee Joker | R | 8 | y | n | PC (growth); **M** | — |
| 125 | `j_merry_andy` | Merry Andy | U | 7 | y | y | add/remove (discards, hand size) | — |
| 126 | `j_oops` | Oops! All 6s | U | 4 | y | y | add/remove (probabilities) | — |
| 127 | `j_idol` | The Idol | U | 6 | y | y | **PC** | target `idol`+A |
| 128 | `j_seeing_double` | Seeing Double | U | 6 | y | y | **M** | — |
| 129 | `j_matador` | Matador | U | 7 | y | y | **M**; **DH** | — |
| 130 | `j_hit_the_road` | Hit the Road | R | 8 | y | y | D; EoR; **M** (generic X) | — |
| 131 | `j_duo` | The Duo | R | 8 | y | y | **M** (generic X, Pair) | — |
| 132 | `j_trio` | The Trio | R | 8 | y | y | **M** (generic X, Three of a Kind) | — |
| 133 | `j_family` | The Family | R | 8 | y | y | **M** (generic X, Four of a Kind) | — |
| 134 | `j_order` | The Order | R | 8 | y | y | **M** (generic X, Straight) | — |
| 135 | `j_tribe` | The Tribe | R | 8 | y | y | **M** (generic X, Flush) | — |
| 136 | `j_stuntman` | Stuntman | R | 7 | y | y | add/remove (hand size); **M** | — |
| 137 | `j_invisible` | Invisible Joker | R | 8 | n | y | EoR; SS | `invisible` (+ `to_do` when copying a To Do List) |
| 138 | `j_brainstorm` | Brainstorm | R | 10 | y | y | copies the leftmost joker | — |
| 139 | `j_satellite` | Satellite | U | 6 | y | y | CO | — |
| 140 | `j_shoot_the_moon` | Shoot the Moon | C | 5 | y | y | **HC** | — |
| 141 | `j_drivers_license` | Driver's License | R | 7 | y | y | frame; **M** | — |
| 142 | `j_cartomancer` | Cartomancer | U | 6 | y | y | **SB** | creation `Tarotcar`+A |
| 143 | `j_astronomer` | Astronomer | U | 8 | y | y | add/remove (prices); passive | — |
| 144 | `j_burnt` | Burnt Joker | R | 8 | y | y | **PD** | — |
| 145 | `j_bootstraps` | Bootstraps | U | 7 | y | y | **M** | — |
| 146 | `j_caino` | Caino | L | 20 | y | y | RPT; **M** | — |
| 147 | `j_triboulet` | Triboulet | L | 20 | y | y | **PC** | — |
| 148 | `j_yorick` | Yorick | L | 20 | y | y | D; **M** (generic X) | — |
| 149 | `j_chicot` | Chicot | L | 20 | y | y | SB; add | — |
| 150 | `j_perkeo` | Perkeo | L | 20 | y | y | **ES** | `perkeo` |

Pool notes (`shop.md` 2.3): Stone Joker, Lucky Cat, Golden Ticket and Glass
Joker are offered only while the full deck holds at least one Stone, Lucky, Gold
or Glass card respectively. Legendaries come only from The Soul.

The data's "blueprint compatible" flag only sets the label shown on Blueprint and
Brainstorm. What a copy really does is decided per effect, as listed here. The
one real mismatch in this range: Luchador and Diet Cola are labelled compatible,
but a copy acts only when the copier itself is sold (section 1.3). Every joker of
this range labelled incompatible has no copyable effect.

### 2.1 By context (for the engine's dispatch)

When the boss debuffs the whole hand, only the debuffed-hand pass and the
'after' pass run (`scoring.md` 10).

| Context | Jokers of this range (copyable ones marked *) |
|---|---|
| 'before' (non-debuffed hands only) | Spare Trousers |
| played-card loop (per trigger) | Photograph*, Lucky Cat (growth), Wee Joker (growth), The Idol*, Smiley Face*, Golden Ticket*, Walkie Talkie*, Rough Gem*, Onyx Agate*, Arrowhead*, Bloodstone*, Ancient Joker*, Triboulet* |
| held-card loop (per pass) | Shoot the Moon*, Reserved Parking* |
| repetition, played cards | Sock and Buskin*, Hanging Chad*, Seltzer* |
| repetition, held cards | none |
| main (joker loop) | Erosion*, Fortune Teller*, Stone Joker*, Lucky Cat*, Bull*, Flash Card*, Popcorn*, Spare Trousers*, Ramen*, Castle*, Campfire*, Acrobat*, Swashbuckler*, Throwback*, Glass Joker*, Flower Pot*, Wee Joker*, Seeing Double*, Matador*, Hit the Road*, The Duo*, The Trio*, The Family*, The Order*, The Tribe*, Stuntman*, Driver's License*, Bootstraps*, Caino*, Yorick* |
| joker-on-joker | Baseball Card* |
| removal report | Caino, Glass Joker |
| 'after' (every hand, debuffed ones too) | Seltzer |
| debuffed hand | Matador* |
| blind start | Chicot (Boss only), Cartomancer* |
| first hand drawn | Certificate* (Trading Card: display only) |
| before discard | Burnt Joker* |
| discard, per card (The Hook's too) | Ramen, Yorick, Trading Card, Castle, Mail-In Rebate*, Hit the Road |
| end of round (main pass) | Campfire, Turtle Bean, Invisible Joker, Popcorn, Gift Card, Hit the Road, Mr. Bones (none copyable) |
| cash-out rows | Golden Joker, Satellite; To the Moon through interest |
| pack opened | Hallucination* |
| another card sold | Campfire |
| this card sold | Luchador, Diet Cola, Invisible Joker; a sold Blueprint or Brainstorm runs its target's (section 1.3) |
| shop reroll | Flash Card |
| leaving the shop | Perkeo* |
| blind skipped | Throwback (display only) |
| consumable used | Glass Joker (The Hanged Man only); Fortune Teller (display only) |
| add to / remove from the joker area | Turtle Bean, To the Moon, Juggler, Drunkard, Troubadour, Merry Andy, Oops! All 6s, Stuntman, Astronomer, Chicot (add only) |
| every frame | Throwback, Driver's License, Stone Joker, Swashbuckler (and the Blueprint/Brainstorm label) |
| found by name (passive) | Smeared Joker (suit tests), Showman (pools), Astronomer (prices) |

## 3. The jokers

### 77. Luchador (`j_luchador`)

- Uncommon, $5. Cannot be Eternal. No stored values.
- **This card sold**: if the current blind is a Boss blind (any blind other than
  Small and Big that has a name) and is not disabled, it is disabled at once, at
  the sale click, before the sale money (which is queued). See `blinds.md` 11 for
  what that does, including the immediate win when the round total already meets
  the reduced target of The Wall or Violet Vessel.
- Small and Big blinds, an already disabled boss, the blind-select screen and the
  shop: nothing (after a boss is beaten, the blind is cleared on the cash-out
  screen). In practice it acts when sold during a Boss round's hand selection;
  selling is refused while cards are in play and during end-of-round processing
  (`timing.md` 6).
- Debuffed when sold: nothing (the routine returns at once).
- Not copied, except when a Blueprint or Brainstorm copying it is itself sold
  (section 1.3).
- Verdant Leaf: selling any joker disables it anyway (through a queued step);
  Luchador disables it at the click.

### 78. Photograph (`j_photograph`)

- Common, $5. Start: extra 2.
- **Played-card loop**, per trigger: Photograph finds the first card among the
  scoring cards, in play order, that passes the face test. If the card being
  scored is that card: X2. Copyable.
- Every trigger of that card gives X2 again (Red seal, Sock and Buskin, Hanging
  Chad, Seltzer, Dusk, Hack).
- A debuffed face card fails the face test, so the next non-debuffed face card is
  the one. Scoring cards include Stone cards, and every played card with Splash.
  With Pareidolia the first non-debuffed scoring card gets it, Stone cards
  included.

### 79. Gift Card (`j_gift`)

- Uncommon, $6. Start: extra 1.
- **End of round** (every round end, in joker order; not copyable; skipped while
  debuffed): every card in the joker area (Gift Card itself, debuffed and Eternal
  jokers included) and every card in the consumable area gets sell bonus +1, and
  its price and sell value are recomputed from current prices (`shop.md` 5.4).
- Several Gift Cards stack. The higher sell values also feed Swashbuckler,
  Ceremonial Dagger and the sale money.

### 80. Turtle Bean (`j_turtle_bean`)

- Uncommon, $6. Cannot be Eternal. Start: extra = {hand size 5, step 1}.
- **Added to the joker area** (bought, created, copied, un-debuffed): hand size
  + its current hand-size value. **Removed** (sold, destroyed, eaten, debuffed):
  hand size − its current value (section 1.7).
- **End of round** (not copyable; skipped while debuffed): if value − 1 ≤ 0, it
  is eaten (removal queued; the removal takes its remaining 1 away). Otherwise
  value −1 and hand size −1.
- A new Turtle Bean therefore gives +5, +4, +3, +2, +1 hand size over its first
  five rounds and is eaten at the fifth round end. A round end while it is
  debuffed (Crimson Heart's last pick) skips a step.
- A copy (Invisible Joker) takes the current value and applies it on arrival.

### 81. Erosion (`j_erosion`)

- Uncommon, $6. Start: extra 4.
- **Main:** d = starting deck size − number of cards in the full deck now (read
  live during the evaluation). If d > 0: +4 × d mult. Copyable.
- Starting deck size is set once, when the run's deck is built (52 for most
  decks, 40 for Abandoned Deck, challenge decks vary), and never changes.
- Every playing card counts wherever it is (draw pile, hand, play, discard pile).
  A DNA copy made in this hand's 'before' pass counts at once. Cards destroyed in
  this hand still count (they leave the list after the hand). Added cards
  (Marble Joker, Certificate, packs, Cryptid, ...) lower d.

### 82. Reserved Parking (`j_reserved_parking`)

- Common, $6. Start: extra = {odds 2, $1}.
- **Held-card loop**, each pass of each held card, per Reserved Parking or copy in
  joker order: only if the card passes the face test, roll `parking`; below p/2:
  +$1 (pending money +1 at once, the money queued). A failed roll returns
  nothing. Copyable (each copy rolls).
- Non-faces draw nothing. A debuffed card is never a face: no roll and no
  "debuffed" message (unlike Baron).
- Retriggers: Mime and a Red seal retrigger a held card only if its first pass
  produced something (`scoring.md` 5); a successful Parking roll counts, a failed
  one does not. Each retrigger rolls again.
- Pareidolia: every non-debuffed held card rolls, Stone cards included.
- Draw order: held cards left to right, each card's passes in a row, jokers in
  order within a pass (`scoring.md` 14).

### 83. Mail-In Rebate (`j_mail`)

- Common, $4. Start: extra 5.
- **Discard**, per discarded card (player discards and The Hook's forced
  discards): if the card is not debuffed and its rank id equals the Mail-In rank
  id (section 1.5): +$5 (queued). Copyable: each copy pays again.
- Stone cards never match. Suit, Wild, Smeared Joker and Pareidolia do not
  matter.
- A card destroyed by Trading Card in the same discard still pays, since every
  joker is asked about every card.

### 84. To the Moon (`j_to_the_moon`)

- Uncommon, $5. Start: extra 1.
- **Added** (also un-debuffed): interest per $5 (`G.GAME.interest_amount`, 1 at
  run start) +1. **Removed or debuffed:** −1. Not copyable.
- Cash-out interest = interest per $5 × min(floor(money / 5), cap / 5), only when
  money ≥ $5 (`round.md` 9). One To the Moon: $2 per $5; two: $3.
- If it is debuffed when the cash-out rows are computed (Crimson Heart's last
  pick lasts until then), that cash-out uses the lower rate.

### 85. Hallucination (`j_hallucination`)

- Common, $4. Start: extra 2.
- **Pack opened** (every booster: Arcana, Celestial, Spectral, Standard, Buffoon;
  from the shop or from a tag), per Hallucination or copy in joker order, at the
  moment the pack opens (after its price is paid, before its contents exist):
  1. No consumable room: nothing, no draw.
  2. Otherwise roll `halu` + A; below p/2: pending +1 at once and a Tarot creation
     is queued (append `hal`, pool `Tarothal` + A, resamples `Tarothal` + A +
     `_resample` + n; no Soul check).
- The Tarot is created after the pack's contents (`shop.md` 3.6; the pack's own
  cards appear about 1.3 time units after opening, while the Tarot waits behind
  the opening animation's blocking steps). So the Tarot's pick treats the pack's
  Tarots as present, and the pack never avoids it.
- Copyable: a copy rolls again; its room check counts the earlier pending Tarot.

### 86. Fortune Teller (`j_fortune_teller`)

- Common, $6. Start: extra 1 (not used by the effect).
- **Main:** if at least one Tarot has been used this run: + (number of Tarot uses
  this run) mult. Copyable.
- The count rises by 1 at the start of every Tarot use (from the consumable
  slots, from an Arcana pack, or buy-and-use in the shop). The same Tarot used
  twice counts twice. Planet and Spectral uses do not count. It never resets.
- **Consumable used** (a Tarot): a message only.

### 87. Juggler (`j_juggler`)

- Common, $4. Start: hand size 1.
- **Added** (also un-debuffed): hand size +1. **Removed or debuffed:** −1
  (section 1.7). Not copyable. No other effect.

### 88. Drunkard (`j_drunkard`)

- Common, $4. Start: discard size 1.
- **Added** (also un-debuffed): base discards per round +1 at once, and discards
  left +1 (queued). **Removed or debuffed:** base −1 and discards left −1 (never
  below 0). Not copyable.
- Bought in the shop it raises the shown discards; the next round starts from the
  new base anyway.

### 89. Stone Joker (`j_stone`)

- Uncommon, $6. Start: extra 25.
- **Every frame:** Stone tally (section 1.4).
- **Main:** if the tally > 0: +25 × tally chips. Copyable.
- Previous-frame value: a Stone card stripped by Vampire, or turned Gold by Midas
  Mask with Pareidolia, still counts this hand; a DNA copy of a Stone card counts
  from the next hand.
- Pool: offered only while the full deck holds a Stone card.

### 90. Golden Joker (`j_golden`)

- Common, $6. Start: extra 4.
- **Cash-out row** (in joker order): +$4 if not debuffed when the rows are
  computed. Not copyable. No other effect.

### 91. Lucky Cat (`j_lucky_cat`)

- Uncommon, $6. Cannot be Perishable. Start: x_mult 1, extra 0.25.
- **Played-card loop** (not copyable; skipped while debuffed): on any trigger of
  a scoring card whose own Lucky rolls hit on that trigger (`lucky_mult` below
  p/5, or `lucky_money` below p/15, or both): x_mult +0.25 at once. At most +0.25
  per trigger. The card's "Lucky hit" mark is set by its own rolls and cleared
  after the jokers' per-card step of the same trigger.
- **Main:** generic X once x_mult > 1. Copyable. Gains in this hand's played-card
  loop already count in this hand's main effect.
- A debuffed Lucky card is skipped (no rolls). A Lucky card stripped by Vampire
  earlier in the 'before' pass rolls nothing.
- Pool: offered only while the full deck holds a Lucky card.

### 92. Baseball Card (`j_baseball`)

- Rare, $8. Start: extra 1.5.
- **Joker-on-joker** (`other_joker`): in the joker loop, right after each card
  C's own main effect (jokers, then consumables), every joker is asked about C,
  left to right. Baseball Card gives X1.5 when C is an Uncommon joker (rarity 2)
  and is not this same Baseball Card. It comes before C's Polychrome X1.5
  (`scoring.md` 6). Copyable: each Blueprint or Brainstorm copying it adds
  another X1.5 per Uncommon joker.
- C's debuff does not matter: a debuffed Uncommon joker still receives X1.5 (it
  gives nothing itself). Consumables are never Uncommon jokers.
- Baseball Card is Rare, so two of them do not boost each other.

### 93. Bull (`j_bull`)

- Uncommon, $6. Start: extra 2.
- **Main:** m = money + pending money (`scoring.md` 13.1, so it includes money
  earned earlier in this hand, and a Matador's $8 if that Matador is to its
  left). If m > 0: +2 × m chips. Copyable.

### 94. Diet Cola (`j_diet_cola`)

- Uncommon, $6. Cannot be Eternal. No stored values.
- **This card sold**: a Double Tag is added to the held tags (one queued step).
  Tags already held react as to any new tag; a Double Tag never copies another
  Double Tag, so the new one simply waits for the next tag gained. No draws.
- Debuffed when sold: nothing. Not copied, except when a Blueprint or Brainstorm
  copying it is itself sold (section 1.3).

### 95. Trading Card (`j_trading`)

- Uncommon, $6. Start: extra 3.
- **Discard** (not copyable; skipped while debuffed): if no discard has been used
  this round yet (the "discards used" count rises only at the end of the discard
  action) and this discard holds exactly 1 card: +$3 (queued), and that card is
  destroyed instead of going to the discard pile (a Glass card is shattered,
  any other card dissolves). A removal report follows (section 1.6: Caino, Glass
  Joker).
- The card's own discard effect (Purple seal) is checked before the jokers, and
  every other joker still sees the card (Mail-In Rebate pays, Castle grows, Ramen
  and Yorick count it).
- The Hook's forced discard runs it too: from the code, when The Hook takes
  exactly 1 card (only 1 card left in hand after the play) while no discard has
  been used this round, Trading Card pays $3 and destroys it (see UNSURE).
- Burnt Joker on the same first discard levels High Card.
- **First hand drawn:** display only.

### 96. Flash Card (`j_flash`)

- Uncommon, $5. Cannot be Perishable. Start: mult 0, extra 2.
- **Shop reroll** (every reroll, paid or free: Chaos the Clown, D6 Tag; not
  copyable; skipped while debuffed): mult +2, after the new shop cards are made.
- **Main:** +mult if mult > 0. Copyable.

### 97. Popcorn (`j_popcorn`)

- Common, $5. Cannot be Eternal. Start: mult 20, extra 4.
- **Main:** +mult if mult > 0. Copyable.
- **End of round** (not copyable; skipped while debuffed): if mult − 4 ≤ 0 it is
  eaten (removal queued); otherwise mult −4. It gives 20, 16, 12, 8, 4 and is
  eaten at the fifth round end.

### 98. Spare Trousers (`j_trousers`)

- Uncommon, $6. Cannot be Perishable. Start: mult 0, extra 2.
- **'Before'** (non-debuffed hands; not copyable): if the played hand contains
  Two Pair or Full House (`hands.md` 4; a Full House also contains Two Pair, so
  this is the Two Pair test): mult +2 at once, so this hand's main effect
  includes it.
- **Main:** +mult if mult > 0. Copyable.
- Contained hands use all played cards. Stone cards never help form them.

### 99. Ancient Joker (`j_ancient`)

- Rare, $8. Start: extra 1.5.
- **Played-card loop:** per trigger of a scoring card that passes the scoring suit
  test for the Ancient suit (section 1.5): X1.5. Copyable.
- Wild: always. Smeared Joker: the other suit of the same colour too. Stone:
  never.
- The suit changes at every round end and never repeats the previous one.

### 100. Ramen (`j_ramen`)

- Uncommon, $6. Cannot be Eternal. Start: x_mult 2, extra 0.01.
- **Discard**, per discarded card (any card: debuffed and Stone included; The
  Hook's too; not copyable; skipped while debuffed): if x_mult − 0.01 ≤ 1, it is
  eaten (removal queued; x_mult is left as it was). Otherwise x_mult −0.01 at
  once.
- With repeated subtraction it survives 99 cards, ending at 1.0099999999999991,
  and the 100th card eats it (section 1.8). Later cards of that same discard
  queue the removal again, which is harmless.
- **Main:** generic X; x_mult is always above 1 while it exists. Copyable.
- The Hook's discards lower it before the hand is scored.

### 101. Walkie Talkie (`j_walkie_talkie`)

- Common, $4. Start: extra = {chips 10, mult 4}.
- **Played-card loop:** per trigger of a scoring card with rank id 10 or 4: +10
  chips, then +4 mult (in that order inside the result). Copyable. Stone never.

### 102. Seltzer (`j_selzer`)

- Uncommon, $6. Cannot be Eternal. Start: extra 10 (hands remaining).
- **Repetition, played cards:** +1 retrigger for every non-debuffed scoring card
  (a fixed 1, not the counter). Copyable: each copy adds 1.
- **'After'** (every played hand, debuffed hands included; not copyable; skipped
  while debuffed): if counter − 1 ≤ 0 it is eaten (removal queued); otherwise
  counter −1. It works for 10 hands and is eaten after the 10th.

### 103. Castle (`j_castle`)

- Uncommon, $6. Cannot be Perishable. Start: extra = {chips 0, step 3}.
- **Discard** (not copyable; skipped while debuffed), per discarded card, The
  Hook's included: if the card is not debuffed and passes the scoring suit test
  for the Castle suit (section 1.5): chips +3 at once.
- **Main:** +chips if chips > 0. Copyable.
- Wild: always. Smeared Joker: the same-colour suit too. Stone: never. The Hook's
  discards add before the hand is scored.

### 104. Smiley Face (`j_smiley`)

- Common, $4. Start: extra 5.
- **Played-card loop:** per trigger of a scoring card that passes the face test:
  +5 mult. Copyable. Pareidolia: every non-debuffed scoring card, Stone cards
  included.

### 105. Campfire (`j_campfire`)

- Rare, $9. Start: x_mult 1, extra 0.25.
- **Another card sold** (`selling_card`): every sale through the sell button, of
  a joker or a consumable, is announced to every joker except the card sold,
  right after its own sale effect. Campfire (not copyable; skipped while
  debuffed): x_mult +0.25 at once. The sold card's debuff does not matter.
- **End of round** (not copyable; skipped while debuffed): if the blind was a Boss
  and x_mult > 1: x_mult = 1. This includes a Boss round saved by Mr. Bones.
- **Main:** generic X once x_mult > 1. Copyable.

### 106. Golden Ticket (`j_ticket`)

- Common, $5. Start: extra 4.
- **Played-card loop:** per trigger of a scoring Gold card: +$4 (pending +4 at
  once; money queued). Copyable.
- A face card turned Gold by Midas Mask earlier in the 'before' pass counts this
  hand; a Gold card stripped by Vampire does not.
- Pool: offered only while the full deck holds a Gold card.

### 107. Mr. Bones (`j_mr_bones`)

- Uncommon, $5. Cannot be Eternal. No stored values.
- **End of round**, main pass only (not copyable; skipped while debuffed): when
  its turn comes, if the loss flag is set and round total ÷ blind target ≥ 0.25,
  the loss flag is cleared for the rest of the pass (jokers to its right see a
  won round), and Mr. Bones dissolves (removal queued).
- The round then continues as a win: end-of-round processing, the cash-out
  screen with $0 for the blind (that row shows as saved; hands left, jokers, tags
  and interest pay as usual), then the shop. Saving an ante-8 boss wins the run
  (`round.md` 7, 11).
- Two Mr. Bones (Showman, copies): only the first one reached with the flag set
  is used; the other stays.
- Debuffed (Crimson Heart's last pick, expired Perishable): no save. A game over
  that skips the end of round (a draw with hand size 0 and an empty hand) cannot
  be saved.

### 108. Acrobat (`j_acrobat`)

- Uncommon, $6. Start: extra 3.
- **Main:** X3 if hands left is 0 during the evaluation (the round's last
  available hand; hands left drops at Play). Copyable.

### 109. Sock and Buskin (`j_sock_and_buskin`)

- Uncommon, $6. Start: extra 1.
- **Repetition, played cards:** +1 retrigger for each non-debuffed scoring card
  that passes the face test. Copyable (+1 per copy). Pareidolia: every
  non-debuffed scoring card.

### 110. Swashbuckler (`j_swashbuckler`)

- Common, $4. Start: mult 1, replaced at the first frame refresh.
- **Every frame:** mult = sum of the sell values of every other card in the joker
  area (debuffed, Eternal, Rental and Negative jokers included; other
  Swashbucklers included; consumables not). A sold joker counts until its
  dissolve ends.
- **Main:** +mult if mult > 0. Copyable (a copy adds the copied Swashbuckler's
  value).

### 111. Troubadour (`j_troubadour`)

- Uncommon, $6. Start: extra = {hand size 2, hands −1}.
- **Added** (also un-debuffed): hand size +2; base hands per round −1.
  **Removed or debuffed:** the reverse. Not copyable. Hands left in the current
  round do not change (section 1.7).

### 112. Certificate (`j_certificate`)

- Uncommon, $6. No stored values.
- **First hand drawn**, copyable (each copy makes another card). The event is
  raised on a draw to hand while no hand has been played and no discard used this
  round, during a blind: normally only the opening draw of each selected blind.
  1. At once: "playing card added" is raised with one card (Hologram +0.25, even
     before the card exists).
  2. Queued, after the drawn cards arrive: pick a rank and suit with `cert_fr`
     from the 52 card keys in sorted order (`rng_keys.md` 2); create a plain card
     (no enhancement, no edition) directly in the hand and add it to the full
     deck; roll `certsl` for its seal: above 0.75 Red, above 0.5 Blue, above 0.25
     Gold, otherwise Purple (it always gets one); run the boss's debuff test on
     it; re-sort the hand by the current sort mode.
- Each Certificate or copy, in joker order, draws `cert_fr` then `certsl` when its
  step runs.
- The hand then holds one card more than the hand size (nothing is discarded).
  The card is a permanent deck card; the deck's displayed limit is not raised.
- It is never drawn, so it is never face down (The Wheel, The House, The Mark).
  The blind's after-draw effect runs after it, so Cerulean Bell may force it.
- From the code, if the hand is emptied by consumables before the first play or
  discard, the automatic refill raises the event again (another card).

### 113. Smeared Joker (`j_smeared`)

- Uncommon, $7. Passive; not copyable; only while held and not debuffed.
- Every suit test treats Hearts and Diamonds as one suit and Spades and Clubs as
  one: a card whose printed suit is red counts as both red suits, a black one as
  both black suits. This covers: flush detection (`hands.md` 3.2; Stone cards
  never, Wild cards unless debuffed), every joker suit test during scoring
  (`scoring.md` 12.8: the four suit-mult jokers, Rough Gem, Bloodstone,
  Arrowhead, Onyx Agate, Ancient Joker, The Idol), Castle's discard test, Flower
  Pot and Seeing Double (their own sections), and the boss suit debuffs (The
  Club, The Goad, The Head, The Window: with Smeared Joker The Club also debuffs
  Spades). Blackboard is unchanged.
- Gaining, losing, debuffing or un-debuffing it queues a fresh boss debuff test of
  every playing card (`jokers_1_76.md` 1.4), so selling it during The Club
  un-debuffs the Spades.

### 114. Throwback (`j_throwback`)

- Uncommon, $6. Start: extra 0.25.
- **Every frame:** x_mult = 1 + 0.25 × blinds skipped this run. The count rises at
  each skip click, before the tag is taken; skips made before Throwback was
  owned count.
- **Main:** generic X once at least one blind was skipped. Copyable.
- **Blind skipped:** a message only.

### 115. Hanging Chad (`j_hanging_chad`)

- Common, $4. Start: extra 2.
- **Repetition, played cards:** +2 retriggers for the first scoring card in play
  order (it may be a Stone card; with Splash, the first played card). Copyable
  (+2 per copy). If that card is debuffed it is skipped entirely and no other
  card gets the retriggers.

### 116. Rough Gem (`j_rough_gem`)

- Uncommon, $7. Start: extra 1.
- **Played-card loop:** per trigger of a scoring card that passes the scoring suit
  test for Diamonds: +$1 (pending +1 at once; money queued). Copyable. Wild:
  yes. Smeared Joker: Hearts too. Stone: never.

### 117. Bloodstone (`j_bloodstone`)

- Uncommon, $7. Start: extra = {odds 2, X 1.5}.
- **Played-card loop**, per trigger, per Bloodstone or copy in joker order: only if
  the card passes the scoring suit test for Hearts, roll `bloodstone`; below p/2:
  X1.5. Non-Hearts draw nothing. Copyable (a copy rolls again, a Blueprint before
  the original). Wild: yes. Smeared Joker: Diamonds too. Stone: never.

### 118. Arrowhead (`j_arrowhead`)

- Uncommon, $7. Start: extra 50.
- **Played-card loop:** per trigger of a scoring card that passes the scoring suit
  test for Spades: +50 chips. Copyable. Wild: yes. Smeared Joker: Clubs too.
  Stone: never.

### 119. Onyx Agate (`j_onyx_agate`)

- Uncommon, $7. Start: extra 7.
- **Played-card loop:** per trigger of a scoring card that passes the scoring suit
  test for Clubs: +7 mult. Copyable. Wild: yes. Smeared Joker: Spades too. Stone:
  never.

### 120. Glass Joker (`j_glass`)

- Uncommon, $6. Cannot be Perishable. Start: x_mult 1, extra 0.75.
- **Removal report** (not copyable; skipped while debuffed): g = number of listed
  cards marked shattered (section 1.6). If g > 0: x_mult + 0.75 × g, applied two
  queued hops later (after the hand's evaluation, or after the discard).
- **Consumable used** (not copyable; skipped while debuffed), only for The Hanged
  Man: g = number of Glass cards among the selected hand cards at that moment,
  debuffed or not; x_mult + 0.75 × g at once.
- **Main:** generic X once x_mult > 1. Copyable.
- Counted: Glass cards shattered by their own roll (only non-debuffed scoring
  Glass cards roll); a Glass 6 destroyed by Sixth Sense; a Glass card destroyed by
  Trading Card; Glass cards destroyed by The Hanged Man. Not counted: Glass cards
  destroyed by Familiar, Grim, Incantation or Immolate (see UNSURE).
- Pool: offered only while the full deck holds a Glass card.

### 121. Showman (`j_ring_master`)

- Uncommon, $5. Key `j_ring_master`. Passive; not copyable; only while held and
  not debuffed (it is searched for in the joker and consumable areas).
- Pools ignore the "already present" rule (`shop.md` 2.3, 2.4): jokers, Tarots,
  Planets and Spectrals that are held or currently shown can be created again, in
  every creation that builds a pool (shop slots, packs, Judgement, Wraith, The
  Soul's legendary, Riff-raff, Top-up Tag, Rare and Uncommon Tags, The Emperor,
  The High Priestess, 8 Ball, Purple seal, Vagabond, Superposition, Seance, Sixth
  Sense, Hallucination, Cartomancer). Vouchers still exclude redeemed ones and the
  one shown in the shop.
- The Soul and Black Hole checks of pack contents (`rng_keys.md` 1, step 1) are
  made even while one already exists, so their `soul_` streams are drawn in cases
  where they otherwise would not be.
- Pool order and stream keys are unchanged; only which entries are unavailable
  changes, and with it the resample draws.
- It is the usual way to hold two copies of one joker (two Chicots, two Mr. Bones,
  two Perkeos...); see those jokers.

### 122. Flower Pot (`j_flower_pot`)

- Uncommon, $6. Start: extra 3.
- **Main:** X3 if the scoring cards fill all four suits under this assignment.
  Copyable.
  1. Each scoring card that is not a Wild card (by enhancement; a debuffed Wild
     card still counts as Wild here), in play order, fills at most one suit: the
     first of Hearts, Diamonds, Spades, Clubs that it matches and that is still
     empty. This match **ignores debuff** (a debuffed card counts by its suit).
     Stone cards match nothing. With Smeared Joker a red card matches both red
     suits and a black card both black suits.
  2. Then each Wild card, in play order, fills the first still-empty suit in the
     order Hearts, Diamonds, Spades, Clubs. A debuffed Wild card fills nothing.
- At least four scoring cards are needed: Two Pair, Four of a Kind or a five-card
  hand (four-card straights and flushes with Four Fingers), or extra Stone cards,
  or Splash. Examples: Heart, Diamond, Spade, Wild: X3.
  With Smeared Joker any two red and two black scoring cards give X3 (red, red
  fill Hearts then Diamonds; black, black fill Spades then Clubs).
- A debuffed Club under The Club still fills Clubs.

### 123. Blueprint (`j_blueprint`)

- Rare, $10. No stored values.
- **Every event it receives** (all families of `jokers_1_76.md` 1.1): if a joker
  sits directly to its right, Blueprint runs that joker's routine for the same
  event with a copy marker. The outermost copier is recorded and a depth counter
  rises by 1; if the depth exceeds (number of jokers + 1), nothing happens (this
  ends copy loops). If the copied routine returns a result, it is credited to the
  outermost copier and applied at that copier's place in the joker order.
  Otherwise nothing: Blueprint has no effect of its own.
- It cannot copy anything outside the joker routine: add/remove effects (Juggler,
  Credit Card, Chaos the Clown, Oops! All 6s, ...), per-frame values, cash-out
  rows (Golden Joker, Satellite, Rocket, Cloud 9, Delayed Gratification), and
  passive effects found by name (Smeared Joker, Showman, Pareidolia, Four
  Fingers, Shortcut, Splash, Astronomer). Inside the routine it does not copy any
  effect marked not copyable in this file or in `jokers_1_76.md`.
- State changes made in copyable branches happen on the copied joker (Loyalty
  Card's counter, pending consumables).
- Selling it: section 1.3. Debuffed Blueprint, or debuffed target: nothing.
- Order of draws: a Blueprint sits to the left of what it copies, so it draws
  before the original (`jokers_1_76.md` 1.3).

### 124. Wee Joker (`j_wee`)

- Rare, $8. Cannot be Perishable. Start: extra = {chips 0, step 8}.
- **Played-card loop** (not copyable; skipped while debuffed): per trigger of a
  scoring card with rank id 2: chips +8 at once. Stone never.
- **Main:** + chips (it always returns its value; 0 adds nothing). Copyable. This
  hand's 2s already count.

### 125. Merry Andy (`j_merry_andy`)

- Uncommon, $7. Start: discard size 3, hand size −1.
- **Added** (also un-debuffed): hand size −1; base discards +3 at once and
  discards left +3 (queued). **Removed or debuffed:** hand size +1 (during hand
  selection this draws a card at once, section 1.7); base discards −3 and
  discards left −3 (never below 0). Not copyable.

### 126. Oops! All 6s (`j_oops`)

- Uncommon, $4. Works through add/remove; not copyable.
- **Added** (also un-debuffed): p (`G.GAME.probabilities.normal`) ×2. **Removed
  or debuffed:** p ÷2. Several stack: p = 2^n with n non-debuffed copies.
- p is read at each roll. It scales exactly these rolls: Lucky card mult
  (`lucky_mult`, p/5) and money (`lucky_money`, p/15), Glass shatter (`glass`,
  p/4), The Wheel of Fortune (`wheel_of_fortune`, p/4), The Wheel boss (`wheel`,
  p/7: more face-down cards), Hallucination (`halu` + A, p/2), Gros Michel
  (`gros_michel`, p/6), Cavendish (`cavendish`, p/1000), 8 Ball (`8ball`, p/4),
  Business Card (`business`, p/2), Bloodstone (`bloodstone`, p/2), Reserved
  Parking (`parking`, p/2), Space Joker (`space`, p/4). The challenge rule
  "flipped cards" is not scaled.
- It never changes how often a stream is drawn: a roll whose chance reaches 1 is
  still drawn (and always succeeds).

### 127. The Idol (`j_idol`)

- Uncommon, $6. Start: extra 2.
- **Played-card loop:** per trigger of a scoring card whose rank id equals the Idol
  rank id and which passes the scoring suit test for the Idol suit (section 1.5):
  X2. Copyable.
- A Wild card of the right rank counts. With Smeared Joker the right rank in the
  other suit of the same colour counts too. Stone: never.

### 128. Seeing Double (`j_seeing_double`)

- Uncommon, $6. Start: extra 2.
- **Main:** X2 if, after this count, Clubs > 0 and at least one of Hearts,
  Diamonds, Spades > 0. Copyable.
  1. Each scoring card that is not a Wild card adds 1 to every suit for which it
     passes the scoring suit test (a debuffed card passes none, a Stone card
     none, with Smeared Joker a card passes both suits of its colour).
  2. Then each Wild card, in play order, adds 1 to the first of Clubs, Diamonds,
     Spades, Hearts that is still 0. A debuffed Wild card adds nothing.
- Examples: a Club and a Heart: X2. One Wild card alone: Clubs only, no. Two Wild
  cards: Clubs then Diamonds: X2. A Wild card and a Heart: X2. With Smeared Joker
  a single non-debuffed scoring Spade or Club counts as both Clubs and Spades:
  X2.
- Ranks do not matter.

### 129. Matador (`j_matador`)

- Uncommon, $7. Start: extra 8.
- **Main**, and **debuffed hand**: if the boss's "triggered" flag is set at that
  moment: +$8 (pending +8 at once; money queued). Copyable (each copy pays).
- The flag is described in `blinds.md` 12 and `scoring.md` 11.2: set when the boss
  zeroes the hand (only the debuffed-hand pass runs then, and Matador pays
  there), when The Arm lowers a level, when The Ox takes the money, whenever The
  Flint is active, or when a scoring card is debuffed. Not by The Hook, The Tooth
  or Crimson Heart alone. A disabled boss sets none of these itself.

### 130. Hit the Road (`j_hit_the_road`)

- Rare, $8. Start: x_mult 1, extra 0.5.
- **Discard** (not copyable; skipped while debuffed), per discarded card, The
  Hook's included: if the card is not debuffed and its rank id is 11 (Jack):
  x_mult +0.5 at once.
- **End of round** (every round end; not copyable; skipped while debuffed): back
  to 1 if above 1.
- **Main:** generic X once x_mult > 1. Copyable.
- Stone never. Pareidolia does not matter (rank based). Jacks taken by The Hook
  raise it before the hand is scored.

### 131. The Duo (`j_duo`)

- Rare, $8. Start: x_mult 2, type Pair.
- **Main:** X2 if the played hand contains a Pair (generic branch, section 1.2).
  Copyable.

### 132. The Trio (`j_trio`)

- Rare, $8. Start: x_mult 3, type Three of a Kind.
- **Main:** X3 if the hand contains Three of a Kind. Copyable.

### 133. The Family (`j_family`)

- Rare, $8. Start: x_mult 4, type Four of a Kind.
- **Main:** X4 if the hand contains Four of a Kind (a rank with at least 4 non-Stone
  played cards, so Five of a Kind and Flush Five too). Copyable.

### 134. The Order (`j_order`)

- Rare, $8. Start: x_mult 3, type Straight.
- **Main:** X3 if the hand contains a Straight (Four Fingers and Shortcut as in
  `hands.md` 3.3). Copyable.

### 135. The Tribe (`j_tribe`)

- Rare, $8. Start: x_mult 2, type Flush.
- **Main:** X2 if the hand contains a Flush (Four Fingers, Wild cards and Smeared
  Joker as in `hands.md` 3.2). Copyable.

"Contains" is decided from all played cards, as for the type jokers 6 to 15
(`jokers_1_76.md`, shared rules for 6 to 15; `hands.md` 4).

### 136. Stuntman (`j_stuntman`)

- Rare, $7. Start: extra = {hand size 2, chips 250}.
- **Main:** +250 chips on every non-debuffed hand. Copyable.
- **Added** (also un-debuffed): hand size −2. **Removed or debuffed:** +2
  (section 1.7).

### 137. Invisible Joker (`j_invisible`)

- Rare, $8. Cannot be Eternal. Start: extra 2, round count 0.
- **End of round** (not copyable; skipped while debuffed): round count +1. Skipped
  blinds and round ends while it is debuffed do not count.
- **This card sold**, when the round count is at least 2 (not via copies;
  skipped while debuffed):
  1. Candidates: every other card in the joker area, debuffed and Eternal ones
     included. With none: nothing.
  2. If the number of cards in the joker area, Invisible Joker included, is above
     the joker slot limit: nothing ("no room"). Normally it is not, so the copy is
     made even with every slot filled; it uses the slot the sale frees.
  3. Pick one candidate with `invisible` (creation order) and copy it with
     `copy_card` (`jokers_1_76.md` 1.7): every stored value, the stickers
     (Eternal, Perishable with its rounds left, Rental), the sell bonus and the
     current debuff state; the edition too, except Negative, which is dropped. A
     copied Invisible Joker's round count is reset to 0. Copying a To Do List
     draws `to_do` once before the original's hand is taken over.
  4. At once, still at the sale click: the copy's add-to-deck effects run and it
     is placed at the right end of the joker area. The sale money and the
     Invisible Joker's dissolve are queued after.
- The copy is already in the joker area when the other jokers are told about the
  sale, so a copied Campfire gains +0.25 from it.
- Add-to-deck effects of the copy apply even if the copy starts debuffed: copying
  Chicot during a Boss round disables the boss at once; copying Oops! All 6s
  doubles p again; copying Turtle Bean adds its current hand size.
- Selling it before 2 rounds: an ordinary sale.

### 138. Brainstorm (`j_brainstorm`)

- Rare, $10. No stored values.
- Same as Blueprint (123), except that it copies the leftmost joker of the joker
  area. A Brainstorm that is itself leftmost copies nothing. Brainstorm copying a
  leftmost Blueprint copies what that Blueprint copies. A Blueprint and a
  Brainstorm that copy each other in a loop give nothing (depth limit).
- Selling it: section 1.3. Order of draws: Brainstorm sits to the right of the
  joker it copies, so it draws after it.

### 139. Satellite (`j_satellite`)

- Uncommon, $6. Start: extra 1.
- **Cash-out row** (in joker order; not copyable): if not debuffed, +$1 × number
  of different Planet cards used this run. Each Planet counts once however often
  it was used; uses from the consumable slots, Celestial packs and buy-and-use all
  count; Black Hole is a Spectral and does not count. No row while that number
  is 0.

### 140. Shoot the Moon (`j_shoot_the_moon`)

- Common, $5. Start: extra 13 (the effect uses a fixed 13).
- **Held-card loop**, each pass of each held Queen (rank id 12; Stone never): if
  the Queen is debuffed, a "debuffed" message only (it counts as an effect, so
  Mime still retriggers the card, again with no value; the card's own Red seal
  never works while it is debuffed); otherwise +13 held mult. Copyable.

### 141. Driver's License (`j_drivers_license`)

- Rare, $7. Start: extra 3.
- **Every frame:** enhanced tally (section 1.4).
- **Main:** X3 if the tally is at least 16. Copyable.
- Previous-frame value: enhancements changed in this hand's 'before' pass
  (Vampire, Midas Mask) count from the next hand.

### 142. Cartomancer (`j_cartomancer`)

- Uncommon, $6. No stored values.
- **Blind start** (every selected blind: Small, Big and Boss; never on skips),
  copyable (each copy makes another Tarot). Skipped if this Cartomancer, or the
  copying card, is marked for destruction at that blind start
  (`jokers_1_76.md` 1.8). If there is consumable room: pending +1 at once; two
  queued hops later a Tarot is created and placed in the consumable area
  (append `car`, pool `Tarotcar` + A, resamples `Tarotcar` + A + `_resample` +
  n; no Soul check).
- No room: nothing, no draw.
- The Tarot exists before the opening draw. Several Cartomancers or copies create
  in joker order; each room check counts the earlier pending ones.

### 143. Astronomer (`j_astronomer`)

- Uncommon, $8. Passive; not copyable; only while held and not debuffed.
- Every Planet card and every Celestial pack costs $0 (`shop.md` 5.1: this
  overrides discounts and the challenge pack surcharge). Prices are set when a
  card is created and are recomputed for every card, through a queued step, when
  Astronomer enters or leaves the joker area (or is debuffed or un-debuffed).
- Sell values follow the new price: a Planet's sell value stays $1 (plus any
  bonus), and a Negative Planet copy (Perkeo) drops from $4 to $1 (plus bonus) at
  base prices.

### 144. Burnt Joker (`j_burnt`)

- Rare, $8. Start: hand size 0 (no effect), extra 4 (not used).
- **Before discard** (`pre_discard`; once per discard action, before any card is
  processed; copyable, each copy levels again): if no discard has been used this
  round yet and this is not The Hook's forced discard, the poker hand formed by
  all the selected cards is detected (with Four Fingers, Shortcut, Smeared Joker
  and Wild cards, as for a played hand; `hands.md`) and that hand gains 1 level
  at once (`hands.md` 6.2).
- Only the round's first player discard. The level-up does not make a secret hand
  visible.

### 145. Bootstraps (`j_bootstraps`)

- Uncommon, $7. Start: extra = {mult 2, per $5}.
- **Main:** k = floor((money + pending money) / 5); if k ≥ 1: +2 × k mult.
  Copyable. Negative money gives nothing.

### 146. Caino (`j_caino`)

- Legendary, $20. Start: extra 1, its own X value (`caino_xmult`) 1.
- **Removal report** (not copyable; skipped while debuffed): f = number of listed
  cards that pass the face test at that moment; if f > 0, X value + f at once
  (section 1.6).
- **Main:** X(X value) if it is above 1. Copyable.
- Sources: face Glass cards that shatter; Sixth Sense's 6 (a face only with
  Pareidolia); a face destroyed by Trading Card; The Hanged Man, Familiar, Grim,
  Incantation and Immolate (counted at use, before the cards disappear).
- A debuffed face card is not a face, so a face destroyed while a boss debuffs it
  adds nothing. With Pareidolia every non-debuffed destroyed card counts.
- Gains from a scored hand arrive after the joker loop, so they count from the
  next hand.

### 147. Triboulet (`j_triboulet`)

- Legendary, $20. Start: extra 2.
- **Played-card loop:** per trigger of a scoring King or Queen (rank id 13 or 12):
  X2. Copyable. Jacks never; Stone never; Pareidolia does not extend it (rank
  based).

### 148. Yorick (`j_yorick`)

- Legendary, $20. Start: x_mult 1, extra = {X +1, every 23}, counter 23.
- **Discard** (not copyable; skipped while debuffed), per discarded card (any card;
  The Hook's too): if the counter is 1 or less, it goes back to 23 and x_mult +1;
  otherwise counter −1. So every 23rd card discarded while it is held (and not
  debuffed) adds X1. The counter never resets otherwise.
- **Main:** generic X once x_mult > 1. Copyable.

### 149. Chicot (`j_chicot`)

- Legendary, $20. No stored values.
- **Blind start**, Boss blinds only (not copyable; skipped if Chicot is marked for
  destruction, for example by a Ceremonial Dagger to its left at the same blind
  start): a step is queued that queues the disable. That disable therefore runs
  after every other blind-start step (Burglar, Marble Joker, Riff-raff,
  Cartomancer's first hop...), after the round's deck shuffle (`nr` + A) and after
  Amber Acorn's three `aajk` shuffles, but before the opening draw. Consequences:
  - draw-time boss effects never happen (The Wheel, The House, The Mark, The
    Fish, Cerulean Bell, Crimson Heart);
  - The Water's discards, The Needle's hands and The Manacle's hand size are back
    before the draw; The Wall's and Violet Vessel's targets are reduced;
  - Amber Acorn still shuffles the jokers (and draws `aajk` three times); they then
    turn face up.
  Details of disabling: `blinds.md` 11.
- **Added to the joker area** (bought, created, copied or un-debuffed) while a
  Boss blind is current and not disabled: disables it at once. This happens for
  The Soul used during a boss round, or a copy by Invisible Joker or Ankh.
- Two Chicots at one Boss blind start (Showman, copies): two disables, because the
  disable step has no guard (section 1.10). From the code: The Water gives its
  discards back twice, The Needle its hands twice, The Manacle gives +1 hand size
  twice (drawing a card each time) while its end-of-blind restore is then skipped
  (a lasting +1 hand size), The Wall's target ends at a quarter and Violet
  Vessel's at a ninth (see UNSURE).
- Debuffed (expired Perishable): no effect. Selling it later changes nothing; a
  disabled boss stays disabled.

### 150. Perkeo (`j_perkeo`)

- Legendary, $20. No stored values.
- **Leaving the shop** (`ending_shop`, raised when the player presses the button
  that leaves the shop), copyable (each copy makes another): if at least one
  consumable is held at that moment, a step is queued. When it runs it picks one
  held consumable with `perkeo` (creation order), copies it (`copy_card`: every
  stored value and the sell bonus), makes the copy Negative, and adds it to the
  consumable area. A Negative consumable adds one consumable slot, so the copy
  always fits.
- Several Perkeos or copies queue their steps in joker order; each picks among the
  consumables held when it runs, so a later one may copy an earlier one's copy.
- No consumable held: no step, no draw.
- The copy's price includes the Negative surcharge, so its sell value is higher
  than the original's (`shop.md` 5.4), except for Planets under Astronomer.

## UNSURE

- **Money from The Hook's forced discard.** Mail-In Rebate, Faceless Joker
  and Trading Card pay through queued money steps. The engine adds that money
  before the hand is scored, so Bull, Bootstraps and Vagabond see it. If those
  steps land after the scoring step in the game, they would not. Check with
  BalatroBot (The Hook, Mail-In Rebate, Bull).
- **Ramen eaten by The Hook's discard.** The engine removes it before the hand
  is scored (the eaten animation is queued behind the discard). If it is still
  in the joker area during scoring, it would give its last X value once more.
- **Two Chicots.** From the code, each Chicot's blind-start disable runs without
  checking whether the boss is already disabled, so the restoring effects stack
  (The Water's discards and The Needle's hands twice, a lasting +1 hand size from
  The Manacle, The Wall's target at a quarter, Violet Vessel's at a ninth). Not
  verified in game; check with BalatroBot before relying on it.
- **Selling a Blueprint or Brainstorm** that copies Luchador or Diet Cola runs
  their sale effect (boss disabled, Double Tag gained). Follows from the copy step
  running for the copier's own sale event; not verified in game.
- **Trading Card and The Hook.** The forced discard does not check the hook flag
  for Trading Card: a single hooked card while no discard was used this round
  should be destroyed and pay $3. From the code; not verified.
- **Glass Joker and Spectral destruction.** Familiar, Grim, Incantation and
  Immolate send their removal report before their Glass cards are marked
  shattered, so Glass Joker should not gain from them (The Hanged Man has its own
  rule). From the code; not verified.
- **Invisible Joker when Negative.** A Negative Invisible Joker sold with every
  slot full still copies (it counts its own extra slot), then its slot leaves
  with it, so the joker area should end one card over its limit. From the code.
- **Invisible Joker copying a debuffed joker.** The copy starts debuffed but its
  add-to-deck effects still run once (Chicot disables the boss, Juggler adds hand
  size). From the code.
- **Crimson Heart and hand-size jokers.** Each re-pick happens during hand
  selection, so any hand-size increase it causes draws cards at once, and any
  decrease discards nothing. That covers un-debuffing Juggler, Turtle Bean or
  Troubadour, and debuffing Merry Andy or Stuntman. With only one joker held, the
  same joker is un-debuffed and debuffed again at every re-pick; for Juggler,
  Turtle Bean or Troubadour that would add cards to the hand each time. From the
  code; not verified.
- **Luchador on the cash-out screen.** Until the beaten boss is cleared (about
  1.2 s after it starts dissolving) it is still a non-disabled Boss blind.
  Selling Luchador in that window would disable it again and, since the round
  total meets the target, ask for another end of round. Selling is probably still
  refused then (the stop-use counter from the cash-out rows), so the window is
  likely unreachable. The engine should not offer Luchador's effect after the
  round is won.
- **Certificate on a second "first draw".** If consumables empty the hand before
  the first play or discard of a round, the refill raises the first-hand-drawn
  event again and Certificate makes another card. From the code.
- **Hallucination timing** (Tarot after the pack contents) is the same open point
  as `shop.md` 3.6.
- **Idol and Mail-In defaults.** With no non-Stone card in the full deck the
  stored rank id is not reset (only the shown rank is). Practically unreachable.
- **Matador** not paying for The Hook, The Tooth or Crimson Heart alone: the same
  open point as `blinds.md` 12.

## Where in the game

- card.lua: `Card:calculate_joker` (branches for this range: `open_booster`,
  `selling_self`, `selling_card`, `reroll_shop`, `ending_shop`, `skip_blind`,
  `first_hand_drawn`, `setting_blind`, `cards_destroyed` (never raised),
  `remove_playing_cards`, `using_consumeable`, `debuffed_hand`, `pre_discard`,
  `discard`, `end_of_round`, `individual`, `repetition`, `other_joker`, `before`,
  `after`, the main context; Blueprint and Brainstorm copy step),
  `Card:set_ability` (start values: Invisible Joker's round count, Caino's X
  value, Yorick's counter; To Do List draw on copies), `Card:add_to_deck` and
  `Card:remove_from_deck` (hand size, discards, Chicot, Turtle Bean, Oops! All
  6s, To the Moon, Astronomer, Troubadour, Stuntman, Negative slots),
  `Card:set_debuff`, `Card:update` (Throwback, Driver's License, Stone Joker,
  Swashbuckler, Blueprint/Brainstorm label), `Card:calculate_dollar_bonus`
  (Golden Joker, Satellite), `Card:set_cost` (Astronomer), `Card:set_edition`,
  `Card:sell_card`, `Card:is_suit` (Smeared Joker), `Card:is_face`,
  `Card:get_id`, `Card:get_chip_mult` and `Card:get_p_dollars` (Lucky hit mark),
  `Card:shatter`, `Card:start_dissolve`, `Card:remove`, `Card:use_consumeable`
  (removal reports of The Hanged Man, Familiar, Grim, Incantation, Immolate),
  `Card:open` (pack-opened event), `Card:redeem`, `Card:explode`,
  `Card:calculate_perishable`, `Card:calculate_rental`
- functions/state_events.lua: `G.FUNCS.evaluate_play` (before, per-card, held,
  repetition, joker loop, joker-on-joker, removal report, after, debuffed hand),
  `G.FUNCS.discard_cards_from_highlighted` (before-discard and discard events,
  The Hook flag, Trading Card removal), `G.FUNCS.get_poker_hand_info` (Burnt
  Joker), `end_round` (end-of-round events, Mr. Bones' loss flag, card-of-the-round
  resets), `new_round` (blind-start events), `G.FUNCS.evaluate_round` (cash-out
  rows, interest, blind defeat)
- functions/common_events.lua: `reset_idol_card`, `reset_mail_rank`,
  `reset_ancient_card`, `reset_castle_card`, `copy_card`, `create_card` (Soul and
  Black Hole checks with Showman), `get_current_pool` (Showman, enhancement
  gates), `create_playing_card` (Certificate), `eval_card`, `ease_dollars`,
  `ease_discard`, `ease_hands_played`, `ease_ante`, `level_up_hand`,
  `card_eval_status_text`
- functions/misc_functions.lua: `find_joker`, `set_consumeable_usage` (Fortune
  Teller and Satellite counts), `playing_card_joker_effects`, `pseudorandom`,
  `pseudoseed`, `pseudorandom_element`, `stop_use`, `dec_stop_use`
- functions/button_callbacks.lua: `G.FUNCS.sell_card`, `G.FUNCS.use_card`,
  `G.FUNCS.buy_from_shop`, `G.FUNCS.reroll_shop`, `G.FUNCS.toggle_shop`,
  `G.FUNCS.skip_blind`, `G.FUNCS.skip_booster`
- blind.lua: `Blind:disable`, `Blind:defeat`, `Blind:set_blind`,
  `Blind:debuff_card`, `Blind:debuff_hand`, `Blind:modify_hand`,
  `Blind:press_play`, `Blind:drawn_to_hand`, `Blind:get_type`
- cardarea.lua: `CardArea:change_size`, `CardArea:emplace`,
  `CardArea:remove_card`, `CardArea:add_to_highlighted`
- game.lua: joker data table (`P_CENTERS`, orders 77 to 150), `P_CARDS`,
  `Game:init_game_object` (probabilities, interest per $5, skips, card-of-the-round
  defaults), `Game:start_run` (starting deck size, first card-of-the-round
  draws), `Game:update_draw_to_hand` (first-hand-drawn event), `Game:update`
  (automatic refill), `Game:update_round_eval`
- tag.lua: `Tag:init`, `Tag:set_ability`, `Tag:apply_to_run` (Double Tag ignores
  Double Tags); functions/UI_definitions.lua: `add_tag`
- engine/event.lua: `Event:handle`, `EventManager:update` (queued-step order,
  Hallucination and Chicot timing)
- localization/en-us.lua: joker descriptions, read for meaning only
