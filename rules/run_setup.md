# Run setup, the deck and drawing

How a new run of Balatro 1.0.1o starts and how playing cards move during a run,
written for the engine. A new run builds a fresh game state (for Red Deck at White
Stake: $4, 4 hands, 4 discards, hand size 8, 5 joker slots, 2 consumable slots),
sets the seed, then draws ante 1's boss, the first shop voucher and the two skip
tags, creates the 52 cards in a fixed sorted order, shuffles the deck once
(stream `shuffle`), and picks the per-round targets of four jokers. Every
shuffle and every random pick from a list of cards first sorts the list by
`sort_id`, a global counter that every created card takes in creation order, so
only the set of cards (and their creation order) matters, never the order they
were in before. Cards are drawn from the end of the deck list; after every single
card drawn the whole hand is re-sorted by the current sort mode (by rank, unless
the player pressed the suit button), so newly drawn cards land in sorted position
and played cards score left to right in that order unless the player rearranges
them. The deck is reshuffled when each blind starts (stream `nr`+ante) and when
cash-out is pressed (stream `cashout`+ante). Random-stream keys, list orders
before picks and the seed are catalogued in `rules/rng_keys.md`; the RNG recipe
itself is in `rules/rng.md`.

Notation: A is the current ante number written as a plain integer (`1`, `10`,
`0`, `-1`). "Stream `x`" means one draw from the named stream `x`.

## 1. Starting values (Red Deck, White Stake)

The game object is built fresh, then the stake and the deck modify its starting
parameters, then the per-round values are copied from the starting parameters.

| Value | Start | Notes |
|---|---|---|
| Money | $4 | |
| Hands per round | 4 | `round_resets.hands`; hands left at run start = 4 |
| Discards per round | 4 | base 3, Red Deck +1; discards left at run start = 4 |
| Hand size | 8 | the hand area's card limit |
| Joker slots | 5 | |
| Consumable slots | 2 | |
| Cards you may select | 5 | hand highlight limit; play area holds 5 |
| Ante | 1 | `round_resets.ante`; `round_resets.blind_ante` = 1 |
| Round counter | 0 | +1 each time a blind is selected |
| Winning ante | 8 | beating the boss of ante 8 wins; play may continue |
| Interest | $1 per full $5 held, counted up to $25 held (max $5) | `interest_amount` 1, `interest_cap` 25; only if money is at least $5 and the deck allows interest |
| Money per remaining hand | $1 | at round end (`money_per_hand` unset means 1) |
| Reroll cost | $5 | base 5; +$1 per paid reroll in the same shop; free rerolls 0 |
| Chance numerator | 1 | `probabilities.normal`; every "1 in N" roll compares with normal/N |
| Shop card slots | 2 | `shop.joker_max` (jokers and consumables share them) |
| Shop voucher slots | 1 | |
| Shop booster slots | 2 | |
| Shop type weights | Joker 20, Tarot 4, Planet 4, playing card 0, Spectral 0 | total 28 |
| Edition rate | 1 | multiplies foil/holo/polychrome chances (not negative) |
| Discount percent | 0 | |
| Inflation | 0 | |
| Rental cost | $3 per round | `rental_rate` |
| Perishable life | 5 rounds | `perishable_rounds` |
| Debt limit | $0 | `bankrupt_at` |
| Ectoplasm hand-size loss | 1 | grows by 1 per Ectoplasm |
| Starting deck size | 52 | recorded after the cards are made |
| Hand sort mode | by rank, descending | see section 8 |

Other starting fields rules depend on:

- `round_resets`: hands 4, discards 4, reroll cost 5, temporary reroll cost and
  temporary hand size unset, ante 1, blind_ante 1, blind states Small = Select,
  Big = Upcoming, Boss = Upcoming, blind choices Small = `bl_small`,
  Big = `bl_big`, Boss = drawn at start (section 3), boss_rerolled false,
  blind tags Small and Big = drawn at start.
- `round_bonus`: next_hands 0, discards 0 (added to the next round's hands and
  discards, then cleared).
- `current_round`: hands left 4, discards left 4, hands played 0, discards
  used 0, reroll cost 5, reroll cost increase 0, free rerolls 0, jokers
  purchased 0, round dollars 0, used packs empty, most played poker hand
  `High Card`, shop voucher = drawn at start, and the four per-round targets
  (defaults Ace of Spades for the Idol card, Ace for Mail-In Rebate, Spades for
  Ancient Joker and Castle, all redrawn at start: section 3).
- Bookkeeping that later rules read: total hands played 0, skips 0, unused
  discards 0, max jokers 0, `used_jokers` empty (a center is marked when any
  card of it is created outside a menu, and unmarked when such a card is
  removed while no copy sits in the joker or consumable slots; marked centers
  leave the random pools unless Showman is held), `used_vouchers` empty, `pool_flags`
  empty (Gros Michel extinction), `banned_keys` empty, tags empty,
  `last_tarot_planet` unset, consumable and joker buffers 0, `bosses_used` =
  every boss blind with count 0, `seeded` true when the player typed a seed.

Starting poker hands (all at level 1; a level adds the per-level amounts; hands
marked hidden become visible when first played):

| Hand | Chips | Mult | +Chips/level | +Mult/level | Visible |
|---|---|---|---|---|---|
| Flush Five | 160 | 16 | 50 | 3 | hidden |
| Flush House | 140 | 14 | 40 | 4 | hidden |
| Five of a Kind | 120 | 12 | 35 | 3 | hidden |
| Straight Flush | 100 | 8 | 40 | 4 | yes |
| Four of a Kind | 60 | 7 | 30 | 3 | yes |
| Full House | 40 | 4 | 25 | 2 | yes |
| Flush | 35 | 4 | 15 | 2 | yes |
| Straight | 30 | 4 | 30 | 3 | yes |
| Three of a Kind | 30 | 3 | 20 | 2 | yes |
| Two Pair | 20 | 2 | 20 | 1 | yes |
| Pair | 10 | 2 | 15 | 1 | yes |
| High Card | 5 | 1 | 10 | 1 | yes |

Each hand also has an `order` from 1 (Flush Five) to 12 (High Card), used to
break ties when finding the most played hand.

## 2. What other decks and stakes change

Decks (applied once at run start, before the seed is set):

| Deck | Change |
|---|---|
| Red | +1 discard |
| Blue | +1 hand |
| Yellow | +$10 (start $14) |
| Green | no interest; at round end $2 per remaining hand and $1 per remaining discard |
| Black | +1 joker slot, -1 hand |
| Magic | Crystal Ball voucher (+1 consumable slot) and two The Fool cards (made right after start, no random draw) |
| Nebula | Telescope voucher, -1 consumable slot |
| Ghost | Spectral shop weight 2; starts with a Hex card (no random draw) |
| Abandoned | no Jacks, Queens or Kings (40 cards) |
| Checkered | right after the run starts, every Club becomes a Spade and every Diamond a Heart (26 + 26); the ante-1 Idol card and Castle suit are picked before this, so they can name a Club or Diamond |
| Zodiac | Tarot Merchant, Planet Merchant and Overstock vouchers |
| Painted | +2 hand size, -1 joker slot |
| Anaglyph | gains a Double Tag after each defeated Boss Blind |
| Plasma | blind targets x2; chips and mult are averaged at the last scoring step |
| Erratic | each of the 52 cards gets a random rank and suit (stream `erratic`, 52 draws) |

A deck's starting vouchers are marked as redeemed before the first voucher is
drawn, so they (not their upgrades) leave the voucher pool. Starting
consumables count as existing cards, so they leave their pools while held.

Stakes (cumulative; each includes all lower ones):

| Stake | Change |
|---|---|
| 1 White | none |
| 2 Red | the Small Blind pays no reward money |
| 3 Green | blind targets grow faster (scaling level 2) |
| 4 Black | shop and pack jokers can be Eternal |
| 5 Blue | -1 discard |
| 6 Purple | blind targets grow faster still (scaling level 3) |
| 7 Orange | shop and pack jokers can be Perishable |
| 8 Gold | shop and pack jokers can be Rental; also changes how unseeded runs pick their seed (`rules/rng_keys.md`, section 5) |

Challenges always start at stake 1 and may replace the deck, the starting
parameters, the jokers, consumables and vouchers, and ban cards, tags and
blinds; banned keys are left out of every pool.

## 3. Run start, step by step

What happens when a new (not continued) run starts, in order. Draws on
different streams never affect each other, so only the order within one stream
matters; the order below is the game's.

1. Build the game object (section 1). Apply the stake's modifiers, then the
   deck's (section 2). Copy hands, discards, reroll cost and money into the
   per-round values.
2. Set the seed (typed, or generated: `rules/rng_keys.md`, section 5) and its
   hash.
3. Ante 1's boss: one draw on stream `boss` (eligible bosses at ante 1 sorted by
   key: `bl_club`, `bl_goad`, `bl_head`, `bl_hook`, `bl_manacle`, `bl_pillar`,
   `bl_psychic`, `bl_window`; the pick also raises that boss's use count).
4. The first shop voucher: stream `Voucher1`, resampling on `Voucher1_resample2`,
   `Voucher1_resample3`, ... while the pick is unavailable.
5. The Small Blind's skip tag, then the Big Blind's: two draws on stream `Tag1`.
   An unavailable pick is redrawn on `Tag1_resample2`, then `Tag1_resample3`,
   and so on; these resample streams are shared by both picks (and by every
   later pick keyed `Tag1`).
6. Create the card areas (consumables, jokers, discard pile, deck, hand, play
   area), then the starting cards (section 4). Erratic draws its 52 picks here.
7. Set discards left and hands left to the round values.
8. Shuffle the deck once on stream `shuffle`. This is the only use of that
   stream. The order it gives matters only if cards are drawn before the first
   blind is played (a pack opened from a skip tag at ante 1).
9. Pick this round's targets, in this order: Idol card (stream `idol1`),
   Mail-In Rebate rank (`mail1`), Ancient Joker suit (`anc1`, the previous suit
   is cleared first so all four suits are possible), Castle suit (`cas1`).
10. Create an empty blind. Nothing else is drawn.
11. After the function returns, queued effects run: Magic Deck's two Fools,
    Ghost Deck's Hex, Checkered Deck's suit changes, a challenge's jokers,
    consumables and vouchers. None of them draws, except that creating a To Do
    List card draws stream `to_do`.
12. The blind-select screen opens. Building it draws stream `orbital` three
    times (for Small, Big, Boss, in that order) to fix the hand an Orbital Tag
    would level for each blind of ante 1, whether or not an Orbital Tag is
    offered (`rules/rng_keys.md`, Orbital Tag).

## 4. The starting cards

### Creation order

1. Collect one prototype per standard card (52) as a suit letter
   (`C`, `D`, `H`, `S`) and a rank letter (`2`-`9`, `T`, `J`, `Q`, `K`, `A`).
   Abandoned Deck leaves out `J`, `Q`, `K`. Erratic Deck replaces each of the 52
   prototypes with a uniformly random card (one draw on `erratic` each, picking
   from the 52 card keys in the key order given in `rules/rng_keys.md`);
   duplicates are allowed. Challenge decks may give an explicit list or filter
   ranks and suits, and may add an enhancement, edition or seal.
2. Sort the prototypes by the string suit + rank + enhancement + edition + seal
   (empty strings for the missing parts), compared byte by byte. For a standard
   deck this gives, within each suit, `2 3 4 5 6 7 8 9 A J K Q T`, and suits in
   the order Clubs, Diamonds, Hearts, Spades.
3. Create the cards in that order. Each card is put at the bottom of the deck
   and appended to the run's list of playing cards.

So for Red Deck the creation order is:

| Positions | Suit | Ranks in order |
|---|---|---|
| 1-13 | Clubs | 2, 3, 4, 5, 6, 7, 8, 9, Ace, Jack, King, Queen, 10 |
| 14-26 | Diamonds | same |
| 27-39 | Hearts | same |
| 40-52 | Spades | same |

Checkered Deck converts suits after creation, so its creation order is still
the one above (former Clubs, then former Diamonds, Hearts, Spades).

### Card identity counters

- `sort_id`: a global counter that never resets (not between runs either).
  Every card object of any kind takes the next value when it is created:
  playing cards, jokers, consumables, the shop's voucher and booster cards,
  pack contents, cards made by tags, jokers or consumables, copies (Cryptid,
  DNA, Ankh, Invisible Joker, Perkeo), and also cards built only for the
  screen (title screen, collection pages, deck view). Only the relative order
  of the cards involved in a shuffle or pick matters, so the engine can use
  any counter that increases in the game's creation order. Converting a card
  in place (Death, Strength, suit and rank changes, enhancements) keeps its
  `sort_id`.
- `unique_val`: 1 minus the object's node id divided by 1603301. Node ids come
  from another global counter that every screen object takes, so for cards it
  also follows creation order: a later card has a smaller `unique_val`. It is
  used only to break ties when sorting the hand (section 8).
- `playing_card`: an index counter for playing cards. It does not affect any
  random draw (the one place that sorts by it, Immolate, re-sorts by `sort_id`
  before shuffling).

### Canonical order before shuffles and picks

- A shuffle first sorts the list by `sort_id`, smallest first, then shuffles
  (`rules/rng.md` gives the shuffle). The order a list was in before is
  irrelevant.
- A random pick from a list of cards also sorts by `sort_id` first. Picks from
  lists that are not cards sort by key. Details and every list's order are in
  `rules/rng_keys.md`, section 2.

## 5. The deck as a stack

- The deck is a list. Drawing always takes the last card of the list (the top).
  Putting a card into the deck always inserts it at the front (the bottom).
- The hand, play area and discard pile append cards at the end. Taking a card
  without naming it takes the first card of the hand or play area and the last
  card of the discard pile.
- After a shuffle, the card that ended in the last position is drawn first.
  For a list L of n cards after shuffling, the draw order is L[n], L[n-1], ...

## 6. A round

### Selecting a blind

1. The round counter goes up by 1 and the selected blind becomes Current.
2. New round: discards left = max(0, round discards + discard bonus); hands
   left = max(1, round hands + hand bonus); hands played, discards used and the
   reroll-cost increase reset to 0; bonuses are cleared; each hand's
   played-this-round count resets; free rerolls = number of Chaos the Clown;
   the reroll cost is recomputed.
3. The blind is set up (its own effects: The Water, The Needle, The Manacle,
   Amber Acorn and so on), then each joker reacts to the blind starting, left to
   right. Marble Joker's Stone card reaches the deck before the shuffle.
4. The deck is shuffled on stream `nr`+A (for ante 1: `nr1`). Each blind
   actually played draws once, so in an ante with no skips the Small, Big and
   Boss blinds use draws 1, 2 and 3 of `nrA`; a skipped blind does not draw.
   Because the shuffle sorts by `sort_id` first, the deck's order before this
   shuffle never matters.
5. Drawing to hand starts (next subsection). Skip tags that act at round start
   (Juggle Tag: +3 hand size for this round) apply before the draw.

### Drawing to hand

- Number of cards: min(cards in deck, hand size - cards in hand). Exception:
  against The Serpent (not disabled), after the first play or discard of the
  round, exactly min(cards in deck, 3) are drawn even if that overfills the
  hand.
- If the hand size is 0 or less and the hand is empty (outside an Arcana or
  Spectral pack), the run is lost immediately.
- Cards come off the top of the deck one at a time. Each drawn card is appended
  at the right end of the hand and then the whole hand is re-sorted by the
  current sort mode (section 8). The result after any draw is therefore the
  fully sorted hand; a manual arrangement made earlier is lost.
- A card may stay face down: The Wheel (stream `wheel`, one draw per card drawn
  into the hand, face down when the value is below normal/7), The House (cards
  drawn before the first play or discard), The Mark (face cards), The Fish
  (cards drawn after a play), and the challenge rule "flipped cards" (stream
  `flipped_card`). Face-down cards sort like any other.
- After the opening draw of a round (no hands played, no discards used),
  jokers with a first-hand-drawn effect act (Certificate adds a card and sorts
  the hand). Then the blind's after-draw effect runs (Cerulean Bell forces a
  card, Crimson Heart debuffs a joker).
- If the hand size goes up while the hand holds cards during drawing or
  selecting (for example a hand-size joker created by Judgement mid-round), the
  extra cards are drawn at once, each followed by a sort. A rise that happens
  while the hand is empty (Juggle Tag at round start) just raises the limit for
  the normal draw.

### Playing a hand

- The selected cards are ordered left to right by their position in the hand
  and moved to the play area in that order; they score in that order. The
  player may rearrange the hand at any time before playing, so the scoring
  order is the hand order at the moment of play.
- After scoring, the played cards that were not destroyed go to the discard
  pile in play order. Destroyed cards (shattered Glass, Sixth Sense, ...) leave
  the run.
- If the blind's target is reached, or no hands are left, the round ends (a loss
  if the target was not reached, unless Mr. Bones saves the run). Otherwise the
  hand is refilled (drawing to hand above).
- The Hook: when a hand is played, after the played cards have left the hand,
  two cards are picked from the remaining hand (stream `hook`, twice, the second
  pick from the cards left after the first) and discarded. This discard does
  not use a discard and does not draw; the normal refill after scoring covers
  it.

### Discarding

- The selected cards, ordered left to right by hand position, go to the
  discard pile in that order (unless a joker destroys them). One discard is
  used, then the hand is refilled the same way.

### When the deck runs out

- Drawing simply takes fewer cards (possibly none). The discard pile is not
  reshuffled into the deck during a round.
- When the hand and the deck are both empty, the round ends at once; it is lost
  unless the score already beats the blind (or Mr. Bones saves the run).

### End of a won round

In this order:

1. Jokers' end-of-round effects, rental costs and perishable counters, left to
   right.
2. End-of-round effects of cards held in hand (Gold cards, Blue seals, ...).
3. The hand goes to the discard pile, leftmost card first.
4. After a Boss Blind: the ante goes up by 1 (so later keys in this step use the
   new A).
5. The discard pile goes back into the deck: the last card of the pile is taken
   first and inserted at the bottom, repeatedly. The result, from bottom to top,
   is: discard pile in its original order (first discarded at the bottom), then
   the cards that were never drawn, in their old order, on top. This order is
   never used: cash-out reshuffles the deck before any card can be drawn.
6. Round evaluation: after a Boss Blind the next shop voucher is drawn (stream
   `Voucher`+A with the new A). Temporary hand size and temporary reroll cost
   end. The four per-round targets are redrawn on `idol`+A, `mail`+A, `anc`+A
   (excluding the current Ancient suit), `cas`+A.

## 7. Cash-out and between rounds

Pressing cash-out:

1. Shuffle the deck on stream `cashout`+A (A after any boss increase: the
   cash-out after ante 1's boss uses `cashout2`).
2. Money for the round is added; hands and discards left are reset for display.
3. If the Boss Blind was just defeated: `blind_ante` = A, then new skip tags for
   Small and Big (two draws on `Tag`+A), then a new boss (stream `boss`), the
   blind states reset to Small = Select, Big and Boss = Upcoming, and boss
   rerolled = false.

Between rounds the deck order matters only when cards are drawn from it, which
happens when an Arcana or Spectral pack is opened (from the shop or from a skip
tag at blind select):

- The hand is filled from the top of the deck (same rule and sort as in a
  round).
- When the pack closes, the hand goes back into the deck, leftmost card first,
  each inserted at the bottom. A second pack opened in the same shop therefore
  draws the next cards from the top, not the same ones.
- Playing cards bought from the shop or taken from a Standard pack are inserted
  at the bottom of the deck.
- The deck at the first blind select of the run is in the `shuffle` order. In
  later shops and blind selects it is in the last `cashout` order, changed only
  by pack draws and additions as above.

## 8. Hand sorting

The hand is sorted with a key computed for each card:

key = rank value + suit value x M + original-suit value x 0.0001 x M
+ face value + 0.000001 x `unique_val`

- Rank value: 2 to 10 for number cards, 10 for Jack, Queen and King, 11 for Ace.
- Face value: Jack 0.1, Queen 0.2, King 0.3, Ace 0.4, others 0.
- Suit value: Diamonds 0.01, Clubs 0.02, Hearts 0.03, Spades 0.04.
- Original-suit value: Diamonds 0.001, Clubs 0.002, Hearts 0.003, Spades 0.004,
  for the suit the card had when it was first made (it survives suit changes; a
  copy made by Cryptid or DNA takes the copied card's current suit as its
  original).
- M is 1 when sorting by rank and 1000 when sorting by suit. For Stone cards M
  is -1000 in both modes, which puts them after every other card.
- The two buttons set the mode to "by rank, descending" or "by suit,
  descending"; the mode stays until the other button is pressed and is reset to
  by rank at the start of a run. Every sort is descending by key, so the
  leftmost card has the largest key.

By rank: Aces, Kings, Queens, Jacks, 10s, ..., 2s, then Stone cards; within a
rank Spades, Hearts, Clubs, Diamonds. By suit: Spades, Hearts, Clubs, Diamonds,
each from Ace down to 2, then Stone cards. Two cards with the same rank, suit
and original suit keep creation order (older card further left). All keys are
distinct, so the result does not depend on the sort algorithm.

Cards added to the hand without a draw (Cryptid and DNA copies, the cards made
by Familiar, Grim and Incantation) are appended at the right end unsorted; the
next draw sorts them in.

## 9. Real-game quirks that matter for validation

These come from how the game saves and shows things, not from the rules. The
engine should model an uninterrupted session; tests against the real game
should avoid them.

- Continuing a saved run restores each card's `sort_id` but not the global
  counter, so cards created after a continue can get smaller `sort_id`s than
  older cards and later shuffles and picks change.
- Continuing a saved run rebuilds the poker-hand table in a different internal
  order, which can change Orbital Tag and To Do List picks
  (`rules/rng_keys.md`, section 2).
- Opening the joker collection page that shows To Do List during a run builds a
  To Do List card, which draws stream `to_do`.
- Discoveries and unlocks are frozen during seeded runs (and challenge runs).
  In unseeded runs a first discovery or unlock can change a pool mid-run.
  Pools also depend on the profile's unlocks and discoveries (Rare, Negative,
  Foil, Holographic and Polychrome tags need Blueprint or that edition
  discovered; upgraded vouchers and many jokers need unlocking). The engine
  needs one fixed profile assumption, most naturally "everything unlocked and
  discovered".

## Where in the game

- `game.lua`: `Game:init_game_object`, `Game:start_run`, `Game:init_item_prototypes`, `Game:prep_stage`, `Game:delete_run`, `Game:update_draw_to_hand`, `Game:update_hand_played`, `Game:update_selecting_hand`, `Game:update_new_round`, `Game:update_blind_select`, `Game:update_round_eval`, `Game:update_shop`, `Game:update_arcana_pack`
- `back.lua`: `Back:init`, `Back:apply_to_run`, `Back:trigger_effect`
- `cardarea.lua`: `CardArea:init`, `CardArea:emplace`, `CardArea:remove_card`, `CardArea:change_size`, `CardArea:set_ranks`, `CardArea:align_cards`, `CardArea:shuffle`, `CardArea:sort`, `CardArea:draw_card_from`, `CardArea:update`, `CardArea:save`, `CardArea:load`
- `card.lua`: `Card:init`, `Card:set_base`, `Card:set_ability`, `Card:get_nominal`, `Card:get_id`, `Card:apply_to_run`, `Card:open`, `Card:use_consumeable`, `Card:get_end_of_round_effect`, `Card:calculate_joker` (Marble Joker, Certificate, DNA), `Card:save`, `Card:load`, `Card:remove`
- `blind.lua`: `Blind:set_blind`, `Blind:press_play`, `Blind:drawn_to_hand`, `Blind:stay_flipped`, `Blind:defeat`
- `tag.lua`: `Tag:init`, `Tag:apply_to_run`
- `functions/state_events.lua`: `end_round`, `new_round`, `G.FUNCS.draw_from_deck_to_hand`, `G.FUNCS.discard_cards_from_highlighted`, `G.FUNCS.play_cards_from_highlighted`, `G.FUNCS.draw_from_play_to_discard`, `G.FUNCS.draw_from_discard_to_deck`, `G.FUNCS.draw_from_hand_to_deck`, `G.FUNCS.draw_from_hand_to_discard`, `G.FUNCS.evaluate_round`
- `functions/common_events.lua`: `draw_card`, `add_joker`, `ease_ante`, `ease_round`, `create_playing_card`, `get_next_voucher_key`, `get_next_tag_key`, `get_current_pool`, `reset_idol_card`, `reset_mail_rank`, `reset_ancient_card`, `reset_castle_card`, `reset_blinds`, `get_new_boss`, `calculate_reroll_cost`, `find_joker`, `discover_card`, `unlock_card`, `check_for_unlock`
- `functions/misc_functions.lua`: `get_starting_params`, `card_from_control`, `pseudoshuffle`, `pseudorandom_element`, `get_X_same`, `get_straight`, `save_run`
- `functions/button_callbacks.lua`: `G.FUNCS.select_blind`, `G.FUNCS.skip_blind`, `G.FUNCS.reroll_boss`, `G.FUNCS.cash_out`, `G.FUNCS.end_consumeable`, `G.FUNCS.use_card`, `G.FUNCS.buy_from_shop`, `G.FUNCS.sort_hand_suit`, `G.FUNCS.sort_hand_value`, `G.FUNCS.start_setup_run`, `G.FUNCS.start_run`
- `functions/UI_definitions.lua`: `create_UIBox_blind_select`, `create_UIBox_blind_choice`, `create_UIBox_blind_tag`, `G.UIDEF.shop`
- `engine/node.lua`: `Node:init`
- `engine/string_packer.lua`: `STR_PACK`, `STR_UNPACK`
