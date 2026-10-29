# Consumables: Tarot, Planet and Spectral cards

This note covers every consumable in Balatro 1.0.1o: the 22 Tarot cards, the 12
Planet cards and the 18 Spectral cards, The Soul and Black Hole included. For
each card it gives when it can be used, what using it does and in which order,
every random stream it draws (key strings, moment, list order before the
pick), the cards it creates, the joker reactions it causes and what the game
records. It also covers selling, Negative consumables, and the differences
between using a card from the consumable slots, from a booster pack and with
Buy and Use in the shop. The text is in our own words. Only numbers, keys,
stream names and the names of the game's functions and contexts come from the
scripts. It builds on `timing.md` (states and gates), `shop.md` (card creation,
pools, "present" names, prices), `rng_keys.md` (stream catalogue, list orders),
`hands.md` (hand levels), `scoring.md` (card values, Observatory) and
`round.md` (drawing, hand order) rather than repeating them.

In short: pressing Use checks the card's condition, takes the card out of its
area and records the use. It then does the parts of the effect that decide
targets (rolls, random picks, hand levels), and the jokers react at that same
moment. The visible changes follow as queued steps in a fixed order: cards
changed, destroyed or created, and money. Only after all of them does the used
card start to dissolve, the "last used" record (for The Fool) update, the card
get removed and control return. The player cannot act in between. The engine
can therefore resolve a use as one step, as long as it keeps the internal order
of section 1.3.

## 0. Conventions

- **p** = `G.GAME.probabilities.normal`: 1, doubled by each non-debuffed Oops!
  All 6s. A roll "1 in n on stream `s`" draws `s` once and succeeds when the
  value is **strictly below p / n**. Only The Wheel of Fortune rolls a chance.
- **A** = the current ante as an integer, as it appears in keys (`rng_keys.md`).
- **Roll / pick / shuffle** as in `shop.md` 0 and `rng.md`: one stream step,
  one reseed, then one `math.random()` (roll), one `math.random(#list)` (pick),
  or a full Fisher-Yates pass from the end (shuffle).
- **List order before a pick or shuffle** (`rng_keys.md` 2): a list of cards is
  sorted by `sort_id` (creation order), whatever its order in hand or in the
  joker row. A literal list keeps its written order (suits `S H D C`; ranks as
  given per card). The enhancement list keeps pool order.
- **Hand** = the cards in the hand area: the held cards during a round, or the
  cards dealt for an open Arcana or Spectral pack.
- **Selection** = the hand cards currently selected (highlighted). The game
  keeps them in the order they were clicked, not in hand order. Only Death
  depends on position, and it uses the cards' on-screen x positions (3.14).
  Every other multi-card effect treats each selected card on its own, so the
  order never matters. A card forced by Cerulean Bell is always selected and
  counts toward the limits.
- **Slots / held**: consumable slots = `G.consumeables.config.card_limit`, 2 at
  start, +1 with Crystal Ball, +1 for each held Negative consumable. Held = the
  cards in the consumable area. Joker slots and held jokers likewise.
- **At the click** = done synchronously when Use is pressed. **Queued step** =
  done later by the event queue, in the order listed. All queued steps of a
  use finish before control returns (1.3).
- **Present** = the duplicate-prevention mark of `shop.md` 2.4.

## 1. Shared mechanics

### 1.1 When Use is allowed

Every consumable needs the gates of `timing.md` 3: no input lock, stop-use
counter 0, play area empty, and the state is not HAND_PLAYED, DRAW_TO_HAND or
PLAY_TAROT (PLAY_TAROT means another consumable is being used). Then the card's
own rule applies. Each rule only ever makes its own cards usable, so the order
in which the game tests them does not matter.

| Rule | Cards | Condition |
|---|---|---|
| Always | The Hermit, Temperance, Black Hole, all 12 Planets | none |
| Joker without edition | The Wheel of Fortune, Ectoplasm, Hex | at least one joker with no edition (Foil, Holographic, Polychrome and Negative all count as editions; debuffed jokers count as candidates) |
| Ankh | Ankh | at least one joker, and joker slots > 1; pressing Use also needs held jokers < joker slots (1.2 step 1) |
| Aura | Aura | exactly one hand card selected, and it has no edition |
| Room | The Emperor, The High Priestess | held < slots, or the card itself is in the consumable slots |
| The Fool | The Fool | as Room, and a last-used Tarot/Planet is recorded and it is not The Fool (1.5) |
| Joker room | Judgement, The Soul, Wraith | held jokers < joker slots |
| Selection | see 2.1 and 2.3 | state is SELECTING_HAND, TAROT_PACK, SPECTRAL_PACK or PLANET_PACK, and min ≤ selected ≤ max |
| Hand | Familiar, Grim, Incantation, Immolate, Sigil, Ouija | same states, and at least 2 cards in hand |

- The maximum used is the card's `max_highlighted` capped at 5 (never reached:
  no card allows more than 3). The minimum is 1, except Death (2).
- In practice, Selection and Hand cards (and Aura) work only during hand
  selection and in Arcana or Spectral packs. A Celestial pack deals no hand,
  and in the shop, at blind select and on the cash-out screen the hand is
  empty. All other cards also work at blind select, on the cash-out screen,
  in the shop and with any pack open (used from the consumable slots).
- Selected cards are ignored by cards that do not use them. For example, The
  Hermit works with cards selected, and Familiar picks among all hand cards
  whatever is selected.

### 1.2 What pressing Use does, in order

1. **Ankh only:** if held jokers ≥ joker slots, a "no space" alert is shown and
   nothing else happens. The card stays where it was and no use is recorded.
   (Buy and Use is the exception, 1.4.)
2. The state becomes PLAY_TAROT, or stays the pack state if a pack is open, and
   input is locked.
3. The card leaves its area at once: consumable slots or pack. With Buy and
   Use it has already left the shop row. From now on it no longer occupies a
   slot. A Negative card's extra slot stays until step 9.
4. The use is recorded (1.5). The "last used" update is queued and lands in
   step 8.
5. If the card is debuffed, its effect is skipped. Nothing debuffs consumables
   in vanilla (1.13).
6. The card's own effect (sections 3 to 5). The click-time part runs now: for
   cards that destroy playing cards this includes the `remove_playing_cards`
   joker pass (1.6). The queued steps are put in the queue in the order given
   per card.
7. Still at the click: every joker, left to right, gets `using_consumeable`
   (1.6). This happens before any queued step of step 6 has run.
8. After all queued steps of the effect have run: the card starts to dissolve,
   and the last-used record is set (Tarots and Planets only).
9. About 0.7 s later the card is removed. Its present mark is cleared (unless
   another card of that name is in the joker or consumable slots), and a
   Negative card's extra slot is removed. If the card had been added to your
   slots (bought, created into the slots, or Buy and Use), a re-test of the
   boss debuff on every playing card is queued (1.7). Cards destroyed by the
   effect have been removed by now too.
10. The previous state returns and input unlocks. The boss re-test of step 9
    runs one frame later; treat it as part of the use. In a pack, a card taken
    from the pack uses one choice: the pack stays open while choices remain and
    closes otherwise (`shop.md` 3.6). A card used from the consumable slots
    while a pack is open uses no choice.
11. Back in SELECTING_HAND with an empty hand (Hanged Man or Immolate took the
    last cards), the game draws up to hand size. If the deck is empty too, the
    round ends (`round.md` 4).

### 1.3 Order inside one use (what the engine must keep)

The player cannot act between steps 2 and 10, so a use can be resolved
atomically. Durations do not matter, but these orderings do:

1. **Draws.** Click-time draws happen before queued-step draws. Each card uses
   its own streams, so only the order within a stream matters (section 6). The
   Wheel of Fortune is the only card that draws one stream both at the click
   and in a step.
2. **Joker reactions see the state at the click**: selected cards still
   selected and unchanged, cards to be destroyed still in hand and not yet
   flagged as shattered (Glass Joker, Caino, 1.6).
3. **Pools are built when the creation step runs.** The used card is still
   present then (step 9 has not happened), so The Emperor never creates The
   Emperor (unless Showman is held). Cards on offer in the shop or in an open
   pack are present too and are excluded. Each created card is present for the
   next creation.
4. **Room checks** in creation steps see the used card already gone from its
   slot, but a used Negative card's extra slot is still counted. A Negative The
   Emperor, The High Priestess or The Fool can therefore leave you with more
   consumables than slots (3.5).
5. **The Fool** reads the last-used record before its own use updates it.
6. **Money**: The Hermit and Temperance read money when their step runs, which
   is after a Buy and Use payment (1.4).
7. **Hand-size changes** (Ouija, Ectoplasm) go through the hand's size-change
   step and land before control returns. They never discard cards (1.12).

Minor timing race, no outcome effect: the creation step of Familiar, Grim and
Incantation runs about 0.7 s after the destroy step, and the destroyed card's
removal about 0.735 s after it. Depending on frame timing either can come
first. The final hand, the deck list and its order are the same either way.

### 1.4 Where the used card comes from

- **Consumable slots.** As in 1.2. Such a card was added to your slots when it
  arrived, so its removal queues the boss debuff re-test (1.7). If it is
  Negative, its extra slot goes when it is removed.
- **Booster pack** (Arcana, Celestial, Spectral). The rules are the same, with
  the pack's own choice accounting (1.2 step 10). A pack card was never added
  to your slots, so its removal queues no boss re-test and changes no slot
  count. The pack's other cards stay present until the pack closes, so they are
  excluded from pools used by the effect. Cards changed, destroyed or created
  in the pack's dealt hand are permanent: at the close, the hand, including
  any new cards, goes back to the deck (`shop.md` 3.5).
- **Buy and Use** (consumables in the shop's joker row only; never pack
  cards).
  - The button is live when price ≤ money − debt floor (or price is 0) and the
    card is usable right now (1.1). No free consumable slot is needed. In the
    shop the hand is empty, so only the cards that need no hand cards qualify.
  - Order:
    1. the card leaves the shop row and is marked as added to your slots;
    2. every joker gets `buying_card`, to which no vanilla joker reacts;
    3. Inflation (challenge) +1, which does not change this card's price;
    4. the price is paid, queued, so the payment lands before any queued step
       of the effect;
    5. then 1.2 from step 1.
  - The card counts as purchased for run statistics only.
  - **Quirk, Ankh with full joker slots.** Its rule (1.1) does not check room,
    so the button is live. The price is paid, then 1.2 step 1 refuses. The card
    is now in no area: not in the shop and not in your slots. Its button
    remains and uses it later if pressed once a joker slot is free. The engine
    should not offer this move (UNSURE, not tried in game).

### 1.5 What is recorded

At step 4 of every use (also when the effect then does nothing, for example a
failed Wheel of Fortune):

- `G.GAME.consumeable_usage[key]`: a per-key count and the card's set (Tarot,
  Planet or Spectral).
- `G.GAME.consumeable_usage_total`: totals for tarot, planet, spectral,
  tarot + planet and all.
- `G.GAME.last_tarot_planet`: the key of the last Tarot or Planet used, The
  Fool included. Spectrals (The Soul and Black Hole included) never change it.
  It is set at step 8, after the card's own effect. It starts empty in every
  run.
- Profile statistics (no effect on the run).

Readers:

- **Fortune Teller**: +1 mult for each Tarot used this run, from any source
  (slots, packs, Buy and Use, The Fool itself). It is read live during
  scoring.
- **Satellite**: $1 at cash-out for each distinct Planet key used this run.
  Black Hole is a Spectral and does not count.
- **The Fool**: copies `last_tarot_planet`.
- **Constellation** does not read the record; it reacts to each use (1.6).

A use refused by Ankh's room check records nothing.

### 1.6 Joker reactions

Debuffed jokers never react. Every reaction below is marked "not copyable", so
Blueprint and Brainstorm add nothing.

- **`using_consumeable`**: every use, at the click (1.2 step 7), jokers left to
  right.
  - Constellation: +0.1 X mult if the used card is a Planet. Keep it as a
    running sum (`jokers_1_76.md` 1.6).
  - Glass Joker, The Hanged Man only: +0.75 X mult for each selected card whose
    enhancement is Glass at the click (debuffed ones included).
  - Fortune Teller: a message only.
- **`remove_playing_cards`**: The Hanged Man, Familiar, Grim, Incantation and
  Immolate, at the click, with the list of cards about to be destroyed.
  - Caino: +1 X mult for each listed card that is a face card at that moment
    (`scoring.md` 12.8: a non-debuffed J, Q or K, or any non-debuffed card
    while a non-debuffed Pareidolia is held).
  - Glass Joker counts listed cards already flagged as shattered. None are
    flagged yet (cards shatter in a later step), so it always gets 0 here.
    Glass Joker therefore gains nothing from Glass Cards destroyed by Familiar,
    Grim, Incantation or Immolate; it gains only from The Hanged Man, through
    `using_consumeable`.
- **`playing_card_added`**: in Familiar's, Grim's and Incantation's creation
  step after all their cards exist, and in Cryptid's step. Hologram gains
  +0.25 X mult per card added: 3, 2, 4 and 2 cards. A Hologram marked for
  destruction at blind start ignores it; that cannot happen during a use.
- **`selling_card`** (selling a consumable): Campfire +0.25 X mult.
- **Jokers created or destroyed.** Judgement, The Soul, Wraith and Ankh's copy
  run the new joker's add-to-deck effects at once: hand size, Credit Card's
  floor, Chaos the Clown's free reroll, Oops! All 6s doubling p, To the Moon,
  Astronomer repricing, a Negative edition's extra slot, and Chicot disabling
  the current boss. Jokers destroyed by Ankh and Hex run their remove-from-deck
  effects when they are removed.

### 1.7 Changing playing cards

- **Enhancement change** (The Magician, The Empress, The Hierophant, The
  Lovers, The Chariot, Justice, The Devil, The Tower).
  - The card stays the same object: same `sort_id`, same place in the deck
    list, same position in hand.
  - Its stored values are rebuilt from the new enhancement: bonus chips, mult,
    X mult, held X mult, Gold money, Lucky values (`scoring.md` 12.3).
  - Kept: Hiker's perma bonus and a Cerulean Bell forced mark.
  - Lost: the played-this-ante mark read by The Pillar.
  - Unchanged: rank, suit, edition, seal.
  - Giving a card the enhancement it already has still rebuilds it.
- **Rank or suit change** (Strength, The Star, The Moon, The Sun, The World,
  Sigil, Ouija).
  - Same object. Only rank and suit are replaced. Enhancement, edition, seal
    and every stored mark (perma bonus, played-this-ante) stay.
  - The card keeps its original-suit sort term (`round.md` 4), so after a draw
    it still sorts as a card of its creation suit among equal cards.
  - Stone Cards change too, invisibly: their hidden rank and suit change.
- **Seal** (Talisman, Deja Vu, Trance, Medium) and **edition** (Aura) are
  replaced outright.
- **Boss debuff re-test.** After an enhancement or rank/suit change, the
  boss's card-debuff test runs on that card at once (`blinds.md` 10). Examples:
  - a Club made Wild under The Club becomes debuffed;
  - a Club made Stone under The Club stops being debuffed;
  - a card re-enhanced under The Pillar loses its played-this-ante mark and
    stops being debuffed;
  - under Verdant Leaf the card stays debuffed.
  Seal and edition changes run no test. With no boss active (shop, blind
  select) the test finds nothing.
- Separately, removing a used card that had been added to your slots re-tests
  every playing card against the boss (1.2 step 9). That is how cards created
  by Familiar, Grim and Incantation during a boss round get the boss's debuff.
  At creation no test is run.
- **Face-down cards.** A card's flips during the effect are cosmetic. A
  face-down card is turned up and back down, so it ends face down.
- **Selection afterwards.** The Tarots that change selected cards and the four
  seal Spectrals clear the selection at the end, keeping a Cerulean Bell card.
  The Hanged Man's selected cards are destroyed. Aura, Cryptid and the
  whole-hand Spectrals leave the selection as it was, minus destroyed cards.

### 1.8 Destroying playing cards

- Who: The Hanged Man (the selected cards), Familiar, Grim and Incantation (1
  random hand card), Immolate (up to 5 random hand cards).
- A card whose enhancement is Glass at the destroy step shatters (it is
  flagged as shattered); any other card dissolves. Either way it leaves the
  hand and the full deck list for good, before control returns.
- Joker hooks: only `remove_playing_cards` at the click (1.6). There is no
  Glass-shatter hook here.

### 1.9 Creating cards

- **Consumables**: The Emperor (`emp`), The High Priestess (`pri`), The Fool
  (`fool`, forced key).
  - The general creator (`shop.md` 2.2) without the Soul check, so never The
    Soul or Black Hole and no `soul_` draws.
  - No edition. Placed at the right end of the consumable slots and added to
    your slots.
- **Jokers**: Judgement (`jud`), Wraith (`wra`, fixed Rare), The Soul (`sou`,
  legendary).
  - The general creator with the joker slots as destination: no
    Eternal/Perishable/Rental rolls; the challenge rule "all eternal" still
    applies.
  - One edition roll `edi<append><A>` at the run's edition rate, Negative
    possible (`shop.md` 2.2 step 7c).
  - A To Do List draws `to_do` when it is made.
  - Placed at the right end of the jokers, then the add-to-deck effects run
    (1.6).
- **Playing cards**: Familiar, Grim, Incantation.
  - Brand-new cards with new `sort_id`s, appended to the full deck list.
  - Placed at the right end of the hand in creation order; the hand is not
    re-sorted until the next draw.
  - Always enhanced (never Stone, never plain). No edition, no seal.
- **Copies**: Cryptid (playing cards) and Ankh (a joker) copy everything stored
  on the original (3.14, 5.16, 5.11). Each copy gets a new `sort_id`.

### 1.10 Selling a consumable

- Allowed for cards in the consumable slots, in any state, whenever the gates
  of 1.1 allow (no lock, stop-use 0, play area empty). This includes while a
  pack is open and during hand selection. Pack cards and shop cards cannot be
  sold.
- Pays the stored sell value: max(1, floor(price / 2)) + bonus. The price
  follows `shop.md` 5.1, and the bonus comes from Gift Card (+$1 per round).
  The value is recomputed only at the moments listed in `shop.md` 5.4.
- Jokers get `selling_card` (Campfire). The card dissolves; on removal its
  present mark is cleared and a Negative card's slot is removed.

| Card | No discount | Clearance Sale | Liquidation |
|---|---|---|---|
| Tarot or Planet | $1 | $1 | $1 |
| Spectral | $2 | $1 | $1 |
| Planet (Negative or not) while a non-debuffed Astronomer is held (price 0) | $1 | $1 | $1 |
| Negative Tarot or Planet | $4 | $3 | $2 |
| Negative Spectral | $4 | $3 | $2 |

All plus any Gift Card bonus. A Negative copy also keeps the original's bonus.
The Negative rows include the +5 edition surcharge (for example
floor((3 + 5 + 0.5) × 0.75) = 6, so it sells for $3 under Clearance Sale).

### 1.11 Negative consumables

- In vanilla only Perkeo makes them (copies made when leaving the shop).
- While held, a Negative consumable adds 1 consumable slot. The slot is added
  when it enters the slots and removed when it is removed (sold, or used and
  dissolved).
- When used, its extra slot still counts during its own effect (1.3 point 4).
- Sell value: 1.10.

### 1.12 Hand-size changes

- Ouija (−1) and Ectoplasm (−1, −2, −3, ...: 5.9) lower hand size for the rest
  of the run.
- The change is queued and applied before control returns. It never discards
  cards: you keep what you hold, and later draws fill up to the new size.
- The game keeps a "real" size that may go below 0; the effective size is
  max(0, real). Later increases add to the real value.
- Outside packs, an attempt to draw with an effective size of 0 and an empty
  hand ends the run (`round.md`).

### 1.13 Debuffed consumables

The code skips the effect of a debuffed consumable (its use is still recorded,
the jokers still get `using_consumeable`), and a debuffed Planet gives no
Observatory bonus. Nothing in vanilla debuffs a consumable. The boss's card
test runs on playing cards and jokers, and on a consumable only when one is
rebuilt by copying (Perkeo's copies). Those copies are made in the shop, when
no boss is active. The engine can ignore this.

## 2. Data

All consumables are unlocked from the start (no profile locks). Base cost:
Tarot 3, Planet 3, Spectral 4; the price is from `shop.md` 5.1. "Order" is the
pool order within the set (`shop.md` 2.7). Keys keep the game's spelling
(`c_heirophant`).

### 2.1 Tarot cards

| Order | Key | Name | Config | Selected cards |
|---|---|---|---|---|
| 1 | `c_fool` | The Fool | none | none |
| 2 | `c_magician` | The Magician | becomes `m_lucky` | 1-2 |
| 3 | `c_high_priestess` | The High Priestess | planets 2 | none |
| 4 | `c_empress` | The Empress | becomes `m_mult` | 1-2 |
| 5 | `c_emperor` | The Emperor | tarots 2 | none |
| 6 | `c_heirophant` | The Hierophant | becomes `m_bonus` | 1-2 |
| 7 | `c_lovers` | The Lovers | becomes `m_wild` | 1 |
| 8 | `c_chariot` | The Chariot | becomes `m_steel` | 1 |
| 9 | `c_justice` | Justice | becomes `m_glass` | 1 |
| 10 | `c_hermit` | The Hermit | extra 20 | none |
| 11 | `c_wheel_of_fortune` | The Wheel of Fortune | extra 4 | none |
| 12 | `c_strength` | Strength | rank up | 1-2 |
| 13 | `c_hanged_man` | The Hanged Man | destroys | 1-2 |
| 14 | `c_death` | Death | copy | exactly 2 |
| 15 | `c_temperance` | Temperance | extra 50 | none |
| 16 | `c_devil` | The Devil | becomes `m_gold` | 1 |
| 17 | `c_tower` | The Tower | becomes `m_stone` | 1 |
| 18 | `c_star` | The Star | suit Diamonds | 1-3 |
| 19 | `c_moon` | The Moon | suit Clubs | 1-3 |
| 20 | `c_sun` | The Sun | suit Hearts | 1-3 |
| 21 | `c_judgement` | Judgement | none | none |
| 22 | `c_world` | The World | suit Spades | 1-3 |

### 2.2 Planet cards

Each levels one hand by +1. The per-level gains are from `hands.md` 6.1.

| Order | Key | Name | Hand | +chips | +mult | In pools only after the hand is played |
|---|---|---|---|---|---|---|
| 1 | `c_mercury` | Mercury | Pair | 15 | 1 | no |
| 2 | `c_venus` | Venus | Three of a Kind | 20 | 2 | no |
| 3 | `c_earth` | Earth | Full House | 25 | 2 | no |
| 4 | `c_mars` | Mars | Four of a Kind | 30 | 3 | no |
| 5 | `c_jupiter` | Jupiter | Flush | 15 | 2 | no |
| 6 | `c_saturn` | Saturn | Straight | 30 | 3 | no |
| 7 | `c_uranus` | Uranus | Two Pair | 20 | 1 | no |
| 8 | `c_neptune` | Neptune | Straight Flush | 40 | 4 | no |
| 9 | `c_pluto` | Pluto | High Card | 10 | 1 | no |
| 10 | `c_planet_x` | Planet X | Five of a Kind | 35 | 3 | yes |
| 11 | `c_ceres` | Ceres | Flush House | 40 | 4 | yes |
| 12 | `c_eris` | Eris | Flush Five | 50 | 3 | yes |

### 2.3 Spectral cards

| Order | Key | Name | Config | Use needs |
|---|---|---|---|---|
| 1 | `c_familiar` | Familiar | destroys 1, makes 3 | 2+ cards in hand |
| 2 | `c_grim` | Grim | destroys 1, makes 2 | 2+ cards in hand |
| 3 | `c_incantation` | Incantation | destroys 1, makes 4 | 2+ cards in hand |
| 4 | `c_talisman` | Talisman | seal Gold | 1 selected |
| 5 | `c_aura` | Aura | none | 1 selected, no edition |
| 6 | `c_wraith` | Wraith | none | joker room |
| 7 | `c_sigil` | Sigil | none | 2+ cards in hand |
| 8 | `c_ouija` | Ouija | none | 2+ cards in hand |
| 9 | `c_ectoplasm` | Ectoplasm | none | joker without edition |
| 10 | `c_immolate` | Immolate | destroy 5, dollars 20 | 2+ cards in hand |
| 11 | `c_ankh` | Ankh | extra 2 (unused) | see 1.1 |
| 12 | `c_deja_vu` | Deja Vu | seal Red | 1 selected |
| 13 | `c_hex` | Hex | extra 2 (unused) | joker without edition |
| 14 | `c_trance` | Trance | seal Blue | 1 selected |
| 15 | `c_medium` | Medium | seal Purple | 1 selected |
| 16 | `c_cryptid` | Cryptid | 2 copies | 1 selected |
| 17 | `c_soul` | The Soul | none | joker room |
| 18 | `c_black_hole` | Black Hole | none | always |

The Soul and Black Hole sit in the Spectral pool as permanent placeholders and
appear only through the Soul check (`shop.md` 2.2 step 1).

## 3. Tarot cards

### 3.1 The Fool (`c_fool`)

- **Use**: Room (held < slots, or used from the slots), and a last-used Tarot
  or Planet is recorded and it is not The Fool.
- **Effect**, one queued step: if slots > held (the used Fool no longer counts;
  a Negative Fool's extra slot still does), create the recorded card into the
  consumable slots. If not, nothing is made.
- **Draws**: none. The card comes by forced key with append `fool`. There is
  no duplicate check: a copy of a held card can be made. Planet X, Ceres and
  Eris can be made even if their hand was never played; in practice they must
  have been used before.
- **After**: the record becomes The Fool, so another Fool cannot be used until
  some other Tarot or Planet has been used. Spectrals never become the target.
  At run start nothing is recorded, so the Magic Deck's two Fools are unusable
  until a Tarot or Planet has been used.

### 3.2 The Magician (`c_magician`)

1-2 selected. Queued: each selected card becomes a Lucky Card (1.7).

### 3.3 The High Priestess (`c_high_priestess`)

- **Use**: Room.
- **Effect**: at the click, n = min(2, slots − held) is computed (the used
  card no longer counts as held). n queued steps follow. Each step creates one
  Planet if slots > held at that moment.
- **Draws**: each creation picks with resample from `Planetpri<A>` (then
  `Planetpri<A>_resample2`, ...). The pool is built at that step:
  - Planets present are excluded: held, in the shop, in an open pack, or made
    by the first step.
  - Planet X, Ceres and Eris are excluded until their hand has been played.
  - No Soul check, so never Black Hole.

### 3.4 The Empress (`c_empress`)

1-2 selected. Queued: each becomes a Mult Card.

### 3.5 The Emperor (`c_emperor`)

- **Use**: Room.
- **Effect**: as The High Priestess, with Tarots: n = min(2, slots − held), and
  each step re-checks room.
- **Draws**: each creation picks with resample from `Tarotemp<A>`. The pool
  excludes present Tarots, The Emperor itself included (still present during
  its own effect, 1.3), unless Showman is held. No Soul check, so never The
  Soul.
- **Negative quirk.** With 2 slots, holding a Negative Emperor (3 slots in
  total) and one other card: at the click held = 1, so n = 2, and both steps
  see 3 > held. Two Tarots are made, then the Emperor's removal drops the slots
  to 2, leaving 3 cards in 2 slots. The same applies to The High Priestess and
  The Fool (one card). UNSURE: from the code, not tried in game.

### 3.6 The Hierophant (`c_heirophant`)

1-2 selected. Queued: each becomes a Bonus Card.

### 3.7 The Lovers (`c_lovers`)

Exactly 1 selected. Queued: it becomes a Wild Card. The boss re-test follows
(under a suit boss a Wild card is debuffed; `scoring.md` 11.3).

### 3.8 The Chariot (`c_chariot`)

Exactly 1 selected. Queued: it becomes a Steel Card.

### 3.9 Justice (`c_justice`)

Exactly 1 selected. Queued: it becomes a Glass Card.

### 3.10 The Hermit (`c_hermit`)

- **Use**: always.
- **Effect**, one queued step: gain max(0, min(money, 20)), with money read at
  that step. That is +money capped at $20, and nothing when money is $0 or in
  debt.
- After Buy and Use, the price has already been paid: with $10, buying and
  using it for $3 leaves $7 and gives +$7.

### 3.11 The Wheel of Fortune (`c_wheel_of_fortune`)

- **Use**: at least one joker without an edition.
- **At the click**: roll `wheel_of_fortune`. Success if the value is strictly
  below p / 4.
- **Failure**: a "Nope!" message; nothing else. The use is still recorded.
- **Success**, one queued step:
  1. Pick from `wheel_of_fortune` (its second draw) over the jokers with no
     edition, in `sort_id` order. Debuffed jokers are included, in any
     position in the joker row.
  2. Roll `wheel_of_fortune` again (third draw) for a guaranteed edition, never
     Negative: Polychrome if the value is above 0.85, Holographic if above 0.5,
     Foil if above 0, and none if exactly 0 (chance 2^-52). The thresholds are
     the f64 values of 1 − 0.006 × 25, 1 − 0.02 × 25 and 1 − 0.04 × 25, which
     are exactly 0.85, 0.5 and 0. Hone and Glow Up do not apply.
  3. The joker gets that edition.
- So the stream advances once on failure and three times on success. The
  eligible list is the one refreshed every frame (`timing.md` 7); the engine
  computes it at the step.

### 3.12 Strength (`c_strength`)

- 1-2 selected. Queued: each selected card's rank goes up by one, suit kept: 2
  → 3, ..., 9 → 10, 10 → J, J → Q, Q → K, K → A, A → 2.
- It reads the printed rank, so a Stone Card's hidden rank goes up too.

### 3.13 The Hanged Man (`c_hanged_man`)

- **Use**: 1-2 selected.
- **At the click**: the destroy list is the selected cards (in reverse click
  order; the order has no effect). Jokers get `remove_playing_cards` (Caino),
  then `using_consumeable` (Glass Joker +0.75 per selected Glass Card).
- **Queued**: each selected card is destroyed (1.8): Glass Cards shatter,
  others dissolve.
- If this empties the hand during a round, a refill draw follows (1.2 step 11).

### 3.14 Death (`c_death`)

- **Use**: exactly 2 selected.
- **At the click**: the "rightmost" card is chosen by on-screen x position,
  i.e. hand position. Start from the first-clicked card and replace it by any
  selected card strictly further right.
- **Queued**: every other selected card (with 2 selected, the left one)
  becomes a full copy of the rightmost card. It takes its enhancement, rank,
  suit, edition (or loses its own if the right card has none), seal (or none),
  every stored value and mark (Hiker's perma bonus, played-this-ante, a
  Cerulean Bell forced mark), and its debuff state.
- It keeps its own identity: `sort_id`, place in the deck list, hand position,
  and its original-suit sort term (`round.md` 4).
- If the right card carries the Cerulean Bell mark, the copy gets it too, so
  two cards can be forced. UNSURE: from the code, not tried in game.
- No draws.

### 3.15 Temperance (`c_temperance`)

- **Use**: always.
- **Effect**, one queued step: gain min(50, the sum of the stored sell values
  of all jokers in the joker area). Debuffed and Eternal jokers count; a
  Rental joker counts $1 plus its bonus.
- The game refreshes this amount every frame. The engine computes it at the
  step (after a Buy and Use payment, before anything the use itself changes;
  Temperance changes no jokers). It can be $0.

### 3.16 The Devil (`c_devil`)

Exactly 1 selected. Queued: it becomes a Gold Card.

### 3.17 The Tower (`c_tower`)

Exactly 1 selected. Queued: it becomes a Stone Card. Its rank and suit stay
stored (Strength, Sigil, Ouija and Death still act on them), but for scoring it
has none (`scoring.md` 12).

### 3.18 The Star (`c_star`)

1-3 selected. Queued: each selected card becomes a Diamond, rank kept (1.7).
Wild and Stone Cards change their printed suit too; a Wild card still counts as
every suit and a Stone Card as none. No draws. The same holds for 3.19, 3.20
and 3.22.

### 3.19 The Moon (`c_moon`)

As The Star, with Clubs.

### 3.20 The Sun (`c_sun`)

As The Star, with Hearts.

### 3.21 Judgement (`c_judgement`)

- **Use**: held jokers < joker slots.
- **Effect**, one queued step: create a joker into the joker slots, with no
  room re-check. Then the add-to-deck effects run.
- **Draws**, in order:
  1. `rarity<A>jud` (above 0.95 Rare, above 0.7 Uncommon, else Common);
  2. pick with resample from `Joker<r>jud<A>`, as in `Joker1jud3` and
     `Joker1jud3_resample2`;
  3. `to_do` if the joker is a To Do List;
  4. `edijud<A>` for the edition.
- The pool excludes present jokers: held ones, the shop row, an open Buffoon
  pack. The enhancement-gated jokers (Steel Joker, Stone Joker, Lucky Cat,
  Golden Ticket, Glass Joker) need such a card in the full deck at that
  moment. Legendaries never come from it.

### 3.22 The World (`c_world`)

As The Star, with Spades.

## 4. Planet cards

Every Planet follows the same rules:

- **Use**: always (any state that passes the gates of 1.1), including Planet
  X, Ceres and Eris whose hand was never played.
- **Effect**, at the click: its hand gains one level (`hands.md` 6.2). New
  level = level + 1; chips = base + per-level chips × (level − 1); mult =
  base + per-level mult × (level − 1). The level change is immediate; the
  hand-level display catches up later. A hidden hand levels too and stays
  hidden until played.
- **Draws**: none.
- **Jokers**: Constellation +0.1 X mult. **Records**: planet count, Satellite's
  distinct-Planet record, last used for The Fool.
- **While held**: with Observatory, each held non-debuffed Planet whose hand
  is the scoring hand gives X1.5 in the joker loop. Two copies give X1.5
  twice. Negative copies count. See `scoring.md` 6.
- **Price**: $3, or $0 in the shop while a non-debuffed Astronomer is held.
  The sell value is in 1.10.
- **Where they come from**: the shop row, Celestial packs (including
  Telescope's forced first card), The High Priestess, The Fool and the Blue
  Seal (`shop.md` 2 and 3.4, seal notes).

Each Planet below levels exactly one hand; the base values and the full
formula are in `hands.md` 6.

### 4.1 Mercury (`c_mercury`)

Pair: +15 chips, +1 mult per level.

### 4.2 Venus (`c_venus`)

Three of a Kind: +20 chips, +2 mult.

### 4.3 Earth (`c_earth`)

Full House: +25 chips, +2 mult.

### 4.4 Mars (`c_mars`)

Four of a Kind: +30 chips, +3 mult.

### 4.5 Jupiter (`c_jupiter`)

Flush: +15 chips, +2 mult.

### 4.6 Saturn (`c_saturn`)

Straight: +30 chips, +3 mult.

### 4.7 Uranus (`c_uranus`)

Two Pair: +20 chips, +1 mult.

### 4.8 Neptune (`c_neptune`)

Straight Flush: +40 chips, +4 mult.

### 4.9 Pluto (`c_pluto`)

High Card: +10 chips, +1 mult. Also the fallback when a Planet pool has no
available entry (`shop.md` 2.3).

### 4.10 Planet X (`c_planet_x`)

Five of a Kind: +35 chips, +3 mult. In pools only once Five of a Kind has
been played this run; usable at any time.

### 4.11 Ceres (`c_ceres`)

Flush House: +40 chips, +4 mult. Pool rule as Planet X, for Flush House.

### 4.12 Eris (`c_eris`)

Flush Five: +50 chips, +3 mult. Pool rule as Planet X, for Flush Five.

## 5. Spectral cards

### 5.1 Familiar (`c_familiar`)

Familiar, Grim and Incantation work the same way and differ only in the cards
they make. This section gives the shared rules; 5.2 and 5.3 give the
differences.

- **Use**: at least 2 cards in hand (selection ignored).
- **At the click**:
  1. Pick the victim from `random_destroy` over all hand cards in `sort_id`
     order. Selected, debuffed and face-down cards are all candidates; hand
     position does not matter. This stream is shared by all three cards.
  2. Jokers get `remove_playing_cards` with that one card (Caino), then
     `using_consumeable`.
- **Queued**, in order:
  1. The victim is destroyed (shatters if it is a Glass Card).
  2. About 0.7 s later, N new cards are made one at a time (Familiar: N = 3).
     For each card, in this order:
     - **rank**: Familiar picks from `familiar_create` over `J Q K`;
     - **suit**: picks from the same stream over `S H D C` (Spades, Hearts,
       Diamonds, Clubs);
     - **enhancement**: picks from `spe_card` over the 7-item list Bonus,
       Mult, Wild, Glass, Steel, Gold, Lucky (the Enhanced pool in order,
       without Stone);
     - **the card is created** into the hand, at the right end (1.9).
  3. Jokers get `playing_card_added` with N cards (Hologram +0.25 × N).
- **Stream order**: `familiar_create` gives rank 1, suit 1, rank 2, suit 2,
  rank 3, suit 3; `spe_card` gives enhancements 1, 2, 3.
- The new cards get the current boss's debuff test only when the used card is
  removed (1.7).

### 5.2 Grim (`c_grim`)

As Familiar, with N = 2 and every new card an Ace (no rank draw). The suit of
each card is picked from `grim_create` over `S H D C`, then its enhancement
from `spe_card`. Stream order: `grim_create` suit 1, suit 2; `spe_card` 1, 2.

### 5.3 Incantation (`c_incantation`)

As Familiar, with N = 4. Each card's rank is picked from `incantation_create`
over `2 3 4 5 6 7 8 9 10`, then its suit from the same stream over `S H D C`,
then its enhancement from `spe_card`. Stream order: `incantation_create` gives
8 draws (rank, suit, per card); `spe_card` gives 4.

### 5.4 Talisman (`c_talisman`)

Exactly 1 selected (the card is fixed at the click). Queued: it gets a Gold
Seal, replacing any seal. Then the selection is cleared. No draws.

### 5.5 Aura (`c_aura`)

- **Use**: exactly 1 hand card selected, with no edition.
- **Queued**: roll `aura` for a guaranteed edition with the Wheel's
  thresholds (3.11): Polychrome above 0.85, Holographic above 0.5, Foil above
  0. Never Negative, and Hone and Glow Up do not apply. The selected card
  (read at this step; it cannot have changed) gets it. The card stays
  selected.
- For playing-card edition values see `scoring.md` 12.4.

### 5.6 Wraith (`c_wraith`)

- **Use**: held jokers < joker slots.
- **Queued**, one step:
  1. Create a Rare joker. The rarity is fixed (value 0.99), so no `rarity`
     draw. Pick with resample from `Joker3wra<A>`, then `to_do` if it is a To
     Do List, then the edition from `ediwra<A>`.
  2. Add it at the right end and run its add-to-deck effects.
  3. If money is not $0, it is set to $0 at once. That is a loss from
     positive money, or a gain from debt.
- After Buy and Use, the price is paid before money is zeroed.

### 5.7 Sigil (`c_sigil`)

- **Use**: at least 2 cards in hand.
- **At the click**: pick the suit from `sigil` over `S H D C`.
- **Queued**: every hand card at the click becomes that suit, rank kept
  (1.7). Wild and Stone Cards included. Cards in the deck are not touched.

### 5.8 Ouija (`c_ouija`)

- **Use**: at least 2 cards in hand.
- **At the click**: pick the rank from `ouija` over
  `2 3 4 5 6 7 8 9 10 J Q K A`.
- **Queued**: every hand card becomes that rank, suit kept. Then hand size −1
  for the rest of the run (1.12).

### 5.9 Ectoplasm (`c_ectoplasm`)

- **Use**: at least one joker without an edition.
- **Queued**, one step:
  1. Pick from `ectoplasm` over the jokers with no edition, in `sort_id` order.
  2. That joker becomes Negative. If it is not debuffed it adds a joker slot
     at once.
  3. Hand size drops by k, where k is a run counter that starts at 1 and goes
     up by 1 after each Ectoplasm: the first use gives −1, the second −2, the
     third −3. The card's description shows the current k.
- **Debuffed target (quirk).** If the chosen joker is debuffed (Crimson Heart,
  or an expired Perishable), no slot is added, and none is added when the
  debuff ends. If it later leaves the jokers while not debuffed (sold,
  destroyed), a slot is removed. The net effect is a permanently lost joker
  slot. A permanently debuffed (expired) joker removes nothing when it leaves.
  UNSURE: from the code, not tried in game.

### 5.10 Immolate (`c_immolate`)

- **Use**: at least 2 cards in hand.
- **At the click**:
  1. Take the hand cards, put them in `sort_id` order, and shuffle with
     `immolate` (one Fisher-Yates pass from the end).
  2. The first min(5, cards in hand) cards of the shuffled list are the
     victims. With 5 or fewer cards in hand, all of them, but the shuffle is
     still drawn.
  3. Jokers get `remove_playing_cards` (Caino), then `using_consumeable`.
- **Queued**: the victims are destroyed (Glass Cards shatter); then +$20
  (queued money).
- An emptied hand during a round triggers a refill draw (1.2 step 11).

### 5.11 Ankh (`c_ankh`)

- **Use**: at least one joker and joker slots > 1; at the click also held
  jokers < joker slots, otherwise nothing happens (1.2 step 1, and the Buy and
  Use quirk in 1.4).
- **At the click**:
  1. The doomed list = every joker that is not Eternal.
  2. Pick the original from `ankh_choice` over all jokers in `sort_id` order.
     Eternal, debuffed and Negative jokers can be picked.
- **Queued**:
  1. Every joker on the doomed list except the original dissolves. They are
     removed, with their remove-from-deck effects, before the next step.
  2. A copy of the original is made: everything stored is copied (sell bonus,
     Eternal/Perishable/Rental and rounds left, scaling values, To Do List
     target, debuff state). A Negative edition is not copied; any other
     edition is. Making the copy runs creation first, so copying a To Do List
     draws `to_do` once (result discarded) (`jokers_1_76.md` 1.7). The copy is
     added at the right end and its add-to-deck effects run, even if it
     carries a debuff copied from the original (UNSURE).
- Result: the original, its copy and every Eternal joker remain.

### 5.12 Deja Vu (`c_deja_vu`)

As Talisman, with a Red Seal.

### 5.13 Hex (`c_hex`)

- **Use**: at least one joker without an edition.
- **Queued**, one step:
  1. Pick from `hex` over the jokers with no edition, in `sort_id` order.
  2. It becomes Polychrome.
  3. Every other joker that is not Eternal dissolves, whatever its edition and
     debuff state. Their remove-from-deck effects run when they are removed.

### 5.14 Trance (`c_trance`)

As Talisman, with a Blue Seal.

### 5.15 Medium (`c_medium`)

As Talisman, with a Purple Seal.

### 5.16 Cryptid (`c_cryptid`)

- **Use**: exactly 1 selected.
- **Queued**, one step: two copies of the selected card are made. Each one:
  - is a new card object (new `sort_id`, new deck index);
  - copies the enhancement, rank, suit, edition and seal, every stored value
    and mark (Hiker's perma bonus, played-this-ante, a Cerulean Bell forced
    mark), and the debuff state;
  - is added to the full deck list, placed at the right end of the hand, and
    marked as added to the deck.
  Then jokers get `playing_card_added` with 2 cards (Hologram +0.5).
- The copies' original-suit sort term is their current suit (unlike Death).
- The original stays selected. No draws.

### 5.17 The Soul (`c_soul`)

- **Use**: held jokers < joker slots.
- **Queued**, one step: create a legendary joker into the joker slots.
- **Draws**:
  1. `rarity<A>sou` (drawn and ignored);
  2. pick with resample from `Joker4` over Caino, Triboulet, Yorick, Chicot
     and Perkeo, in that order. There is no append and no ante; a held
     legendary is excluded unless Showman is held, and resamples use
     `Joker4_resample2`, ...;
  3. `edisou<A>` for the edition (Negative possible).
- If all five are present, the fallback is Joker (`j_joker`), still drawn
  from `Joker4`.
- A Chicot made during a boss blind disables the boss at once.
- **Where it comes from**: The Soul check in Arcana and Spectral packs
  (`soul_Tarot<A>`, `soul_Spectral<A>`; `shop.md` 3.4), never in the shop or
  from jokers. Taken from an Arcana pack it is still a Spectral (recorded as
  one; not a target for The Fool).

### 5.18 Black Hole (`c_black_hole`)

- **Use**: always.
- **At the click**: every one of the 12 hands gains one level, hidden ones
  included (they stay hidden until played). No draws.
- It is a Spectral: no Constellation gain, not counted by Satellite, not
  recorded for The Fool, no Observatory bonus while held.
- **Where it comes from**: the Black Hole check in Celestial and Spectral
  packs and in Arcana-pack Spectral cards (`soul_Planet<A>`,
  `soul_Spectral<A>`; `shop.md` 3.4).

## 6. Random streams drawn by consumables

| Card | Stream (in draw order) | Kind | When |
|---|---|---|---|
| The Wheel of Fortune | `wheel_of_fortune` ×1, or ×3 on success | roll; then pick over editionless jokers (`sort_id`); then guaranteed edition roll | roll at the click; pick and edition in its step |
| The Emperor | `Tarotemp<A>` (+ `_resample<n>`) | pool pick | each creation step, up to 2 |
| The High Priestess | `Planetpri<A>` (+ `_resample<n>`) | pool pick | each creation step, up to 2 |
| Judgement | `rarity<A>jud`, `Joker<r>jud<A>` (+ resamples), [`to_do`], `edijud<A>` | joker creation | its step |
| Familiar | `random_destroy`; `familiar_create` ×6; `spe_card` ×3 | picks | victim at the click; the rest in the creation step |
| Grim | `random_destroy`; `grim_create` ×2; `spe_card` ×2 | picks | same |
| Incantation | `random_destroy`; `incantation_create` ×8; `spe_card` ×4 | picks | same |
| Aura | `aura` | guaranteed edition roll | its step |
| Wraith | `Joker3wra<A>` (+ resamples), [`to_do`], `ediwra<A>` | joker creation | its step |
| Sigil | `sigil` | pick over `S H D C` | at the click |
| Ouija | `ouija` | pick over `2`..`A` | at the click |
| Ectoplasm | `ectoplasm` | pick over editionless jokers (`sort_id`) | its step |
| Immolate | `immolate` | shuffle of the hand (`sort_id` first) | at the click |
| Ankh | `ankh_choice`, [`to_do` if the original is a To Do List] | pick over all jokers (`sort_id`) | pick at the click; `to_do` in the copy step |
| Hex | `hex` | pick over editionless jokers (`sort_id`) | its step |
| The Soul | `rarity<A>sou` (ignored), `Joker4` (+ `Joker4_resample<n>`), `edisou<A>` | joker creation | its step |

No draws: The Fool (forced key), The Magician, The Empress, The Hierophant, The
Lovers, The Chariot, Justice, The Hermit, Strength, The Hanged Man, Death,
Temperance, The Devil, The Tower, The Star, The Moon, The Sun, The World, all
Planets, Talisman, Deja Vu, Trance, Medium, Cryptid, Black Hole. A refused Ankh
draws nothing. A Wheel of Fortune that fails draws once.

Notes on the other files (checked against the scripts):

- `rng_keys.md` 3.8 to 3.10 and section 2 agree with this note. One precision
  for 3.5: the guaranteed edition (Aura, Wheel of Fortune) is Foil only above
  0; a value of exactly 0 gives no edition.
- `shop.md` 2.4 says a used card's removal "can be treated as immediate". That
  holds for later player actions, but not inside the card's own use: during
  its effect the used card is still present and a Negative used card's extra
  slot still counts (1.3 points 3 and 4).

## UNSURE

- Buy and Use on Ankh with full joker slots: the price is paid and the card is
  left in no area, usable later from its leftover button (1.4). From the code,
  not tried in game.
- Negative The Emperor, The High Priestess or The Fool overfilling the
  consumable slots (3.5). From the code.
- Ectoplasm on a debuffed joker losing a joker slot for good (5.9). From the
  code.
- Ankh copying a debuffed joker: the copy is debuffed but its add-to-deck
  effects (hand size and the like) still apply (5.11). From the code.
- Death or Cryptid copying a Cerulean Bell forced mark, leaving two forced
  cards (3.14, 5.16). From the code.
- Glass Joker gaining nothing from Glass Cards destroyed by Familiar, Grim,
  Incantation or Immolate (1.6), and The Emperor never creating The Emperor
  (3.5). Both follow directly from the code; neither has been tried in game.
- The boss debuff re-test after a use lands one frame after control returns
  (1.2 step 10). The engine treats it as part of the use; a player cannot act
  in a single frame.

## Where in the game

- `card.lua`: `Card:use_consumeable`, `Card:can_use_consumeable`,
  `Card:check_use`, `Card:set_ability`, `Card:set_base`, `Card:change_suit`,
  `Card:set_seal`, `Card:set_edition`, `Card:set_cost`, `Card:add_to_deck`,
  `Card:remove_from_deck`, `Card:remove`, `Card:update` (selection cap,
  Temperance amount, eligible-joker lists), `Card:calculate_joker` (Planet
  Observatory branch; contexts `using_consumeable`, `remove_playing_cards`,
  `playing_card_added`, `selling_card`, `buying_card`), `Card:sell_card`,
  `Card:can_sell_card`, `Card:calculate_dollar_bonus` (Satellite),
  `Card:shatter`, `Card:start_dissolve`, `Card:flip`, `Card:is_face`,
  `Card:get_nominal`, `Card:open`
- `functions/button_callbacks.lua`: `G.FUNCS.use_card`,
  `G.FUNCS.can_use_consumeable`, `G.FUNCS.can_buy_and_use`,
  `G.FUNCS.buy_from_shop`, `G.FUNCS.sell_card`, `G.FUNCS.end_consumeable`
- `functions/common_events.lua`: `create_card`, `get_current_pool`,
  `poll_edition`, `copy_card`, `create_playing_card`, `level_up_hand`,
  `ease_dollars`, `draw_card`, `generate_card_ui` (Ectoplasm's shown amount)
- `functions/misc_functions.lua`: `set_consumeable_usage`, `stop_use`,
  `dec_stop_use`, `playing_card_joker_effects`, `pseudorandom`, `pseudoseed`,
  `pseudorandom_element`, `pseudoshuffle`, `find_joker`
- `functions/state_events.lua`: `G.FUNCS.draw_from_deck_to_hand` (hand size 0),
  `G.FUNCS.draw_from_hand_to_deck`
- `functions/UI_definitions.lua`: `G.UIDEF.use_and_sell_buttons`,
  `G.UIDEF.card_focus_ui`, `create_shop_card_ui` (Buy and Use button)
- `cardarea.lua`: `CardArea:emplace`, `CardArea:remove_card`,
  `CardArea:change_size`, `CardArea:add_to_highlighted`,
  `CardArea:unhighlight_all`, `CardArea:align_cards`, `CardArea:sort`
- `blind.lua`: `Blind:debuff_card`, `Blind:set_blind` (reset mode),
  `Blind:defeat`
- `back.lua`: `Back:apply_to_run` (Magic and Ghost Deck starting consumables)
- `game.lua`: consumable, enhancement, edition and voucher definitions and pool
  sorting in `Game:init_item_prototypes`; `Game:init_game_object` (usage
  tables, `last_tarot_planet`, `ecto_minus`, hands table); `Game:update`
  (empty-hand redraw), `Game:update_selecting_hand`, `Game:update_play_tarot`
- `engine/event.lua`: `Event:handle`, `EventManager:add_event`,
  `EventManager:update`
