# Round flow, playing, end of round and money

A round starts when the player selects a blind: counters reset, the blind's
target and effects are set, jokers react, the whole deck is shuffled with
stream `nr<ante>`, and the hand is filled up to hand size. The player then
plays or discards 1 to 5 selected cards at a time (4 hands and 4 discards for
Red Deck at White Stake, hand size 8), refilling the hand after each action.
The score of a hand is computed in one step after the cards reach the play
area. The round is won as soon as the total chips reach the target (checked
after each scored hand); it is lost when the last hand is spent short of the
target or the hand and deck both run out, unless Mr. Bones saves it (total at
least 25% of the target). A won round runs end-of-round effects in a fixed
order (jokers, held Gold Cards and Blue Seals, the ante step after a boss,
new random "card of the round" picks), then the cash-out screen adds up: blind
reward + $1 per hand left (deck-dependent) + joker payouts + tag payouts +
interest of $1 per full $5 held, at most $5, computed on the money held at
that moment. Pressing Cash Out pays it, reshuffles the deck with stream
`cashout<ante>`, and (after a boss) draws the next ante's tags and boss. Beating
the ante 8 boss wins the run; play may continue in endless mode.

Stream names and their per-stream draw order are given in section 13. Boss
effects are in `rules/blinds.md`; what the player may do at each moment is in
`rules/timing.md`.

## 1. Starting values

Red Deck at White Stake:

| Item | Value |
|------|-------|
| Money | $4 |
| Hands per round | 4 |
| Discards per round | 4 (3 base + 1 Red Deck) |
| Hand size | 8 |
| Joker slots | 5 |
| Consumable slots | 2 |
| Shop reroll base cost | $5 |
| Interest | $1 per full $5 held, cap counts money up to $25 (so at most $5) |
| Ante / round | ante 1, round counter 0 (the first blind played is round 1) |
| Winning ante | 8 |
| Bankruptcy floor | $0 (Credit Card lowers it to -$20) |
| Most played hand (for The Ox) | High Card |

Stakes stack (each includes all lower ones):

| Stake | Effect relevant here |
|-------|----------------------|
| 1 White | base rules |
| 2 Red | Small Blind pays no reward |
| 3 Green | chip targets use scaling 2 |
| 4 Black | shop jokers can be Eternal (shop notes) |
| 5 Blue | -1 discard per round |
| 6 Purple | chip targets use scaling 3 |
| 7 Orange | shop jokers can be Perishable (shop notes) |
| 8 Gold | shop jokers can be Rental (shop notes) |

Stake effects are applied before the deck's own effects (so Red Deck at Blue
Stake has 3 discards).

Deck effects that touch rounds and money: Blue +1 hand; Yellow +$10 starting
money; Green pays $2 per hand left and $1 per discard left at cash out and
earns no interest; Black -1 hand and +1 joker slot; Painted +2 hand size and -1
joker slot; Plasma doubles every chip target (and balances chips and mult when
scoring); Anaglyph gains a Double Tag after each won boss round; Nebula -1
consumable slot; Magic and Zodiac start with vouchers; Abandoned has no face
cards; Erratic randomises the deck. Vouchers bought later: Grabber/Nacho Tong
+1 hand each, Wasteful/Recyclomancy +1 discard each, Paint Brush/Palette +1
hand size each, Hieroglyph -1 ante and -1 hand, Petroglyph -1 ante and -1
discard, Seed Money raises the interest cap to $50 (at most $10), Money Tree to
$100 (at most $20). To the Moon adds $1 to the interest paid per $5.

## 2. The cycle of states

1. Blind select: choose the blind on deck, skip it (Small/Big), or reroll the
   boss (with vouchers). See `rules/blinds.md`.
2. Selecting a blind starts the round (section 3), draws the hand (section 4)
   and enters hand selection.
3. Play (section 5) or discard (section 6); after each, the hand refills and
   selection resumes, until the round ends (section 7).
4. A won round runs end-of-round processing (section 8), then the cash-out
   screen (section 9). Pressing Cash Out (section 10) leads to the shop.
5. Leaving the shop returns to blind select with the next blind on deck.

A skipped blind goes straight from blind select to blind select. The ante
rises only when a Boss round is won.

## 3. Starting a round (after Select)

In order:

1. Use is blocked briefly (stop-use counter, see timing notes); the game marks
   that a blind is being faced.
2. The round counter rises by 1; the chosen blind becomes the current one and
   its slot is marked Current.
3. Counters for the new round: discards left = max(0, base discards); hands
   left = max(1, base hands); hands played this round = 0; discards used = 0;
   the shop reroll increase = 0; the shop's pack slots are cleared; every
   poker hand's "played this round" count = 0; face-down marks on cards are
   cleared; free shop rerolls = number of (non-debuffed) Chaos the Clown
   jokers; the shop reroll cost is recomputed from its base.
4. The blind is set: its target (see `rules/blinds.md`), its reward, its card
   debuffs, and its start-of-blind effects (The Water removes discards, The
   Needle removes hands, The Manacle lowers hand size, Amber Acorn flips and
   queues its joker shuffles).
5. Every joker, left to right, gets the "blind is being set" event (for
   example Chicot disables a boss, Burglar adds hands and removes discards,
   Marble Joker adds a Stone Card).
6. After a short pause: the whole deck is shuffled with stream `nr<ante>` (the
   shuffle first orders cards by creation, so the previous deck order never
   matters), and the hand is drawn.

## 4. Drawing and hand order

1. When the hand is to be drawn, every held tag first gets the "round start
   bonus" check (Juggle Tag: +3 hand size for this round).
2. Cards to draw = min(cards in deck, hand size - cards in hand); nothing if
   that is 0 or less. Exception, The Serpent: once a hand has been played or a
   discard used this round, exactly min(3, cards in deck), ignoring hand size.
3. Cards are drawn one at a time from the end of the deck list (the "top").
   Boss face-down rules are rolled per card as it arrives (The Wheel draws
   stream `wheel` once per card).
4. Every time a card is drawn from the deck into the hand, the whole hand is
   re-sorted by the current sort mode. Any manual arrangement is lost at each
   draw. (Cards created straight into the hand by jokers or consumables follow
   their own rules; see the card notes.) The mode starts as
   "by rank"; pressing Sort by Suit or Sort by Rank sorts at once and makes
   that mode the one used from then on.
   - By rank, descending: key = rank value (2-10 face value, J/Q/K 10, A 11)
     + 0.1 J / 0.2 Q / 0.3 K / 0.4 A + suit tie-break (Diamonds 0.01, Clubs
     0.02, Hearts 0.03, Spades 0.04) + an original-suit term (0.001-0.004 in
     the same suit order for the suit the card was created with, times
     0.0001) + a last term of at most 0.000001 that is larger for
     earlier-created cards. Effective order: rank, suit, original suit,
     earlier-created first.
   - By suit, descending: both suit terms are multiplied by 1000 so suit
     dominates (Spades, Hearts, Clubs, Diamonds), then rank, then original
     suit, then creation.
   - Stone Cards multiply both suit terms by -1000 in both modes, which puts
     them at the right end in either mode.
   - UNSURE: the last term comes from a global object counter that also
     counts UI objects; for two cards of equal rank and suit that differ only
     in original suit and were created very far apart in a long run, it could
     outweigh the original-suit term. Engine: use the effective order above.
5. On the first draw of a round only (no hand played, no discard used), every
   joker gets the "first hand drawn" event; it is raised as soon as the draws
   are queued, so effects it creates (Certificate's card) arrive after the
   drawn cards.
6. When the draw is complete the game enters hand selection and the blind's
   "after draw" effect runs (Cerulean Bell forces a card, Crimson Heart picks
   a joker to debuff).
7. If the hand ever becomes empty during hand selection while the deck still
   has cards, the game draws again automatically.
8. If hand size rises during a round, the extra cards are drawn at once; if it
   falls, nothing is discarded.

Scoring order of played cards and held cards is their left-to-right position,
so the player's arrangement between draws matters.

## 5. Selecting and playing a hand

Selection: clicking a hand card toggles it; at most 5 cards can be selected
(a sixth click does nothing). A card forced by Cerulean Bell cannot be
deselected. Right-click deselects all (except a forced card).

Play is possible when 1 to 5 cards are selected, the play area is empty, and
the boss warning is not showing (cosmetic pause of about a second at the start
of a boss round). Steps, in order:

1. Use is blocked (stop-use counter); the blind's "triggered" flag is cleared;
   every card's forced-selection mark is cleared.
2. The selected cards are ordered by on-screen position, left to right.
3. State becomes "hand played"; hands left goes down by 1 (so it is already
   reduced while the hand is scored).
4. Each played card: times-played +1, marked "played this ante" (The Pillar),
   moved to the play area in that order.
5. The blind's play effect runs: The Hook discards up to 2 held cards, The
   Tooth takes $1 per played card, Crimson Heart and The Fish arm themselves
   for the next draw. The Hook's discard and The Tooth's charge finish before
   step 6.
6. The hand is scored in one step (scoring notes): hand type found, its play
   counts updated, "last hand played" recorded, boss hand checks (The Psychic,
   The Eye, The Mouth, The Arm, The Ox), jokers, cards, held cards, then the
   chip total for this hand = floor(chips x mult) is queued to be added to the
   round total. Money gained while scoring is queued and lands afterwards.
7. The played cards go to the discard pile (destroyed ones are removed
   instead); hands played this round +1; hands played this run +1.
8. Round end check: if round total >= target, or hands left < 1, the round
   ends (section 7); otherwise the hand is refilled (section 4).

The check happens only here, after a complete hand; a round never ends
in the middle of scoring.

## 6. Discarding

Discard is possible when at least 1 card is selected and at least 1 discard
is left (at most 5 cards, the selection limit). Steps, in order:

1. Use is blocked; forced-selection marks are cleared.
2. The selected cards are ordered left to right.
3. Every joker gets the "before discard" event with the whole set.
4. For each card in that order: its seal reacts (Purple Seal makes a Tarot if
   there is room), then every joker gets the "card discarded" event (a joker
   may destroy the card); the card goes to the discard pile, or is destroyed.
5. If any card was destroyed, jokers are told which.
6. A challenge discard cost would be charged here (none in normal runs).
7. Discards left goes down by 1 (applied a moment later); discards used +1
   (applied at once, so discard jokers see the count before this discard).
8. The hand is refilled (section 4).

The Hook's forced discard runs the same steps 2-5 for its 2 cards but uses no
discard, does not count as a discard used, and does not refill the hand on its
own; Burnt Joker ignores it.

## 7. When the round ends

The round ends:

1. After a scored hand, when the round total is at or above the target
   (win), or when no hands are left (loss unless saved).
2. During hand selection, when the hand and the deck are both empty (loss
   unless saved, since a total at the target would already have ended it).
3. When a boss is disabled and the total already meets the (possibly halved)
   target (win).

Immediate loss without any save: when a draw is about to happen (outside an
Arcana or Spectral pack) while hand size is 0 or less and the hand is empty.

End-of-round start (about 0.2 s later, one step):

1. Loss is assumed unless round total >= target.
2. Every joker, left to right, gets the end-of-round event with the current
   loss flag, then its Rental charge ($3) and its Perishable countdown run.
   Mr. Bones, if not debuffed and the loss flag is set and round total /
   target >= 0.25, saves the run: the loss flag is cleared for the jokers
   after it, and Mr. Bones destroys itself. Only one Mr. Bones can save.
3. If this was a Boss blind and the ante equals the winning ante (8), the run
   is marked won.
4. If the loss flag is still set: game over. (Quirk: losing the ante 8 boss
   also sets the won mark, which only affects profile statistics.)

## 8. End of a won round, in order

1. Unused discards this run += discards left (Garbage Tag).
2. After a Boss round: the "most played hand" for The Ox is recomputed (ties:
   see `rules/blinds.md`).
3. If the run was just won, the win screen is queued for when the cash-out
   screen opens.
4. Held cards, left to right. For each card the game computes its end-of-round
   effect (jokers are also asked for held-card end-of-round effects; no
   base-game joker gives one); if the card has any effect, a Red Seal adds one
   repetition and each Mime (or Blueprint/Brainstorm copying Mime) adds one;
   each repetition computes the effect again:
   - Gold Card: +$3 per repetition.
   - Blue Seal: creates the Planet card of the last poker hand played this
     run, per repetition, only while consumable slots (counting planets
     already being created) are not full. No random draw.
   - A debuffed card gives nothing (boss debuffs are still active here).
5. All held cards go to the discard pile.
6. After a Boss round: the ante rises by 1 (challenge rules for that ante
   would apply here).
7. All discard-pile cards go back to the deck.
8. Then, in one step:
   - The state becomes the cash-out screen.
   - The blind's slot is marked Defeated. After a Boss round: the shop voucher
     for the new ante is drawn (stream `Voucher<new ante>`) and every card's
     "played this ante" mark is cleared.
   - Juggle Tag's extra hand size is removed; D6 Tag's free reroll base is
     removed.
   - The four "card of the round" picks are redrawn with the current ante:
     Idol card (stream `idol<ante>`), Mail-In Rebate rank (`mail<ante>`),
     Ancient Joker suit (`anc<ante>`, never the previous suit), Castle suit
     (`cas<ante>`). Each picks among non-Stone playing cards ordered by
     creation.
   - Every card's "discarded this round" and forced-selection marks are
     cleared.

## 9. The cash-out amount (computed when the cash-out screen opens)

Computed in one step, in this row order, and fixed from then on:

1. Blind: the blind's reward if round total >= target, else $0 (Mr. Bones
   save). Then the blind is visibly removed (its debuffs end shortly after).
2. Deck "eval" effect: Anaglyph Deck gains a Double Tag if the blind was a
   boss (no money).
3. Hands: if hands left > 0 (and no challenge forbids it): hands left x $1
   (Green Deck: x $2).
4. Discards: only for a deck that pays them (Green Deck): discards left x $1.
5. Jokers, left to right, skipping debuffed ones: Golden Joker $4; Cloud 9
   $1 per 9 in the full deck; Rocket its current payout (starts $1, +$2 per
   boss beaten, raised during end of round); Satellite $1 per distinct Planet
   used this run; Delayed Gratification $2 per discard left if no discard
   was used this round.
6. Tags: Investment Tag $25, only if the last blind was a boss.
7. Interest: if money >= $5 (and the deck or challenge allows interest):
   interest per $5 (1, +1 per To the Moon) x min(floor(money / 5),
   cap / 5), with cap $25 (Seed Money $50, Money Tree $100).
8. Total = sum of the rows.

"Money" in step 7 is the player's money at that moment: it already includes
everything earned or spent during the round, the Gold Card money from step 4
of section 8, Rental charges and other end-of-round effects, and it excludes
all rows of this table. Negative money earns nothing. Spending or earning on
the cash-out screen does not change the computed total.

Red Deck at White Stake, no jokers: total = 3/4/5/8 (Small/Big/Boss/
showdown) + hands left + (min(floor(money/5), 5) if money >= 5, else 0).

## 10. Cash Out (the button)

In order, all at the moment of the press:

1. Use is blocked.
2. The deck is shuffled with stream `cashout<ante>` (ante already raised after
   a boss). This order is what Arcana/Spectral packs in the shop and pack tags
   at the next blind select draw from; the next round reshuffles anyway.
3. Queued: jokers-bought-this-round counter = 0; discards left and hands left
   are reset to their base values for display; the state becomes the shop;
   Coupon and D6 tag flags for the shop are cleared.
4. The total is added to money; the round total of chips goes back to 0.
5. If the Boss slot is Defeated (a new ante begins): the preview ante is set
   to the ante; the Small tag then the Big tag are drawn (stream
   `Tag<ante>`); all three slots go back to Upcoming with Small on deck; a new
   boss is drawn (stream `boss`); the boss-rerolled flag is cleared.
6. The shop is then built (shop notes).

## 11. Ante, winning and endless

- The ante rises by 1 only during end-of-round processing of a won Boss
  round. Hieroglyph and Petroglyph lower it by 1 when redeemed.
- The run is won when a Boss round of ante 8 (the winning ante) is won, Mr.
  Bones saves included. The win screen appears with the cash-out screen; the
  player may start a new run or continue ("endless"). Continuing keeps the
  same flow: ante 9 onward, showdown bosses again at antes 16, 24, ...,
  targets from the formula in `rules/blinds.md`. No further wins are
  recorded; a later game over still ends the run.

## 12. What resets when

| Counter | Reset or change |
|---------|-----------------|
| Hands left | set at round start (base, at least 1) and at Cash Out; -1 per Play |
| Discards left | set at round start (base, at least 0) and at Cash Out; -1 per discard |
| Hands played this round, discards used | 0 at round start |
| Hands played this run | +1 per scored hand, never reset (Handy Tag) |
| Unused discards this run | += discards left at each won round (Garbage Tag) |
| Skips this run | +1 per skip (Throwback, Skip Tag) |
| Round counter | +1 when a blind is selected (not on skip) |
| Round chip total | back to 0 at Cash Out |
| Shop reroll cost | back to base at round start (so every shop starts at base); +1 per paid reroll; Chaos the Clown rerolls are free and do not raise it |
| Free rerolls | set to the number of Chaos the Clown at round start |
| Poker hand "played this round" | 0 at round start |
| "Played this ante" card marks | cleared when a Boss round is won |
| "Discarded" and forced marks | cleared at end of each won round (forced marks also on every play and discard) |
| Temporary hand size (Juggle) | removed at end of the won round |
| Boss rerolled flag | cleared at Cash Out after a Boss round |
| Idol / Mail-In / Ancient / Castle picks | redrawn at end of every won round |
| Shop voucher | redrawn at end of every won Boss round |

## 13. Random streams used by the round flow

Each named stream is independent, so only the order of draws within the same
stream matters (plus the game state each draw sees).

| Stream | Drawn when |
|--------|-----------|
| `boss` | run start, each Cash Out after a won Boss round, each boss reroll |
| `Tag<ante>` (+ `Tag<ante>_resample<k>`, k = 2, 3, ...) | run start (ante 1), each Cash Out after a won Boss round; Small then Big |
| `orbital` | three times per ante (Small, Big, Boss) when the blind panels are first built |
| `Voucher<ante>` (+ `_resample<k>`) | run start; end of each won Boss round (new ante) |
| `shuffle` | the deck shuffle at run start |
| `nr<ante>` | deck shuffle at every round start |
| `cashout<ante>` | deck shuffle at every Cash Out |
| `idol<ante>`, `mail<ante>`, `anc<ante>`, `cas<ante>` | run start (ante 1, in that order) and end of every won round |
| `hook`, `wheel`, `cerulean_bell`, `crimson_heart`, `aajk` | boss effects (`rules/blinds.md`) |
| `gros_michel`, `cavendish`, `to_do` | joker end-of-round effects (joker notes) |

Run start order (fresh run, tutorial finished, so no forced tutorial values):
stake modifiers, deck effects, `boss`, `Voucher1`, `Tag1` (Small), `Tag1`
(Big), the deck is built (Erratic Deck draws `erratic` per card), `shuffle`,
`idol1`, `mail1`, `anc1`, `cas1`; then the first blind-select screen draws
`orbital` three times.

## Where in the game

- `game.lua`: `Game:init_game_object`, `Game:start_run`, `Game:update`,
  `Game:update_selecting_hand`, `Game:update_hand_played`,
  `Game:update_draw_to_hand`, `Game:update_new_round`,
  `Game:update_blind_select`, `Game:update_round_eval`,
  `Game:update_game_over`.
- `functions/state_events.lua`: `win_game`, `end_round`, `new_round`,
  `G.FUNCS.draw_from_deck_to_hand`, `G.FUNCS.discard_cards_from_highlighted`,
  `G.FUNCS.play_cards_from_highlighted`, `G.FUNCS.evaluate_play`,
  `G.FUNCS.draw_from_play_to_discard`, `G.FUNCS.draw_from_discard_to_deck`,
  `G.FUNCS.draw_from_hand_to_discard`, `G.FUNCS.evaluate_round`.
- `functions/button_callbacks.lua`: `G.FUNCS.select_blind`,
  `G.FUNCS.cash_out`, `G.FUNCS.can_play`, `G.FUNCS.can_discard`,
  `G.FUNCS.sort_hand_suit`, `G.FUNCS.sort_hand_value`.
- `functions/common_events.lua`: `ease_dollars`, `ease_discard`,
  `ease_hands_played`, `ease_ante`, `ease_round`, `ease_chips`, `draw_card`,
  `add_round_eval_row`, `calculate_reroll_cost`, `reset_idol_card`,
  `reset_mail_rank`, `reset_ancient_card`, `reset_castle_card`,
  `reset_blinds`, `create_card`.
- `functions/misc_functions.lua`: `get_starting_params`, `stop_use`,
  `pseudoshuffle`.
- `cardarea.lua`: `CardArea:init`, `CardArea:emplace`, `CardArea:remove_card`,
  `CardArea:change_size`, `CardArea:add_to_highlighted`,
  `CardArea:remove_from_highlighted`, `CardArea:align_cards`,
  `CardArea:shuffle`, `CardArea:sort`, `CardArea:draw_card_from`.
- `card.lua`: `Card:init`, `Card:set_base`, `Card:get_nominal`,
  `Card:get_end_of_round_effect`, `Card:calculate_dollar_bonus`,
  `Card:calculate_rental`, `Card:calculate_perishable`, `Card:add_to_deck`,
  `Card:apply_to_run`, `Card:calculate_joker` (end-of-round and Mr. Bones
  branches).
- `blind.lua`: `Blind:set_blind`, `Blind:defeat`, `Blind:get_type`.
- `back.lua`: `Back:apply_to_run`, `Back:trigger_effect`.
- `tag.lua`: `Tag:apply_to_run`.
