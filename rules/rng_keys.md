# Random-stream keys

Every named random stream in vanilla Balatro 1.0.1o, how its key string is
built, what it decides, when it is drawn, and how lists are put into a fixed
order before a random pick or shuffle. A stream's state depends only on how
many times that one key has been drawn (`rules/rng.md`), so the engine must
build exactly the same key strings and draw each key exactly as often as the
game does, including draws whose result is thrown away. Keys are case-sensitive
strings built by plain concatenation; most end with the current ante, and
card-creation keys carry a short "append" naming the source (`sho` for the shop,
`buf` for Buffoon packs, ...). Before any pick or shuffle, lists of cards are
sorted by `sort_id` (creation order) and other lists by key; the one list whose
order comes from the Lua runtime itself (the poker-hand names behind the Orbital
Tag and To Do List) is worked out in section 2. A few game rules call
`math.random` without reseeding; none of them can change an outcome
(section 4). The seed string, its normalisation and the stream table are in
section 5. The game runs on LuaJIT 2.0.5, not 2.1 (section 6).

Notation: A = `G.GAME.round_resets.ante` written as a plain integer (`1`, `12`,
`0`, `-1`); P = the append given to card creation (may be empty); `+` means
string concatenation. "Draw" means one call that steps the stream once.

## 1. How keys are built

### Card creation (`create_card`)

`create_card(type, area, legendary, rarity, ..., soulable, forced key, P)`
draws, in this order and only where stated:

1. Soul check, only if no forced key, `soulable` is set and The Soul is not
   banned:
   - type Tarot, Spectral or Tarot_Planet, and no The Soul card currently exists
     (or Showman is held): draw `soul_` + type + A; above 0.997 the card becomes
     The Soul.
   - type Planet or Spectral, and no Black Hole currently exists (or Showman):
     draw `soul_` + type + A again; above 0.997 the card becomes Black Hole
     (this overrides a Soul from the first check).
   - So a Spectral-type creation draws `soul_Spectral` + A twice (once per
     check whose condition holds).
2. Type `Base` is forced to the plain card center.
3. If a key is forced (and not banned): no pool draw at all. Otherwise build the
   pool and pick from it:
   - Joker: rarity = the given rarity number, or else draw `rarity` + A + P.
     Above 0.95 Rare (3), above 0.7 Uncommon (2), else Common (1). A legendary
     request is rarity 4 but still draws `rarity` + A + P when no rarity number
     is given (the value is ignored). Pool key: `Joker` + rarity + P + A, or
     just `Joker4` for legendary (no append, no ante).
   - Other types: pool key = type + P + A (`Tarotsho1`, `Tarotar11`,
     `Spectralar21`, `Planetpl11`, `Enhancedsta1`).
   - Pick with the pool key. If the pick is `UNAVAILABLE`, pick again with
     pool key + `_resample` + n for n = 2, 3, 4, ... until available. Each
     resample key is its own stream, shared by every later pick with the same
     pool key.
   - A rarity number of 0 counts as given (no `rarity` draw).
4. If type is Base or Enhanced: draw the rank and suit from the 52 card keys
   with `front` + P + A.
5. The card object is created (`sort_id` taken). A To Do List draws `to_do`
   here.
6. Jokers only:
   - Created in the shop's card area or a pack: draw `etperpoll` + A (shop) or
     `packetper` + A (pack), always, at every stake. Eternal if eternals are
     enabled and the value is above 0.7; else Perishable if perishables are
     enabled and the value is above 0.4 and at most 0.7. Eternal needs the
     joker to allow it; Perishable needs it to allow it and not be Eternal.
   - Then, only if rentals are enabled (Gold Stake): draw `ssjr` + A (shop) or
     `packssjr` + A (pack); above 0.7 the joker is Rental.
   - Edition: draw `edi` + P + A (section 3.5).

### Other key families

| Family | Key |
|---|---|
| Shop voucher | `Voucher` + A; resamples `Voucher` + A + `_resample` + n |
| Voucher Tag | `Voucher_fromtag` (no ante); resamples `Voucher_fromtag_resample` + n |
| Skip tags | `Tag` + A; resamples `Tag` + A + `_resample` + n |
| Shop booster | `shop_pack` + A |
| Shop card type | `cdt` + A |
| Deck shuffles | `shuffle`, `nr` + A, `cashout` + A |
| Per-round targets | `idol` + A, `mail` + A, `anc` + A, `cas` + A |
| Standard pack | `stdset` + A, `standard_edition` + A, `stdseal` + A, `stdsealtype` + A |
| Hallucination | `halu` + A |
| Everything else | a fixed word (`boss`, `glass`, `wheel_of_fortune`, ...) |

Appends used in vanilla:

| P | Source |
|---|---|
| `sho` | shop card slots |
| `ar1` | Arcana pack, Tarot card |
| `ar2` | Arcana pack, Spectral card (Omen Globe) |
| `pl1` | Celestial pack |
| `spe` | Spectral pack |
| `sta` | Standard pack |
| `buf` | Buffoon pack |
| `top` | Top-up Tag (rarity 0: Common) |
| `rta` | Rare Tag (rarity 1: Rare) |
| `uta` | Uncommon Tag (rarity 0.9: Uncommon) |
| `rif` | Riff-raff (rarity 0: Common) |
| `wra` | Wraith (rarity 0.99: Rare) |
| `jud` | Judgement |
| `sou` | The Soul (legendary) |
| `emp` | The Emperor |
| `pri` | The High Priestess |
| `8ba` | 8 Ball and Purple Seal (they share it) |
| `hal` | Hallucination |
| `car` | Cartomancer |
| `vag` | Vagabond |
| `sup` | Superposition |
| `sea` | Seance |
| `sixth` | Sixth Sense |
| `fool` | The Fool (forced key: no draws) |
| `blusl` | Blue Seal (forced key: no draws) |
| `deck` | Magic and Ghost Deck starting cards (forced key: no draws) |

## 2. Canonical order before a pick or a shuffle

Every seeded pick or shuffle first reseeds the generator with the stream's
value (`rules/rng.md`), then orders the list, then draws.

- Shuffle: if the first element of the list has a `sort_id`, the whole list is
  sorted by `sort_id`, smallest first. Then the shuffle runs on the sorted list.
  Used on the deck, the jokers (Amber Acorn) and Immolate's copy of the hand.
- Pick: collect every (key, value) pair of the table. If the value of the first
  pair is a table with a `sort_id` (a card), sort the pairs by the values'
  `sort_id`; otherwise sort them by key with plain `<` (numbers numerically,
  strings byte by byte, as LuaJIT compares strings without locale). Then take
  pair number `math.random(count)` and return its value and its original key.
- `sort_id` values and keys are unique in every list the game uses, so the
  sort algorithm (not stable) never matters.

Consequences for each list the game picks from:

| List | Order used |
|---|---|
| Any list of cards (hand, jokers, consumables, all playing cards, eligible jokers) and every shuffled list | creation order (`sort_id`) |
| Pools (jokers, tarots, planets, spectrals, vouchers, tags, enhancements, boosters) | the pool's array order, which is center `order` ascending, with `UNAVAILABLE` placeholders kept in place |
| The 52 card keys (`front`, `erratic`, `cert_fr`, `marb_fr`) | sorted keys: `C_2 C_3 C_4 C_5 C_6 C_7 C_8 C_9 C_A C_J C_K C_Q C_T`, then `D_...`, `H_...`, `S_...` in the same rank order |
| Eligible bosses (`boss`) | blind keys sorted byte by byte (`bl_arm`, `bl_club`, `bl_eye`, `bl_fish`, ...) |
| Sigil and the create spectrals' suits | `S`, `H`, `D`, `C` |
| Ouija ranks | `2 3 4 5 6 7 8 9 T J Q K A` |
| Familiar ranks | `J Q K` |
| Incantation ranks | `2 3 4 5 6 7 8 9 T` |
| Ancient Joker suits | `Spades, Hearts, Clubs, Diamonds` without the current suit |
| Enhancements for Familiar, Grim, Incantation (`spe_card`) | Bonus, Mult, Wild, Glass, Steel, Gold, Lucky (Stone left out) |
| Legendaries (`Joker4`) | Legendary pool in `order` |
| Poker-hand names (`orbital`, `to_do`) | runtime table order, see below |

### The poker-hand list (Orbital Tag, To Do List)

These two picks build their list by walking the game's poker-hand table with
Lua's `pairs`, keeping only visible hands (To Do List also drops its current
hand). The pick then keeps that walk order. The walk order of a string-keyed
table is set by LuaJIT's internal hashing, so it is not alphabetical and not
the order in the source.

Computed walk order for the table as built at the start of a run under LuaJIT
2.0.5:

Flush House, Full House, Flush, Pair, High Card, Straight Flush, Straight,
Two Pair, Flush Five, Five of a Kind, Three of a Kind, Four of a Kind.

With only the 9 hands visible at the start of a run the list is:
Full House, Flush, Pair, High Card, Straight Flush, Straight, Two Pair,
Three of a Kind, Four of a Kind. A secret hand that becomes visible joins at its
place in the full order.

How this was obtained: by simulating LuaJIT 2.0.5's seedless string hash and
its table insertion (template table growing from 2 to 16 slots, collision
handling, free-slot search). The insertion simulation matched real LuaJIT 2.1
in 40 of 40 runs when fed 2.1's own slot numbers, and the hash formula matched
2.1's hash exactly once 2.1's random hash seed was recovered; a fixed-size table
gives the same order too. Confirmed: the game's own `lua51.dll` walks a table
built the way the game builds its hand table in exactly this order
(`results/rng/game_runtime.txt`, from `make rng-check` on the PC). Phase 3 still
checks Orbital Tag hands and a To Do List's first hand on known seeds. LuaJIT
2.1 cannot be used to produce this order: it randomises string placement per
process.
Continuing a saved run rebuilds the table in a different order (computed:
Flush House, Full House, Flush, Pair, Four of a Kind, Straight Flush, Straight,
Three of a Kind, Flush Five, Two Pair, Five of a Kind, High Card), so tests
against the real game must not continue saved runs.

## 3. Catalogue

Every seeded draw in the vanilla scripts. "Pick" = random element of a list,
"roll" = one number in [0, 1), "shuffle" = shuffle of a list. Thresholds use
`n` = `probabilities.normal` (1; each Oops! All 6s held doubles it).

### 3.1 Deck and rounds

| Key | Kind | Decides | When |
|---|---|---|---|
| `shuffle` | shuffle | deck order at run start | once per run, after the cards are made |
| `nr` + A | shuffle | deck order for a blind | when each blind starts (skipped blinds do not draw) |
| `cashout` + A | shuffle | deck order in the shop | each cash-out, A after any boss increase |
| `erratic` | pick of a card key | rank and suit of each Erratic Deck card | 52 draws at run start |
| `flipped_card` | roll | card drawn face down if below 1/value (not scaled by Oops! All 6s) | challenge rule "flipped cards" (value 4), every card drawn into the hand |

### 3.2 Blinds and bosses

| Key | Kind | Decides | When |
|---|---|---|---|
| `boss` | pick | next boss | run start; each cash-out after a boss; each boss reroll (Director's Cut, Retcon, Boss Tag) |
| `hook` | pick x2 | The Hook's two discarded cards | each hand played; second pick from the cards left after the first; fewer draws if the hand holds fewer than 2 cards |
| `wheel` | roll | The Wheel: card face down if below n/7 | every card drawn into the hand while The Wheel is active and not disabled (cards added straight to the hand by Cryptid, DNA, Certificate and the like do not roll) |
| `cerulean_bell` | pick | Cerulean Bell's forced card | after each draw to hand when no card is already forced |
| `crimson_heart` | pick | Crimson Heart's debuffed joker | after the opening draw of the round and after the refill that follows each played hand (not after discards), if jokers exist; first un-debuffs every joker, then picks among those that were not debuffed (all jokers if fewer than 2) |
| `aajk` | shuffle x3 | joker order under Amber Acorn | when Amber Acorn is set up with 2+ jokers; each shuffle re-sorts by `sort_id` first, so only the third decides the order, but all three draw |

The Hook, The Wheel, Cerulean Bell and Crimson Heart draw only while that boss
is the current blind and not disabled (Chicot, or selling Luchador, stops
further draws). Amber Acorn draws when it is set up, even if it is disabled
afterwards.

Boss choice: eligible bosses are the non-final bosses with minimum ante at most
max(1, A), when max(1, A) is not a multiple of the winning ante (8) or A < 2;
when A is a multiple of 8 and at least 2, only the final (showdown) bosses.
The maximum-ante field is ignored. Banned bosses are removed. Among the rest,
only those with the lowest use count are kept, then one is picked; its use count
goes up by 1 (a rerolled-away boss keeps its count).

### 3.3 Skip tags

| Key | Kind | Decides | When |
|---|---|---|---|
| `Tag` + A (+ resamples) | pick | skip tag of the Small Blind, then the Big Blind | run start; each cash-out after a boss |
| `orbital` | pick of a hand name | the hand an Orbital Tag levels, one per blind (Small, Big, Boss) of each ante | the first time the blind-select screen (or Run Info) is built for that ante: three draws, Small then Big then Boss, whether or not an Orbital Tag is offered; a Double Tag copy reuses the hand |

Tag pool (array order): Uncommon, Rare, Negative, Foil, Holographic,
Polychrome, Investment, Voucher, Boss, Standard, Charm, Meteor, Buffoon, Handy,
Garbage, Ethereal, Coupon, Double, Juggle, D6, Top-up, Skip, Orbital, Economy.
Unavailable: Negative, Standard, Meteor, Buffoon, Handy, Garbage, Ethereal,
Top-up and Orbital before ante 2; Rare unless Blueprint is discovered;
Negative, Foil, Holographic and Polychrome unless that edition is discovered.
Tags ignore the duplicate rule.

Tags that create things draw through other keys: Top-up (`top`, Common jokers
into the joker slots), Rare and Uncommon (`rta`, `uta`, into a shop slot, so
they also draw `etperpoll`), Voucher Tag (`Voucher_fromtag`), Boss Tag
(`boss`), Charm, Meteor, Ethereal, Standard and Buffoon Tags (the pack's keys,
section 3.6).

### 3.4 Shop

| Key | Kind | Decides | When |
|---|---|---|---|
| `cdt` + A | roll x total weight | card type of a shop slot: Joker, Tarot, Planet, playing card, Spectral, in that order of weight ranges (each range excludes its lower end) | every shop card slot filled (shop opening and rerolls), unless a Rare or Uncommon Tag fills it |
| `illusion` | roll | with Illusion: playing card is Enhanced if above 0.6, else Base | every shop slot filled while Illusion is redeemed (drawn even when the type is not a playing card) |
| `illusion` | roll | with Illusion: a shop playing card gets an edition if above 0.8 | only when a playing card was chosen |
| `illusion` | roll | that edition: Polychrome above 0.85, Holographic above 0.5, else Foil | only if the previous roll passed |
| `shop_pack` + A | roll x total weight | a booster slot | each booster slot when a shop opens: 2 per shop, except 1 in the run's first shop |
| `Voucher` + A (+ resamples) | pick | the voucher offered in every shop of the ante until it is bought | run start; end of each boss round |

Then the slot's card is made by `create_card` with append `sho`
(`rarity` + A + `sho`, `Joker` + R + `sho` + A, `etperpoll` + A, `ssjr` + A,
`edisho` + A, `Tarotsho` + A, `Planetsho` + A, `Spectralsho` + A,
`Enhancedsho` + A, `frontsho` + A). Shop cards never get The Soul.

Booster pick: walk the booster pool in order, adding weights; the first pack
whose cumulative range contains the roll is chosen (both ends inclusive). The
very first booster slot of the run skips the draw and is a Buffoon Pack
(section 4).

Voucher pool (array order): Overstock, Overstock Plus, Clearance Sale,
Liquidation, Hone, Glow Up, Reroll Surplus, Reroll Glut, Crystal Ball, Omen
Globe, Telescope, Observatory, Grabber, Nacho Tong, Wasteful, Recyclomancy,
Tarot Merchant, Tarot Tycoon, Planet Merchant, Planet Tycoon, Seed Money, Money
Tree, Blank, Antimatter, Magic Trick, Illusion, Hieroglyph, Petroglyph,
Director's Cut, Retcon, Paint Brush, Palette. Unavailable: redeemed vouchers,
vouchers whose prerequisite is not redeemed, vouchers locked in the profile
(every upgrade starts locked), and vouchers already on display in the shop.

### 3.5 Editions, seals and stickers

| Key | Kind | Decides | When |
|---|---|---|---|
| `edi` + P + A | roll | joker edition | every joker made by `create_card` |
| `standard_edition` + A | roll | Standard pack card edition (rate x2, no Negative) | each Standard pack card |
| `aura` | roll | Aura's edition (guaranteed, no Negative) | each use |
| `wheel_of_fortune` | roll | Wheel of Fortune's edition (guaranteed, no Negative) | after a successful roll and pick (same stream, section 3.8) |
| `stdseal` + A | roll | Standard pack card gets a seal if above 0.8 | each Standard pack card |
| `stdsealtype` + A | roll | that seal: Red above 0.75, Blue above 0.5, Gold above 0.25, else Purple | only if the previous roll passed |
| `certsl` | roll | Certificate's seal, same thresholds | each Certificate trigger |
| `etperpoll` + A / `packetper` + A | roll | Eternal or Perishable | every joker made in the shop or a pack (section 1) |
| `ssjr` + A / `packssjr` + A | roll | Rental | same jokers, only when rentals are enabled |

Edition thresholds for a roll x (edition rate r = 1, 2 with Hone, 4 with Glow
Up; m = 1, or 2 for Standard packs):

- Normal: Negative if x > 1 - 0.003m (not for Standard packs); else Polychrome
  if x > 1 - 0.006rm; else Holographic if x > 1 - 0.02rm; else Foil if
  x > 1 - 0.04rm; else none.
- Guaranteed (Aura, Wheel of Fortune): Polychrome if x > 0.85, Holographic if
  x > 0.5, else Foil.

The default keys `edition_generic` (edition) and `pack_generic` + A (booster)
are never used in vanilla.

### 3.6 Packs

Per card, in pack order (the pack's size: 3, 5, 2 or 4 cards):

| Pack | Draws per card |
|---|---|
| Arcana | with Omen Globe, roll `omen_globe` (above 0.8: Spectral card with append `ar2`); a Tarot draws `soul_Tarot` + A then `Tarotar1` + A; a Spectral draws `soul_Spectral` + A twice then `Spectralar2` + A |
| Celestial | `soul_Planet` + A, then `Planetpl1` + A; with Telescope the first card is the planet of the most played visible hand (forced: no draws; if no hand was played yet, normal draws) |
| Spectral | `soul_Spectral` + A twice, then `Spectralspe` + A |
| Standard | `stdset` + A (above 0.6: Enhanced, else plain); Enhanced: `Enhancedsta` + A; both: `frontsta` + A; then `standard_edition` + A, `stdseal` + A, maybe `stdsealtype` + A |
| Buffoon | `rarity` + A + `buf`, `Joker` + R + `buf` + A, `packetper` + A, (`packssjr` + A), `edibuf` + A |

A card that becomes The Soul or Black Hole skips its pool draw. Opening a pack
also lets Hallucination roll (section 3.7).

### 3.7 Jokers

| Key | Kind | Decides | When |
|---|---|---|---|
| `to_do` | pick of a hand name | To Do List's hand | whenever a To Do List card object is created (shop, pack, copies, collection screen) and at each round end for every To Do List held (excluding its current hand) |
| `gros_michel` | roll | Gros Michel dies if below n/6 | each round end |
| `cavendish` | roll | Cavendish dies if below n/1000 | each round end |
| `8ball` | roll | 8 Ball makes a Tarot if below n/4 | each scored 8, only if a consumable slot is free (counting pending ones) |
| `business` | roll | Business Card pays $2 if below n/2 | each scored face card |
| `bloodstone` | roll | Bloodstone x1.5 mult if below n/2 | each scored Heart |
| `parking` | roll | Reserved Parking pays $1 if below n/2 | each face card held in hand at scoring |
| `space` | roll | Space Joker levels the hand if below n/4 | each hand played |
| `misprint` | integer 0..23 | Misprint's mult | each hand played |
| `halu` + A | roll | Hallucination makes a Tarot if below n/2 | each booster opened, only if a consumable slot is free |
| `invisible` | pick | joker copied by Invisible Joker | when sold after its rounds; picks among the other jokers |
| `perkeo` | pick | consumable copied by Perkeo | leaving the shop, if any consumable is held |
| `madness` | pick | joker destroyed by Madness | each non-boss blind start; among jokers that are not Madness itself, not Eternal and not already being destroyed |
| `cert_fr` | pick of a card key | Certificate card's rank and suit | first hand drawn each round |
| `certsl` | roll | its seal | right after |
| `marb_fr` | pick of a card key | rank and suit under Marble Joker's Stone card | each blind start |

Jokers that make cards draw through `create_card`: Riff-raff (`rif`), 8 Ball
and Purple Seal (`8ba`), Hallucination (`hal`), Cartomancer (`car`), Vagabond
(`vag`), Superposition (`sup`), Seance (`sea`), Sixth Sense (`sixth`).

Blueprint and Brainstorm run the copied joker's code, so a copy draws the same
key again, right after the original in joker order. Exceptions that ignore
copies: Madness, To Do List, Gros Michel, Cavendish, Invisible Joker. Jokers are
evaluated left to right, so with two rolling jokers on one key (two 8 Balls, or
8 Ball and its Blueprint) the leftmost draws first.

### 3.8 Tarots

| Key | Kind | Decides | When |
|---|---|---|---|
| `wheel_of_fortune` | roll, then pick, then edition roll | success if below n/4; joker without an edition; its edition | the roll at use; the pick and edition only on success; all three on the same stream |
| `Tarotemp` + A | pool pick | The Emperor's Tarots | up to 2, limited by free slots |
| `Planetpri` + A | pool pick | The High Priestess's Planets | up to 2 |
| `rarity` + A + `jud`, `Joker` + R + `jud` + A, `edijud` + A | joker creation | Judgement's joker | each use |

The Fool uses a forced key (no draw). Other Tarots draw nothing.

### 3.9 Planets

Planet cards draw nothing when used. They are only drawn as pool picks
(`Planet` + P + A) and Black Hole only through `soul_Planet` or
`soul_Spectral`.

### 3.10 Spectrals

| Key | Kind | Decides | When |
|---|---|---|---|
| `random_destroy` | pick | the hand card destroyed by Familiar, Grim or Incantation | each use |
| `familiar_create` | pick x2 per card | rank (J, Q, K) then suit, for each of 3 cards | each Familiar |
| `grim_create` | pick | suit of each of 2 Aces | each Grim |
| `incantation_create` | pick x2 per card | rank (2-10) then suit, for each of 4 cards | each Incantation |
| `spe_card` | pick | enhancement of each card made by those three | one per new card, after its rank and suit |
| `immolate` | shuffle | Immolate destroys the first 5 cards of the shuffled hand | each use |
| `sigil` | pick | Sigil's suit | each use |
| `ouija` | pick | Ouija's rank | each use |
| `ankh_choice` | pick | the joker Ankh copies (from all jokers, Eternal included) | each use |
| `ectoplasm` | pick | joker made Negative (among jokers without an edition) | each use |
| `hex` | pick | joker made Polychrome (same list) | each use |
| `aura` | edition roll | Aura's edition | each use |
| `Joker3wra` + A, `ediwra` + A | joker creation | Wraith's Rare joker | each use (no `rarity` draw) |
| `rarity` + A + `sou` (ignored), `Joker4`, `edisou` + A | joker creation | The Soul's legendary | each use; a legendary already held is unavailable, so the pick may continue on `Joker4_resample2`, ... |

Cryptid, Deja Vu, Trance, Medium, Talisman and Black Hole draw nothing.

### 3.11 Playing cards

| Key | Kind | Decides | When |
|---|---|---|---|
| `lucky_mult` | roll | Lucky card +20 mult if below n/5 | every time a non-debuffed Lucky card's mult is scored (each retrigger) |
| `lucky_money` | roll | Lucky card +$20 if below n/15 | every time its money is scored (each retrigger) |
| `glass` | roll | Glass card shatters if below n/4 | after scoring, for each scoring non-debuffed Glass card, even if a joker already destroys it |

### 3.12 Per-round targets

| Key | Kind | Decides | When |
|---|---|---|---|
| `idol` + A | pick of a playing card | The Idol's rank and suit | run start and every round end; from all playing cards except Stone cards (default Ace of Spades if none) |
| `mail` + A | pick of a playing card | Mail-In Rebate's rank | same |
| `anc` + A | pick of a suit | Ancient Joker's suit, never the current one (at run start the current one is cleared, so all four) | same |
| `cas` + A | pick of a playing card | Castle's suit | same |

At a boss round end, A is already the new ante.

### 3.13 Decks and challenges

- `erratic`: Erratic Deck (section 3.1).
- `edition_deck`: picks playing cards for a deck that grants editions. No
  vanilla deck uses it.
- `flipped_card`: challenge rule (section 3.1).
- Challenge decks, jokers and vouchers are fixed; creating a To Do List draws
  `to_do`.

### 3.14 Seed prediction

- `Joker4` with the predict argument: the first legendary a seed's first The
  Soul would give, computed without touching the run (section 5).

### Keys that look random but are not drawn in vanilla

`front` with no append and no ante (playing card made without a rank),
`edition_generic`, `pack_generic` + A, `false_to_do` (only for To Do List cards
already sitting on the title screen), `edition_deck`.

## 4. `math.random` without reseeding

The game seeds `math.random` with the clock at launch and every seeded draw
reseeds it, so a call that does not reseed reads a state left by the last
seeded draw plus any number of cosmetic calls (particles, sounds, card wobble,
each card's resting spot on the discard pile). These calls are not
reproducible. The ones inside game rules:

| Where | What | Can it change an outcome? |
|---|---|---|
| A Stone card's rank id | a fresh number from -1000000 to -100 on every query | No. It never equals a real rank (2-14), is never a face or in a straight, and two Stone cards that happen to collide in pair detection are filed under a negative index that the pair search never reads |
| First shop's guaranteed Buffoon Pack | which of the two Buffoon Pack art variants | No: both have the same contents, cost and weight |
| Charm Tag, Meteor Tag | which of the two Mega Arcana or Mega Celestial art variants | No, same reason |
| Win and loss quips, menu cards, screen-wipe card | text and pictures | No |
| `pseudoseed('seed')` | returns an unseeded number | never called in vanilla |

The engine can ignore all of them (record the Buffoon Pack as one pack type).

## 5. The seed

- The run's seed is a string. A typed seed comes from an 8-character text box:
  letters are upper-cased, `0` becomes `O`, characters other than `1`-`9`,
  `A`-`Z` are ignored, extra characters beyond 8 are dropped, and a pasted seed
  goes through the same filter one character at a time. UNSURE: the start
  button does not check for an empty box, so a seeded run with an empty seed
  (`""`) looks possible.
- With no typed seed, a profile that has not finished the tutorial plays the
  seed `TUTORIAL` (with a scripted voucher, tags and shop). Otherwise the game
  generates 8 characters from an unseeded generator fed by the cursor position
  and time: each character is a digit `1`-`9` with probability 0.3, a letter
  `A`-`N` with 0.385, or a letter `P`-`Z` with 0.315 (uniform within each
  group). Generated seeds never contain `0` or `O`.
- At Gold Stake, unseeded runs keep generating seeds until the predicted first
  legendary (section 3.14) is one the profile has not yet won with at Gold
  Stake, but only while some, not all, legendaries have that win.
- A typed seed marks the run as seeded: no unlocks, discoveries, streaks or
  high-score uploads during it. Rules are otherwise identical.
- `G.GAME.pseudorandom` holds the seed string (`seed`), its hash
  (`hashed_seed`, recomputed whenever a run starts or is continued) and one
  entry per stream key holding that stream's current value. A stream's entry
  is created on its first draw from the hash of key + seed (`rules/rng.md`).
  The table is saved with the run; on continue, any entry equal to 0 is
  re-initialised from its key.
- The predict argument (only used for `Joker4` in Gold Stake seed generation):
  hash key + candidate seed, step it once, average with the candidate seed's
  hash. This equals the first draw of that key in a run with that seed.

## 6. Runtime notes for `rules/rng.md`

- The game's `lua51.dll` identifies itself as LuaJIT 2.0.5 (Windows, x64),
  linked against the Windows Universal C runtime. The repo's committed
  fixtures come from LuaJIT 2.1.
- Confirmed on the game's own `lua51.dll` (`make rng-check`, results in
  `results/rng/`): 2,326,488 reference lines (hashes, stream steps, rounding,
  `math.random` after reseeding, shuffles and picks) match the Rust engine value
  for value. So 2.0.5's `math.random` and `math.randomseed` are the same as
  2.1's.
- In 2.0.5, `string.format` hands `%.13f` to the C runtime, which rounds exact
  halves to even (confirmed: 2^-14 prints as `0.0000610351562`); LuaJIT 2.1
  rounds them away from zero. `tonumber` is LuaJIT's own exact parser. NaN
  prints as `nan` and infinity as `inf`.
- Table walk order differs between 2.0.5 and 2.1 (section 2); nothing else in
  the random streams depends on it.

## Where in the game

- `functions/misc_functions.lua`: `pseudohash`, `pseudoseed`, `pseudorandom`, `pseudorandom_element`, `pseudoshuffle`, `random_string`, `generate_starting_seed`, `get_first_legendary`, `get_starting_params`, `card_from_control`, `find_joker`, `get_X_same`, `get_straight`, `inc_career_stat`, `check_and_set_high_score`
- `functions/common_events.lua`: `create_card`, `get_current_pool`, `poll_edition`, `copy_card`, `create_playing_card`, `get_pack`, `get_next_voucher_key`, `get_next_tag_key`, `get_new_boss`, `reset_blinds`, `reset_idol_card`, `reset_mail_rank`, `reset_ancient_card`, `reset_castle_card`, `draw_card`, `add_joker`, `ease_ante`, `check_for_unlock`, `unlock_card`, `discover_card`
- `functions/state_events.lua`: `new_round`, `end_round`, `G.FUNCS.evaluate_play`, `G.FUNCS.draw_from_deck_to_hand`
- `functions/button_callbacks.lua`: `G.FUNCS.cash_out`, `G.FUNCS.reroll_boss`, `G.FUNCS.reroll_shop`, `G.FUNCS.skip_blind`, `G.FUNCS.text_input_key`, `G.FUNCS.paste_seed`, `G.FUNCS.start_setup_run`, `G.FUNCS.start_run`, `G.FUNCS.toggle_seeded_run`
- `functions/UI_definitions.lua`: `create_card_for_shop`, `create_UIBox_blind_select`, `create_UIBox_blind_choice`, `create_UIBox_blind_tag`, `create_text_input`, `create_UIBox_your_collection_jokers`
- `card.lua`: `Card:init`, `Card:set_ability`, `Card:set_eternal`, `Card:set_perishable`, `Card:get_id`, `Card:get_chip_mult`, `Card:get_p_dollars`, `Card:get_end_of_round_effect`, `Card:use_consumeable`, `Card:open`, `Card:calculate_seal`, `Card:calculate_joker`, `Card:update`, `Card:remove`
- `cardarea.lua`: `CardArea:shuffle`, `CardArea:draw_card_from`
- `blind.lua`: `Blind:set_blind`, `Blind:press_play`, `Blind:drawn_to_hand`, `Blind:stay_flipped`
- `tag.lua`: `Tag:init`, `Tag:set_ability`, `Tag:apply_to_run`
- `back.lua`: `Back:apply_to_run`
- `game.lua`: `Game:start_run`, `Game:init_game_object`, `Game:init_item_prototypes`, `Game:update_shop`, `Game:main_menu`, `Game:splash_screen`
- `main.lua`: startup seeding of `math.random`
- `globals.lua`: `G.SEED`
- `engine/string_packer.lua`: `STR_PACK`, `STR_UNPACK`
- `lua51.dll`: version string only
