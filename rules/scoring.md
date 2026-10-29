# Scoring a played hand

Pressing Play fixes the play order and queues the moves of the selected cards
into the play area. Boss press-play effects (The Hook, The Tooth) resolve next.
Then one step, the play evaluation, computes the entire hand at once: counters,
the boss's hand check, the 'before' joker effects, the hand's base chips and
mult (and The Flint), each scoring card in play order with its retriggers, each
card still held in hand, each joker and then each consumable, the deck's final
step (Plasma), card destruction (Sixth Sense, Glass), the score
floor(chips × mult) added to the round total, and finally the 'after' joker
effects. Every random roll and every addition or multiplication happens inside
that one step. What the player then watches (cards lighting up, numbers ticking,
the total counting up) only replays values already computed, so moving jokers or
cards during the animation cannot change anything. Chips and mult are
floating-point numbers throughout. The only rounding is The Flint's halving,
Plasma Deck's balancing, and one floor of chips × mult at the end. `hands.md`
covers detection, contained hands and levels.

Conventions: "trigger" is one evaluation pass of a card; a retrigger is an
extra pass. `p` is the run's probability numerator, 1 by default and doubled by
each non-debuffed Oops! All 6s. A roll "1 in n" succeeds when the stream's value
is strictly below p/n. Stream keys are exact strings (see `rng.md`).

## 1. From pressing Play to the evaluation

### 1.1 At the click

These steps happen at the moment Play is pressed, in this order:

1. If the play area already holds cards, the click does nothing.
2. Selling jokers and using consumables are blocked from now on (they also stay
   blocked while the play area holds cards).
3. The boss's "triggered" flag is cleared (section 11.2).
4. Every playing card's forced-selection mark (Cerulean Bell) is cleared.
5. The selected cards are sorted by their current x position in hand. **This
   fixes the play order.** It is the left-to-right order in hand at the click.
6. Queued: the game state becomes "hand played"; hands left −1; a 0.4 s pause.
7. For each selected card in play order:
   - its times-played counter +1;
   - its played-this-ante flag is set (The Pillar reads it later);
   - a move into the play area is queued (≈0.1 s each). Each card is appended
     at the right end, so the play area keeps play order. Cards in the play
     area cannot be dragged.
8. The boss's press-play hook runs (section 11.1). The Hook and The Tooth queue
   their actions plus pauses. Crimson Heart and The Fish only set flags.
9. The evaluation is queued behind all of the above.

### 1.2 Queue execution

The queued steps then run in order:

1. state change;
2. hands left −1, so effects that test "final hand" see 0 on the last hand;
3. pause;
4. the card moves into the play area;
5. The Hook's discard of 2 random held cards, including its card moves, and
   The Tooth's −$1 per played card;
6. pauses;
7. **the evaluation** (sections 2 to 10), as one uninterrupted computation;
8. about 0.1 s later: hands played this run +1 and hands played this round +1.
   The played cards that were not destroyed are queued to go to the discard
   pile;
9. the queued display of the evaluation (≈ several seconds), then the round
   total counts up to its new value over 0.5 s, then the played cards move to
   the discard pile and destroyed cards finish dissolving;
10. the round check: if round chips ≥ blind chips, or no hands are left, the
    round ends; otherwise cards are drawn back up to hand size.

So during the evaluation, hands left already counts this hand, while the
hands-played counters do not yet. DNA and Sixth Sense test "first hand of the
round" as hands played this round = 0. Loyalty Card uses the run count before
this hand.

## 2. The evaluation, step by step (overview)

1. Detect the hand type and scoring cards (`hands.md`).
2. Hand counters: played +1, played this round +1, last played hand set,
   visible set (`hands.md`, 6.3).
3. Add Stone cards (or all cards with Splash) and sort into play order.
4. Boss hand check (section 3.1). If it debuffs the hand, jump to section 10.
5. Read the hand's chips and mult (section 3.3).
6. 'Before' joker effects, jokers left to right (section 3.2).
7. Re-read the hand's chips and mult; apply The Flint (section 3.3).
8. Played-card loop (section 4).
9. Held-card loop (section 5).
10. Joker loop, jokers then consumables (section 6).
11. Deck final step: Plasma Deck (section 7).
12. Destruction: Sixth Sense and Glass rolls; jokers told about removed cards
    (section 8).
13. Score: floor(chips × mult) is added to the round total (section 9).
14. 'After' joker effects (section 9.3).
15. Challenge rule "played cards become permanently debuffed", if active.

## 3. Before the cards are scored

### 3.1 Boss hand check

The boss gets the played cards, the hand table and the hand type. If the boss
is disabled (Chicot, Luchador) nothing happens. Otherwise, for every blind, the
triggered flag is first set to false, then:

| Boss | Hand debuffed (score 0) when | Side effect |
|---|---|---|
| The Psychic | fewer than 5 cards played | none |
| The Eye | this hand type was already played this blind | records the type if not debuffed |
| The Mouth | a type is recorded and this type differs | records the type on the first play |
| The Arm | never | if the type's level > 1: level −1 now, before the chips and mult are read; flag set |
| The Ox | never | if the type is the most played hand (`hands.md`, 6.3): money set to $0 immediately (from debt this raises it to 0); flag set |

When the hand is debuffed the triggered flag is set and the rest goes to section
10. The Eye and The Mouth record only on real plays, not in the preview.

### 3.2 'Before' joker effects

For each joker, left to right (jokers only, not consumables), in one pass.
Debuffed jokers do nothing. Blueprint and Brainstorm act at their own position
by running the copied joker's logic. Copies of effects marked "not copyable"
below do nothing. Vanilla jokers acting here:

- Spare Trousers: +2 mult to itself if the hand contains Two Pair or Full House
  (not copyable).
- Space Joker: 1 in 4, stream `space`: level up the played hand type by 1 now.
  A Blueprint copy rolls again on the same stream at its own position.
- Square Joker: +4 chips to itself if exactly 4 cards were played (not copyable).
- Runner: +15 chips to itself if the hand contains a Straight (not copyable).
- Midas Mask: every face card among the scoring cards (non-debuffed; with
  Pareidolia, every non-debuffed card) becomes a Gold Card now (not copyable).
- Vampire: every scoring card that is enhanced and not debuffed loses its
  enhancement now; Vampire gains X0.1 per card (not copyable).
- To Do List: +$4 if the type is its target (money is queued, section 13.1).
- DNA: if this is the round's first hand and exactly 1 card was played, a full
  copy of it (enhancement, edition, seal, perma bonus) is added to the deck and
  placed at the **right end of the hand now**. It then takes part in the
  held-card loop of this same hand. Hologram gains immediately. Copyable.
- Ride the Bus: reset to 0 if any scoring card is a face card, else +1 (not
  copyable).
- Obelisk: if the played type's play count, counting this play, is strictly
  greater than that of every other visible type, reset to X1; otherwise +X0.2
  (not copyable).
- Green Joker: +1 mult (not copyable).

Joker order therefore matters here. Midas Mask before Vampire turns faces Gold,
then Vampire strips them and gains. Vampire before Midas Mask strips first.
Card changes made here (Midas, Vampire) apply to the scoring below. A Glass card
stripped by Vampire no longer gives X2 and no longer rolls to shatter. A Stone
card stripped by Vampire scores as its printed rank.

Changing a card's enhancement re-runs the boss's card-debuff test on that card.
Example: a Stone card of Clubs stripped by Vampire under The Club becomes
debuffed and does not score. The rebuilt card also loses its played-this-ante
flag.

### 3.3 Base chips and mult; The Flint

- After the 'before' pass the game reads the hand type's stored chips and mult
  (`hands.md`, 6.1/6.2). A Space Joker level-up is included.
- The Flint (not disabled): mult = max(1, floor(mult × 0.5 + 0.5)) and
  chips = max(0, floor(chips × 0.5 + 0.5)), i.e. halved, with halves rounded up.
  The flag is always set.
- Challenge "Rich get Richer": every time chips change during the evaluation
  (including this read), chips are capped at max(money, 0). Money is the value
  when evaluation started (section 13.1).

## 4. The played-card loop

For each scoring card, in play order (kickers get nothing):

1. If the card is debuffed: the boss flag is set and the card is skipped
   entirely. No chips (not even rank chips), mult, money, X-mult or edition, no
   retriggers, no per-card joker effects, no seal.
2. Otherwise count its triggers, before any evaluation: 1, plus 1 for a Red
   seal, plus the retriggers each joker grants, jokers left to right:
   - Sock and Buskin: +1 if the card is a face card;
   - Hanging Chad: +2 if it is the first scoring card in play order;
   - Dusk: +1 if no hands are left (final hand);
   - Seltzer: +1;
   - Hack: +1 if rank 2, 3, 4 or 5.
   Copies via Blueprint/Brainstorm add again. The order only changes which
   message is shown; every trigger is a full, identical pass.
3. For each trigger, one full pass:
   1. **The card's own effects are computed** in this order. (a) Chips = rank
      chips + enhancement bonus + perma bonus (section 12). (b) Mult: Mult card
      +4; Lucky card rolls `lucky_mult`, 1 in 5, for +20. (c) X-mult: Glass X2.
      (d) Money: Gold seal $3 plus, for a Lucky card, a roll on `lucky_money`,
      1 in 15, for +$20. (e) Edition: Foil +50 chips, Holographic +10 mult,
      Polychrome X1.5. Only non-zero parts are kept.
   2. **Each joker's per-card effect** is computed, left to right (jokers only).
      Examples: suit and rank mult/chips jokers, Photograph (X2 on the first
      face card in play order), The Idol, Triboulet, Ancient Joker, Bloodstone
      (`bloodstone`, 1 in 2, X1.5, Hearts), Business Card (`business`, 1 in 2,
      $2, face), 8 Ball (`8ball`, 1 in 4, a Tarot, rank 8, only if a consumable
      slot is free counting pending creations), Golden Ticket, Rough Gem, Hiker,
      Wee Joker, Lucky Cat.
      - Some of these change state now. Hiker adds +5 to this card's perma
        bonus, which counts from the card's next trigger, since (a) was already
        computed. Wee Joker grows and Lucky Cat grows if (b) or (d) hit on this
        trigger.
   3. The card's Lucky mark is cleared.
   4. **Effects are applied**, the card's own first, then each joker's in joker
      order. Inside each effect the order of fields is: chips (+); mult (+);
      card money ($); joker money ($); "extra" (an additive mult/chips or
      chips↔mult swap slot, unused for score in vanilla; 8 Ball's Tarot is
      queued here); X-mult (×); edition chips (+); edition mult (+); edition
      X-mult (×).

Examples:

- A Polychrome Mult card adds +4 mult, then multiplies mult by 1.5.
- A Holographic Glass card multiplies mult by 2, then adds +10.
- A Foil Bonus 7 with a Gold seal adds 7 + 30 chips, earns $3, then adds 50
  chips.
- Only after the card's own effects do the jokers' per-card effects follow, one
  joker at a time.

Money is never added to the money total during the loop; it is queued (section
13.1).

## 5. The held-card loop

For each card still in hand, left to right. This covers the cards not played,
minus The Hook's discards, plus a DNA copy at the right end:

1. Pass 1:
   1. The card's own held effect: Steel Card X1.5 (nothing if debuffed). No
      other enhancement does anything while held during scoring. A held mult
      value exists but is 0 for every card.
   2. Each joker's held effect, left to right:
      - Shoot the Moon: +13 mult for a Queen.
      - Baron: X1.5 for a King.
      - Reserved Parking: face card, 1 in 2 on `parking`, +$1.
      - Raised Fist: +2 × the rank chips of the lowest-ranked held card. It
        picks the non-Stone card with the lowest rank (Ace high) among all held
        cards; on ties, the rightmost one. It applies on that card only.
      - For a debuffed card, Shoot the Moon, Baron and Raised Fist give a
        "debuffed" message with no value. Raised Fist still picks a debuffed
        card if it is the lowest, and then gives nothing. Reserved Parking never
        even rolls, because a debuffed card is not a face card.
   3. **Retriggers are decided once, here, from pass 1's results.** A held card
      "did something" if its own effect is non-empty (a non-debuffed Steel card)
      or any joker returned an effect for it, including a "debuffed" message or
      a successful Parking roll.
      - Red seal: +1 retrigger if the card has a Red seal, is not debuffed, and
        did something.
      - Mime (and copies): +1 retrigger each if the card did something.
   4. Apply pass 1's effects: the card's own first, then each joker's in order.
      Inside each effect: money; held mult (+); X-mult (×); message.
2. Each retrigger is a full new pass (1.1, 1.2 and 1.4). Reserved Parking rolls
   again. Retriggers are never gathered again from later passes.

A held card with a Red seal and no effect (e.g. a plain 5 with no relevant
jokers) is not retriggered. A Steel King with a Red seal and Baron: each of its
2 passes applies X1.5 (Steel) then X1.5 (Baron).

## 6. The joker loop

Over all jokers left to right, then all consumables left to right. For each such
card C:

1. **C's edition, additive part.** Skipped if C is debuffed. Foil: +50 chips.
   Holographic: +10 mult. Negative: nothing.
2. **C's main effect.** Nothing if C is debuffed. One result per joker, applied
   as: mult (+), then chips (+), then X-mult (×). Blueprint/Brainstorm produce
   the copied joker's main effect here, at their own position. Consumables: a
   Planet card whose hand is the played type gives X1.5 if the Observatory
   voucher is owned. Each matching Planet counts separately. Nothing else
   happens for consumables.
3. **Joker-on-joker effects for C.** Each joker J, left to right, may react to
   C. In vanilla only Baseball Card does (and copies of it): X1.5 when C is an
   Uncommon joker other than that Baseball Card. C being debuffed does not stop
   it. Applied as mult (+), chips (+), X-mult (×).
4. **C's edition, multiplicative part.** Skipped if C is debuffed. Polychrome:
   X1.5.

So a Polychrome joker's X1.5 comes after its own effect and after any Baseball
Card bonus for it. A Foil or Holographic joker's bonus comes before its own
effect.

## 7. Deck final step (Plasma Deck)

With Plasma Deck: total = chips + mult; then chips = floor(total / 2) and
mult = floor(total / 2). Example: 101 chips × 13.5 mult becomes 57 × 57. Every
other deck leaves chips and mult unchanged. (Plasma's doubled blind size is
handled elsewhere.)

## 8. Destruction of scoring cards

After the joker loop and Plasma, for each scoring card in play order:

1. Ask the jokers, left to right, whether to destroy it. Stop at the first yes.
   In vanilla only Sixth Sense says yes, and copies do not work. Its condition:
   exactly 1 card played, that card is a 6, and it is the round's first hand.
   A Spectral card is queued (key append `sixth`) if a slot is free. The 6 is
   destroyed even without a free slot.
2. **Glass roll:** if the card is currently a Glass Card and not debuffed, roll
   `glass`: shatter if below p/4. This roll happens once per Glass scoring card,
   whatever its retriggers, and even if step 1 already destroyed it. Kickers
   never roll; only scoring cards are checked.
3. A destroyed Glass card is marked shattered, any other destroyed card is
   marked destroyed. Either way it leaves the deck.

Then every joker is told which cards were removed. Caino gains X1 per removed
face card (its value changes now). Glass Joker gains X0.75 per shattered Glass
card (its value changes a moment later through the queue). Neither change can
affect this hand, whose score is already fixed. Removed cards disappear from the
deck after the animations. They are not moved to the discard pile.

## 9. The score

### 9.1 Computation

- hand score = floor(chips × mult), computed once from the final chips and mult
  (after Plasma). Neither chips nor mult is rounded before that, except by The
  Flint and Plasma. Mult is often fractional. Use IEEE double arithmetic in
  exactly the order above to reproduce the game's results bit for bit,
  overflow to infinity included.
- New round total = (round total before this hand) + hand score. This value is
  computed during the evaluation and handed to a 0.5 s count-up. The visible
  total reaches it after the animation. Nothing else changes the round total in
  between.
- The round-end check (round total ≥ blind chips) happens only after the whole
  hand has finished (section 1.2).

### 9.2 Where the score sits in the order

The score is computed after destruction is decided. Destruction cannot change
chips or mult, so the order does not matter for the value. The score is added
before the 'after' effects run. Those effects cannot change it either.

### 9.3 'After' joker effects

Jokers left to right (not consumables). This also runs when the hand was
debuffed:

- Ice Cream loses 5 chips, and is destroyed if that would reach 0.
- Seltzer counts down 1, and is destroyed at 0.
- Blueprint copies of these do nothing.

Then, only in the "Double or Nothing" challenge, the scoring cards become
permanently debuffed.

## 10. Debuffed hand (The Psychic, The Eye, The Mouth)

When the boss check debuffs the hand:

- chips = 0 and mult = 0, so the hand scores 0;
- no 'before' effects (so no Space Joker level-up), no card or held or joker
  loops, no Plasma, no Glass rolls and no destruction;
- each joker's "debuffed hand" effect runs, left to right. Only Matador acts:
  +$8 when the boss flag is set, which it always is here;
- the hand counters from section 2, step 2 have already counted this play
  (played, played this round, visible, last hand);
- the 'after' effects still run (section 9.3), and the run and round
  hands-played counters still increase;
- the played cards still go to the discard pile.

## 11. Boss hooks around a play

### 11.1 At the click (press-play hook)

Nothing happens if the boss is disabled.

| Boss | Effect |
|---|---|
| The Hook | Queues an action that runs after the played cards have moved. It picks up to 2 cards from the cards then in hand, one pick at a time, using `pseudorandom_element` with the stream `hook`. The candidates are ordered by card creation id, and a picked card is removed before the next pick; picks stop when the hand runs out. The picked cards are discarded as a forced discard. That discard triggers Purple seals and joker discard effects, but not Burnt Joker. It uses no discard, costs nothing, and its jokers' state changes (Green Joker, Ramen, Castle, Hit the Road, Yorick, ...) apply before this hand is scored. Adds 0.7 s + 0.4 s of pauses. |
| The Tooth | Queues: lose $1 per played card, all cards not just scoring ones, before the evaluation. |
| Crimson Heart | If any joker is owned: sets its flags. The disabled joker is re-picked after the next draw (stream `crimson_heart`), not during scoring. |
| The Fish | Sets a flag so the cards drawn after this hand come face down. |

The Hook's and The Tooth's effects always resolve before the evaluation starts,
because their queued follow-ups (card moves, money) sit ahead of it in the
queue.

### 11.2 The boss "triggered" flag (read by Matador)

The flag is cleared at the click, possibly set by the press-play hook, and then
**cleared again by the boss hand check** (section 3.1). This happens for every
non-disabled blind, since every blind has a debuff table. It ends up set only
if:

- the hand is debuffed; or
- The Arm lowered a level; or
- The Ox hit the most played hand; or
- The Flint is active; or
- at least one scoring card is debuffed (suit bosses, The Plant, The Pillar,
  Verdant Leaf). A debuffed kicker does not count.

Matador pays $8 in the joker loop (or in section 10) if the flag is set. So
Matador does not pay for The Hook, The Tooth or Crimson Heart. That follows from
the code; not yet checked in game.

### 11.3 Debuffed cards and cards changing mid-hand

- Card debuffs are decided when the blind is set, when the boss is disabled, and
  whenever a card's rank, suit or enhancement changes (plus a few card-creation
  paths). They are not re-checked when cards are played.
- Suit bosses also debuff Wild cards (they test suits with debuff ignored, and a
  Wild card has every suit). With Smeared Joker they also debuff the other suit
  of the same colour. Stone cards are never hit by suit bosses.

## 12. Card-level values

### 12.1 Rank chips

2 to 10: their number. Jack, Queen, King: 10. Ace: 11.

### 12.2 Chips a scored card adds per trigger

- Normal card: rank chips + enhancement bonus + perma bonus.
- Stone Card: 50 + perma bonus (no rank chips).
- Enhancement bonus: Bonus Card 30, Stone 50, all others 0. A card's bonus is
  reset to its new enhancement's value whenever its enhancement changes.
- **Perma bonus** (Hiker): +5 per trigger of the scored card while Hiker is
  owned. It stays on the card for the rest of the run, through enhancement
  changes, rank/suit changes and copying (DNA, Cryptid), and counts for every
  card type including Stone.

### 12.3 Enhancements

| Enhancement | When scored (per trigger) | When held during scoring (per trigger) | Other |
|---|---|---|---|
| Bonus | +30 chips (on top of rank chips) | nothing | |
| Mult | +4 mult | nothing | |
| Wild | nothing extra | nothing | counts as every suit, unless debuffed (`hands.md`, 2) |
| Glass | X2 mult | nothing | after scoring, 1 in 4 (`glass`) to shatter, once per hand |
| Steel | nothing | X1.5 mult | |
| Stone | 50 chips instead of rank chips; always scores | nothing | no rank, no suit, never part of a pattern |
| Gold | nothing | nothing | +$3 at end of round per trigger if held, not debuffed |
| Lucky | 1 in 5 (`lucky_mult`) +20 mult; then 1 in 15 (`lucky_money`) +$20 | nothing | both rolled on every trigger, independently; either hit marks the card for Lucky Cat |

### 12.4 Editions on playing cards

Applied on every trigger of a scored card, after the card's chips, mult, money
and X-mult. Nothing while held, and nothing if the card is debuffed.

- Foil: +50 chips.
- Holographic: +10 mult.
- Polychrome: X1.5 mult.
- Negative: nothing.

Joker editions use the same numbers but are applied around the joker's effect
(section 6).

### 12.5 Seals

| Seal | Effect |
|---|---|
| Gold | +$3 every time the card triggers while scoring (with retriggers, $3 each). Nothing when held. |
| Red | +1 trigger. Scored: always (if not debuffed). Held in scoring: only if the card did something (section 5). Held at end of round: only if it produced money (Gold Card) or a Blue seal effect. |
| Blue | At end of round, if held and not debuffed: creates the Planet for the last played hand type if a consumable slot is free, counting pending creations (key append `blusl`). It is evaluated on every end-of-round trigger, so a Red seal or Mime can make two. |
| Purple | When discarded (including by The Hook), if not debuffed: creates a Tarot if a slot is free (key append `8ba`, the same append as 8 Ball). |

### 12.6 What a debuff turns off (playing cards)

- **Scored:** everything. No chips (rank chips included), mult, X-mult, money,
  edition, Red seal or per-card joker effects. No Glass roll. The card still
  occupies its scoring position and still sets the boss flag.
- **Held:** Steel X1.5, Red seal; the held-card jokers show "debuffed" instead.
- **End of round:** Gold $3, Blue seal.
- **Discard:** Purple seal.
- **Detection:** not affected (rank and printed suit still count), except that a
  debuffed Wild card stops being wild.
- **Suit and face tests:** a debuffed card fails every suit test and face test
  used by jokers. Bosses use their own tests that ignore debuff.

### 12.7 Everything stored on a card that affects score

Printed rank and suit; enhancement (bonus chips, mult, X-mult, held X-mult, Lucky
values, Gold end-of-round money); perma bonus; edition; seal; debuff state; the
Lucky mark (lasts one trigger). No other per-card value enters scoring in
vanilla.

### 12.8 Suit, face and rank tests used by jokers during scoring

These differ from the detection tests in `hands.md`, section 2.

- **Suit test.** Checked in this order:
  1. a debuffed card has no suit;
  2. a Stone card has no suit;
  3. a Wild card has every suit;
  4. with an active Smeared Joker, a card has both suits of its colour;
  5. otherwise the card has its printed suit.
- **Face test.** A debuffed card is never a face card, except in boss checks.
  Otherwise a card is a face card if it is a Jack, Queen or King, or always
  while an active Pareidolia is owned, Stone cards included.
- **Rank tests.** Ranks are compared by rank id (Ace = 14). A Stone card's id is
  a fresh random negative number, so it fails every rank test. Raised Fist skips
  Stone cards explicitly.
- **Exceptions** (details belong in the jokers notes):
  - Blackboard checks held cards with the detection test from `hands.md`.
  - Flower Pot ignores debuff for non-Wild cards.
  - Flower Pot and Seeing Double assign Wild cards to suits only after the other
    cards.

## 13. Deferred state, snapshots and buffers

### 13.1 Money

Every money change produced during the evaluation is queued and lands after it.
This covers Gold seal, Lucky, Golden Ticket, Business Card, Rough Gem, Reserved
Parking, To Do List and Matador. During the evaluation the game keeps a running
"pending money" total of these amounts. It is cleared after the evaluation.

- Bull and Bootstraps read money + pending, so they see money earned earlier in
  this same hand.
- Everything else reads money as it was when the evaluation started, after The
  Tooth's losses and The Ox's reset. That includes Vagabond's "$4 or less" test
  and the "Rich get Richer" chip cap.

### 13.2 Consumable room

Effects that create a consumable (8 Ball, Sixth Sense, Vagabond, Superposition,
Seance) queue the creation and count it as pending. "Is there a free slot" means
held consumables + pending < slots.

The creations run after the evaluation, in the order they were triggered: 8 Ball
Tarots (played-card loop, card and trigger order), then Vagabond, Superposition
and Seance (joker loop, joker order), then Sixth Sense (destruction). They use
the card-creation streams for Tarot/Spectral with appends `8ba`, `vag`, `sup`,
`sea`, `sixth`. Tarot creations share streams, so this order matters.

### 13.3 Values refreshed once per frame

These are recomputed every frame, not during the evaluation, so they hold their
pre-play values for the whole evaluation:

- Steel Joker (count of Steel cards in the full deck);
- Stone Joker (Stone count);
- Driver's License (enhanced count);
- Joker Stencil (empty slots + Stencils);
- Swashbuckler (sum of other jokers' sell values);
- Throwback (skips);
- outside scoring: Cloud 9 and Temperance.

Changes made during 'before' (Vampire, Midas Mask, DNA) reach these values only
from the next hand. Values read live include Abstract Joker (joker count), Blue
Joker (cards in deck), Erosion (deck size), Raised Fist and Blackboard (held
cards). Erosion sees a DNA copy at once; Blue Joker does not change, because the
copy goes to hand.

## 14. Random streams used by a play

All rolls happen inside the evaluation, except The Hook's, which happen just
before it.

| Order | Stream | Source | Draws |
|---|---|---|---|
| 1 | `hook` | The Hook | up to 2, one per pick, before the evaluation |
| 2 | `space` | Space Joker (and copies) | 1 per instance, in joker order, every non-debuffed hand |
| 3 | `lucky_mult` | Lucky scoring card | 1 per trigger |
| 3 | `lucky_money` | Lucky scoring card | 1 per trigger, right after `lucky_mult` |
| 3 | `8ball` | 8 Ball | per trigger of a scored 8, only while a consumable slot is free |
| 3 | `business` | Business Card | per trigger of a scored face card |
| 3 | `bloodstone` | Bloodstone | per trigger of a scored Heart (Wild and Smeared count) |
| 4 | `parking` | Reserved Parking | per held pass of a non-debuffed held face card |
| 5 | `misprint` | Misprint | 1 per instance, a whole number 0 to 23 inclusive, in the joker loop |
| 6 | `glass` | Glass scoring card | 1 per non-debuffed Glass scoring card, after the joker loop |

- Order within one scored trigger: `lucky_mult`, then `lucky_money` (card's own),
  then the jokers' per-card rolls in joker order.
- Across the hand: scoring cards in play order, each card's triggers
  consecutively. Then held cards left to right, each card's passes
  consecutively. Then the joker loop, then glass in play order.
- Only the order within one stream affects outcomes, because streams are
  independent. It still matters across sources of the same stream: two
  Bloodstones (or Bloodstone and its Blueprint copy) share `bloodstone`. They
  draw for each trigger in joker order.
- Rolls compare strictly below p/n. With one Oops! All 6s, p = 2.
- Not drawn during scoring: Wheel, Cerulean Bell, Crimson Heart and
  card-creation streams (queued until afterwards).

## 15. What depends on order

Played cards, in play order (fixed at the click):

- Interleaving of additions and multiplications: a Glass card's X2 multiplies
  the mult accumulated so far. A later card's +mult is not multiplied.
- "First" effects: Hanging Chad (first scoring card, even if that card is
  debuffed, in which case nobody gets it) and Photograph (first non-debuffed
  face card).
- Hiker's growing perma bonus, Wee Joker and Lucky Cat growth timing, and the
  order of 8 Ball creations.
- Stream sequences: `lucky_*`, `bloodstone`, `business`, `8ball`, `glass`.

Held cards, left to right:

- Steel/Baron multiplications against Shoot the Moon and Raised Fist additions.
- Raised Fist's tie-break (rightmost lowest card).
- `parking` sequence.

Jokers, left to right:

- 'Before' interactions (Midas Mask vs Vampire).
- Per-card effect order within each trigger.
- The main-effect order (additions before or after multiplications).
- Joker editions placement, Baseball Card timing.
- Blueprint (copies its right neighbour) and Brainstorm (copies the leftmost
  joker).
- Shared-stream order (`space`, `misprint`, `bloodstone`, ...).

Consumables, left to right: only Observatory, which is multiplicative and comes
after all jokers.

Not order-dependent: the hand type and which cards score. The total number of
retriggers is also fixed (only messages depend on their order).

## 16. Is the score fixed when the hand is played?

Yes, once the evaluation runs. The scripts show it in four ways:

1. The whole computation, from detection to the final total, is one function
   that runs to completion inside a single queued step. Chips and mult are
   plain variables updated in its loops. Every random roll happens in those
   loops.
2. Everything visual (running chips and mult display, card pop-ups, joker
   shakes, the count-up of the round total) is queued as display steps. Each
   carries numbers that were already computed when it was queued. The round
   total's target value is passed by value at that moment.
3. The loops read the joker list, the held cards and the scoring cards at the
   moment the evaluation runs. Reordering them later changes only the stored
   order for future hands.
4. Joker state changes that happen during the evaluation (Glass Joker gains,
   Ice Cream and Seltzer removal) and all money arrive later, through the
   queue. They cannot reach back into the result.

When the evaluation runs:

- The play order of the played cards is fixed at the click (they are sorted
  then, and cards in the play area cannot be dragged).
- The evaluation itself runs only after all played cards have arrived in the
  play area (≈0.4 s + 0.1 s per card), after The Hook's discard and The Tooth's
  charges, and after the related pauses.
- The scripts do not lock dragging of jokers or of the cards left in hand during
  that short window. Selling and using are locked. So in principle a joker or
  held-card reorder made within that fraction of a second after the click would
  be used.
- UNSURE: whether a player can do that in practice has not been tried in game.
  For the engine and the bot, use the joker and hand order at the moment Play is
  pressed. All legal reordering happens before Play.

## 17. Other decks, stakes and challenges

- Red Deck at White Stake: nothing in this file changes.
- Plasma Deck: section 7.
- Other decks change only the deck or resources.
- Stakes do not touch scoring rules. Perishable jokers (Orange Stake and up)
  become debuffed after their rounds run out, and then do nothing (section 6).
- Challenges:
  - "Rich get Richer": chips capped at money (section 3.3).
  - "Double or Nothing": scoring cards permanently debuffed after each hand
    (section 9.3).

## UNSURE

- Whether jokers or held cards can be reordered in the ≈0.5–1.5 s between
  pressing Play and the evaluation (section 16).
- Matador not paying for The Hook, The Tooth or Crimson Heart (section 11.2): it
  follows from the code, but has not been checked in game.
- The Ox's tie-break follows Lua table order (`hands.md`, 6.3).

## Where in the game

- functions/state_events.lua: `G.FUNCS.play_cards_from_highlighted`,
  `G.FUNCS.evaluate_play`, `G.FUNCS.get_poker_hand_info`,
  `G.FUNCS.draw_from_play_to_discard`, `G.FUNCS.discard_cards_from_highlighted`,
  `end_round`, `new_round`
- functions/common_events.lua: `eval_card`, `level_up_hand`, `update_hand_text`,
  `card_eval_status_text`, `draw_card`, `ease_dollars`, `ease_hands_played`,
  `delay`, `copy_card`
- functions/misc_functions.lua: `mod_chips`, `mod_mult`, `find_joker`,
  `pseudorandom`, `pseudoseed`, `pseudorandom_element`, `stop_use`,
  `playing_card_joker_effects`
- card.lua: `Card:get_chip_bonus`, `Card:get_chip_mult`, `Card:get_chip_x_mult`,
  `Card:get_chip_h_mult`, `Card:get_chip_h_x_mult`, `Card:get_edition`,
  `Card:get_p_dollars`, `Card:get_end_of_round_effect`, `Card:calculate_seal`,
  `Card:calculate_joker`, `Card:set_ability`, `Card:set_edition`,
  `Card:set_debuff`, `Card:is_suit`, `Card:is_face`, `Card:get_id`,
  `Card:update`, `Card:remove`, `Card:can_sell_card`, `Card:can_use_consumeable`
- blind.lua: `Blind:press_play`, `Blind:debuff_hand`, `Blind:modify_hand`,
  `Blind:debuff_card`, `Blind:drawn_to_hand`, `Blind:set_blind`
- back.lua: `Back:trigger_effect`
- cardarea.lua: `CardArea:emplace`, `CardArea:align_cards`,
  `CardArea:set_ranks`, `CardArea:remove_card`
- engine/event.lua: `Event:handle`, `EventManager:add_event`,
  `EventManager:update`
- engine/controller.lua: `Controller:update`, `Controller:L_cursor_press`
- game.lua: `Game:update`, `Game:update_hand_played`, enhancement, edition,
  blind, joker and voucher definitions, `Game:init_game_object`
- challenges.lua: challenge rules (Rich get Richer, Double or Nothing)
