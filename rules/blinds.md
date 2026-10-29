# Blinds, bosses and skip tags

Every ante has three blinds faced in a fixed order: Small, Big, then Boss. The
Small and Big blinds can each be skipped in exchange for the tag shown under
them; the Boss can never be skipped. Both skip tags of an ante are drawn
together when the ante begins (stream `Tag<ante>`), and the ante's boss is drawn
at the same moment from the single stream `boss`, using a pool limited by each
boss's minimum ante, by the showdown rule (antes 8, 16, 24, ...) and by a
least-used rule that makes bosses rotate before any repeats. A blind's chip
target is the ante's base amount (one of three stake tables, with a growth
formula past ante 8) times the blind's multiplier (Small x1, Big x1.5, bosses
x2 except The Wall x4, The Needle x1, Violet Vessel x6) times the deck's ante
scaling (Plasma Deck 2), with no rounding. Rewards are $3 / $4 / $5, or $8 for
a showdown boss; from Red Stake up the Small Blind pays nothing. The boss can be
rerolled for a flat $10 with Director's Cut (once per ante) or Retcon (no
limit), or for free by a Boss Tag. The last sections list every boss effect
with its numbers, its random stream and the moment it acts.

Terms used below: "ante" means the game's current ante counter at the moment
described; "stream `x`" means a named random stream (see `rules/rng.md`); "pick"
means: seed LuaJIT's generator from one draw of the stream, then take item
`random(n)` (1-based) of a list in the stated order.

## 1. The sequence inside an ante

1. Each blind slot (Small, Big, Boss) has a state: Upcoming, Select (the one
   on deck), Current (being played), Defeated or Skipped. A new ante starts
   with all three Upcoming and Small on deck.
2. The blind on deck is the first of Small, Big, Boss that is neither Defeated
   nor Skipped. Only the blind on deck can be selected or skipped; the other
   two buttons are inactive.
3. Selecting a blind plays a round (see `rules/round.md`). Winning a Small or
   Big round leads to the cash-out screen and a shop, then back to blind
   select with the next blind on deck. Winning the Boss round ends the ante.
4. Skipping (Small or Big only) marks that blind Skipped and puts the next one
   on deck immediately: no round, no reward, no interest, no shop, and the
   round counter does not advance.
5. The Boss slot has no tag and no skip button. It must be played.
6. Winning the Boss round increases the ante by 1 during end-of-round
   processing (before the cash-out screen). The next ante's tags and boss are
   drawn when the player presses Cash Out (see section 6 and section 3).

## 2. Skipping a blind: what happens, in order

Pressing Skip on the blind on deck (Small or Big):

1. Blocks other use for a moment (the game's "stop use" counter, see
   `rules/timing.md`) and locks input for 2.5 seconds of game time.
2. Adds 1 to the run's skip counter (used by Throwback and the Skip Tag).
3. Adds the tag shown under that blind to the player's tag list. Adding a tag
   first offers it to every tag already held; an unused Double Tag reacts by
   adding a copy of the new tag (Double Tag never copies a Double Tag).
4. Marks the skipped blind Skipped and the next blind (Small -> Big,
   Big -> Boss) as the one on deck.
5. Then, in one queued step: every joker is told a blind was skipped; the run
   is saved; every held tag gets the "immediate" check (in tag-list order);
   then the held tags get the "new blind choice" check, stopping at the first
   tag that acts (so at most one pack-opening or boss-reroll tag fires here;
   the rest fire later, see section 5).

What the player gains is only the tag (plus the skip count). What the player
gives up is that blind's reward, the end-of-round money (hands left, interest,
joker payouts, gold cards held), the shop visit, and any per-round joker growth.

## 3. How the two skip tags are drawn

When: at the start of a new run (for ante 1), and at Cash Out after the Boss
round is won (for the new ante). In both cases Small's tag is drawn first, then
Big's tag, and both are fixed for the whole ante (the Big tag is already
decided when the player looks at the Small blind). At run start the draws
happen right after the first boss and the first shop voucher are drawn; at
Cash Out they happen right after the deck's cash-out shuffle and right before
the next boss draw.

How, for each tag:

1. Build the tag list: all 24 tags in the fixed order below. A tag is
   replaced by the placeholder UNAVAILABLE if its minimum ante is above the
   current ante, if it requires a discovered item that the profile has not
   discovered, or if a challenge bans it.
2. Pick from the full 24-entry list (placeholders included) with stream
   `Tag<ante>`, for example `Tag1`, `Tag2`.
3. If the pick is UNAVAILABLE, pick again from the same list with stream
   `Tag<ante>_resample2`, then `Tag<ante>_resample3`, and so on, until a real
   tag comes up. Each resample key is its own stream and keeps advancing
   across later calls that reach it.
4. If every entry were unavailable, the result would be the Handy Tag (cannot
   happen in normal play).

| # | Tag | Min ante | Needs discovered |
|---|-----|----------|------------------|
| 1 | tag_uncommon | - | - |
| 2 | tag_rare | - | j_blueprint |
| 3 | tag_negative | 2 | e_negative |
| 4 | tag_foil | - | e_foil |
| 5 | tag_holo | - | e_holo |
| 6 | tag_polychrome | - | e_polychrome |
| 7 | tag_investment | - | - |
| 8 | tag_voucher | - | - |
| 9 | tag_boss | - | - |
| 10 | tag_standard | 2 | - |
| 11 | tag_charm | - | - |
| 12 | tag_meteor | 2 | - |
| 13 | tag_buffoon | 2 | - |
| 14 | tag_handy | 2 | - |
| 15 | tag_garbage | 2 | - |
| 16 | tag_ethereal | 2 | - |
| 17 | tag_coupon | - | - |
| 18 | tag_double | - | - |
| 19 | tag_juggle | - | - |
| 20 | tag_d_six | - | - |
| 21 | tag_top_up | 2 | - |
| 22 | tag_skip | - | - |
| 23 | tag_orbital | 2 | - |
| 24 | tag_economy | - | - |

So at ante 1 (with a fully discovered profile) 15 tags are possible and the
other 9 positions force resamples; from ante 2 on all 24 are possible.

UNSURE: the "needs discovered" test reads the player's profile collection. In
a seeded run the game never discovers anything new during the run, so the pool
is fixed by the profile for the whole run. Our engine should take the profile's
discovery set as an input (default: everything discovered) and the test runs
against BalatroBot must state which profile they used.

## 4. Orbital Tag hand choice (stream `orbital`)

Whenever a blind's panel is built for an ante that has no stored "orbital
hand" for that blind type, the game picks one and stores it per (ante, blind
type). The panels are built in the order Small, Big, Boss when the
blind-select screen first appears in an ante (the Blinds tab of the Run Info
screen builds them too, in the same order, if the player opens it earlier; no
hand can be played in between, so the result is the same). So stream
`orbital` is drawn three times per ante, Small then Big then Boss, even though
the Boss has no tag.

Each pick is from the list of currently visible poker hands. At the start of
a run the nine standard hands are visible; Five of a Kind, Flush House and
Flush Five become visible once played. A Tag created for the Small or Big panel
is an Orbital Tag only if the tag draw said so; it then levels up the stored
hand for its (ante, blind type) by 3 when it fires.

The list of visible hands is built by walking the game's hand table in Lua's
hash-iteration order, which is not alphabetical and not the display order. The
game ships LuaJIT 2.0.5 (`lua51.dll`, LOVE 11.5), not the 2.1 used in our cloud
tools. `rules/rng_keys.md` (section 2, "The poker-hand list") gives the walk
order computed for 2.0.5 for a fresh run: Flush House, Full House, Flush, Pair,
High Card, Straight Flush, Straight, Two Pair, Flush Five, Five of a Kind,
Three of a Kind, Four of a Kind (a continued saved run has a different order).
Confirmed on the game's own `lua51.dll`: a table built the way the game
builds its hand table walks in exactly this order (`results/rng/game_runtime.txt`,
`make rng-check`). Phase 3 still checks Orbital Tag hands on known seeds with
BalatroBot. The same walk decides The Ox's target on ties (see the boss table).

## 5. When held tags act (summary for the round flow)

Tags are checked at fixed points; each tag acts at most once and is then
removed. Details of each tag's effect belong to the tag notes.

| Check | When it happens | Tags that act there |
|-------|-----------------|---------------------|
| immediate | right after a skip; on entering the blind-select screen | Economy, Skip, Handy, Garbage, Top-up, Orbital |
| new blind choice (first one only) | after a skip; on entering blind select; after a boss reroll; when a pack closes | Charm, Meteor, Ethereal, Standard, Buffoon (open a free pack), Boss (free boss reroll) |
| round start bonus | each time the hand is about to be drawn | Juggle (+3 hand size for that round) |
| eval | when the cash-out amounts are computed | Investment ($25, only after a Boss round) |
| shop start / voucher add / final pass / joker create or modify | while the shop is built | D6, Voucher, Coupon, Uncommon, Rare, Foil, Holographic, Polychrome, Negative |
| tag added | whenever a new tag is gained | Double |

Boss Tag uses the same reroll as Director's Cut but costs nothing.

## 6. Boss selection

When a boss is drawn:

1. At the start of a new run, after the stake and deck are applied and before
   the first shop voucher and the two skip tags.
2. At Cash Out after the Boss round is won (ante already increased), after the
   two new skip tags.
3. On every boss reroll (Director's Cut, Retcon, Boss Tag).

Each draw uses the next value of the single stream `boss` (no ante in the
key), so the n-th boss decision of the run always uses the n-th value of that
stream, rerolls included.

How a boss is drawn, with A = current ante and W = winning ante (8):

1. Regular boss eligible: its minimum ante is at most max(1, A), and either
   max(1, A) is not a multiple of W or A is below 2.
2. Showdown boss eligible: A is a multiple of W and A is at least 2 (antes 8,
   16, 24, ...). At those antes only showdown bosses are eligible; at all other
   antes only regular bosses are.
3. The data also gives every boss a maximum ante of 10; the game never checks
   it. Ante 0 or below (after Hieroglyph or Petroglyph) behaves like ante 1.
4. Remove any boss banned by a challenge.
5. Usage rule: every boss has a use counter, all starting at 0. Among the
   eligible bosses, keep only those whose counter equals the smallest counter
   among the eligible ones.
6. Sort the remaining keys by plain byte-wise string order and pick one with
   stream `boss`.
7. Add 1 to the chosen boss's counter. A boss that is rerolled away keeps its
   increased counter (it counts as used).

Consequences: within each tier the bosses rotate without repeats until all
eligible ones have been used equally often; bosses that just became eligible
(counter 0) are always preferred over older ones; showdown bosses rotate among
themselves.

Byte-wise order of the regular boss keys (minimum ante in brackets):
`bl_arm` (2), `bl_club` (1), `bl_eye` (3), `bl_fish` (2), `bl_flint` (2),
`bl_goad` (1), `bl_head` (1), `bl_hook` (1), `bl_house` (2), `bl_manacle`
(1), `bl_mark` (2), `bl_mouth` (2), `bl_needle` (2), `bl_ox` (6), `bl_pillar`
(1), `bl_plant` (4), `bl_psychic` (1), `bl_serpent` (5), `bl_tooth` (3),
`bl_wall` (2), `bl_water` (2), `bl_wheel` (2), `bl_window` (1).

Showdown keys in order: `bl_final_acorn`, `bl_final_bell`, `bl_final_heart`,
`bl_final_leaf`, `bl_final_vessel`.

Example: the first boss of a run is a pick from the 8 minimum-ante-1 bosses in
the order club, goad, head, hook, manacle, pillar, psychic, window.

The boss is chosen with the ante as it is at that moment and is not redrawn if
the ante changes later. Example: after the ante 8 boss is beaten the ante
becomes 9 and Cash Out draws a regular boss; if Hieroglyph is then bought, the
run is back at ante 8 but keeps that regular boss (no second showdown).

## 7. Boss reroll

1. The Reroll Boss button is shown on the blind-select screen whenever
   Director's Cut or Retcon has been redeemed, whichever blind is on deck.
2. It is active only if the player's money minus the bankruptcy floor
   (normally 0; Credit Card makes it -20) is at least 10, and either Retcon is
   owned, or Director's Cut is owned and the boss has not been rerolled yet
   this ante.
3. Pressing it costs exactly $10 (the voucher discount does not apply), marks
   the boss as rerolled for this ante, draws a new boss as in section 6, and
   then re-runs the "new blind choice" tag check.
4. The rerolled flag clears only when a Boss round is won (at Cash Out).
5. A Boss Tag performs the same reroll for free, but it also sets the
   rerolled flag, so a Director's Cut cannot be used afterwards in that ante
   (Retcon still can).

## 8. Chip targets

Base amount by ante. Stakes pick the table: White and Red use scaling 1;
Green, Black and Blue use scaling 2; Purple, Orange and Gold use scaling 3.

| Ante | Scaling 1 | Scaling 2 | Scaling 3 |
|------|-----------|-----------|-----------|
| 0 or below | 100 | 100 | 100 |
| 1 | 300 | 300 | 300 |
| 2 | 800 | 900 | 1000 |
| 3 | 2000 | 2600 | 3200 |
| 4 | 5000 | 8000 | 9000 |
| 5 | 11000 | 20000 | 25000 |
| 6 | 20000 | 36000 | 60000 |
| 7 | 35000 | 60000 | 110000 |
| 8 | 50000 | 100000 | 200000 |

Past ante 8, with B = that table's ante 8 value, c = ante - 8 and
d = 1 + 0.2 c, all in 64-bit floats:

1. raw = floor(B * (1.6 + (0.75 c)^d)^c)
2. step = 10^(floor(log10(raw) - 1))
3. base = raw - (raw - floor(raw / step) * step), which keeps the two leading
   digits and zeroes the rest (Lua's `%` is a - floor(a/b)*b).

Resulting base amounts:

| Ante | Scaling 1 | Scaling 2 | Scaling 3 |
|------|-----------|-----------|-----------|
| 9 | 110000 | 230000 | 460000 |
| 10 | 560000 | 1100000 | 2200000 |
| 11 | 7200000 | 14000000 | 29000000 |
| 12 | 3.0e8 | 6.0e8 | 1.2e9 |
| 13 | 4.7e10 | 9.4e10 | 1.8e11 |
| 14 | 2.9e13 | 5.8e13 | 1.1e14 |
| 15 | 7.7e16 | 1.5e17 | 3.0e17 |
| 16 | 8.6e20 | 1.7e21 | 3.4e21 |

At ante 39 and above, step 1 overflows to infinity and step 3 then yields NaN
(infinity minus an undefined remainder), so the target is NaN and no score can
ever reach it (the "at or above the target" test is false). UNSURE: above about
ante 15 the numbers exceed 2^53 and the last bits depend on the C library's
`pow` used by the game's LuaJIT 2.0.5 on Windows; check against BalatroBot
before any study relies on very high antes.

Final target, set when the blind starts (and shown on the blind-select panel):

target = base(ante) * blind multiplier * deck ante scaling

- Blind multipliers: Small 1, Big 1.5, every boss 2 except The Wall 4,
  The Needle 1, Violet Vessel 6.
- Deck ante scaling: 1, except Plasma Deck 2.
- No rounding is applied. All products are whole numbers in practice (bases
  are multiples of 10 from ante 1 up).
- Disabling The Wall halves its target; disabling Violet Vessel divides it by
  3 (both become 2 x base).
- The blind-select panel computes its preview from a separate "blind ante"
  value that tracks the ante (set at each boss Cash Out, and lowered together
  with the ante by Hieroglyph/Petroglyph); the real target uses the ante.

Red Deck at White Stake, Small / Big / Boss (regular x2 boss):

| Ante | Small | Big | Boss |
|------|-------|-----|------|
| 1 | 300 | 450 | 600 |
| 2 | 800 | 1200 | 1600 |
| 3 | 2000 | 3000 | 4000 |
| 4 | 5000 | 7500 | 10000 |
| 5 | 11000 | 16500 | 22000 |
| 6 | 20000 | 30000 | 40000 |
| 7 | 35000 | 52500 | 70000 |
| 8 | 50000 | 75000 | 100000 |

## 9. Blind rewards

| Blind | Reward | Notes |
|-------|--------|-------|
| Small | $3 | $0 from Red Stake (stake 2) upward |
| Big | $4 | all stakes |
| Regular boss | $5 | all stakes, all 23 regular bosses |
| Showdown boss | $8 | all 5 showdown bosses |

The reward is paid at Cash Out only if the round was won by reaching the
target. A round saved by Mr. Bones pays $0 for the blind (the other cash-out
lines still pay). Challenges can remove rewards per blind type. Disabling a
boss does not change its reward. No other stake changes rewards.

## 10. Boss effects

Columns: minimum ante (from the data; showdown bosses appear only at antes 8,
16, ...), chip multiplier, reward, and the effect with the moment it acts.
Unless stated, an effect stops when the boss is disabled.

| Key | Name | Min | x | $ | Effect |
|-----|------|-----|---|---|--------|
| bl_arm | The Arm | 2 | 2 | 5 | When a hand is scored, if that hand type's level is above 1 it drops by 1 first, and the hand scores at the lowered level. Level 1 hands are untouched. |
| bl_club | The Club | 1 | 2 | 5 | Every card that counts as a Club is debuffed (Wild Cards always; with Smeared Joker, Spades too). Stone Cards never. |
| bl_eye | The Eye | 3 | 2 | 5 | Each poker hand type may score once per round; playing a type already played this round scores 0. |
| bl_fish | The Fish | 2 | 2 | 5 | Cards drawn right after a played hand arrive face down. The opening hand and draws after discards are face up. |
| bl_flint | The Flint | 2 | 2 | 5 | The hand's base chips and base mult (its level values, after "before" joker effects) are halved, rounding halves up; mult is at least 1. Card and joker bonuses are not halved. |
| bl_goad | The Goad | 1 | 2 | 5 | Every card that counts as a Spade is debuffed (rules as The Club). |
| bl_head | The Head | 1 | 2 | 5 | Every card that counts as a Heart is debuffed (rules as The Club). |
| bl_hook | The Hook | 1 | 2 | 5 | After each Play, before scoring: up to 2 random cards still in hand are discarded (one pick per card from stream `hook`, cards ordered by creation). This does not use a discard; discard-triggered seals and jokers still fire, except Burnt Joker. |
| bl_house | The House | 2 | 2 | 5 | Every card drawn while no hand has been played and no discard used this round is face down (the opening hand). |
| bl_manacle | The Manacle | 1 | 2 | 5 | Hand size -1 for the round (given back when the blind is beaten or disabled; disabling also draws 1 card). |
| bl_mark | The Mark | 2 | 2 | 5 | Face cards (all cards with Pareidolia) are drawn face down. |
| bl_mouth | The Mouth | 2 | 2 | 5 | The first hand type scored this round becomes the only allowed type; any other type scores 0. |
| bl_needle | The Needle | 2 | 1 | 5 | At blind start, hands left are reduced by (base hands per round - 1), normally leaving 1 hand. |
| bl_ox | The Ox | 6 | 2 | 5 | If the played hand type equals the stored "most played hand", money drops to exactly $0 at once (from any amount, negative included), before the hand is scored. See note below. |
| bl_pillar | The Pillar | 1 | 2 | 5 | At blind start, every card that was played earlier in this ante is debuffed. The "played this ante" mark is cleared when a Boss round is won. |
| bl_plant | The Plant | 4 | 2 | 5 | Face cards are debuffed (all cards with Pareidolia). |
| bl_psychic | The Psychic | 1 | 2 | 5 | A played hand of fewer than 5 cards scores 0 (it still uses the hand). |
| bl_serpent | The Serpent | 5 | 2 | 5 | Once a hand has been played or a discard used this round, every refill draws exactly min(3, cards in deck) cards, ignoring hand size. The opening draw is normal. |
| bl_tooth | The Tooth | 3 | 2 | 5 | After each Play, before scoring: money goes down by $1 for every card in the played hand (scoring or not). |
| bl_wall | The Wall | 2 | 4 | 5 | Target is 4 x base (2 x base if disabled). |
| bl_water | The Water | 2 | 2 | 5 | At blind start all remaining discards are removed (given back if disabled). |
| bl_wheel | The Wheel | 2 | 2 | 5 | Each card drawn into the hand rolls stream `wheel`; below (probability factor)/7, normally 1 in 7, it arrives face down. |
| bl_window | The Window | 1 | 2 | 5 | Every card that counts as a Diamond is debuffed (rules as The Club). |
| bl_final_acorn | Amber Acorn | showdown | 2 | 8 | At blind start all jokers are flipped face down; with 2 or more jokers they are then shuffled three times, each time from stream `aajk` (each shuffle first orders jokers by creation, so only the third result matters for the order, but all three advance the stream). Jokers turn face up when the blind is beaten or disabled. |
| bl_final_bell | Cerulean Bell | showdown | 2 | 8 | After every draw (start of round, after each play, after each discard), if no card in hand is forced, all hand cards are deselected and one random hand card (stream `cerulean_bell`, ordered by creation) is selected and cannot be deselected. Forcing is cleared on every play and discard. |
| bl_final_heart | Crimson Heart | showdown | 2 | 8 | After the opening draw and after the draw that follows each played hand (not after discards): candidates are the jokers not debuffed right now (all jokers if there is only one); every joker is un-debuffed (expired Perishables stay debuffed); one candidate (stream `crimson_heart`, ordered by creation) is debuffed. So the same joker is never chosen twice in a row when there are 2 or more. The debuff lasts through end-of-round processing, so that joker's end-of-round effect and cash-out money are lost. |
| bl_final_leaf | Verdant Leaf | showdown | 2 | 8 | From blind start, every playing card carries a debuff; the first joker sale during the blind disables the boss and lifts it. |
| bl_final_vessel | Violet Vessel | showdown | 6 | 8 | Target is 6 x base (2 x base if disabled). |

Notes on the table:

- Debuffs from bosses are applied to all playing cards when the blind starts,
  and again to any card whose suit or enhancement changes during the blind
  (and to cards created during it). A debuffed card held in hand at the end of
  the round also gives no Gold Card money and no Blue Seal planet, because the
  boss is only removed after end-of-round processing.
- The Ox target: the stored "most played hand" is updated only when a Boss
  round is won, so during ante N it reflects play up to the end of the
  previous ante's boss round (it starts as High Card). Ties: the game walks
  the hand table in hash order and keeps the last hand it meets whose count
  is at least the best so far, so among tied hands the last one in that order
  wins (the game also compares against a hand-order value, but that value is
  never updated, so the comparison always passes). With the walk order of
  section 4, a tie is won by the hand that comes later in: Flush House, Full
  House, Flush, Pair, High Card, Straight Flush, Straight, Two Pair, Flush
  Five, Five of a Kind, Three of a Kind, Four of a Kind (for example Pair tied
  with Two Pair gives Two Pair; High Card tied with Flush gives High Card).
  All 12 hands are walked here, visible or not. The order is confirmed on the
  game's runtime (section 4).
- The Hook's two picks: the first happens if at least 1 card is in hand, the
  second if at least 2 are. Each pick draws `hook` once and chooses among the
  remaining hand cards ordered by creation.
- The Tooth's money loss and The Hook's discard both finish before the played
  hand is scored (so Bull and Bootstraps see the reduced money, and held-card
  effects no longer see the hooked cards).
- Face-down cards play and score normally; they only hide information.
- When a showdown boss is beaten at ante 8, the run is won (see
  `rules/round.md`).

## 11. Disabling and defeating a boss

Ways to disable: Chicot (when a boss blind starts, or when Chicot is gained
during a boss blind), selling Luchador during a boss blind, and selling any
joker during Verdant Leaf. Disabling:

1. Turns face-down jokers face up.
2. The Water gives the removed discards back; The Needle gives the removed
   hands back; The Manacle gives +1 hand size and draws 1 card.
3. The Wheel, The House, The Mark, The Fish: face-down cards in hand turn
   face up.
4. The Wall's target is halved; Violet Vessel's target is divided by 3.
5. Cerulean Bell's forced selection is cleared.
6. All debuffs are recomputed (boss debuffs disappear).
7. If the current chips already meet the new target, the round ends at once
   as a win.

When the blind is beaten, at the cash-out screen: jokers turn face up, The
Manacle's hand size comes back (if not disabled), and the blind is cleared,
which removes its card debuffs.

## 12. The "triggered" flag

The blind keeps a flag saying its ability hit the current hand; Matador pays
$8 when it is set at the moment Matador is evaluated during scoring (or when
the hand is zeroed). The flag changes in this order for each played hand:

1. Cleared when Play is pressed.
2. Set by the play effect of The Hook, The Tooth, and Crimson Heart (if any
   joker is held).
3. At the start of scoring, the blind's hand check clears it again for every
   blind (every blind carries a debuff table, even an empty one), then sets it
   if The Psychic, The Eye or The Mouth zeroes the hand, if The Arm lowers a
   level, or if The Ox takes the money.
4. Set by The Flint when it halves the base.
5. Set whenever a debuffed card is in the scoring hand (any blind, including
   Small and Big if a card is debuffed for another reason).

So the flag set in step 2 never survives to scoring: The Hook, The Tooth and
Crimson Heart do not pay Matador by themselves. UNSURE: confirm in-game with
BalatroBot, since players may expect otherwise.

## Where in the game

- `game.lua`: `Game:init_item_prototypes` (blind and tag tables, stakes),
  `Game:init_game_object`, `Game:start_run`, `Game:update_blind_select`.
- `blind.lua`: `Blind:set_blind`, `Blind:get_type`, `Blind:press_play`,
  `Blind:modify_hand`, `Blind:debuff_hand`, `Blind:drawn_to_hand`,
  `Blind:stay_flipped`, `Blind:debuff_card`, `Blind:disable`, `Blind:defeat`,
  `Blind:alert_debuff`.
- `functions/common_events.lua`: `get_new_boss`, `reset_blinds`,
  `get_next_tag_key`, `get_next_voucher_key`, `get_current_pool`.
- `functions/misc_functions.lua`: `get_blind_amount`, `pseudorandom_element`,
  `pseudoshuffle`, `pseudoseed`.
- `functions/button_callbacks.lua`: `G.FUNCS.skip_blind`,
  `G.FUNCS.select_blind`, `G.FUNCS.reroll_boss_button`, `G.FUNCS.reroll_boss`,
  `G.FUNCS.blind_choice_handler`, `G.FUNCS.cash_out`.
- `functions/UI_definitions.lua`: `create_UIBox_blind_select`,
  `create_UIBox_blind_choice`, `create_UIBox_blind_tag`, `add_tag`,
  `G.UIDEF.current_blinds`.
- `tag.lua`: `Tag:init`, `Tag:set_ability`, `Tag:apply_to_run`.
- `card.lua`: `Card:is_suit`, `Card:is_face`, `Card:set_debuff`,
  `Card:sell_card`, `Card:add_to_deck`, `Card:apply_to_run`,
  `Card:calculate_joker` (Chicot, Luchador, Matador branches).
- `functions/state_events.lua`: `end_round`, `G.FUNCS.draw_from_deck_to_hand`,
  `G.FUNCS.evaluate_play`.
- `back.lua`: `Back:apply_to_run`.
- `localization/en-us.lua`: blind descriptions (read for meaning only).
