# Timing and legality of player actions

The game runs as a state machine (blind select, hand selection, hand played,
drawing, end of round, cash-out screen, shop, open pack, consumable in use)
plus a single ordered queue of small timed steps that carry out every action.
What the player may do at a given moment is decided by four gates: which
buttons exist in the current state, an input lock that blocks all mouse
presses while certain animations run (using a card, selling, rerolling, a tag
firing, screen changes), a "stop use" counter that blocks selling and using
consumables until the work queued by the last action has run, and the rule
that nothing can be sold or used while cards sit in the play area. Dragging
(reordering) jokers, consumables and hand cards is gated only by the input
lock, so it stays possible while a hand is being scored. The score of a played
hand is computed in a single queued step, with all chips and mult kept in
local values and every animation only replaying numbers already decided; that
step runs a short moment (about 0.5 to 3 seconds of game time) after Play is
pressed, while the cards fly to the play area. During that short window only
dragging is possible, and a joker or held-card reorder made in it does change
the coming score; after the step nothing the player can do changes the hand's
result. The engine can therefore treat the hand order and joker order as
frozen when Play is pressed.

## 1. Game states

| # | State | Meaning |
|---|-------|---------|
| 1 | SELECTING_HAND | Round in progress, player choosing cards; Play/Discard/Sort buttons exist only here |
| 2 | HAND_PLAYED | Play pressed: cards moving, hand scored, cards leaving |
| 3 | DRAW_TO_HAND | Refilling the hand (round start, after play, after discard) |
| 4 | GAME_OVER | Run lost |
| 5 | SHOP | Shop open |
| 6 | PLAY_TAROT | A consumable or voucher is being applied outside a pack (input locked) |
| 7 | BLIND_SELECT | Choosing / skipping blinds, boss reroll |
| 8 | ROUND_EVAL | Cash-out screen |
| 9 | TAROT_PACK | Arcana pack open (hand drawn from deck) |
| 10 | PLANET_PACK | Celestial pack open |
| 15 | SPECTRAL_PACK | Spectral pack open (hand drawn from deck) |
| 17 | STANDARD_PACK | Standard pack open |
| 18 | BUFFOON_PACK | Buffoon pack open |
| 19 | NEW_ROUND | End of round being processed (win or loss) |

States 11-14 and 16 are menus, tutorial, splash and demo screens.

Main transitions: BLIND_SELECT -> (select) DRAW_TO_HAND -> SELECTING_HAND;
SELECTING_HAND -> (play) HAND_PLAYED -> DRAW_TO_HAND or NEW_ROUND;
SELECTING_HAND -> (discard) DRAW_TO_HAND; NEW_ROUND -> ROUND_EVAL or
GAME_OVER; ROUND_EVAL -> (cash out) SHOP -> (next round) BLIND_SELECT. Using a
consumable switches to PLAY_TAROT and back (inside a pack the pack state is
kept). Opening a pack switches to the pack state and back to the state it was
opened from (SHOP, or BLIND_SELECT for pack tags).

## 2. How actions are carried out

- Player input is read once per frame, after the queue and the state
  handlers have run for that frame. A button press calls its handler at
  once; the handler does a few things immediately and queues the rest.
- The queue is processed in order. Each pass handles the first waiting step;
  a step marked blocking (the default) stops later blockable steps from
  starting in that pass. So in practice one step starts per frame, and timed
  steps (delays, card moves) hold the queue for their duration. New steps go
  to the end of the queue, so a step queued while another step runs happens
  after everything already waiting.
- Delays run on game time, which is scaled by the game-speed setting and
  sped up further during long scoring sequences. Only order matters to the
  engine, not durations.
- Many player-visible values (money, discards left, hands left, ante, round
  chip total) are changed by queued steps, so they change a moment after the
  action that caused them. Section 7 lists the cases that matter.

## 3. The gates

**Input lock.** While any lock is set, mouse presses and releases are
ignored (no click, no drag start, no hover). Locks are set:

| Lock | While |
|------|-------|
| load | 3.5 s after a run starts or loads |
| use | a consumable or voucher is being applied, or a card is being taken from a pack (not while a pack opens) |
| selling_card | from Sell until the sale finishes |
| shop_reroll | during a shop reroll |
| toggle_shop | 0.5 s while leaving the shop |
| skip_blind | 2.5 s after a skip |
| boss_reroll | about 0.8 s during a boss reroll |
| one per tag | while that tag is firing |
| edition / seal | while an edition or seal is visibly applied |
| no_space | the "no space" alert when buying without room |
| wipe, frame | screen transitions and overlay menus |

**Stop-use counter.** A counter that is raised by 1 and then lowered again
only after a chain of 7 queue hops (each hop re-queues itself at the end of the
queue), so it stays raised until the work queued around the action has run.
It is raised by: Play, Discard, Select blind, Skip blind, boss reroll, shop
reroll, leaving the shop, Cash Out, opening a pack, closing a pack, using a
consumable, redeeming a voucher, selling, any tag firing, and on entering
BLIND_SELECT, SHOP and ROUND_EVAL. While it is above 0: no selling, no
consumable use, no right-click deselect.

**Play area.** While any card is in the play area: no selling, no consumable
use, no right-click deselect, and Play does nothing. Played cards stay there
until the whole scoring animation has finished.

**Buttons.** Play, Discard and the two Sort buttons exist only in
SELECTING_HAND. Shop buttons exist only while the shop is on screen (it slides
away while a pack is open or a consumable is used). The blind-select buttons
exist only in BLIND_SELECT. The Cash Out button appears on the cash-out screen
once all rows are shown. Most buttons can be pressed once and are rebuilt
when their screen returns.

## 4. Legality of each action

| Action | Allowed when | Extra conditions |
|--------|--------------|------------------|
| Select / deselect a hand card (click) | any state except HAND_PLAYED, if not locked (so SELECTING_HAND, DRAW_TO_HAND, Arcana and Spectral packs) | at most 5 selected; a Cerulean Bell card cannot be deselected |
| Deselect all (right-click) | a card is selected, not locked, stop-use 0, play area empty | keeps a forced card |
| Rearrange hand cards (drag) | not locked; any state with hand cards, HAND_PLAYED included | the next card drawn re-sorts the whole hand (`rules/round.md` section 4) |
| Sort by rank / by suit | SELECTING_HAND | also sets the sort mode used for later draws |
| Play | SELECTING_HAND | 1-5 cards selected; play area empty; boss warning not showing |
| Discard | SELECTING_HAND | 1-5 cards selected; discards left >= 1 |
| Rearrange jokers or consumables (drag) | not locked, in any state (HAND_PLAYED and NEW_ROUND included) | pack cards and shop cards cannot be dragged; dragging never moves a card to another area |
| Sell a joker or consumable | not locked, stop-use 0, play area empty, any state | not Eternal; gives its sell value (shop notes) |
| Use a consumable | not locked, stop-use 0, play area empty, state not HAND_PLAYED, DRAW_TO_HAND or PLAY_TAROT | per-card conditions in section 5 |
| Buy a card | SHOP | cost <= money - bankruptcy floor (or cost 0); jokers need a free joker slot and consumables a free consumable slot, a Negative card counting one extra slot; playing cards and vouchers need no slot |
| Buy and use (shop consumable) | SHOP | affordable and usable right now (section 5); no free consumable slot needed |
| Redeem a voucher | SHOP | cost <= money - bankruptcy floor |
| Open a pack | SHOP (buying it opens it at once) | cost 0 or cost <= money - bankruptcy floor; pack tags open their free pack at BLIND_SELECT |
| Take or use a pack card | the pack state | jokers need a free slot unless Negative; consumables follow section 5; the pack closes after its number of choices |
| Skip a pack | the pack state | Celestial, Standard and Buffoon at once; Arcana and Spectral once the hand has been drawn (or hand size is 0 or less) |
| Reroll the shop | SHOP | cost 0, or money - bankruptcy floor - cost >= 0 |
| Leave the shop | SHOP | none |
| Select a blind | BLIND_SELECT | only the blind on deck |
| Skip a blind | BLIND_SELECT | only Small or Big, only when on deck |
| Reroll the boss | BLIND_SELECT | Director's Cut or Retcon owned; see `rules/blinds.md` section 7 |
| Cash out | ROUND_EVAL | after the total row is shown |

The bankruptcy floor is $0, or -$20 with Credit Card.

## 5. When each consumable can be used

All of these also need: not locked, stop-use 0, play area empty, and a state
other than HAND_PLAYED, DRAW_TO_HAND and PLAY_TAROT.

| Group | Cards | Extra condition | Where usable in practice |
|-------|-------|-----------------|--------------------------|
| Always | The Hermit, Temperance, every Planet, Black Hole | none | hand selection, blind select, cash-out screen, shop, any open pack |
| Needs a joker without edition | The Wheel of Fortune, Ectoplasm, Hex | at least one joker without an edition | same |
| Needs room | The Emperor, The High Priestess | consumable slots not full, or the card is used from the consumable slots | same |
| The Fool | The Fool | as Emperor, and the most recently used Tarot or Planet of the run exists and is not The Fool (using The Fool makes it the most recent one; the record updates a moment after each use) | same |
| Creates a joker | Judgement, The Soul, Wraith | fewer jokers than joker slots | same |
| Ankh | Ankh | at least one joker and more than 1 joker slot; when used, also needs a free joker slot (otherwise a "no space" alert and nothing happens) | same |
| Aura | Aura | exactly 1 hand card selected and it has no edition | hand selection, Arcana/Spectral packs |
| Selected cards | The Magician (1-2), The Empress (1-2), The Hierophant (1-2), The Lovers (1), The Chariot (1), Justice (1), Strength (1-2), The Hanged Man (1-2), Death (exactly 2), The Devil (1), The Tower (1), The Star (1-3), The Moon (1-3), The Sun (1-3), The World (1-3), Talisman (1), Deja Vu (1), Trance (1), Medium (1), Cryptid (1) | state is SELECTING_HAND or a pack state, and the number of selected hand cards is within the range | hand selection, Arcana/Spectral packs |
| Needs a hand | Familiar, Grim, Incantation, Immolate, Sigil, Ouija | state is SELECTING_HAND or a pack state, and at least 2 cards in hand | hand selection, Arcana/Spectral packs |

The upper limit for selected cards is the card's maximum capped at 5; the
lower limit is 1 except Death (2).

Quirk (from the code, `consumables.md`): Buy and Use on Ankh with full joker
slots passes the shop's check (Ankh's own rule doesn't look at room), pays the
price, then the use is refused with a "no space" alert, leaving the card in no
area. Its button can still use it later once a slot is free.

## 6. During scoring: what the player can do, and whether it matters

Question: while a played hand is being scored, can the player drag jokers or
cards, sell, or use consumables, and could anything they do change the hand's
score? Is the score computed in one go before the animation plays?

Evidence:

1. Pressing Play raises the stop-use counter at once and queues, in order:
   the switch to HAND_PLAYED, hands left -1, a 0.4 s pause, one 0.1 s move per
   played card into the play area, the blind's play effect (The Hook picks
   and discards 2 held cards; The Tooth charges $1 per card), and finally a
   step that queues the scoring step.
2. The scoring step is one function call. It finds the hand type, applies the
   boss hand checks, runs every joker, card and held-card effect, and keeps
   chips and mult in local values the whole time. Every on-screen change it
   causes (popups, the chips x mult display, card highlights, joker wiggles)
   is queued as a later step that only shows a value computed in this call.
   Even the round total is queued as a slide from the old total to
   old + floor(chips x mult), with that target fixed during the call.
3. Things deferred out of the call are decided inside it: random rolls
   (Lucky, Glass, ...) are drawn during the call in scoring order; Glass and
   other destroyed cards are only flagged and removed later; money earned is
   applied later, but Bull and Bootstraps read money plus a "pending money"
   buffer that the call itself fills, so they see the hand's earlier
   payouts. The Ox's money loss is applied instantly inside the call.
4. Nothing queued after the call reads jokers, cards or the hand again for
   this hand's score.
5. During the whole scoring animation the played cards are still in the
   play area and the stop-use counter is still raised, and the state is
   HAND_PLAYED: selling is refused, consumable use is refused, clicks on hand
   cards are ignored, the Play/Discard buttons do not exist.
6. No input lock is set by playing a hand, and dragging is not checked
   against the state. The joker row and the hand are re-sorted every frame by
   on-screen position, and the scoring call walks them in their current
   order. So a player can drag jokers, consumables and held cards during
   HAND_PLAYED.

Conclusion:

- The score is computed in one go, in the single scoring step, before any of
  the scoring animation plays.
- Between pressing Play and that step (roughly 0.4 s + 0.1 s per played card
  at normal speed; about 1.3 s more with The Hook, about 0.6 s + 0.23 s per
  card more with The Tooth) the player can only drag. A new joker order or
  held-card order made in this window is used by the scoring step. Nothing
  new becomes known in this window except which cards The Hook discarded and
  The Tooth's charge, so any order the player could choose there they could
  also have chosen before pressing Play, except a reaction to The Hook's
  random picks.
- After the scoring step, nothing the player can do changes that hand's
  result. Dragging still works but only changes the order for later events.
  If this hand ends the round, end-of-round effects read the joker and hand
  order at that later moment; in the base game the only order effects there
  are which of two Mr. Bones saves the run and, for two identical jokers
  sharing a stream (Gros Michel, Cavendish, To Do List), which draws first.
- Selling and consumables become available again only after the refill draw
  (in practice in SELECTING_HAND), because the stop-use chain from Play runs
  past the scoring animation, the move to the discard pile, the round-end
  check and the queued draw. If the hand ended the round, end-of-round
  processing and the cash-out sum are computed before the counter can drop,
  so for example The Hermit cannot be used between the last hand and the
  interest calculation. UNSURE: this follows from the queue order; it has not
  been timed in the real game.

Engine recommendation: take the hand order and joker order as they are when
Play is chosen, resolve the whole hand (blind play effect, scoring, cards to
discard, round-end check, refill) as one action, and offer selling, consumables
and reordering again only at the next decision point. Optional refinement, not
needed for scores: a "reorder after The Hook resolves" decision. UNSURE whether
that is worth modelling.

## 7. Other timing facts the engine must reproduce

1. Hands left drops before the hand is scored; "hands played this round" and
   "hands played this run" rise after it. So "first hand" checks see 0 on the
   first hand and "last hand" checks see 0 hands left on the last.
2. On a discard, the discard jokers see the old "discards used" count (it
   rises after them) and the old "discards left" (it drops a moment later).
3. The Tooth's charge and The Hook's discard finish before scoring; The Ox
   empties money instantly during scoring, before jokers act.
4. Money changes from scoring, selling and cash-out are applied by queued
   steps; the cash-out sum and interest use the money held when the cash-out
   screen is computed (`rules/round.md` section 9) and do not change if the
   player sells or spends on that screen.
5. Blind debuffs stay active through end-of-round processing and the
   cash-out computation; they end shortly after the cash-out screen appears.
6. Some joker values are recomputed every frame instead of when used:
   Throwback (from skips), Joker Stencil (from empty joker slots), Steel
   Joker, Stone Joker, Driver's License and Cloud 9 tallies, Swashbuckler (sum
   of other jokers' sell values), Temperance's payout, and the eligible-joker
   lists of The Wheel of Fortune, Ectoplasm and Hex. Inside one step they hold
   the value of the previous frame. The engine can compute them on demand;
   results differ only when the counted things change inside the same step.
7. The boss warning at the start of a boss round blocks Play for about a
   second (cosmetic; Discard is not blocked).
8. Hand-size increases during a round draw the extra cards at once (only in
   SELECTING_HAND or DRAW_TO_HAND, if the hand is not empty).

## Where in the game

- `globals.lua`: the state list in `Game:set_globals`.
- `game.lua`: `Game:update`, `Game:update_selecting_hand`,
  `Game:update_shop`, `Game:update_play_tarot`, `Game:update_hand_played`,
  `Game:update_draw_to_hand`, `Game:update_new_round`,
  `Game:update_blind_select`, `Game:update_round_eval`,
  `Game:update_arcana_pack`, `Game:update_spectral_pack`, `Game:start_run`.
- `engine/event.lua`: `Event:init`, `Event:handle`, `EventManager:add_event`,
  `EventManager:update`.
- `engine/controller.lua`: `Controller:update`, `Controller:set_cursor_position`,
  `Controller:set_cursor_hover`, `Controller:L_cursor_press`,
  `Controller:L_cursor_release`, `Controller:queue_R_cursor_press`.
- `engine/node.lua`: `Node:init`, `Node:can_drag`; `engine/moveable.lua`:
  `Moveable:drag`; `engine/ui.lua`: `UIElement:click`.
- `cardarea.lua`: `CardArea:set_ranks`, `CardArea:align_cards`,
  `CardArea:can_highlight`, `CardArea:add_to_highlighted`,
  `CardArea:remove_from_highlighted`, `CardArea:unhighlight_all`,
  `CardArea:change_size`, `CardArea:update`.
- `card.lua`: `Card:click`, `Card:highlight`, `Card:can_use_consumeable`,
  `Card:check_use`, `Card:can_sell_card`, `Card:sell_card`,
  `Card:use_consumeable`, `Card:open`, `Card:redeem`, `Card:get_p_dollars`,
  `Card:update`.
- `functions/misc_functions.lua`: `stop_use`, `dec_stop_use`.
- `functions/button_callbacks.lua`: `G.FUNCS.can_play`, `G.FUNCS.can_discard`,
  `G.FUNCS.can_reroll`, `G.FUNCS.can_buy`, `G.FUNCS.can_buy_and_use`,
  `G.FUNCS.can_redeem`, `G.FUNCS.can_open`, `G.FUNCS.can_use_consumeable`,
  `G.FUNCS.can_select_card`, `G.FUNCS.can_sell_card`,
  `G.FUNCS.can_skip_booster`, `G.FUNCS.use_card`, `G.FUNCS.sell_card`,
  `G.FUNCS.check_for_buy_space`, `G.FUNCS.buy_from_shop`,
  `G.FUNCS.toggle_shop`, `G.FUNCS.select_blind`, `G.FUNCS.skip_booster`,
  `G.FUNCS.end_consumeable`, `G.FUNCS.blind_choice_handler`,
  `G.FUNCS.skip_blind`, `G.FUNCS.reroll_boss`, `G.FUNCS.reroll_shop`,
  `G.FUNCS.cash_out`.
- `functions/state_events.lua`: `G.FUNCS.play_cards_from_highlighted`,
  `G.FUNCS.discard_cards_from_highlighted`, `G.FUNCS.evaluate_play`,
  `G.FUNCS.draw_from_deck_to_hand`, `end_round`.
- `functions/UI_definitions.lua`: `create_UIBox_buttons`,
  `G.UIDEF.use_and_sell_buttons`, `G.UIDEF.shop`, `create_shop_card_ui`,
  `create_UIBox_blind_select`.
- `blind.lua`: `Blind:press_play`, `Blind:alert_debuff`.
- `tag.lua`: `Tag:yep`, `Tag:apply_to_run`.
