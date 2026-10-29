# Shop, booster packs, vouchers, card creation and prices

A shop opens after every blind you play and win. It holds a joker row (2 slots,
plus 1 for each Overstock voucher), one voucher slot (the ante's voucher, plus
one for each Voucher Tag) and two booster slots. Each joker-row slot first
rolls its kind from stream `cdt<ante>`. The weights are Joker 20, Tarot 4,
Planet 4, playing card 0 and Spectral 0, and vouchers and decks change them.
The slot then calls the general card creator. That creator (for jokers) rolls a
rarity and builds the item's candidate list in a fixed order. Every candidate
that is locked, banned, gated or already on the table as a card becomes an
`UNAVAILABLE` placeholder that keeps its position. The creator picks from a
stream named after the set, a source tag (`sho`, `buf`, `ar1`, ...) and the
ante, and if it lands on a placeholder it picks again from `..._resample2`,
`..._resample3` and so on. A joker then always uses up one sticker roll (even
at White Stake) and one edition roll. Booster slots draw packs by weight from
`shop_pack<ante>`, except that the first pack slot of the run's first shop is
always a plain Buffoon Pack. The ante's voucher is drawn from `Voucher<ante>`
when the ante starts and stays in every shop of that ante until bought.
A price is `max(1, floor((base + edition surcharge + 0.5) * (100 - discount) / 100))`
and a sell value is `max(1, floor(price / 2))` plus any bonus the card has built
up. A reroll costs $5 plus $1 for each paid reroll in the same shop. The
low-level generator (stream stepping, LuaJIT's PRNG, how a pick turns into an
index) is described in `rules/rng.md`. This file says which streams are used,
in what order and over which lists.

---

## 0. Conventions used below

- **`key<ante>`** means the key followed directly by the current ante number
  in decimal, with no separator (`cdt` in ante 3 is `cdt3`). The ante is the
  run's current ante at the moment of the draw. It is already the new ante in
  the shop after a boss, and it can be 0 or negative after Hieroglyph or
  Petroglyph (`cdt0`, `cdt-1`).
- **Roll from S**: one value u in [0, 1) from stream S (step S, reseed, one
  `math.random()`; see rng.md).
- **Pick from S over list L**: reseed from S once, then take entry
  `floor(u * #L) + 1`. For a pool list the order is exactly the list order
  given in section 2.7, with placeholders kept in place.
- **Pick with resample**: pick from S. While the result is `UNAVAILABLE`, pick
  again over the same list from `S_resample2`, then `S_resample3`, and so on.
  The counter starts at 2. Every resample key is its own stream that lasts the
  whole run, so a second resample of the same key in the same ante uses that
  stream's next value.
- Streams are independent, so only the order of draws within one stream
  matters. The interleaving of different streams does not. The only unseeded
  `math.random` calls in this area pick cosmetic art variants (noted where they
  occur) and never affect outcomes.
- Item keys keep the game's internal identifiers: `j_` jokers, `c_`
  consumables, `v_` vouchers, `p_` packs, `m_` enhancements. One spelling
  quirk is `c_heirophant`.

---

## 1. The shop

### 1.1 When it happens and what it holds

- A shop opens after the cash-out of every blind that was played and won. A
  skipped blind gives no shop. Just before the shop, at cash-out, the deck is
  shuffled with stream `cashout<ante>`. After a boss, the next ante's tags and
  boss are also chosen at cash-out (round-flow notes). The ante has already
  gone up, so the shop after a boss uses the new ante in every key.
- **Joker row**: `joker_max` slots. It starts at 2. Overstock adds 1 and
  Overstock Plus adds 1 (most 4). Zodiac Deck starts with Overstock, so 3.
- **Voucher slot**: 1, plus 1 for each Voucher Tag that fires in this shop.
- **Booster slots**: always 2.
- The shop is destroyed when you leave. Every unbought card in it is removed,
  which matters for duplicate prevention (2.4).

### 1.2 Build order when the shop opens

1. Tags of type "shop start" fire, in the order the tags were obtained. Only
   the D6 Tag has this type, and only one D6 Tag can fire per shop. It changes
   reroll pricing (5.5) and uses no RNG.
2. **Joker row**: slots are built one at a time, left to right, for slots 1 to
   `joker_max`, each as in 1.3.
3. **Voucher slot**: if the ante's voucher (section 4) has not been bought,
   it is placed here. No RNG.
4. **Booster slots 1 then 2**: each picks a pack as in 3.2. The two choices
   are stored for this shop. Opening a pack marks its slot used. The store is
   cleared at the start of every round, so each shop gets two fresh packs.
5. **Voucher Tags**: each held Voucher Tag adds one extra voucher (4.5).
6. **Coupon Tag**: the first held Coupon Tag (one per shop) makes every card
   now in the joker row and both booster slots cost $0. Vouchers are not
   affected, and cards that arrive later from rerolls are not free.

### 1.3 Building one joker-row slot

**Step A: tag override.** Walk the held tags in the order obtained. The first
Uncommon Tag, or Rare Tag that succeeds, supplies this slot's card (1.5) and
the slot is finished. Step B is skipped, so `cdt` is not rolled. A failing
Rare Tag is used up and the walk continues to the next tag. If no tag
supplies a card, continue.

**Step B: kind roll.**
1. `total = joker_rate + tarot_rate + planet_rate + playing_card_rate + spectral_rate`,
   added in that order as 64-bit floats.
2. `x = roll(cdt<ante>) * total`.
3. Only if Illusion has been redeemed, roll once from `illusion` (no ante in
   the key). This roll is made every time Step B runs, whatever kind is chosen,
   and comes after `cdt`. If it is above 0.6 the playing-card kind is
   *Enhanced*, otherwise *Base*. Without Illusion the playing-card kind is
   always *Base* and `illusion` is not touched.
4. Walk the kinds in the order Joker, Tarot, Planet, playing card, Spectral
   with a running sum, and take the first kind whose interval
   `(sum_before, sum_before + weight]` contains x. A kind with weight 0 is
   never chosen.

**Step C: create.** Call the general creator (section 2) with the chosen
kind, destination = shop row, no Soul check, and source tag `sho`.

**Step D: Illusion edition (playing cards only).** If the kind is Base or
Enhanced and Illusion is redeemed, roll `illusion`. If it is above 0.8, roll
`illusion` again: above 0.85 gives polychrome, above 0.5 holographic, and
anything else foil. That is a 20% edition chance, split 15/35/50 among
polychrome/holo/foil. Hone and Glow Up do not change this. Shop playing cards
never get a seal in this version.

**Step E: edition tags.** A queued step lets a held Foil, Holographic,
Polychrome or Negative Tag apply to this card if it qualifies (1.5).

**Weights** (`joker, tarot, planet, playing card, spectral`):

| Situation | Weights |
|---|---|
| Default (Red Deck, any stake) | 20, 4, 4, 0, 0 |
| Tarot Merchant / Tarot Tycoon | tarot weight set to `4 * (9.6/4)` = 9.6 / `4 * (32/4)` = 32 |
| Planet Merchant / Planet Tycoon | planet weight 9.6 / 32 (same formulas) |
| Magic Trick or Illusion | playing-card weight 4 |
| Ghost Deck | spectral weight 2 |
| Challenge rule "no shop jokers" | joker weight 0 |

Compute the merchant weights exactly as `4 * (9.6 / 4)` and `4 * (32 / 4)` in
f64 so the totals match bit for bit.

Resulting kind chances (for orientation): default 71.4% Joker, 14.3% Tarot,
14.3% Planet. With Magic Trick: 62.5% Joker and 12.5% each for the other
three. With Tarot Merchant: 59.5% / 28.6% / 11.9%. With Tarot Tycoon:
35.7% / 57.1% / 7.1%. Ghost Deck: 66.7% Joker, 13.3% Tarot, 13.3% Planet,
6.7% Spectral. Zodiac Deck (both merchants): 51.0% / 24.5% / 24.5%.

### 1.4 Rerolls and mid-shop restocks

- A reroll replaces only the joker row. Every card in the row is removed
  first, which frees their names for the pools (2.4), so a rerolled-away item
  can come straight back. Then `joker_max` slots are rebuilt as in 1.3, and
  tags can still fire.
- Vouchers and boosters are never rerolled.
- After the reroll, held jokers get a "shop rerolled" event. Flash Card uses
  it, with no RNG.
- **Overstock bought mid-shop**: `joker_max` goes up by 1. Then the row is
  filled up to `joker_max`, building (`joker_max` minus the cards still in the
  row) new slots as in 1.3. Slots emptied by purchases are refilled too.
- Buying or selling anything else never restocks the shop.

### 1.5 Tags that act in the shop

Tags are walked in the order they were obtained.

- **Uncommon Tag**: makes the slot's card an uncommon joker. The rarity is
  fixed, so there is no rarity roll. Source tag `uta`, destination shop row,
  so the sticker roll applies (2.2 step 7). It costs $0.
- **Rare Tag**: counts the distinct rare jokers in your joker slots. If that
  count is below 20 (the size of the rare list, locked ones included), it
  makes a rare joker with source `rta`, costing $0. Otherwise it fails, is used
  up, and uses no RNG.
- **Foil / Holographic / Polychrome / Negative Tag**: applies to the first
  shop joker created from now on (initial build, rerolls, Overstock refills,
  tag jokers, later shops; never pack jokers or Top-up Tag jokers) that is a
  Joker with no edition. Only one such tag goes on a card: the
  first in tag order. The card gets that edition, its price is recomputed, and
  it costs $0. No RNG. A tag joker from Uncommon or Rare Tag gets this check
  straight away. A normal slot gets it in a queued step right after the slot is
  built.
- **D6 Tag**: rerolls in this shop start at $0 (5.5). One per shop.
- **Coupon Tag**: see 1.2 step 6. One per shop.
- **Voucher Tag**: see 4.5. Every held Voucher Tag fires.
- **Pack tags** (Charm: Mega Arcana; Meteor: Mega Celestial; Ethereal: normal
  Spectral; Standard: Mega Standard; Buffoon: Mega Buffoon) open a free pack
  at blind select, not in the shop. The contents follow section 3 with the
  same keys. Charm and Meteor choose between art variants `_1` and `_2` with
  an unseeded `math.random`. This is cosmetic only.
- Every tag's full rules, check moments and streams: `rules/vouchers_tags.md`
  section 2.

### 1.6 Every stream used while a shop is built, in order

Initial build. For each joker-row slot, left to right:

- Supplied by an Uncommon or Rare Tag: the joker sequence J below with source
  `uta` or `rta`. No `cdt`.
- Otherwise:
  1. `cdt<ante>`
  2. `illusion` (only if Illusion is redeemed)
  3. depending on the kind:
     - **Joker**: sequence J with source `sho`.
     - **Tarot**: pick with resample from `Tarotsho<ante>`.
     - **Planet**: pick with resample from `Planetsho<ante>`.
     - **Spectral**: pick with resample from `Spectralsho<ante>`.
     - **Base**: pick from `frontsho<ante>` over the 52 cards.
     - **Enhanced**: pick with resample from `Enhancedsho<ante>`, then
       `frontsho<ante>`.
  4. Base or Enhanced with Illusion: `illusion`, then `illusion` again if the
     first value was above 0.8.

**Joker sequence J** (source `s`, shop row):

1. `rarity<ante><s>`. Note that the ante comes before the source here, as in
   `rarity1sho`. This roll is skipped when the rarity is fixed (`uta`, `rta`).
2. Pick with resample from `Joker<r><s><ante>`, as in `Joker2sho1` and
   `Joker2sho1_resample2`.
3. `etperpoll<ante>`, at every stake.
4. `ssjr<ante>`, only when rentals are on (Gold Stake).
5. `edi<s><ante>`, as in `edisho1`.
6. If the card is To Do List, its creation also picks a target hand from
   `to_do`. The jokers notes cover this, and see the UNSURE list.

Booster slot 1: no draw if this is the run's first-ever shop pack (forced
Buffoon Pack, 3.2). Otherwise roll `shop_pack<ante>`.
Booster slot 2: roll `shop_pack<ante>`.
Each Voucher Tag: pick with resample from `Voucher_fromtag`.

A reroll, or the refill after Overstock, runs the same per-slot sequence for
each slot it builds.

Example: the first shop of a run, ante 1, White Stake, no vouchers or tags.
Slot 1 is `cdt1`. If it is a joker: `rarity1sho`, `Joker1sho1` (plus any
resamples), `etperpoll1`, `edisho1`. Slot 2 is `cdt1` (second value); if it is
a Tarot: `Tarotsho1`. Packs: slot 1 is a forced Buffoon Pack with no draw,
and slot 2 is `shop_pack1` (first value).

---

## 2. Card creation (the general creator)

Almost every new joker, consumable and shop or pack playing card comes from
one routine. Copies (Invisible Joker, Ankh, Perkeo, DNA, Cryptid) and some
playing-card makers (Familiar, Grim, Incantation, Certificate) use other code
and are covered in their own notes.

### 2.1 Inputs

- kind: Joker, Tarot, Planet, Spectral, Base, Enhanced, or Tarot_Planet
  (only used with a forced key)
- destination: shop row, pack, joker slots or consumable slots
- legendary flag
- fixed rarity value (a number standing in for the rarity roll)
- Soul-check flag (true only for pack contents)
- forced key
- source tag (the "append" string)

### 2.2 Steps, in order

1. **Soul check.** This only happens when there is no forced key, the
   Soul-check flag is on, and The Soul is not banned.
   - (a) If the kind is Tarot, Spectral or Tarot_Planet, and no card of The
     Soul currently exists (2.4), roll `soul_<kind><ante>`, as in
     `soul_Tarot1`. Above 0.997 forces The Soul. A non-debuffed Showman
     ignores the "exists" condition.
   - (b) If the kind is Planet or Spectral, and no Black Hole exists, roll
     `soul_<kind><ante>`. Above 0.997 forces Black Hole, overriding (a).
   - For Spectral both (a) and (b) draw from the same stream
     `soul_Spectral<ante>`, so it can advance twice per card.
   - The source tag is not in these keys, so every pack of that kind shares
     the stream.
   - A check whose card already exists is skipped completely. The stream does
     not advance.
2. **Base kind**: forced to the plain card `c_base`.
3. **Forced key** (if set and not banned): use it. Skip the pool and its
   stream. The card's kind becomes the forced card's set, so The Soul from an
   Arcana pack is a Spectral card.
4. **Otherwise**: build the pool (2.3) and pick with resample from its key.
5. **Base or Enhanced only**: pick the rank and suit from
   `front<source><ante>` (for example `frontsho2`, `frontsta1`) over the 52
   standard cards in the order given in 2.7.
6. **Make the card.** This marks the card's name as present at once (2.4). It
   has no RNG of its own, except To Do List picking its hand.
7. **Jokers only**:
   - (a) Challenge "all eternal": eternal.
   - (b) Only if the destination is the shop row or a pack:
     - Roll `etperpoll<ante>` (shop) or `packetper<ante>` (pack). This roll is
       always made, at every stake.
     - If eternals are on (stake 4 or higher) and the roll is above 0.7: the
       card becomes eternal if the joker allows it. If it doesn't, nothing
       happens; it does not fall through to perishable.
     - Else, if perishables are on (stake 7 or higher) and the roll is in
       (0.4, 0.7]: the card becomes perishable if the joker allows it, with 5
       rounds left.
     - Then, only if rentals are on (stake 8): roll `ssjr<ante>` (shop) or
       `packssjr<ante>` (pack). Above 0.7 makes the card rental. There is no
       compatibility check for rental.
   - (c) **Edition**, for every joker from this creator whatever the
     destination (shop, pack, Judgement, Riff-raff, Wraith, The Soul,
     Top-up Tag, Rare and Uncommon Tags). Roll `edi<source><ante>`. Let r be
     the edition rate: 1 normally, 2 after Hone, 4 after Glow Up. Glow Up sets
     the rate to 4; it does not multiply.
     - negative if u > 0.997
     - else polychrome if u > 1 - 0.006·r
     - else holographic if u > 1 - 0.02·r
     - else foil if u > 1 - 0.04·r
     - else no edition

     | rate | negative | polychrome | holographic | foil | none |
     |---|---|---|---|---|---|
     | 1 | 0.3% | 0.3% | 1.4% | 2.0% | 96.0% |
     | 2 (Hone) | 0.3% | 0.9% | 2.8% | 4.0% | 92.0% |
     | 4 (Glow Up) | 0.3% | 2.1% | 5.6% | 8.0% | 84.0% |

     The negative chance never scales. Consumables and playing cards from this
     creator never roll an edition. Packs and Illusion handle playing-card
     editions themselves.

Joker stickers by compatibility (relevant from Black Stake up):

- Cannot be eternal: `gros_michel`, `ice_cream`, `cavendish`, `luchador`,
  `turtle_bean`, `diet_cola`, `popcorn`, `ramen`, `selzer`, `mr_bones`,
  `invisible`.
- Cannot be perishable: `ceremonial`, `ride_the_bus`, `runner`,
  `constellation`, `green_joker`, `red_card`, `madness`, `square`, `vampire`,
  `hologram`, `rocket`, `obelisk`, `lucky_cat`, `flash`, `trousers`, `castle`,
  `glass`, `wee`.
- A rental card costs $1 to buy, and $3 is charged per round (rental rate 3,
  economy notes).

### 2.3 Building a pool

**Jokers.**

- The rarity value is the fixed value if one is given (no roll). Otherwise it
  is a roll from `rarity<ante><source>`.
- With the legendary flag the rarity is 4. The rarity roll is still made if no
  fixed value was given. The Soul's joker uses up `rarity<ante>sou` and
  ignores it.
- Mapping: value above 0.95 gives Rare (3), above 0.7 gives Uncommon (2),
  otherwise Common (1). That is 70% / 25% / 5%.
- The list is every joker of that rarity in pool order (2.7).
- The key is `Joker<r><source><ante>`. For a legendary it is just `Joker4`,
  with no source and no ante.

**Other sets.** The list is every item of the set in pool order. The key is
`<Set><source><ante>`: `Tarotsho3`, `Planetpl12` (source `pl1` then ante 2),
`Spectralspe1`, `Enhancedsta4`, `Voucher5` (vouchers have no source).

Each candidate either stays as its key or becomes `UNAVAILABLE`, keeping its
position. The checks are:

- **Enhanced**: always available, apart from banned keys.
- **All other sets**:
  1. Unavailable while its name is present (2.4), unless a non-debuffed
     Showman is held in your joker or consumable slots.
  2. Unavailable if locked in the player's profile. Legendary jokers are
     exempt. Discovery does not matter: undiscovered items appear normally.
     See 2.7 for which items start locked. For research runs assume a fully
     unlocked profile.
  3. **Vouchers**: unavailable if already redeemed this run, if any
     prerequisite has not been redeemed, or if the same voucher is in the
     shop's voucher slot right now.
  4. **Planets**: Planet X, Ceres and Eris are available only if their poker
     hand (Five of a Kind, Flush House, Flush Five) has been played at least
     once this run.
  5. **Jokers that need an enhancement** (Steel Joker: Steel; Stone Joker:
     Stone; Lucky Cat: Lucky; Golden Ticket: Gold; Glass Joker: Glass) are
     available only if at least one card in your whole deck (deck, hand and
     discard) has that enhancement. Cards in the shop or in a pack do not
     count.
  6. The Soul and Black Hole are always unavailable here. They only come
     from 2.2 step 1.
- **Pool flags** (all sets): Gros Michel becomes unavailable once a Gros
  Michel has died this run. Cavendish is available only after that.
- **Banned keys** (challenges): unavailable.

There is no minimum ante for any shop item. Only tags have one.

**Fallback.** If every entry is unavailable, the list becomes one item: Joker
gives `j_joker`, Tarot gives `c_strength`, Planet gives `c_pluto`, Spectral
gives `c_incantation`, Voucher gives `v_blank`. The pick is still drawn from
the pool key, using one value.

### 2.4 "Present" names (duplicate prevention)

- Whenever a card object is created for an item, its name is marked present.
  This covers shop row, voucher slot, booster slot, pack contents, creation
  straight into your slots, and copies. Marking is by name across all items
  with that name; only booster packs share names.
- When a card object is removed, its mark is cleared unless a card with the
  same name is still in your joker or consumable slots (debuffed ones
  count). Removal happens when the card is sold, used, destroyed, rerolled
  away, left in the shop when you leave, or not chosen when a pack closes.
- What follows from this:
  - No duplicates within one shop build, and none within one pack: each new
    card sees the earlier ones.
  - Items in the current shop (joker row and voucher slot) cannot appear in a
    pack opened from that shop.
  - Held jokers and consumables never appear again while held.
  - Rerolled-away and unbought items can come back.
  - A used consumable becomes available again.
  - With Showman, anything except redeemed or currently shown vouchers can
    repeat.
- Forced-key creations skip the check but still mark themselves present.
  These are Telescope's planet, the Blue Seal planet, The Fool's copy,
  Magic/Ghost Deck starting cards, and The Soul and Black Hole. So the Magic
  Deck's two Fools keep The Fool out of every pool while held.
- At most one The Soul and one Black Hole per pack, and none while one is
  held or shown, because the Soul check is skipped while the card exists.
- Playing cards are never excluded, so repeated ranks, suits and
  enhancements are normal.
- Timing: a sold, used or destroyed card is removed after its dissolve
  animation, but the event system finishes that removal before any later
  player action is processed. The engine can treat it as immediate, with
  one exception: a consumable is still present while its own effects run,
  so The Emperor never creates The Emperor and a used Negative card's extra
  slot still counts during its own effect (`consumables.md`).

### 2.5 Where the creator is called

| Source tag | Caller | Kind | Rarity input | Soul check | Destination (sticker rolls?) |
|---|---|---|---|---|---|
| `sho` | shop row slot | any | rolled | no | shop row (yes) |
| `uta` | Uncommon Tag | Joker | fixed Uncommon | no | shop row (yes) |
| `rta` | Rare Tag | Joker | fixed Rare | no | shop row (yes) |
| `buf` | Buffoon pack | Joker | rolled | n/a | pack (yes) |
| `ar1` | Arcana pack | Tarot | n/a | yes | pack |
| `ar2` | Arcana pack card turned Spectral (Omen Globe) | Spectral | n/a | yes | pack |
| `pl1` | Celestial pack | Planet | n/a | yes, except the Telescope card | pack |
| `spe` | Spectral pack | Spectral | n/a | yes | pack |
| `sta` | Standard pack | Base / Enhanced | n/a | n/a | pack |
| `jud` | Judgement | Joker | rolled | n/a | joker slots (no) |
| `sou` | The Soul | Joker, legendary | roll made, then ignored | n/a | joker slots (no) |
| `wra` | Wraith | Joker | fixed Rare | n/a | joker slots (no) |
| `rif` | Riff-raff | Joker | fixed Common | n/a | joker slots (no) |
| `top` | Top-up Tag | Joker | fixed Common | n/a | joker slots (no) |
| `emp` | The Emperor | Tarot | n/a | no | consumables |
| `pri` | The High Priestess | Planet | n/a | no | consumables |
| `8ba` | 8 Ball, and Purple Seal | Tarot | n/a | no | consumables |
| `hal` | Hallucination | Tarot | n/a | no | consumables |
| `car` | Cartomancer | Tarot | n/a | no | consumables |
| `vag` | Vagabond | Tarot | n/a | no | consumables |
| `sup` | Superposition | Tarot | n/a | no | consumables |
| `sixth` | Sixth Sense | Spectral | n/a | no | consumables |
| `sea` | Séance | Spectral | n/a | no | consumables |
| `blusl` | Blue Seal | Planet, forced | n/a | no | consumables |
| `fool` | The Fool | forced copy | n/a | no | consumables |
| `deck` | Magic or Ghost Deck start | forced | n/a | no | consumables |

The fixed rarity values are 0 (Common), 0.9 (Uncommon), 1 (Rare from Rare
Tag) and 0.99 (Rare from Wraith). With a fixed value no rarity stream is
touched.

Resulting keys, for example: Judgement in ante 4 uses `rarity4jud`, then
`Joker<r>jud4`, then `edijud4`. Wraith uses `Joker3wra<ante>` then
`ediwra<ante>`. The Soul uses `rarity<ante>sou`, `Joker4` and `edisou<ante>`.
The Emperor uses `Tarotemp<ante>`. Purple Seal and 8 Ball share
`Tarot8ba<ante>`.

### 2.6 Shop-time joker reactions that use RNG (for completeness)

- **Hallucination**, when any pack is opened and there is consumable room:
  roll `halu<ante>`. Below (probabilities.normal) / 2 creates a Tarot with
  source `hal`. See 3.6 for timing.
- **Perkeo**, when leaving the shop with at least one consumable: picks one
  of your consumables from stream `perkeo` (no ante) and adds a negative copy.
- Flash Card (reroll), Red Card (skipped pack), Campfire (sale) and
  Hologram (playing card added) use no RNG here.

### 2.7 List orders (pool order = sorted by each item's internal order number)

**Jokers.** Keys are listed without the `j_` prefix, with base cost. `*`
marks items locked in a fresh profile (unlocked through profile
achievements). Legendaries ignore locks.

- **Common (61)**: 1 joker 2, 2 greedy_joker 5, 3 lusty_joker 5,
  4 wrathful_joker 5, 5 gluttenous_joker 5, 6 jolly 3, 7 zany 4, 8 mad 4,
  9 crazy 4, 10 droll 4, 11 sly 3, 12 wily 4, 13 clever 4, 14 devious 4,
  15 crafty 4, 16 half 5, 17 credit_card 1, 18 banner 5, 19 mystic_summit 5,
  20 8_ball 5, 21 misprint 4, 22 raised_fist 5, 23 chaos 4, 24 scary_face 4,
  25 abstract 4, 26 delayed_grat 4, 27 gros_michel 5, 28 even_steven 4,
  29 odd_todd 4, 30 scholar 4, 31 business 4, 32 supernova 5,
  33 ride_the_bus 6, 34 egg 4, 35 runner 5, 36 ice_cream 5, 37 splash 3,
  38 blue_joker 5, 39 faceless 4, 40 green_joker 4, 41 superposition 4,
  42 todo_list 4, 43 cavendish 4, 44 red_card 5, 45 square 4, 46 riff_raff 6,
  47 photograph 5, 48 reserved_parking 6, 49 mail 4, 50 hallucination 4,
  51 fortune_teller 6, 52 juggler 4, 53 drunkard 4, 54 golden 6,
  55 popcorn 5, 56 walkie_talkie 4, 57 smiley 4, 58 ticket 5*,
  59 swashbuckler 4*, 60 hanging_chad 4*, 61 shoot_the_moon 5*.
- **Uncommon (64)**: 1 stencil 8, 2 four_fingers 7, 3 mime 5,
  4 ceremonial 6, 5 marble 6, 6 loyalty_card 5, 7 dusk 5, 8 fibonacci 8,
  9 steel_joker 7, 10 hack 6, 11 pareidolia 5, 12 space 5, 13 burglar 6,
  14 blackboard 6, 15 sixth_sense 6, 16 constellation 6, 17 hiker 5,
  18 card_sharp 6, 19 madness 7, 20 seance 6, 21 vampire 7, 22 shortcut 7,
  23 hologram 7, 24 cloud_9 7, 25 rocket 6, 26 midas_mask 7, 27 luchador 5,
  28 gift 6, 29 turtle_bean 6, 30 erosion 6, 31 to_the_moon 5, 32 stone 6,
  33 lucky_cat 6, 34 bull 6, 35 diet_cola 6, 36 trading 6, 37 flash 5,
  38 trousers 6, 39 ramen 6, 40 selzer 6, 41 castle 6, 42 mr_bones 5*,
  43 acrobat 6*, 44 sock_and_buskin 6*, 45 troubadour 6*, 46 certificate 6*,
  47 smeared 7*, 48 throwback 6*, 49 rough_gem 7*, 50 bloodstone 7*,
  51 arrowhead 7*, 52 onyx_agate 7*, 53 glass 6*, 54 ring_master 5*
  (Showman), 55 flower_pot 6*, 56 merry_andy 7*, 57 oops 4*, 58 idol 6*,
  59 seeing_double 6*, 60 matador 7*, 61 satellite 6*, 62 cartomancer 6*,
  63 astronomer 8*, 64 bootstraps 7*.
- **Rare (20)**: 1 dna 8, 2 vagabond 8, 3 baron 8, 4 obelisk 8,
  5 baseball 8, 6 ancient 8, 7 campfire 9, 8 blueprint 10*, 9 wee 8*,
  10 hit_the_road 8*, 11 duo 8*, 12 trio 8*, 13 family 8*, 14 order 8*,
  15 tribe 8*, 16 stuntman 7*, 17 invisible 8*, 18 brainstorm 10*,
  19 drivers_license 7*, 20 burnt 8*.
- **Legendary (5)**: 1 caino, 2 triboulet, 3 yorick, 4 chicot, 5 perkeo.
  All cost 20.

**Tarot (22, all $3)**: fool, magician, high_priestess, empress, emperor,
heirophant, lovers, chariot, justice, hermit, wheel_of_fortune, strength,
hanged_man, death, temperance, devil, tower, star, moon, sun, judgement,
world.

**Planet (12, all $3)**: mercury (Pair), venus (Three of a Kind), earth
(Full House), mars (Four of a Kind), jupiter (Flush), saturn (Straight),
uranus (Two Pair), neptune (Straight Flush), pluto (High Card), planet_x
(Five of a Kind, needs the hand played), ceres (Flush House, needs the hand
played), eris (Flush Five, needs the hand played).

**Spectral (18, all $4)**: familiar, grim, incantation, talisman, aura,
wraith, sigil, ouija, ectoplasm, immolate, ankh, deja_vu, hex, trance,
medium, cryptid, soul (always a placeholder), black_hole (always a
placeholder). A Spectral pick therefore resamples about 2 times in 18.

**Enhanced (8)**: bonus, mult, wild, glass, steel, stone, gold, lucky.

**Standard cards (52)**, picked from `front...` streams. The order is the
card codes (`<suit>_<rank>`, suits C, D, H, S; ranks 2 to 9, T, J, Q, K, A)
sorted byte-wise:
C_2, C_3, C_4, C_5, C_6, C_7, C_8, C_9, C_A, C_J, C_K, C_Q, C_T, then the
same 13 for D, then H, then S. So the rank order inside a suit is
2 3 4 5 6 7 8 9 A J K Q 10. Every deck draws from all 52, including faces in
Abandoned Deck.

**Vouchers** and **Boosters**: see sections 4.2 and 3.1.

---

## 3. Booster packs

### 3.1 The packs (pool order)

| # | Key | Kind | Weight | Cards shown | Choose | Cost |
|---|---|---|---|---|---|---|
| 1–4 | p_arcana_normal_1..4 | Arcana | 1 each | 3 | 1 | 4 |
| 5–6 | p_arcana_jumbo_1..2 | Arcana | 1 each | 5 | 1 | 6 |
| 7–8 | p_arcana_mega_1..2 | Arcana | 0.25 each | 5 | 2 | 8 |
| 9–12 | p_celestial_normal_1..4 | Celestial | 1 each | 3 | 1 | 4 |
| 13–14 | p_celestial_jumbo_1..2 | Celestial | 1 each | 5 | 1 | 6 |
| 15–16 | p_celestial_mega_1..2 | Celestial | 0.25 each | 5 | 2 | 8 |
| 17–20 | p_standard_normal_1..4 | Standard | 1 each | 3 | 1 | 4 |
| 21–22 | p_standard_jumbo_1..2 | Standard | 1 each | 5 | 1 | 6 |
| 23–24 | p_standard_mega_1..2 | Standard | 0.25 each | 5 | 2 | 8 |
| 25–26 | p_buffoon_normal_1..2 | Buffoon | 0.6 each | 2 | 1 | 4 |
| 27 | p_buffoon_jumbo_1 | Buffoon | 0.6 | 4 | 1 | 6 |
| 28 | p_buffoon_mega_1 | Buffoon | 0.15 | 4 | 2 | 8 |
| 29–30 | p_spectral_normal_1..2 | Spectral | 0.3 each | 2 | 1 | 4 |
| 31 | p_spectral_jumbo_1 | Spectral | 0.3 | 4 | 1 | 6 |
| 32 | p_spectral_mega_1 | Spectral | 0.07 | 4 | 2 | 8 |

Numbered variants of the same pack differ only in art. Total weight is
22.42. By kind: Arcana, Celestial and Standard 6.5 each (29.0%), Buffoon 1.95
(8.7%), Spectral 0.97 (4.3%). Within a kind, mega packs are 7.7% of Arcana,
Celestial and Standard, and 7.7% of Buffoon.

### 3.2 Choosing a shop pack (per booster slot)

1. **First pack of the run.** The first time the run ever picks a shop pack
   (booster slot 1 of the first shop visited), it is a normal Buffoon Pack:
   2 cards, choose 1, $4. No seeded draw is made, so `shop_pack<ante>` is not
   advanced. Variant `_1` or `_2` is chosen with an unseeded `math.random`;
   this is cosmetic. This rule is skipped if `p_buffoon_normal_1` is banned
   (challenge). It applies once per run, whatever the ante.
2. **Otherwise**: `x = roll(shop_pack<ante>) * W`, where W is the sum of the
   weights of all non-banned packs, added in pool order as f64. Walk the
   packs in pool order, skipping banned ones, keeping a running sum of
   weights, and take the first pack where both of these hold in f64:
   running sum ≥ x, and (running sum − that pack's weight) ≤ x. The second
   test is normally implied by the first. It only matters when float
   rounding of the subtraction lands just above x, in which case that pack
   is passed over. Reproduce both tests literally.

Two packs per shop. Rerolls never change them. Each new shop picks two new
ones.

### 3.3 Opening a pack

1. Pay the pack's price (nothing if it costs $0).
2. The pack's slot is marked used. The pack size and number of choices come
   from the table.
3. After a short animation, held jokers react to "pack opened" in order.
   Hallucination may roll `halu<ante>` (2.6).
4. The Inflation challenge adds +1 to all prices per pack opened (5.1).
5. The contents are generated (3.4). For Arcana and Spectral packs, a hand is
   also dealt (3.5).

### 3.4 Contents, card by card (cards 1 to n in order)

- **Arcana**, each card:
  1. With Omen Globe: roll `omen_globe` (no ante). Above 0.8 makes this card
     come from the Spectral path (20%).
  2. Tarot path: create a Tarot with source `ar1`. If The Soul does not
     exist, roll `soul_Tarot<ante>` (The Soul if above 0.997). If it was not
     forced, pick with resample from `Tarotar1<ante>` (for example
     `Tarotar11` in ante 1).
  3. Spectral path: create a Spectral with source `ar2`. If The Soul does not
     exist, roll `soul_Spectral<ante>`. If Black Hole does not exist, roll
     `soul_Spectral<ante>` again. If neither was forced, pick with resample
     from `Spectralar2<ante>`.
- **Celestial**, each card:
  1. If Telescope is redeemed and this is card 1: find your most played
     visible poker hand this run. Walk the hands from strongest to weakest
     (Flush Five, Flush House, Five of a Kind, Straight Flush, Four of a Kind,
     Full House, Flush, Straight, Three of a Kind, Two Pair, Pair, High Card)
     and keep a hand only if its play count is strictly greater than the best
     so far, starting from 0. Ties therefore go to the stronger hand. If such
     a hand exists, card 1 is that hand's planet, forced: no Soul roll, no
     pool pick. If no hand has been played, card 1 is made normally.
  2. Otherwise: if Black Hole does not exist, roll `soul_Planet<ante>`. If
     not forced, pick with resample from `Planetpl1<ante>`.
- **Spectral**, each card: roll `soul_Spectral<ante>` for The Soul (if it
  does not exist), then again for Black Hole (if it does not exist). Black
  Hole wins if both hit. If neither was forced, pick with resample from
  `Spectralspe<ante>`.
- **Standard**, each card:
  1. Roll `stdset<ante>`. Above 0.6 means Enhanced (40%), otherwise Base.
  2. Enhanced: pick from `Enhancedsta<ante>` (never unavailable).
  3. Pick the rank and suit from `frontsta<ante>`.
  4. Edition: roll `standard_edition<ante>`. The thresholds are the joker
     ones with the multiplier doubled and negative impossible. With rate r:
     polychrome if u > 1 - 0.012·r, holographic if u > 1 - 0.04·r, foil if
     u > 1 - 0.08·r. At rate 1 that is 1.2% / 2.8% / 4.0% (8% any). With
     Hone it is 2.4% / 5.6% / 8% (16%). With Glow Up it is 4.8% / 11.2% / 16%
     (32%).
  5. Seal: roll `stdseal<ante>`. Above 0.8 (20%) adds a seal. Then roll
     `stdsealtype<ante>`: above 0.75 Red, above 0.5 Blue, above 0.25 Gold,
     otherwise Purple (25% each).
- **Buffoon**, each card: joker sequence J with source `buf`:
  `rarity<ante>buf`, `Joker<r>buf<ante>` (with resamples),
  `packetper<ante>`, `packssjr<ante>` (Gold Stake only), `edibuf<ante>`.

Within a pack each card sees the earlier ones (2.4), so there are no repeats
without Showman. Tarots, planets and spectrals in packs never get editions.

### 3.5 The hand dealt by Arcana and Spectral packs

- When an Arcana or Spectral pack opens (from the shop or from a tag), cards
  are drawn into your hand. The number is the smaller of the cards left in
  the deck and (hand size minus cards already in hand).
- They come off the top of the current deck, which is the end of the deck
  list. In the shop that is the order left by the `cashout<ante>` shuffle.
  The hand keeps your chosen sort, rank descending by default (round-flow
  notes).
- Celestial, Standard and Buffoon packs deal no hand.
- When the pack closes, every card in hand goes back to the deck. Each is
  taken from the front of the hand and put at the bottom of the deck (the
  start of the list). The returned cards end up at the bottom in reverse hand
  order, and the rest of the deck's order is unchanged. A second Arcana or
  Spectral pack in the same shop therefore shows the next cards from the top.
  This order only matters until the next shuffle: a shuffle first sorts the
  deck by card creation order (rng.md), so earlier arrangement does not carry
  over.

### 3.6 Choosing, finishing and timing

- **Joker**: needs a free joker slot unless it is negative. It goes to your
  joker slots.
- **Consumable**: used at once. It must be usable right now (for example,
  needing selected hand cards). It is never kept.
- **Playing card**: added to the bottom of the deck (no room needed). Jokers
  see a "playing card added" event.
- **Mega packs**: two picks, one after the other.
- **Skip**: allowed at any time. Red Card reacts to it. For Arcana and
  Spectral the hand must have been dealt.
- **Closing**: unchosen cards are discarded, which frees their names.
- **Hallucination timing**: the `halu<ante>` roll happens when the pack
  opens, before the contents are made. The Tarot it creates appears after
  the pack's contents are generated (event timing: the contents appear about
  1.3 time units after opening, while the Tarot waits behind the opening
  animation, about 1.95 units). So Hallucination's Tarot is picked from a
  pool that already excludes the pack's cards, and the pack's cards are not
  blocked by it. UNSURE: worked out from the event-queue rules, not confirmed
  in play.
- **Tag packs** open at blind select with price $0. When a pack closes, the
  next "new blind choice" tag fires, which chains pack tags.

---

## 4. Vouchers

Each voucher's effect in full, the redeem steps and the Voucher Tag are in
`rules/vouchers_tags.md` section 1.

### 4.1 When the ante's voucher is chosen

- At the start of the run, after the deck's own vouchers are applied, pick
  from `Voucher1`.
- After each boss is beaten: the ante goes up first, then the next voucher is
  picked from `Voucher<new ante>`. This happens before the round-eval screen,
  so before the shop.
- The chosen voucher is offered in every shop of that ante: after the boss,
  after the Small Blind and after the Big Blind, until it is bought.
- Hieroglyph and Petroglyph lower the ante, so a `Voucher<n>` stream can be
  used twice in a run.

### 4.2 Pool (order, cost $10 each, prerequisites)

Odd numbers are tier 1 (unlocked from the start). Even numbers are tier 2:
locked in a fresh profile, and in a run they need the tier-1 voucher above
them already redeemed.

| # | Key | # | Key (needs) |
|---|---|---|---|
| 1 | v_overstock_norm | 2 | v_overstock_plus (overstock_norm) |
| 3 | v_clearance_sale | 4 | v_liquidation (clearance_sale) |
| 5 | v_hone | 6 | v_glow_up (hone) |
| 7 | v_reroll_surplus | 8 | v_reroll_glut (reroll_surplus) |
| 9 | v_crystal_ball | 10 | v_omen_globe (crystal_ball) |
| 11 | v_telescope | 12 | v_observatory (telescope) |
| 13 | v_grabber | 14 | v_nacho_tong (grabber) |
| 15 | v_wasteful | 16 | v_recyclomancy (wasteful) |
| 17 | v_tarot_merchant | 18 | v_tarot_tycoon (tarot_merchant) |
| 19 | v_planet_merchant | 20 | v_planet_tycoon (planet_merchant) |
| 21 | v_seed_money | 22 | v_money_tree (seed_money) |
| 23 | v_blank | 24 | v_antimatter (blank) |
| 25 | v_magic_trick | 26 | v_illusion (magic_trick) |
| 27 | v_hieroglyph | 28 | v_petroglyph (hieroglyph) |
| 29 | v_directors_cut | 30 | v_retcon (directors_cut) |
| 31 | v_paint_brush | 32 | v_palette (paint_brush) |

- The pick runs over all 32 in this order (1, 2, 3, ..., 32), not in the
  two-column layout of the table. Unavailable entries are placeholders and
  are resampled with `Voucher<ante>_resample<n>`.
- Unavailable means: redeemed, prerequisite missing, locked, currently in the
  shop's voucher slot, or present as a card. The present rule hardly matters
  for vouchers because unbought ones are removed when you leave the shop.
- An unbought voucher can be offered again in the next ante.
- If nothing is available, the fallback is Blank, even if Blank was already
  redeemed.
- Vouchers from the deck count as redeemed from the start: Magic Deck gives
  Crystal Ball, Nebula Deck gives Telescope, Zodiac Deck gives Tarot
  Merchant, Planet Merchant and Overstock. Their tier-2 upgrades can appear
  in ante 1.

### 4.3 Buying a voucher; restocking

- Buying requires `price ≤ dollars − debt floor` (5.6).
- The voucher is marked redeemed straight away, its effect is applied, and
  held jokers see a "card bought" event.
- **No restock.** The ante's voucher slot stays empty for the rest of the
  ante. The next voucher only appears after the next boss.
- Quirk: buying *any* voucher, including a Voucher Tag's extra one, clears
  the ante's pending voucher. If you buy only the tag voucher, the ante's
  voucher stays in this shop (its card is already there) but will not come
  back in later shops of the ante.

### 4.4 Effects that touch shop, packs or prices

Most effects are applied in a queued step right after the purchase, before
any further action.

| Voucher | Effect |
|---|---|
| Overstock / Overstock Plus | +1 joker-row slot each, and the row is refilled at once (1.4) |
| Clearance Sale / Liquidation | discount set to 25% / 50% (set, not added). Every card's price and sell value is recomputed at once, including cards you own (5.4) |
| Hone / Glow Up | edition rate set to 2 / 4 |
| Reroll Surplus / Reroll Glut | base reroll cost −$2 each. The current reroll price also drops by $2 at once, but not below $0 |
| Tarot Merchant / Tycoon | Tarot weight 9.6 / 32 |
| Planet Merchant / Tycoon | Planet weight 9.6 / 32 |
| Magic Trick / Illusion | playing-card weight 4. Illusion adds 40% Enhanced and the 20% edition chance (1.3) |
| Crystal Ball | +1 consumable slot |
| Omen Globe | Arcana pack cards may be Spectral (20% each, 3.4) |
| Telescope | first Celestial pack card is the most played hand's planet (3.4) |

The other vouchers have no shop effect: Observatory, Grabber, Nacho Tong,
Wasteful, Recyclomancy, Seed Money, Money Tree, Blank, Antimatter,
Hieroglyph, Petroglyph, Director's Cut, Retcon, Paint Brush and Palette.

### 4.5 Voucher Tag

- When the shop is built, each held Voucher Tag adds one voucher card (the
  voucher slot gets one more space).
- The card is picked with resample from `Voucher_fromtag`. This key has no
  ante and is shared by every Voucher Tag in the run.
- The pick happens just after the shop is built. The ante's voucher is
  already in the slot, so it is excluded, and a second Voucher Tag excludes
  the first tag's voucher.
- Tag vouchers are offered in this shop only.

---

## 5. Prices and money

### 5.1 Buy price

A card's price is recomputed at the moments listed in 5.4:

1. `extra` = inflation. Inflation is 0 except in the Inflation challenge,
   where it goes up by 1 for each card bought, pack opened or voucher
   redeemed.
2. Add the edition surcharge: foil +2, holographic +3, polychrome +5,
   negative +5. Seals and enhancements cost nothing extra.
3. `price = max(1, floor((base + extra + 0.5) * (100 − discount) / 100))`.
   Compute it in f64 in this order: sum, times (100 − discount), divide by
   100, floor.
4. Booster with the "booster ante scaling" modifier: add (ante − 1). No
   vanilla challenge uses this modifier.
5. Booster while the tutorial's shop step is unfinished: +3 (profile-only;
   ignore for research).
6. Planet cards and Celestial packs while a non-debuffed Astronomer is held:
   price 0.
7. Rental joker: price 1.
8. Sell value is now computed from this price (5.4).
9. A card marked free (Coupon Tag, Uncommon or Rare Tag, an edition tag)
   costs 0 while it sits in the joker row or a booster slot.

**Base costs**:

| Item | Base cost |
|---|---|
| Jokers | see 2.7 (1 to 10; legendaries 20) |
| Tarot, Planet | 3 |
| Spectral (including The Soul and Black Hole) | 4 |
| Voucher | 10 |
| Pack: normal / jumbo / mega | 4 / 6 / 8 |
| Playing card, plain or enhanced | 1 |

**Price after discount** (`b` = base plus surcharge):

| b | 0% | Clearance (25%) | Liquidation (50%) |
|---|---|---|---|
| 1 | 1 | 1 | 1 |
| 2 | 2 | 1 | 1 |
| 3 | 3 | 2 | 1 |
| 4 | 4 | 3 | 2 |
| 5 | 5 | 4 | 2 |
| 6 | 6 | 4 | 3 |
| 7 | 7 | 5 | 3 |
| 8 | 8 | 6 | 4 |
| 9 | 9 | 7 | 4 |
| 10 | 10 | 7 | 5 |
| 11 | 11 | 8 | 5 |
| 12 | 12 | 9 | 6 |
| 13 | 13 | 10 | 6 |
| 15 | 15 | 11 | 7 |
| 20 | 20 | 15 | 10 |
| 25 | 25 | 19 | 12 |

Playing cards in the shop cost $1; with Illusion's foil $3, holographic $4,
polychrome $6.

### 5.2 Affordability (5.6 gives the debt floor)

- Joker-row cards and packs can be bought or opened if
  `price ≤ dollars − floor`, or if the price is 0.
- Vouchers can be bought if `price ≤ dollars − floor`.
- A joker or consumable also needs a free slot, with one extra slot if the
  card is negative. Playing cards and vouchers need no room.
- "Buy and use" for consumables skips the slot check but needs the card to
  be usable now.

### 5.3 Where bought things go

- Joker: to your joker slots.
- Consumable: to your consumable slots.
- Playing card: to the bottom of the deck, and added to your deck list.
- After any purchase, held jokers see a "card bought" event.

### 5.4 Sell value

- `sell = max(1, floor(price / 2)) + bonus`, where price is the value from
  5.1 steps 1 to 7, before any free marking.
- So a card you got free from a tag still sells for half its normal price.
- A planet bought with Astronomer held sells for 1.
- A rental card sells for 1 plus its bonus.
- Eternal cards cannot be sold.
- Only cards in your joker or consumable slots can be sold, and selling pays
  the stored sell value.
- The sell value is stored and only recomputed when one of these happens:
  - the card is created
  - its edition or seal changes
  - it becomes rental
  - Egg or Gift Card adds bonus
  - Clearance Sale or Liquidation is bought (every card)
  - an Astronomer enters or leaves your slots (every card)
  - Inflation rises (every card, challenge only)
- Bonus growth at the end of each round:
  - Egg: +$3 to itself.
  - Gift Card: +$1 to every joker and every consumable you hold, Gift Card
    included.
- Because of the "every card" recomputation, buying Clearance Sale lowers the
  sell value of jokers you already own. For example, a $6 joker sells for $3
  before and $2 after. Price changes for owned cards only show up in their
  sell value.

### 5.5 Reroll price

- **Base reroll cost**: $5, then −$2 for each of Reroll Surplus and Reroll
  Glut redeemed ($3, then $1). Challenges can set a different start. No deck
  changes it.
- **Count of paid rerolls**: reset to 0 at the start of every round (blind),
  so it is effectively per shop.
- **Free rerolls**: at the start of every round, equal to the number of
  non-debuffed Chaos the Clown jokers held. Buying a Chaos the Clown
  mid-shop adds 1 at once, and selling one removes 1 (not below 0).
- **Current price** = 0 while free rerolls remain. Otherwise it is (the D6
  override if active, else the base) plus the paid-reroll count.
- **Each reroll**:
  1. Pay the current price if it is above 0.
  2. If it was a free reroll, use one up. The count does not rise.
  3. Otherwise add 1 to the count. A $0 reroll under the D6 Tag still counts.
  4. Recompute the price.
- **D6 Tag**: for the shop where it fires, the base is replaced by $0, so
  rerolls cost $0, $1, $2, .... The override is cleared at the end of the
  next round.
- **Reroll Surplus or Glut bought mid-shop**: the base drops by $2, and the
  shown price drops by $2 at once (not below 0). The count is kept. If a D6
  Tag is active in this shop, the D6 override (not the base) still sets the
  price, so the lowered price holds for the very next reroll only, and after
  it the price is $0 + count again (`rules/vouchers_tags.md` 1.7).
- **Allowed** when `dollars − floor − price ≥ 0`, or when the price is 0.
- Example, no vouchers: $5, $6, $7, .... With one Chaos the Clown: $0
  (free), then $5, $6, ....

### 5.6 Money limits

- There is no upper limit on money.
- The debt floor is $0. Each held Credit Card lowers it by $20 (−$20 with
  one) while held, and selling it raises it again. All purchases and rerolls
  check against this floor.

---

## 6. Decks, stakes and challenges (shop-related only)

- **Red Deck, White Stake** (first studies): every default above applies.
  The sticker stream `etperpoll<ante>` / `packetper<ante>` is still used for
  every shop and Buffoon-pack joker.
- **Stakes**:
  - Black (4) and above: eternal on a sticker roll above 0.7.
  - Orange (7) and above: perishable on a roll in (0.4, 0.7].
  - Gold (8): rental on a separate roll `ssjr` / `packssjr` above 0.7.
  - The other stake changes (blind rewards, scaling, discards) do not touch
    the shop.
- **Decks**:
  - Magic: Crystal Ball, plus two The Fool cards held at the start, which
    keep The Fool out of the pools.
  - Nebula: Telescope.
  - Zodiac: Tarot Merchant, Planet Merchant and Overstock (3 joker slots).
  - Ghost: Spectral weight 2, plus a Hex held at the start.
  - Abandoned and Checkered: shop and Standard-pack playing cards still use
    all 52 cards.
  - The others do not change the shop.
- **Challenges**:
  - banned keys
  - "no shop jokers" (joker weight 0)
  - Inflation
  - "all eternal"
  - starting vouchers, which count as redeemed
  - a possible custom starting reroll cost

---

## 7. Open points

- UNSURE: locked items depend on the player's profile. The engine should take
  a locked-key set as configuration; default is everything unlocked.
- Settled: the order of the hand list To Do List picks from. It is built by
  walking the run's poker-hand table in the runtime's hash order (confirmed on
  the game's runtime; `rules/rng_keys.md`, section 2), then picked by index.
  Its stream `to_do` is used whenever a To Do List card is created, including
  in the shop and in packs.
- UNSURE: Hallucination's Tarot is created after the pack's contents. This
  was worked out from event timing (3.6), not seen in play.
- Settled: hand sorting and the deck order at blind select for tag packs
  (3.5). No shuffle happens at a skip or at blind select. The deck order is
  the one left by the run-start `shuffle` (before the first round) or the
  last `cashout<ante>`, changed only by later Arcana or Spectral pack hands
  (top to hand, back to the bottom) and by cards added at the bottom. `nr<ante>`
  is never seen by a pack. The hand is re-sorted after each card drawn
  (`rules/round.md` 4). Details: `rules/vouchers_tags.md` 2.5.
- Weights and prices must be added and multiplied in the same order as
  written above, in f64. The order is written out exactly, but nobody has yet
  compared the cumulative pack and kind boundaries against the real game.

---

## Where in the game

- `functions/UI_definitions.lua`: `G.UIDEF.shop`, `create_card_for_shop`,
  `create_shop_card_ui`, `G.UIDEF.use_and_sell_buttons`, the pack UI builders
  (pack card area)
- `functions/common_events.lua`: `create_card`, `get_current_pool`,
  `poll_edition`, `get_pack`, `get_next_voucher_key`, `get_next_tag_key`,
  `create_playing_card`, `copy_card`, `calculate_reroll_cost`,
  `change_shop_size`, `draw_card`, `ease_dollars`, `ease_ante`
- `functions/misc_functions.lua`: `pseudorandom`, `pseudoseed`, `pseudohash`,
  `pseudorandom_element`, `pseudoshuffle`, `find_joker`,
  `get_starting_params`, `EMPTY`, `remove_all`
- `functions/button_callbacks.lua`: `G.FUNCS.reroll_shop`,
  `G.FUNCS.buy_from_shop`, `G.FUNCS.check_for_buy_space`, `G.FUNCS.can_buy`,
  `G.FUNCS.can_buy_and_use`, `G.FUNCS.can_redeem`, `G.FUNCS.can_open`,
  `G.FUNCS.can_reroll`, `G.FUNCS.can_select_card`, `G.FUNCS.can_skip_booster`,
  `G.FUNCS.use_card`, `G.FUNCS.sell_card`, `G.FUNCS.toggle_shop`,
  `G.FUNCS.cash_out`, `G.FUNCS.skip_booster`, `G.FUNCS.end_consumeable`
- `functions/state_events.lua`: `end_round`, `new_round`,
  `G.FUNCS.draw_from_deck_to_hand`, `G.FUNCS.draw_from_hand_to_deck`
- `card.lua`: `Card:init`, `Card:set_ability`, `Card:set_cost`,
  `Card:set_edition`, `Card:set_seal`, `Card:set_eternal`,
  `Card:set_perishable`, `Card:set_rental`, `Card:add_to_deck`,
  `Card:remove_from_deck`, `Card:open`, `Card:redeem`, `Card:apply_to_run`,
  `Card:sell_card`, `Card:can_sell_card`, `Card:calculate_joker` (pack
  opened, shop rerolled, shop ended, end of round for Egg and Gift Card),
  `Card:calculate_seal`, `Card:use_consumeable` (creation calls),
  `Card:explode`, `Card:start_dissolve`, `Card:remove`
- `tag.lua`: `Tag:apply_to_run`, `Tag:yep`, `Tag:nope`
- `game.lua`: `Game:init_item_prototypes` (item tables and pool sorting),
  `Game:init_game_object`, `Game:start_run`, `Game:update_shop`,
  `Game:update_arcana_pack`, `Game:update_spectral_pack`,
  `Game:update_standard_pack`, `Game:update_buffoon_pack`,
  `Game:update_celestial_pack`
- `back.lua`: `Back:apply_to_run`
- `blind.lua`: `Blind:defeat`, `Blind:set_blind`
- `cardarea.lua`: `CardArea:emplace`, `CardArea:remove_card`,
  `CardArea:draw_card_from`, `CardArea:shuffle`, `CardArea:sort`,
  `CardArea:remove`
- `engine/event.lua`: `Event:init`, `Event:handle`, `EventManager:update`
- `engine/ui.lua`: `UIBox:remove`, `UIElement:remove`
- `globals.lua`: hand list order
- `challenges.lua`: challenge rule names (scanned only)
