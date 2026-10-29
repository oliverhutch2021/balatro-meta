# Vouchers and skip tags

There are 32 vouchers in 16 pairs. Every voucher has a base price of $10. The
first voucher of a pair is open from the start. The second needs the first
redeemed in the same run and unlocked in the profile. Vouchers reach the shop
three ways: the ante's voucher, drawn from stream `Voucher<ante>` and offered in
every shop until any voucher is redeemed; one extra voucher per Voucher Tag,
drawn from `Voucher_fromtag`; and the starting vouchers of a few decks.
Redeeming marks the voucher as owned at once and applies its effect before the
player can act again. Only Overstock and Overstock Plus draw random numbers
while being redeemed (they refill the joker row). Hieroglyph and Petroglyph
lower the ante, and with it the ante number in every ante-keyed stream.

There are 24 skip tags. Skipping a Small or Big Blind gives the tag shown under
it. Held tags form a list in the order they were gained. Ten kinds of check run
at fixed moments, and each tag answers only to its own kind. A tag that acts is
used up. A tag whose moment has not come waits, across shops and antes. Six
tags act right after the skip (money, jokers or a hand level). Five open a
free pack at blind select and one rerolls the boss. The rest act in the next
shop, the next round, the next cash-out after a boss, or when the next tag is
gained (Double Tag). How the two tags of an ante are drawn
is in `rules/blinds.md` (sections 3 and 4). Shop mechanics, card creation and
pack contents are in `rules/shop.md`. This file covers what every voucher and
tag does, when it acts and which streams it draws.

---

## 0. Conventions used below

- Notation as in `rules/shop.md` section 0: `key<ante>` is the key followed
  directly by the current ante number. "Roll", "pick" and "pick with
  resample" mean the same as there.
- **At once / queued.** The game does part of an action immediately and puts
  the rest on its queue of steps (`rules/timing.md` section 2). After a voucher
  is redeemed or a tag fires, the player cannot act until those steps have run.
  The engine can apply each purchase or tag as one step, in the order given
  here. Where the order of queued steps changes a result, it is stated.
- **Held tags**: the run's tag list, oldest first. Every check walks it from
  the start.
- **Fires**: the tag acts and is used up.
- **Profile assumptions**: fully unlocked and discovered profile, tutorial
  finished, seeded run. A seeded run never unlocks or discovers anything during
  play, so availability is fixed for the whole run. See UNSURE.

---

## 1. Vouchers

### 1.1 Data

Pool order is the `#` column. All 32 cost $10 before discounts.

| # | Key | Name | Needs redeemed | Number in the data | What the effect uses |
|---|---|---|---|---|---|
| 1 | `v_overstock_norm` | Overstock | - | none | +1 joker-row slot (fixed in code) |
| 2 | `v_overstock_plus` | Overstock Plus | `v_overstock_norm` | none | +1 joker-row slot (fixed) |
| 3 | `v_clearance_sale` | Clearance Sale | - | 25 | discount set to 25% |
| 4 | `v_liquidation` | Liquidation | `v_clearance_sale` | 50 | discount set to 50% |
| 5 | `v_hone` | Hone | - | 2 | edition rate set to 2 |
| 6 | `v_glow_up` | Glow Up | `v_hone` | 4 | edition rate set to 4 |
| 7 | `v_reroll_surplus` | Reroll Surplus | - | 2 | reroll base cost -$2 |
| 8 | `v_reroll_glut` | Reroll Glut | `v_reroll_surplus` | 2 | reroll base cost -$2 |
| 9 | `v_crystal_ball` | Crystal Ball | - | 3 (never read) | +1 consumable slot (fixed) |
| 10 | `v_omen_globe` | Omen Globe | `v_crystal_ball` | 4 (never read) | Spectral chance per Arcana card, threshold 0.8 (fixed) |
| 11 | `v_telescope` | Telescope | - | 3 (never read) | first Celestial pack card forced |
| 12 | `v_observatory` | Observatory | `v_telescope` | 1.5 | X1.5 mult per matching Planet held |
| 13 | `v_grabber` | Grabber | - | 1 | +1 hand per round |
| 14 | `v_nacho_tong` | Nacho Tong | `v_grabber` | 1 | +1 hand per round |
| 15 | `v_wasteful` | Wasteful | - | 1 | +1 discard per round |
| 16 | `v_recyclomancy` | Recyclomancy | `v_wasteful` | 1 | +1 discard per round |
| 17 | `v_tarot_merchant` | Tarot Merchant | - | 9.6/4 | Tarot shop weight set to 4 × (9.6/4) = 9.6 |
| 18 | `v_tarot_tycoon` | Tarot Tycoon | `v_tarot_merchant` | 32/4 | Tarot shop weight set to 4 × (32/4) = 32 |
| 19 | `v_planet_merchant` | Planet Merchant | - | 9.6/4 | Planet shop weight set to 9.6 |
| 20 | `v_planet_tycoon` | Planet Tycoon | `v_planet_merchant` | 32/4 | Planet shop weight set to 32 |
| 21 | `v_seed_money` | Seed Money | - | 50 | interest cap set to $50 |
| 22 | `v_money_tree` | Money Tree | `v_seed_money` | 100 | interest cap set to $100 |
| 23 | `v_blank` | Blank | - | 5 (never read) | nothing |
| 24 | `v_antimatter` | Antimatter | `v_blank` | 15 (never read) | +1 joker slot (fixed) |
| 25 | `v_magic_trick` | Magic Trick | - | 4 | playing-card shop weight set to 4 |
| 26 | `v_illusion` | Illusion | `v_magic_trick` | 4 | playing-card shop weight set to 4, plus the Illusion rolls |
| 27 | `v_hieroglyph` | Hieroglyph | - | 1 | ante -1, hands per round -1 |
| 28 | `v_petroglyph` | Petroglyph | `v_hieroglyph` | 1 | ante -1, discards per round -1 |
| 29 | `v_directors_cut` | Director's Cut | - | 10 (never read) | boss reroll for a fixed $10, once per ante |
| 30 | `v_retcon` | Retcon | `v_directors_cut` | 10 (never read) | boss reroll for a fixed $10, no limit |
| 31 | `v_paint_brush` | Paint Brush | - | 1 (never read) | +1 hand size (fixed) |
| 32 | `v_palette` | Palette | `v_paint_brush` | 1 (never read) | +1 hand size (fixed) |

- Odd numbers are tier 1 and start unlocked. Even numbers are tier 2. They
  start locked in a fresh profile, and each needs exactly one voucher, the
  tier-1 voucher just above it.
- "Never read" means the number is in the data but the effect uses a fixed
  value in the code, or does nothing.
- The merchants and tycoons also carry a display multiplier (2 and 4). It is
  only shown in the description.
- Compute the merchant weights in f64 exactly as written. 4 × (9.6/4) gives
  back the f64 nearest to 9.6, and 4 × (32/4) is exactly 32.

### 1.2 Where vouchers come from and which can appear

`rules/shop.md` section 4 has the pool table and the draw moments. This
subsection adds the exact rules.

**The pool.** Every voucher draw builds a list of all 32 in pool order. An
entry becomes a placeholder if any of these holds:

1. its key is banned (challenges only)
2. it is locked in the profile
3. it has been redeemed in this run
4. a voucher it needs has not been redeemed
5. a card with its name exists right now and no non-debuffed Showman is held.
   For vouchers this only ever means a card in the shop's voucher slot.
6. it is in the shop's voucher slot right now. This test ignores Showman.

The pick is drawn from the pool key and resampled on `<key>_resample<n>`, n
= 2, 3, ..., while it lands on a placeholder. If every entry is a placeholder,
the list becomes Blank alone (`v_blank`), even if Blank is already redeemed,
and one value is still drawn from the key.

**The ante's voucher.**

- It is drawn from `Voucher1` at run start. The deck's and the challenge's
  starting vouchers are already marked redeemed by then, and the first boss has
  already been drawn.
- It is drawn again at the end of every won Boss round, from `Voucher<ante>`
  with the new ante. The ante has just gone up, in the same end-of-round
  processing, before the cash-out screen.
- The result is stored as the run's pending voucher. Whenever a shop is built,
  a card for the pending voucher is put in the voucher slot (no draw). That
  card is marked as the ante's card.
- Redeeming any voucher clears the pending voucher. It does not matter whether
  the redeemed card was the ante's card or a Voucher Tag's card. A card already
  on display stays in the current shop.
- Leaving a shop without redeeming anything keeps the pending voucher, so it
  shows again in the next shop of the ante. The next Boss win replaces it
  whether or not it was bought.
- Leaving the shop destroys every voucher card still in the slot, the ante's
  and the Voucher Tags'. Only the pending key survives. A shop reroll never
  touches the voucher slot.
- There is no restock. Once cleared, the slot stays empty until the next Boss
  win. The game's state has a "voucher restock" flag, but it is only ever
  cleared, never set.
- Hieroglyph and Petroglyph do not touch the pending voucher. Because they
  lower the ante, a later Boss win draws `Voucher<n>` again for an ante number
  already used. The stream continues where it stopped.

**Voucher Tag vouchers** (2.14) are drawn from `Voucher_fromtag`, with no
ante in the key. The pending voucher's card is already in the slot by then, so
it is excluded. Each tag's card is offered in that shop only and is not marked
as the ante's card.

**Starting vouchers.**

- Magic Deck: Crystal Ball. Nebula Deck: Telescope. Zodiac Deck: Tarot
  Merchant, Planet Merchant and Overstock. Some challenges add more.
- They are marked redeemed before `Voucher1` is drawn, and their effect is
  applied at run start. They never go through the redeem steps of 1.3: no
  money, no clearing of the pending voucher.
- A deck's vouchers apply at once. A challenge's apply in queued steps after
  the run is set up.
- Their upgrades can therefore be drawn in ante 1.
- Zodiac's Overstock only raises the row size, because no shop exists yet.

### 1.3 Redeeming a voucher, step by step

1. The Redeem button works when price ≤ money − debt floor (`rules/shop.md`
   5.6). No slot is needed. No tag makes a voucher free, and every price is at
   least $1. Redeeming does not go through the shop's Buy path. It uses the
   same path as using a consumable or opening a pack.
2. The card leaves the voucher slot. The slot is not refilled. The card is
   moved to the play area, and the round's purchase counter (a statistic) goes
   up by 1.
3. The voucher is marked redeemed for the run at once. Pools built from now on
   exclude it and allow its upgrade. No voucher pool is built again in the same
   shop.
4. The price is charged: the price shown at that moment, in a queued step that
   comes before the effect's queued steps. So Clearance Sale costs $10, and
   Liquidation costs $7 when Clearance Sale is owned.
5. The pending voucher is cleared (1.2).
6. The effect is applied (1.4 to 1.19). Some parts happen at once, the rest in
   queued steps right after.
7. Held jokers get the "card bought" event with the voucher. No vanilla joker
   reacts to it.
8. Inflation challenge only: inflation +1 and every price is recomputed.
9. After its animation the card is destroyed. This clears its name mark, which
   no longer matters because the voucher is redeemed. The shop is usable
   again.

Profile bookkeeping also happens at redeem. Discovery, unlock checks and
career statistics are skipped in a seeded run. The profile's per-voucher
redeem count is still kept, but nothing in a seeded run reads it.

### 1.4 Overstock and Overstock Plus

- Each adds 1 to the joker-row size: 2, then 3, then 4. This is permanent.
  Every later shop and every reroll builds this many slots.
- In a queued step right after the purchase:
  1. The row size goes up by 1.
  2. The row is filled. It builds (row size − cards still in the row) new
     slots, one at a time, left to right, exactly as a reroll builds them
     (`rules/shop.md` 1.3 and 1.6): first the Uncommon and Rare Tag walk, then
     `cdt<ante>`, `illusion` when Illusion is owned, and the item's own
     streams.
  3. Example: with both row cards already bought, Overstock builds 3 new
     cards.
- Held Uncommon, Rare and edition tags can act on the new slots (2.7 to
  2.12). A Coupon Tag that fired when the shop opened does not reach them.
- This refill is the only voucher effect that draws random numbers.

### 1.5 Clearance Sale and Liquidation

- The discount is set to 25% or 50%. It is set, not added. Liquidation needs
  Clearance Sale, so the sequence is 0, 25, 50.
- In a queued step: the discount is set, then every card object in the run
  has its price and sell value recomputed. That covers the joker row, the
  voucher slot, the boosters, owned jokers and consumables, and all other
  cards. Shop cards marked free stay at $0.
- Results for the formula in `rules/shop.md` 5.1:

  | Item | No discount | Clearance Sale | Liquidation |
  |---|---|---|---|
  | Voucher | $10 | $7 | $5 |
  | Pack: normal / jumbo / mega | $4 / $6 / $8 | $3 / $4 / $6 | $2 / $3 / $4 |

- Owned cards' sell values drop with their recomputed prices (`rules/shop.md`
  5.4).
- Not discounted: the shop reroll price and the $10 boss reroll.
- No random draws.

### 1.6 Hone and Glow Up

- The edition rate is set to 2 or 4. It is set, not multiplied. The order is
  1, 2, 4.
- Set in a queued step right after the purchase. It changes every edition roll
  made afterwards, including rolls later in the same shop:
  - every joker made by the general card creator, on `edi<source><ante>`: the
    shop row, Uncommon and Rare Tags, Buffoon packs, Judgement, Wraith, The
    Soul, Riff-raff and the Top-up Tag
  - Standard pack cards, on `standard_edition<ante>`, where the pack also
    doubles the thresholds
- Not changed:
  - the joker Negative chance, always 0.3%
  - Aura and Wheel of Fortune, which use fixed thresholds
  - Illusion's shop playing-card editions, which use their own rolls
  - the editions that Hex, Ectoplasm and the edition tags give
- Threshold tables: `rules/shop.md` 2.2 step 7(c) and 3.4.
- No random draws at redeem.

### 1.7 Reroll Surplus and Reroll Glut

- Each lowers the base reroll cost by $2. The base starts at $5, so it goes
  $5, $3, $1. The base sets the price at every round start, as base plus the
  count of paid rerolls (0), and again after each reroll.
- At purchase, the current shop's displayed reroll price also drops by $2, at
  once in the same queued step, but not below $0. The paid-reroll count is
  kept.
- **With a D6 Tag active in this shop** (2.25), prices are computed from the
  D6 override ($0), and the voucher does not change the override. The
  displayed price still drops by $2 (not below $0). So the very next reroll
  costs that lower amount, and after it the price is back to $0 + count. Under
  a D6 with 3 paid rerolls, the price is $3. Buying Reroll Surplus makes it
  $1, that reroll is charged $1, and the next price is $4.
- While Chaos the Clown free rerolls remain, the price is $0 and stays $0.
- No random draws.

### 1.8 Crystal Ball and Omen Globe

- **Crystal Ball**: +1 consumable slot, in a queued step. The data number 3
  is never read. The Magic Deck starts with it.
- **Omen Globe**: no effect at redeem. Every Arcana pack card made afterwards
  first rolls `omen_globe`. This covers shop Arcana packs and the Charm Tag's
  pack. The key has no ante, and one stream serves the whole run.
  - Above 0.8, the card is made as a Spectral card with source `ar2`: The Soul
    check and then the Black Hole check, both on `soul_Spectral<ante>` and
    each made only while that card does not exist, then the pick from
    `Spectralar2<ante>`.
  - Otherwise it is a Tarot with source `ar1`: `soul_Tarot<ante>` (only while
    The Soul does not exist), then `Tarotar1<ante>`. See `rules/shop.md` 3.4.
  - The `omen_globe` roll happens for every Arcana card, whatever it turns
    out to be.

### 1.9 Telescope and Observatory

- **Telescope**: no effect at redeem. In every Celestial pack opened
  afterwards (shop packs and the Meteor Tag's pack), card 1 is the Planet of
  the run's most played hand.
  - The hands are walked from strongest to weakest: Flush Five, Flush House,
    Five of a Kind, Straight Flush, Four of a Kind, Full House, Flush,
    Straight, Three of a Kind, Two Pair, Pair, High Card.
  - A hand is kept only if its run play count is strictly above the best so
    far, which starts at 0. Ties go to the stronger hand. The walk also
    requires the hand to be visible, but every played hand is visible.
  - The Planet is forced. There is no Black Hole roll and no pool pick for card
    1, and the "already present" rule is ignored, so it can repeat a Planet you
    hold or one in the shop. Cards 2 onward are made normally and exclude it.
  - If no hand has been played yet, card 1 is made normally.
  - The Nebula Deck starts with Telescope.
- **Observatory**: no effect at redeem. While a hand is scored, after all
  jokers, each Planet card in the consumable slots whose hand matches the
  scored hand type gives X1.5 mult, left to right. Each matching Planet counts
  separately. See `rules/scoring.md` section 6.

### 1.10 Grabber and Nacho Tong

- Each gives +1 hand per round.
  - At once: the base hands per round go up by 1.
  - Queued: the hands counter shown in the shop goes up by 1. That counter is
    reset from the base at the next round start, so the change only matters
    from the next round.
- No random draws.

### 1.11 Wasteful and Recyclomancy

- Each gives +1 discard per round, with the same pattern as 1.10. The base
  goes up at once. The displayed counter goes up in a queued step and is reset
  at the next round start.
- Discards left unused at the end of a won round feed the Garbage Tag's count
  (2.20), so extra discards can raise it.

### 1.12 Tarot Merchant and Tarot Tycoon

- The Tarot weight of the shop's kind roll is set to 9.6 or 32. It is set, not
  added, in a queued step.
- Every joker-row slot built afterwards uses the new weight: later rerolls and
  Overstock refills in this shop, and later shops.
- Kind chances: `rules/shop.md` 1.3. No random draws.

### 1.13 Planet Merchant and Planet Tycoon

- As 1.12 for the Planet weight: 9.6 or 32.
- The Zodiac Deck starts with both merchants (and Overstock).

### 1.14 Seed Money and Money Tree

- The interest cap is set to $50 or $100, in a queued step. Interest is
  (interest per $5) × min(floor(money / 5), cap / 5). With $1 per $5 that is
  at most $10 or $20.
- It is used at the next cash-out computation (`rules/round.md` section 9).
- No random draws.

### 1.15 Blank and Antimatter

- **Blank** does nothing in the run. Its data number 5 is never read.
  Redeeming it only feeds a profile counter that unlocks Antimatter. It still
  counts as a redeemed voucher: it leaves the pool and clears the pending
  voucher.
- **Antimatter**: +1 joker slot, in a queued step. The data number 15 is never
  read.

### 1.16 Magic Trick and Illusion

- Both set the playing-card weight of the shop's kind roll to 4, in a queued
  step. Illusion needs Magic Trick, so the weight is already 4 when Illusion is
  bought.
- **Magic Trick**: shop playing cards are plain cards (`frontsho<ante>`), with
  no enhancement, edition or seal.
- **Illusion** adds three uses of the `illusion` stream (no ante in the key)
  for shop slots built afterwards. See `rules/shop.md` 1.3 steps B and D.
  1. On every slot that reaches the kind roll, `illusion` is rolled once
     right after `cdt<ante>`, whatever kind is chosen. If the kind is playing
     card, above 0.6 makes it Enhanced (`Enhancedsho<ante>`, then
     `frontsho<ante>`).
  2. A playing card then rolls `illusion` again. Above 0.8 it gets an edition.
  3. In that case a third roll picks it: above 0.85 polychrome, above 0.5
     holographic, otherwise foil.
- Shop playing cards never get seals in this version.
- Tag-supplied slots skip the kind roll and so never roll `illusion`.

### 1.17 Hieroglyph and Petroglyph

Both lower the ante by 1. Hieroglyph also removes 1 hand per round and
Petroglyph 1 discard per round.

- **When**:
  - The ante drops in a queued step right after the purchase.
  - The ante used for the blind-select preview drops at once (`rules/blinds.md`
    8).
  - Hieroglyph: the base hands per round drop by 1 at once, and the shop's
    displayed hands counter drops in a queued step.
  - Petroglyph: the base discards per round drop by 1 at once, and the
    displayed discards counter drops in a queued step, unless it is already 0.
  - At round start hands are at least 1 and discards at least 0.
- **What uses the lower ante from then on.** Every ante-keyed stream:
  - draws later in the same shop: rerolls and refills (`cdt`, `Joker...sho`,
    `edisho`, `etperpoll`, ...) and the contents of packs opened afterwards
    (`soul_...`, `Tarotar1`, `Planetpl1`, `stdset`, ...)
  - the next blinds' `nr<ante>` and `cashout<ante>`, and the end-of-round
    `idol`, `mail`, `anc` and `cas` picks
  - the base chip amount of every blind still to be played (`rules/blinds.md`
    8; ante 0 or below uses 100)
- **What is kept**: the current ante's boss, even a showdown boss drawn for
  ante 8; both skip tags; the blind states (which blinds were played or
  skipped); the pending voucher; the boss-rerolled flag.
- **Consequences**:
  - The run needs one more Boss win to reach the winning ante.
  - The next Boss win raises the ante back. `Voucher<n>`, `Tag<n>` and the
    boss draw then reuse an ante number, and each stream continues where it
    stopped.
  - The ante can reach 0 or below, for example Hieroglyph bought in ante 1,
    and -1 with Petroglyph after it. Keys then read `cdt0`, `cdt-1`, `Tag0`
    and so on. Tags with minimum ante 2 drop out of the tag pool again at
    those antes.
- **Orbital Tag hands.** Each blind type's stored "orbital hand" is keyed by
  ante number (`rules/blinds.md` 4). The blind-select screen is rebuilt when
  the shop is left. Each tag shown under a blind is made fresh at that moment
  and reads the entry for the current, lowered ante.
  - If the blind-select screen was already built once at that ante number,
    its entries exist and no `orbital` draw happens. An Orbital Tag then
    levels the hand stored on that earlier visit, not the one it showed before
    the purchase.
  - If the number has no entries yet (0 or below), the screen draws `orbital`
    three times, for Small, Big and Boss.
  - When the ante later climbs back to a number that already has entries,
    they are reused without a draw. If Hieroglyph was bought in the first shop
    of an ante, before that ante's blind-select screen was ever built, that
    ante's entries do not exist yet and are drawn when the ante comes back to
    it.
- No other random draws.

### 1.18 Director's Cut and Retcon

- No effect at redeem. The Reroll Boss button is added when the blind-select
  screen is built, so it appears from the next blind select.
- It works when money − debt floor ≥ $10. Director's Cut also requires that
  the boss has not been rerolled yet this ante. Retcon has no limit.
- Each press costs a fixed $10 with no discount (the data number 10 is never
  read). It sets the boss-rerolled flag, draws a new boss from `boss`, rebuilds
  the boss panel and then runs the new-blind-choice tag check (2.3).
- Full rules: `rules/blinds.md` 6 and 7. A Boss Tag sets the same flag
  (2.15).

### 1.19 Paint Brush and Palette

- Each gives +1 hand size, in a queued step. The fixed +1 is used, and the
  data number is never read.
- No card is drawn in the shop. The next hand drawn uses the larger size:
  an Arcana or Spectral pack opened later in the same shop, a pack tag at
  blind select, or the next round.

### 1.20 Stakes and decks

- No stake changes a voucher's price, effect or pool.
- The Red Deck has no starting voucher. The Magic, Nebula and Zodiac Decks do
  (1.2).
- Challenges can give starting vouchers and ban vouchers.

---

## 2. Skip tags

### 2.1 Data

Pool order is the `#` column, as in `rules/blinds.md` 3.

| # | Key | Name | Check | Min ante | Needs discovered | Numbers |
|---|---|---|---|---|---|---|
| 1 | `tag_uncommon` | Uncommon Tag | store joker create | - | - | - |
| 2 | `tag_rare` | Rare Tag | store joker create | - | `j_blueprint` | odds 3 (never read) |
| 3 | `tag_negative` | Negative Tag | store joker modify | 2 | `e_negative` | odds 5 (never read) |
| 4 | `tag_foil` | Foil Tag | store joker modify | - | `e_foil` | odds 2 (never read) |
| 5 | `tag_holo` | Holographic Tag | store joker modify | - | `e_holo` | odds 3 (never read) |
| 6 | `tag_polychrome` | Polychrome Tag | store joker modify | - | `e_polychrome` | odds 4 (never read) |
| 7 | `tag_investment` | Investment Tag | eval | - | - | $25 |
| 8 | `tag_voucher` | Voucher Tag | voucher add | - | - | - |
| 9 | `tag_boss` | Boss Tag | new blind choice | - | - | - |
| 10 | `tag_standard` | Standard Tag | new blind choice | 2 | - | - |
| 11 | `tag_charm` | Charm Tag | new blind choice | - | - | - |
| 12 | `tag_meteor` | Meteor Tag | new blind choice | 2 | - | - |
| 13 | `tag_buffoon` | Buffoon Tag | new blind choice | 2 | - | - |
| 14 | `tag_handy` | Handy Tag | immediate | 2 | - | $1 per hand |
| 15 | `tag_garbage` | Garbage Tag | immediate | 2 | - | $1 per discard |
| 16 | `tag_ethereal` | Ethereal Tag | new blind choice | 2 | - | - |
| 17 | `tag_coupon` | Coupon Tag | shop final pass | - | - | - |
| 18 | `tag_double` | Double Tag | tag added | - | - | - |
| 19 | `tag_juggle` | Juggle Tag | round start bonus | - | - | +3 hand size |
| 20 | `tag_d_six` | D6 Tag | shop start | - | - | - |
| 21 | `tag_top_up` | Top-up Tag | immediate | 2 | - | 2 jokers |
| 22 | `tag_skip` | Skip Tag | immediate | - | - | $5 per skip |
| 23 | `tag_orbital` | Orbital Tag | immediate | 2 | - | 3 levels |
| 24 | `tag_economy` | Economy Tag | immediate | - | - | at most $40 |

The edition tags also name their edition in the data. The "odds" numbers are
never read: no tag rolls a chance. Pool culling and the `Tag<ante>` draw are
in `rules/blinds.md` 3.

### 2.2 Gaining, holding and losing tags

- **Ways to gain a tag**:
  1. Skipping the Small or Big Blind on deck gives the tag shown under it
     (`rules/blinds.md` 2).
  2. A Double Tag copies a newly gained tag (2.23).
  3. Selling Diet Cola adds a Double Tag, in a queued step.
  4. The Anaglyph Deck adds a Double Tag after every Boss win, when the
     cash-out is computed.

  Nothing else adds tags.
- **Adding a tag**: every held tag first gets the "tag added" check (only
  Double Tags react). Then the new tag is appended to the end of the held
  list. The held list is therefore in order of gaining. Double Tag copies are
  appended right after the original, in the order the Double Tags were gained
  (2.4).
- **Tags shown under blinds** are new tag objects made each time the
  blind-select screen is built, from the two keys drawn for the ante. Only
  skipping adds one to the held list. A shown Orbital Tag reads its hand when
  it is made (2.28).
- **Firing**: each tag can fire once. When it does, it is flagged so no later
  check can fire it again, and it is removed from the list shortly after its
  animation, before the player can act. The engine can remove it at once.
- A failing Rare Tag is also used up (2.8).
- A tag that fires with no effect is still used up: Top-up with full joker
  slots, Economy with $0 or less, Handy or Garbage with a count of 0.
- Tags never expire. A tag whose moment has not come stays held across
  rounds, shops and antes. Examples: Investment waiting for a Boss win, an
  edition tag waiting for a joker without an edition.

### 2.3 The ten checks and where they run

| Check | Tags | Runs | Which tags fire |
|---|---|---|---|
| tag added | Double | whenever a tag is gained, before it is appended | every held Double Tag that has not fired, unless the new tag is a Double Tag |
| immediate | Top-up, Skip, Garbage, Handy, Economy, Orbital | (a) right after a skip; (b) when the blind-select screen is built fresh: run start, after leaving a shop, continuing a saved run | all of them, in list order |
| new blind choice | Boss, Standard, Charm, Meteor, Buffoon, Ethereal | (a) after the skip's immediate check; (b) after the blind-select build's immediate check; (c) after every boss reroll; (d) every time any pack closes, shop packs included | only the first such tag in list order; it always fires |
| round start bonus | Juggle | each time the hand is about to be drawn in a round: round start, after each play, after each discard | all of them |
| eval | Investment | when the cash-out amounts are computed after a won round, after the joker money rows and before interest | every Investment Tag whose condition holds |
| shop start | D6 | when a shop opens, before the joker row is built | the first D6 Tag only (one per shop) |
| store joker create | Uncommon, Rare | before each joker-row slot is built: shop opening, reroll, Overstock refill | walk in order until one supplies a card |
| store joker modify | Foil, Holographic, Polychrome, Negative | after each joker-row card is made: at once for a tag-supplied card, in a queued step for a normal slot | walk in order until one applies, so at most one per card |
| voucher add | Voucher | when a shop opens, after the boosters are placed | all of them |
| shop final pass | Coupon | right after the voucher-add check | the first Coupon Tag only (one per shop) |

The moments, in the order they occur:

- **Skip** (`rules/blinds.md` 2):
  1. The skip counter goes up by 1.
  2. The tag is gained. The tag-added check runs, and Double Tags queue their
     copies.
  3. The skipped blind is marked and the next blind goes on deck.
  4. One queued step: every joker is told a blind was skipped, the run is
     saved, the immediate check runs, then the new-blind-choice check.
- **Blind-select screen built fresh**:
  1. The three blind panels are built, with any `orbital` draws.
  2. The immediate check runs.
  3. The new-blind-choice check runs.

  In normal play no immediate or new-blind-choice tag is still held here,
  because each fired at its own skip or in the chain that followed.
- **A pack closes**:
  1. The hand goes back to the deck.
  2. The game returns to the state the pack was opened from.
  3. The new-blind-choice check runs. In a shop it normally finds nothing.
- **A boss reroll** (Director's Cut, Retcon, Boss Tag): the new boss is shown,
  then the new-blind-choice check runs. Boss and pack tags held together
  therefore fire one after another, in list order, each as soon as the one
  before has finished.
- **Every draw to hand in a round**: the round-start-bonus check runs, then
  the cards are drawn.
- **Cash-out computation**: the eval check runs.
- **A shop opening fresh**:
  1. Shop start.
  2. The joker-row slots, left to right. Each slot gets the create check and
     then the modify check.
  3. The pending voucher's card is placed.
  4. Booster slots 1 and 2.
  5. The voucher-add check.
  6. The shop-final-pass check.
  7. Then, in queue order: the queued modify checks of the normal slots, and
     the queued effects of the tags that fired, in the order they were
     queued.
- **A shop reroll or Overstock refill**: each new slot gets the create check
  and then the modify check, as above.

### 2.4 Order inside one check

- Every check walks the held list from the oldest tag.
- Each tag that fires runs its effect in its own queued step. These steps run
  in list order, each finishing before the next.
- Skip, Handy and Garbage read their counter when the check runs, and Orbital
  changes the level at once. Their money lands in queued steps.
- Economy reads the money only when its own step runs. It therefore doubles
  money already paid by tags before it in the list during the same check, and
  not money from tags after it.
- Double Tag copies are appended when the Double Tag's step runs. That step was
  queued when the tag was gained, before the skip queued its checks, so a copy
  takes part in the same immediate and new-blind-choice checks as the
  original.

### 2.5 How a pack tag opens its pack

This covers the Standard, Charm, Meteor, Buffoon and Ethereal Tags.

1. When the tag fires (new-blind-choice check, at blind select), its queued
   step makes a fresh pack card:

   | Tag | Pack | Cards | Choose | Hand dealt |
   |---|---|---|---|---|
   | Standard | `p_standard_mega_1` | 5 | 2 | no |
   | Charm | `p_arcana_mega_1` or `_2` | 5 | 2 | yes |
   | Meteor | `p_celestial_mega_1` or `_2` | 5 | 2 | no |
   | Buffoon | `p_buffoon_mega_1` | 4 | 2 | no |
   | Ethereal | `p_spectral_normal_1` | 2 | 1 | yes |

   Charm and Meteor choose the art variant with an unseeded `math.random`.
   The variants have identical contents, so this is cosmetic. The pack is not
   in a booster slot, its price is $0, and it does not count as a purchase.
   Tag packs never trigger the first-shop Buffoon rule (`rules/shop.md` 3.2).
2. It is opened through the normal pack path (`rules/shop.md` 3.3 to 3.6) with
   no money charged. The game enters the pack's state and returns to blind
   select when the pack closes.
3. Held jokers react to the opening as for a shop pack: Hallucination may roll
   `halu<ante>`. Skipping the pack triggers Red Card. The Inflation challenge
   counts the opening.
4. Contents are made card by card exactly as for the same pack in the shop,
   with the current ante in every key:

   | Tag | Draws per card |
   |---|---|
   | Standard | `stdset<ante>`; `Enhancedsta<ante>` if Enhanced; `frontsta<ante>`; `standard_edition<ante>`; `stdseal<ante>`; `stdsealtype<ante>` if sealed |
   | Charm | `omen_globe` with Omen Globe; Tarot: `soul_Tarot<ante>` (only if no The Soul exists), `Tarotar1<ante>` with resamples; Spectral: `soul_Spectral<ante>` for The Soul, `soul_Spectral<ante>` for Black Hole (each only if that card does not exist), `Spectralar2<ante>` with resamples |
   | Meteor | Telescope's card 1 is forced with no draws (1.9); otherwise `soul_Planet<ante>` (only if no Black Hole exists), `Planetpl1<ante>` with resamples |
   | Buffoon | `rarity<ante>buf`, `Joker<r>buf<ante>` with resamples, `packetper<ante>`, `packssjr<ante>` (Gold Stake only), `edibuf<ante>`; a To Do List also draws `to_do` |
   | Ethereal | `soul_Spectral<ante>` for The Soul, `soul_Spectral<ante>` for Black Hole (each only if that card does not exist), `Spectralspe<ante>` with resamples |

   A card forced to The Soul or Black Hole skips its pool pick. Cards within
   one pack never repeat, and cards in your slots are excluded, as in
   `rules/shop.md` 2.4.
5. **The hand (Charm and Ethereal only).**
   - Cards are drawn into the hand: min(cards in the deck, hand size − cards
     in hand). At blind select the hand is empty and every playing card is in
     the deck, so this is normally the full hand size: 8 for Red Deck, with
     lasting changes from vouchers and held jokers (Paint Brush, Juggler,
     Stuntman, ...). A held Juggle Tag does not count yet. No boss effect
     applies, because no blind is active.
   - Cards come off the top of the deck (the end of the deck list). After each
     card, the hand is re-sorted by the current sort mode (`rules/round.md`
     4).
   - The deck order at blind select is the order left by the most recent
     shuffle: the run-start `shuffle` if no round has been played yet,
     otherwise the last `cashout<ante>`. Skips do not shuffle. Two kinds of
     change apply on top:
     1. Every Arcana or Spectral pack hand since that shuffle, in a shop or
        from an earlier tag, took cards from the top and put them back at the
        bottom.
     2. Cards added since went to the bottom: bought playing cards and
        Standard pack picks.
   - A second hand-dealing pack before the next round, from a Double Tag copy
     or a later skip, therefore sees the next cards down.
6. **Choosing.** Tarot and Spectral cards are used at once and may act on the
   hand cards. Planets are used at once. Jokers need a free joker slot unless
   Negative. Standard pack cards go to the bottom of the deck and join the deck
   list. Details are in `rules/shop.md` 3.6.
7. **Closing**, when the choices are used up or the pack is skipped:
   1. Unchosen pack cards are destroyed, which frees their names.
   2. Every hand card goes back to the deck. The leftmost hand card is taken
      first and put at the bottom, then the next, so the returned cards end
      at the bottom in reverse hand order, and the rest of the deck keeps its
      order.
   3. The game returns to blind select.
   4. The new-blind-choice check runs again, which opens the next pack tag or
      fires the next Boss Tag.
8. **The deck afterwards.** Changes to cards persist: enhancements, suits,
   ranks, editions, seals, destroyed cards and added cards. The arrangement
   only matters for further pack hands before the next round. Selecting a
   blind sorts the deck by creation order and shuffles it with `nr<ante>`
   (`rules/round.md` 3), so nothing of the arrangement carries into the round.

### 2.6 How the edition tags choose their joker

This covers the Foil, Holographic, Polychrome and Negative Tags.

- They are offered every joker-row card made from now on: the initial build
  of every later shop, rerolls and Overstock refills.
  - A card supplied by an Uncommon or Rare Tag is offered right after it is
    made.
  - A normal slot's card is offered in a queued step right after it is made,
    so after its own edition roll.
  - Slots are always built and offered in left-to-right order. Tag-supplied
    slots come first in any build.
  - Jokers in packs, jokers from the Top-up Tag and jokers from consumables are
    never offered.
- A card qualifies if it is a joker (not a consumable or playing card), has no
  edition, and no edition tag is already being applied to it.
- The walk goes through the held tags in order. The first edition tag that has
  not fired takes the card, and the walk stops, so at most one edition tag goes
  on a card. Each further card is offered to the tags still waiting.
- Effect, in the tag's queued step: the card gets the tag's edition, is marked
  free, and its price is recomputed.
  - Its price in the shop is $0.
  - Its sell value is computed from the full price including the edition
    surcharge (+$2 foil, +$3 holographic, +$5 polychrome, +$5 negative;
    `rules/shop.md` 5.1 and 5.4).
- No random draws. A tag that never meets a qualifying card stays held.

### 2.7 Uncommon Tag (`tag_uncommon`)

- **When**: the next joker-row slot built while it is held. In practice that
  is slot 1 of the next shop, unless an older Uncommon or Rare Tag claims it
  first. A tag still held after a build is used by a reroll or refill in the
  same shop.
- **Effect**: that slot's card is an Uncommon joker. The kind roll is skipped:
  no `cdt<ante>` and no `illusion`. The rarity is fixed, so there is no rarity
  roll.
  - Draws: `Joker2uta<ante>` (resamples `Joker2uta<ante>_resample<n>`),
    `etperpoll<ante>` (every stake), `ssjr<ante>` (Gold Stake only), then
    `ediuta<ante>`.
  - The card is then offered to the edition tags at once (2.6).
  - It is marked free ($0 in the shop). Its sell value comes from its normal
    price.
- If every Uncommon joker is unavailable, the card is the plain Joker
  (`j_joker`), still drawn from `Joker2uta<ante>` with one value.

### 2.8 Rare Tag (`tag_rare`)

- **When**: as the Uncommon Tag, at the next joker-row slot built.
- **Condition**: count the distinct Rare jokers in your joker slots
  (debuffed ones count, duplicates count once). It succeeds if the count is
  below 20, the number of Rare jokers in the game including locked ones.
- **Success**: a Rare joker for the slot, made like 2.7 with `Joker3rta<ante>`
  (and resamples), `etperpoll<ante>`, `ssjr<ante>` (Gold Stake) and
  `edirta<ante>`. It is free and offered to the edition tags at once. If every
  Rare joker is unavailable (held, shown in the shop, or locked), the card is
  the plain Joker (`j_joker`).
- **Failure** (20 distinct Rares held): the tag shows a refusal and is used
  up, with no draws. The walk continues to later tags for the same slot. If
  none supplies a card, the slot is built normally.
- It only appears in the tag pool when Blueprint is discovered.

### 2.9 Negative Tag (`tag_negative`)

- Edition tag (2.6) giving Negative. Minimum ante 2. It needs the Negative
  edition discovered.
- A Negative joker needs no free joker slot to buy, and it adds a slot when
  it joins your jokers.

### 2.10 Foil Tag (`tag_foil`)

- Edition tag (2.6) giving Foil. It needs Foil discovered.

### 2.11 Holographic Tag (`tag_holo`)

- Edition tag (2.6) giving Holographic. It needs Holographic discovered.

### 2.12 Polychrome Tag (`tag_polychrome`)

- Edition tag (2.6) giving Polychrome. It needs Polychrome discovered.

### 2.13 Investment Tag (`tag_investment`)

- **When**: the eval check of every cash-out while held.
- **Condition**: the blind of the round just won was a Boss Blind. A Boss
  round saved by Mr. Bones counts.
- **Effect**: +$25 as its own cash-out row, after the joker money rows and
  before interest. It is paid with the rest of the total when Cash Out is
  pressed. It does not add to the money interest is computed on.
- If the round was not a boss, it stays held. Skipped blinds have no cash-out.
  So a tag gained by skipping the Small or Big Blind pays at the end of that
  ante's boss.

### 2.14 Voucher Tag (`tag_voucher`)

- **When**: the voucher-add check of the next shop, after the joker row, the
  pending voucher's card and both boosters are in place. Every held Voucher
  Tag fires there, in list order.
- **Effect**: the voucher slot gets one more space, and a voucher is picked
  with resample from `Voucher_fromtag` (resamples
  `Voucher_fromtag_resample<n>`).
  - The pool is the voucher pool of 1.2. The pending voucher's card and the
    cards of earlier Voucher Tags in this shop are already in the slot, so
    they are excluded.
  - The fallback is Blank.
  - The card is offered in this shop only. Its price follows the discount, and
    it is never free.
- Buying it clears the pending voucher, like any voucher (1.2).

### 2.15 Boss Tag (`tag_boss`)

- **When**: the new-blind-choice check, at blind select.
- **Effect**: a free boss reroll. It works like Director's Cut without the
  $10 and without its conditions.
  1. The boss-rerolled flag is set, so a Director's Cut cannot be used for the
     rest of this ante. Retcon still can.
  2. A new boss is drawn from `boss`, with the usual eligibility and
     least-used rules (`rules/blinds.md` 6). The rerolled-away boss keeps its
     raised use count.
  3. The boss panel is rebuilt. Its orbital entry already exists, so there is
     no `orbital` draw.
  4. The new-blind-choice check runs again.

### 2.16 Standard Tag (`tag_standard`)

- Pack tag (2.5): Mega Standard Pack (`p_standard_mega_1`), 5 playing cards,
  choose 2, no hand dealt. Minimum ante 2.

### 2.17 Charm Tag (`tag_charm`)

- Pack tag (2.5): Mega Arcana Pack (`p_arcana_mega_1` or `_2`), 5 cards,
  choose 2, deals a hand.
- With Omen Globe, each card may be a Spectral card (1.8).

### 2.18 Meteor Tag (`tag_meteor`)

- Pack tag (2.5): Mega Celestial Pack (`p_celestial_mega_1` or `_2`), 5
  Planets, choose 2, no hand dealt. Minimum ante 2.
- With Telescope, card 1 is the most played hand's Planet (1.9).

### 2.19 Handy Tag (`tag_handy`)

- **When**: the immediate check. Minimum ante 2.
- **Effect**: +$1 × the hands played this run. That is every hand played so
  far in any round, including hands a boss made worthless. Each hand is
  counted just after it is scored. The count is read at the check.

### 2.20 Garbage Tag (`tag_garbage`)

- **When**: the immediate check. Minimum ante 2.
- **Effect**: +$1 × the run's unused discards. That count is the total of
  discards left at the end of every won round so far. Skipped blinds add
  nothing. The count is read at the check.

### 2.21 Ethereal Tag (`tag_ethereal`)

- Pack tag (2.5): Spectral Pack (`p_spectral_normal_1`), 2 cards, choose 1,
  deals a hand. Minimum ante 2.

### 2.22 Coupon Tag (`tag_coupon`)

- **When**: the shop-final-pass check of the next shop. Only one Coupon Tag
  fires per shop. A second one waits for the following shop.
- **Effect**: in its queued step, every card then in the joker row (of any
  kind) and both boosters are marked free: price $0, sell value from the
  normal price.
- Not affected:
  - vouchers, including Voucher Tag vouchers
  - cards that arrive later in the shop, from rerolls or an Overstock refill
- The once-per-shop flag is cleared at every Cash Out.

### 2.23 Double Tag (`tag_double`)

- **When**: the tag-added check, each time a tag is gained while it is held.
- **Effect**: if the new tag is not a Double Tag, every held Double Tag that
  has not fired fires and adds one copy of the new tag. With n Double Tags,
  that is n copies.
- Copies do not set off further copies, because the Double Tags have already
  fired when the copies are added.
- A gained Double Tag is never copied. It simply joins the list, so you can
  hold several.
- A copy of an Orbital Tag levels the same hand as the original.
- Copies join the same checks as the original (2.4).
- A Double Tag gained from Diet Cola or the Anaglyph Deck works the same way
  on the next tag gained.

### 2.24 Juggle Tag (`tag_juggle`)

- **When**: the round-start-bonus check, which runs at the first draw of the
  next round played.
- **Effect**: +3 hand size for that round. It is applied before the opening
  hand is drawn, so that draw already takes 3 more cards. Several Juggle Tags
  stack.
- The extra size is removed when the round is won.

### 2.25 D6 Tag (`tag_d_six`)

- **When**: the shop-start check of the next shop. Only one D6 Tag fires per
  shop. A second one waits for the following shop.
- **Effect**: for this shop, rerolls are priced from $0 instead of the base
  cost, plus the paid-reroll count. Chaos the Clown free rerolls still come
  first. A $0 reroll counts as paid.
  - Example: $0, $1, $2, and so on (`rules/shop.md` 5.5).
  - With Reroll Surplus or Glut bought in the same shop, see 1.7.
- The override is removed at the end of the next won round. The once-per-shop
  flag is cleared at every Cash Out.

### 2.26 Top-up Tag (`tag_top_up`)

- **When**: the immediate check. Minimum ante 2.
- **Effect**: up to 2 Common jokers go straight into your joker slots, one
  at a time. Each is made only if the joker slots are not full at that moment.
  If the first one comes out Negative, it raises the slot limit, so the second
  can still fit.
- Draws per joker:
  - `Joker1top<ante>` (resamples `Joker1top<ante>_resample<n>`), with no
    rarity roll
  - `editop<ante>` for the edition
  - `to_do`, if the joker is a To Do List
  - no sticker rolls at any stake
- Each joker's on-add effects apply as it joins, for example a Chaos the
  Clown free reroll or Credit Card's debt floor.

### 2.27 Skip Tag (`tag_skip`)

- **When**: the immediate check.
- **Effect**: +$5 × the run's skip count, read at the check. That count
  already includes the skip that just happened. The first skip of a run pays
  $5, a tag gained on the third skip pays $15, and so on.

### 2.28 Orbital Tag (`tag_orbital`)

- **When**: the immediate check. Minimum ante 2.
- **Effect**: the tag's hand goes up 3 levels in one step, at once.
  - The new level is the old level + 3.
  - Chips and mult are recomputed from the hand's base and per-level values
    (`rules/hands.md` 6.2).
  - No draw at this point.
- **Which hand**: the hand stored for the blind type the tag was shown under,
  in the ante that was current when the blind-select screen was last built.
  It was drawn from `orbital` when that entry was first needed
  (`rules/blinds.md` 4). After Hieroglyph or Petroglyph that is the lowered
  ante's entry (1.17). A Double Tag copy uses the original's hand.

### 2.29 Economy Tag (`tag_economy`)

- **When**: the immediate check.
- **Effect**: + min($40, max($0, money)), with the money read when the tag's
  queued step runs (2.4). It doubles your money up to a $40 gain. Money of $0
  or less gains nothing.

### 2.30 Several tags of the same kind

| Tag | With several held |
|---|---|
| Uncommon, Rare | one per joker-row slot, in list order; leftovers wait for the next slot built (reroll, refill, next shop) |
| Edition tags | one per qualifying joker, in list order |
| Investment | all pay $25 at the same Boss cash-out, each as its own row |
| Voucher | all fire in the same shop, each adding one voucher; the picks exclude each other |
| Coupon, D6 | one per shop; the rest wait |
| Boss and pack tags | one at a time at blind select, in list order; each boss reroll or pack close fires the next |
| Double | all fire on the next tag gained: one copy each |
| Juggle | all fire at the same round start: +3 each |
| Top-up | each makes up to 2 jokers in turn, while slots are free |
| Skip, Handy, Garbage | each pays the full amount separately |
| Economy | each doubles in turn: the second reads the money after the first, so two give up to +$40 then up to +$40 again |
| Orbital | each levels its own stored hand; Double copies level the same hand |

### 2.31 Stakes

- No stake changes a tag's numbers, the tag pool or the minimum antes.
- Black Stake (4) and up: jokers from the Uncommon and Rare Tags can be
  Eternal (shop-row sticker roll `etperpoll<ante>`), and Buffoon Tag pack
  jokers can be too (`packetper<ante>`). From Orange Stake (7) the same roll
  can also make them Perishable (`rules/shop.md` 2.2 step 7). Top-up jokers
  never get stickers.
- Gold Stake (8): those jokers can also be Rental (`ssjr<ante>` or
  `packssjr<ante>`). A Rental joker from an Uncommon or Rare Tag still costs
  $0 in the shop, because the free mark wins over the Rental $1 price. It
  then charges $3 per round once owned.
- Red Stake (2) and up: the Small Blind pays nothing, so skipping it gives up
  no blind reward. The tags themselves are unchanged.
- Blue Stake (5): one discard fewer per round, so the Garbage Tag's count
  grows more slowly.

---

## UNSURE

- Profile dependence:
  - Tier-2 vouchers are unavailable while locked in the profile.
  - The Rare, Negative, Foil, Holographic and Polychrome Tags are unavailable
    until Blueprint or their edition is discovered.
  - A seeded run never unlocks or discovers anything, so these sets are fixed
    for the run. An unseeded run can change them mid-run. For example,
    Liquidation unlocks once a run holds 10 redeemed vouchers, and owning
    Blueprint or a joker with an edition discovers it.
  - The engine should take the locked and discovered sets as configuration
    (default: everything open), and tests against the real game should use
    seeded runs with a stated profile.
- Profiles that have not finished the tutorial get a scripted first voucher
  (Grabber), scripted first tags (Handy, Garbage) and scripted first shop
  cards, with no draws. Research assumes the tutorial is finished.
- Hieroglyph and Petroglyph with the Orbital Tag (1.17): the lookup by the
  lowered ante, the reuse of stored entries without drawing, and the fresh
  `orbital` draws at ante 0 or below were read from the code. Not yet
  confirmed in play.
- D6 Tag together with Reroll Surplus or Glut bought in the same shop (1.7):
  one reroll at the lowered price, then back to $0 + count. Read from the
  code, not yet confirmed in play.
- Top-up Tag: a first joker that comes out Negative lets the second one fit.
  Read from the code, not yet seen.
- Rare and Uncommon Tags fall back to the plain Joker when every joker of
  their rarity is unavailable. Read from the code. This is practically
  unreachable with a full profile.
- Economy Tag with other money tags in the same check (2.4): the result
  depends on queue order. It was worked out from the queue rules, and should
  be checked once with BalatroBot (Skip Tag then Economy Tag versus the
  reverse order).
- Some tags take no input lock while their effect is pending: D6, Voucher,
  Coupon, Uncommon and Rare. A very fast click in the real game could see a
  state the engine never shows, for example a reroll before the D6 price
  change has landed, or a purchase before a tag joker is marked free. The
  engine lets the player act only after the queue has drained. BalatroBot
  checks should wait the same way.

---

## Where in the game

- `card.lua`: `Card:redeem`, `Card:apply_to_run` (all voucher effects),
  `Card:open` (Omen Globe roll, Telescope card), `Card:calculate_joker`
  (Observatory branch for Planet cards, `open_booster` for Hallucination,
  `skipping_booster` for Red Card, `selling_self` for Diet Cola, the empty
  `buying_card` branch), `Card:set_ability` (name marks), `Card:set_cost`,
  `Card:set_edition`, `Card:add_to_deck` (Negative slot), `Card:remove`,
  `Card:check_use`.
- `tag.lua`: `Tag:init`, `Tag:set_ability` (Orbital hand), `Tag:apply_to_run`
  (contexts `tag_add`, `immediate`, `new_blind_choice`, `round_start_bonus`,
  `eval`, `shop_start`, `store_joker_create`, `store_joker_modify`,
  `voucher_add`, `shop_final_pass`), `Tag:yep`, `Tag:nope`, `Tag:remove`,
  `Tag:remove_from_game`.
- `game.lua`: `Game:init_item_prototypes` (voucher and tag data, pool
  sorting), `Game:init_game_object` (starting rates, discount, interest cap,
  shop size, skip count, tag list), `Game:start_run` (deck and challenge
  vouchers, first voucher and tags), `Game:update_shop`,
  `Game:update_blind_select`, `Game:update_draw_to_hand`,
  `Game:update_arcana_pack`, `Game:update_spectral_pack`.
- `back.lua`: `Back:apply_to_run` (deck vouchers), `Back:trigger_effect`
  (Anaglyph Double Tag).
- `functions/common_events.lua`: `get_next_voucher_key`, `get_next_tag_key`,
  `get_current_pool`, `create_card`, `poll_edition`, `calculate_reroll_cost`,
  `change_shop_size`, `level_up_hand`, `ease_dollars`, `ease_ante`,
  `ease_hands_played`, `ease_discard`, `get_new_boss`, `reset_blinds`,
  `discover_card`, `check_for_unlock`, `draw_card`, `add_round_eval_row`.
- `functions/UI_definitions.lua`: `G.UIDEF.shop`, `create_card_for_shop`,
  `create_shop_card_ui`, `add_tag`, `create_UIBox_blind_select`,
  `create_UIBox_blind_choice` (orbital entries), `create_UIBox_blind_tag`.
- `functions/button_callbacks.lua`: `G.FUNCS.use_card` (voucher and pack
  branches), `G.FUNCS.can_redeem`, `G.FUNCS.can_open`, `G.FUNCS.can_reroll`,
  `G.FUNCS.buy_from_shop`, `G.FUNCS.reroll_shop`, `G.FUNCS.toggle_shop`,
  `G.FUNCS.skip_blind`, `G.FUNCS.reroll_boss_button`, `G.FUNCS.reroll_boss`,
  `G.FUNCS.skip_booster`, `G.FUNCS.end_consumeable`, `G.FUNCS.cash_out`.
- `functions/state_events.lua`: `end_round` (voucher draw, unused discards,
  Juggle and D6 reset, the unused restock flag), `new_round` (reroll and pack
  slot reset), `G.FUNCS.evaluate_round` (eval check),
  `G.FUNCS.play_cards_from_highlighted` (hands-played count),
  `G.FUNCS.evaluate_play` (consumable loop for Observatory),
  `G.FUNCS.draw_from_deck_to_hand`, `G.FUNCS.draw_from_hand_to_deck`.
- `functions/misc_functions.lua`: `pseudorandom`, `pseudoseed`,
  `pseudorandom_element`, `get_starting_params`, `stop_use`,
  `set_voucher_usage`.
- `blind.lua`: `Blind:set_blind` (last-blind boss flag), `Blind:defeat`.
- `cardarea.lua`: `CardArea:change_size`, `CardArea:emplace`,
  `CardArea:remove_card`, `CardArea:draw_card_from`, `CardArea:sort`.
- `engine/event.lua`: `EventManager:update` (queue order).
- `globals.lua`: hand list order (Telescope).
- `challenges.lua`: starting and banned vouchers and tags (scanned only).
