# Poker hands: detection, contained hands and levels

When a hand is played, the game looks at the played cards (1 to 5, in their
left-to-right play order) and runs four helpers: groups of exactly N cards of
one rank, a flush test, a straight test and a highest-card pick. From these it
builds a table with one entry per hand type. Each entry is either empty or holds
card groups. A hand "contains" every type whose entry is non-empty; jokers such
as Jolly Joker or Spare Trousers test exactly that. The hand's type is the first
non-empty entry in a fixed priority list, and its scoring cards are the first
group in that entry. Stone cards are then added (or every played card, with
Splash), and the scoring cards are put back into play order. Each of the 12 hand
types has a level with stored chips and mult. Everything starts at level 1. A
level change recomputes both values from a linear formula with minimums. Three
types are hidden until first played. This file covers detection and the hand
table. `scoring.md` covers the score itself.

Throughout, "play order" means the left-to-right order of the cards in the play
area. That is the left-to-right order the selected cards had in the hand when
Play was pressed (see `scoring.md`, section 1).

## 1. Inputs

- Played cards: 1 to 5. Up to 5 cards can be selected in hand. Play is disabled
  with 0 selected or more than 5. The play area holds 5 cards.
- Detection runs on the cards in the play area, in play order. The hand type
  and the set of scoring cards never depend on the order. Only the internal
  order of groups does, and that order is discarded at the end (section 5).
- The same routine also runs for the hand preview while cards are selected
  (on the selection, in click order) and for Burnt Joker (on discarded cards).
  Neither affects scoring. The preview does not add Stone cards or apply Splash.

## 2. What detection reads from each card

**Rank id.** 2 to 10 are themselves, Jack 11, Queen 12, King 13, Ace 14.

- A Stone Card has no usable rank. Each rank lookup on a Stone card returns a
  fresh random large negative number. It comes from the global LuaJIT generator
  without reseeding, so it never touches a seeded stream. The effect is
  deterministic: a Stone card never matches any other card's rank and is never
  part of a straight.
- Debuff does not change the rank. Debuffed cards still form pairs and
  straights.

**Flush suit test.** "Does this card count as suit S for a flush" is decided by
the first rule that applies:

1. A Stone Card never counts.
2. A Wild Card that is not debuffed counts as every suit.
3. With a Smeared Joker active, the card counts when its printed suit and S
   have the same colour. Hearts and Diamonds are one colour; Spades and Clubs
   are the other.
4. Otherwise the card counts only when its printed suit is S.

A debuffed non-Wild card still counts for its printed suit. A debuffed Wild card
loses its wildness here and counts as its printed suit (or by colour with
Smeared Joker).

**Joker switches.** Four Fingers, Shortcut, Smeared Joker and Splash (and
Pareidolia elsewhere) work only while at least one copy is owned and not
debuffed. The game's joker search skips debuffed jokers by default. Extra
copies change nothing. Pareidolia does not affect detection; it only changes
face-card checks used by jokers and bosses.

## 3. The four helpers

### 3.1 Groups of exactly N (N = 5, 4, 3, 2)

- For every rank, count the played cards with that rank id (Stone cards never
  match). A rank whose count is exactly N forms a group of all its cards. A
  rank with 4 cards forms a group for N = 4 only, not for N = 3 or 2.
- Groups are listed from the highest rank (Ace) down.
- Inside a group the cards are in play order.

### 3.2 Flush

1. If more than 5 cards are played, or fewer than 5 (fewer than 4 with Four
   Fingers), there is no flush.
2. Otherwise try the suits in this fixed order: **Spades, Hearts, Clubs,
   Diamonds**. For each suit, collect the played cards that pass the flush suit
   test (section 2), in play order.
3. The first suit whose collection reaches 5 cards (4 with Four Fingers) is the
   flush. Its cards are the whole collection, possibly 5 even with Four Fingers.
   Later suits are not tried.

With Wild cards or Smeared Joker one card can qualify for several suits. The
suit order then decides which collection is used. Example: Smeared Joker with 3
Hearts, 1 Diamond and 1 Spade: Spades collects 1 card, Hearts collects the 4 red
cards. With Four Fingers that is a flush of those 4.

### 3.3 Straight

1. If more than 5 cards are played, or fewer than 5 (fewer than 4 with Four
   Fingers), there is no straight. The required run length is 5, or 4 with Four
   Fingers.
2. Bucket the non-Stone played cards by rank id. Several cards of one rank share
   a bucket, in play order.
3. Walk the positions 1 to 14 in order. Position 1 means "Ace as low" and looks
   at the Ace bucket. Positions 2 to 14 look at their own rank, so 14 is the Ace
   again, as high. Keep a run length (starts at 0), a collected list (starts
   empty), a found flag (starts false) and a skipped flag (starts false). At
   each position:
   - **Rank present:** run length +1, skipped flag cleared, and every card in
     that bucket is appended to the collected list.
   - **Rank missing, Shortcut active, skipped flag not set, and the position is
     not 14:** set the skipped flag. The run length is unchanged.
   - **Rank missing otherwise:** run length back to 0 and skipped flag cleared.
     If the found flag is set, stop walking. If not, empty the collected list.
   - After any of the three cases, if the run length has reached the required
     length, set the found flag.
4. If the found flag is set, the straight's cards are the collected list.
   Otherwise there is no straight.

Consequences:

- An Ace works low (A-2-3-4-5) and high (10-J-Q-K-A). Runs never wrap
  (Q-K-A-2-3 is not a straight).
- With Shortcut, a missing rank can be stepped over as long as no two missing
  ranks are adjacent. Skipped ranks do not count towards the length, so 5
  present ranks are still needed (4 with Four Fingers). Examples: 2-4-6-8-10 and
  A-3-5-7-9 are straights with Shortcut.
- Once the run has reached the required length, further present ranks (and,
  with Shortcut, ranks just beyond a single gap) are still collected until the
  run breaks. Example: Four Fingers and Shortcut with 2-3-4-5-7 collects all 5
  cards. Four Fingers alone with 2-3-4-5-7 collects 2-3-4-5.
- Duplicate ranks: every card of a present rank is collected. With Four Fingers,
  5-6-7-8-8 is a straight whose cards are all 5, both 8s included.
- Debuffed cards take part. Stone cards never do.

### 3.4 Highest card

- If at least one card is played, the result is a single card: the one with the
  greatest sort value, scanning in play order and replacing only on a strictly
  greater value.
- Sort value = rank value + suit value + a tiny original-suit term + a tiny
  creation-order term:
  - rank value: 2 to 10 count as 2 to 10, Jack 10.1, Queen 10.2, King 10.3,
    Ace 11.4 (rank chips plus a face bonus of 0.1 / 0.2 / 0.3 / 0.4);
  - suit value: Diamonds 0.01, Clubs 0.02, Hearts 0.03, Spades 0.04;
  - the suit the card had when created, worth 1/100,000 of the suit values
    (10^-7 for Diamonds up to 4×10^-7 for Spades). A copy takes the suit its
    source had when copied.
  - up to 10^-6 for card age: 10^-6 × (1 − creation id / 1,603,301), where the
    id is a global counter of all game objects, so older cards score higher.
  - For a Stone Card the suit and original-suit terms are multiplied by -1000
    instead of 1. That puts every Stone card far below every non-Stone card.
- Debuff is ignored, so a debuffed card can be the highest card.
- Ties beyond rank never matter for play. In a High Card hand all non-Stone
  cards have different ranks, since a repeated rank would be a Pair. All Stone
  cards are added to the scoring cards anyway (section 5). The engine can treat
  the pick as "the highest-ranked non-Stone card, Ace high", or any Stone card
  if only Stone cards were played.

## 4. The hand table: what a hand contains

The table is built from the helper results as follows (N-groups as in 3.1):

| Entry | Non-empty when | Groups stored |
|---|---|---|
| Flush Five | a 5-group exists and a flush exists | the 5-group |
| Flush House | a 3-group, a 2-group and a flush exist | one group: the 3-group's cards then the 2-group's |
| Five of a Kind | a 5-group exists | the 5-group |
| Straight Flush | a flush and a straight exist (not necessarily the same cards) | one group: all flush cards, then the straight cards not already in it |
| Four of a Kind | a 4-group exists, or a 5-group exists (see below) | the 4-group (or the 5-group) |
| Full House | a 3-group and a 2-group exist | one group: 3-group cards then 2-group cards |
| Flush | a flush exists | the flush cards |
| Straight | a straight exists | the straight cards |
| Three of a Kind | a 3-group exists, or any 4- or 5-group (see below) | the 3-group (or the larger group) |
| Two Pair | exactly two 2-groups, or one 3-group and one 2-group | one group: first pair, then second pair (or the 3-group) |
| Pair | any 2-group, or any 3/4/5-group (see below) | see below |
| High Card | at least one card was played | the highest card |

Filling the derived entries, in this order, after the table above:

1. If Five of a Kind is non-empty, Four of a Kind is replaced by the 5-group.
2. Then, if Four of a Kind is non-empty, Three of a Kind is replaced by that
   group (4 or 5 cards).
3. Then, if Three of a Kind is non-empty, Pair is replaced by that group (3, 4
   or 5 cards). In a Full House this drops the real pair from the Pair entry.
   The entry stays non-empty, and no joker looks at its contents.

What a played hand contains, with r = the number of non-Stone played cards of
one rank (≤ 5 cards):

- High Card: always.
- Pair: some r ≥ 2.
- Two Pair: two different ranks with r = 2 each, or a full house (r = 3 and
  r = 2). Four or five of a kind do **not** contain Two Pair.
- Three of a Kind: some r ≥ 3.
- Four of a Kind: some r ≥ 4.
- Five of a Kind: some r = 5.
- Full House: one rank with r = 3 and another with r = 2.
- Straight, Flush: as in sections 3.2 and 3.3.
- Straight Flush: a straight and a flush both exist. With Four Fingers they may
  use different cards. Example: 2♠ 3♠ 4♠ 9♠ 5♥ has a 4-card spade flush and a
  2-3-4-5 straight. It is a Straight Flush scoring all 5 cards.
- Flush House: Full House and a flush. Flush Five: Five of a Kind and a flush.

Examples of what jokers see:

- A Full House contains Full House, Two Pair, Three of a Kind, Pair and High
  Card. It does not contain Flush or Straight unless those exist too. So Spare
  Trousers (Two Pair or Full House), Mad Joker and Clever Joker (Two Pair), and
  Jolly, Sly and The Duo (Pair) all trigger on it.
- Four of a Kind contains Four of a Kind, Three of a Kind, Pair and High Card,
  but not Two Pair.
- With Four Fingers, K♠ K♥ 2♠ 5♠ 9♠ contains Flush, Pair and High Card. Its type
  is Flush.

Only non-emptiness of entries is ever read by jokers and bosses (Jolly, Zany,
Mad, Crazy, Droll, Sly, Wily, Clever, Devious, Crafty, The Duo, Trio, Family,
Order, Tribe, Spare Trousers, Runner, Superposition, Seance). Group contents are
used only to pick the scoring cards.

## 5. Choosing the hand type and the scoring cards

1. **Type.** The first non-empty entry in this priority order: Flush Five,
   Flush House, Five of a Kind, Straight Flush, Four of a Kind, Full House,
   Flush, Straight, Three of a Kind, Two Pair, Pair, High Card.
2. **Scoring cards.** The first group of that entry:
   - High Card: the highest card (1 card).
   - Pair: the 2 cards. Two Pair: the 4 cards; the fifth card does not score.
   - Three of a Kind: 3 cards. Four of a Kind: 4 cards. Five of a Kind: 5.
   - Full House and Flush House: all 5.
   - Flush: every card of the chosen suit collection (4 or 5 with Four Fingers;
     an off-suit card does not score).
   - Straight: every collected straight card, duplicates included.
   - Straight Flush: flush cards plus straight-only cards (section 4).
   - Flush Five: all 5.
3. **Stone cards.** Without an active Splash, every played Stone Card not
   already among the scoring cards is added. Stone cards always score.
4. **Splash.** With an active Splash, every played card becomes a scoring card,
   kickers included.
5. **Order.** The scoring cards are sorted into play order (by their x position
   in the play area, which follows play order). All later processing uses this
   order.
6. Debuffed cards stay in the scoring cards and keep their place. They are
   skipped when effects are applied (`scoring.md`, section 4). This matters for
   "first scoring card" checks such as Hanging Chad.
7. The type name shown on screen becomes "Royal Flush" when the type is
   Straight Flush and every scoring card has rank 10 or higher. This is display
   only; the hand type stays Straight Flush (same level, planet and counters).

Fewer than 5 cards: the N-of-a-kind types work with any count. Flush and
straight need exactly 5 cards, or 4 or 5 with Four Fingers. Below that only Four
of a Kind (4 cards), Two Pair (4 cards), Three of a Kind, Pair and High Card are
possible. More than 5 cards cannot be played. The helpers would then refuse any
flush or straight, but nothing in vanilla reaches that case.

## 6. Hand levels

### 6.1 Base values and per-level increments

All hands start at level 1 with chips and mult equal to the base values below.
"Order" is the game's own listing order of hand types.

| Order | Hand | Visible at start | Base chips | Base mult | Chips per level | Mult per level | Planet |
|---|---|---|---|---|---|---|---|
| 1 | Flush Five | no | 160 | 16 | 50 | 3 | Eris |
| 2 | Flush House | no | 140 | 14 | 40 | 4 | Ceres |
| 3 | Five of a Kind | no | 120 | 12 | 35 | 3 | Planet X |
| 4 | Straight Flush | yes | 100 | 8 | 40 | 4 | Neptune |
| 5 | Four of a Kind | yes | 60 | 7 | 30 | 3 | Mars |
| 6 | Full House | yes | 40 | 4 | 25 | 2 | Earth |
| 7 | Flush | yes | 35 | 4 | 15 | 2 | Jupiter |
| 8 | Straight | yes | 30 | 4 | 30 | 3 | Saturn |
| 9 | Three of a Kind | yes | 30 | 3 | 20 | 2 | Venus |
| 10 | Two Pair | yes | 20 | 2 | 20 | 1 | Uranus |
| 11 | Pair | yes | 10 | 2 | 15 | 1 | Mercury |
| 12 | High Card | yes | 5 | 1 | 10 | 1 | Pluto |

### 6.2 Changing a level

A level change by `amount` (default +1) does the following, immediately:

1. new level = max(0, level + amount)
2. mult = max(1, base mult + mult per level × (new level − 1))
3. chips = max(0, base chips + chips per level × (new level − 1))

Chips and mult are stored and change only through this step. A score reads the
stored values. Example: Pair at level 3 has 10 + 2×15 = 40 chips and
2 + 2×1 = 4 mult.

Sources of level changes in vanilla:

| Source | Change | When |
|---|---|---|
| Planet card | +1 to its hand | on use |
| Black Hole | +1 to all 12 hands, hidden ones included | on use |
| Orbital Tag | +3 to its chosen hand | when the tag triggers |
| Space Joker | +1 to the hand being played, 1 in 4 chance (stream `space`) | during scoring, before cards are scored (`scoring.md`, section 3) |
| Burnt Joker | +1 to the type of the discarded cards | first discard of a round, not on The Hook's forced discard |
| The Arm (boss) | −1 to the hand being played, only if its level is above 1 | each play, before scoring |

Because The Arm only acts above level 1, levels stay at 1 or more in vanilla.
The minimums (level 0, mult 1, chips 0) are never reached. The play code also
checks a "first used hand level" bonus that nothing in vanilla sets. Ignore it.

### 6.3 Per-hand counters and hidden hands

Each hand type stores:

- **played**: run total. It grows by 1 at the start of the play evaluation,
  before the boss's hand check, so it counts plays the boss made worthless too.
- **played this round**: same timing. Reset to 0 for every hand when a new
  round (blind) starts.
- **visible**: starts false for Five of a Kind, Flush House and Flush Five, true
  for the others. Set to true the first time the hand is played, also at the
  start of the evaluation and also for a debuffed hand. Visibility is read by
  the run-info screen, To Do List's target choice, Obelisk's "most played"
  comparison and Telescope's Celestial pack choice.

The game also stores the last played hand type (used by the Blue seal) at the
same moment.

Planet X, Ceres and Eris can appear in shops and packs only once their hand has
`played` > 0. This is a different test from visible, though both become true on
the first play. Black Hole still levels hidden hands. A level gained while a
hand is hidden applies once it is played.

The Ox's "most played hand" is a separate value. It starts as High Card and is
recomputed only when a Boss Blind is beaten, from the `played` counts. On ties
the scan keeps the last tied hand it meets, and the scan follows Lua's table
iteration order: LuaJIT 2.0.5's hash order for the hands table, confirmed on
the game's own runtime. `rules/blinds.md` (section 4 and the notes on The Ox)
gives the order and the tie rule.

### 6.4 Decks, stakes and challenges

I found nothing in the vanilla decks, stakes or challenges that changes the hand
table, the starting levels or the detection rules. Plasma Deck changes the final
score step, not hand values (see `scoring.md`).

## Where in the game

- functions/misc_functions.lua: `evaluate_poker_hand`, `get_X_same`,
  `get_flush`, `get_straight`, `get_highest`, `find_joker`, `pseudorandom_element`
- functions/state_events.lua: `G.FUNCS.get_poker_hand_info`,
  `G.FUNCS.evaluate_play` (first part: counters, Stone/Splash additions, sort),
  `G.FUNCS.play_cards_from_highlighted`, `new_round`, `end_round`
- card.lua: `Card:get_id`, `Card:get_nominal`, `Card:is_suit`, `Card:is_face`,
  `Card:set_base`, `Card:init` (unique value), `Card:use_consumeable` (Black Hole)
- cardarea.lua: `CardArea:parse_highlighted`, `CardArea:align_cards`,
  `CardArea:add_to_highlighted`
- functions/common_events.lua: `level_up_hand`, `copy_card`, `get_current_pool`
  (planet softlock)
- functions/button_callbacks.lua: `G.FUNCS.can_play`
- game.lua: `Game:init_game_object` (hands table), planet definitions,
  `G.P_BLINDS`
- globals.lua: hand list order
- blind.lua: `Blind:debuff_hand` (The Arm)
- tag.lua: `Tag:apply_to_run` (Orbital Tag)
