//! The leak tally: what a run did with what it had, move by move, without changing a move (lab issue #19).
//!
//! Watching runs, the user saw value left on the table: Blue Seals thrown or played, Purple Seals played with
//! discards left, The High Priestess held for rounds, rounds won at once with hands and discards to spare. The
//! tally counts each of these from the run as it is played (or replayed: it needs only the position before each
//! move, the move and the position after, plus what `planner::finish_choice` noted when the run is played live),
//! so the same counts can be taken again on a fixed bot as the proof that the behaviour changed.
//!
//! What it counts, per run (`Leaks::json`):
//! - **Seals.** Blue and Purple Seal cards drawn (each card once per round); each one thrown (while digging, or
//!   as a spare once a winning hand was in hand), played (in a hand that didn't end the round, or in the one that
//!   did), held at the round's end; the Planets and Tarots they made, and the ones not made because every
//!   consumable slot was full; Purple Seals played or left in hand with discards still left.
//! - **Rounds.** Rounds begun and won; of the won ones, those where a winning hand was in hand with two or more
//!   hands or any discard left, and whether the round ended on that very move; the hands and discards left at the
//!   end; rounds won with discards left while a Purple Seal was in hand or still to draw; what each way of
//!   finishing fared in `finish_choice` (offered, failed a try, passed every try but gained too little, kept).
//! - **Consumables.** Rounds begun holding each kind, and rounds it was still held at the end (not used); rounds
//!   ended with every slot full; uses and sales by kind; which card each targeted use went on: its rank among the
//!   cards in hand by `value::card_value` (0 the most valuable) and whether it already had a seal.
//! - **Shops and packs.** Spectral packs: kinds offered and taken; shops left with money above the interest cap
//!   even after the cheapest pack still on sale; the last shop's money on leaving, and whether Bull or Bootstraps
//!   was held; jokers bought and sold by ante; vouchers offered, affordable and redeemed, by kind.
//! - **Hands.** Hands played by type.
//!
//! Nothing here is play or training, and nothing hidden is read beyond what the run itself did: it is a
//! diagnosis of the bot, like `baseline --diagnose`.

use std::collections::{BTreeMap, HashSet};

use crate::card::Seal;
use crate::content::{ConsumableKind, ConsumableSet, PackKind, BOOSTERS};
use crate::env::MoveKind;
use crate::joker_data::JokerKind;
use crate::packs::PackCard;
use crate::planner::{FarmOutcome, FinishNote};
use crate::shop::RowItem;
use crate::state::{CardId, Phase, Run};

/// Counts by name, kept sorted so the line is the same on any thread.
type Counts = BTreeMap<&'static str, u64>;

fn bump_owned(c: &mut BTreeMap<String, u64>, k: String) {
    *c.entry(k).or_insert(0) += 1;
}

fn bump(c: &mut Counts, k: &'static str) {
    *c.entry(k).or_insert(0) += 1;
}

/// One targeted use: the consumable, the rank of each targeted card among the hand by `card_value` (0 the most
/// valuable), and whether each already had a seal.
#[derive(Clone, Debug, Default)]
struct Target {
    kind: &'static str,
    ranks: Vec<usize>,
    sealed: Vec<bool>,
    hand: usize,
}

/// The tally of one run (see the module's notes).
#[derive(Clone, Debug, Default)]
pub struct Leaks {
    /// Seal events: "blue_drawn", "blue_thrown_digging", ... (see `seal_moves`).
    seals: Counts,
    rounds: Counts,
    /// `finish_choice` ways: "throw:failed", "extra5:kept", ...
    ways: BTreeMap<String, u64>,
    /// The hands and discards left when each won round ended: (hands, discards) → rounds.
    left_at_win: BTreeMap<(i32, i32), u64>,
    held_at_start: Counts,
    held_through: Counts,
    used: Counts,
    sold: Counts,
    targets: Vec<Target>,
    spectral_offered: Counts,
    spectral_taken: Counts,
    shops: Counts,
    last_shop: Option<(i64, bool)>,
    /// (ante, joker, how): "buy", "pack", "sell".
    jokers: Vec<(i32, &'static str, &'static str)>,
    vouchers_offered: Counts,
    vouchers_affordable: Counts,
    vouchers_redeemed: Counts,
    /// Boss rerolls, by the boss rerolled away.
    boss_rerolled_away: Counts,
    hands: Counts,
    /// Card Sharp (S, lab #30): "plays_held", "rounds_held" (rounds with a hand played while it was held),
    /// "rounds_fired" (rounds in which its x3 fired at least once), "fired:<hand>", "plays:<hand>".
    sharp: BTreeMap<String, u64>,
    /// Consumables used the moment they were taken, never held: from a pack (a Tarot or Planet picked is used on the
    /// spot) or bought and used from the shop, by kind. `used` counts only those used from the consumable slots.
    used_at_once: Counts,
    /// Packs (S, lab #30): "offered:<kind>", "affordable:<kind>" (each shop's first look), "opened:<kind>" and
    /// "opened:<kind>:a<ante>"; shop consumables bought: "bought:<set>".
    packs: BTreeMap<String, u64>,
    sealed_by_ante4: Option<bool>,
    // State within the run.
    /// Cards seen in hand this round (for "drawn").
    seen_round: HashSet<CardId>,
    /// Kinds held as the round began.
    start_kinds: Vec<ConsumableKind>,
    /// The round's first decision with a winning hand in hand and room to take value: (hands, discards) then.
    winning_with_room: Option<(i32, i32)>,
    /// A winning hand was in hand at some decision of this round.
    had_winning: bool,
    /// Shops already counted, by (ante, blinds behind).
    shops_seen: HashSet<(i32, usize)>,
    /// The open pack's offer is counted.
    pack_counted: bool,
    /// This round: a hand played with Card Sharp held; its x3 fired.
    sharp_round: (bool, bool),
}

fn behind(run: &Run) -> usize {
    run.slots
        .iter()
        .filter(|s| {
            matches!(
                s,
                crate::blinds::SlotState::Defeated | crate::blinds::SlotState::Skipped
            )
        })
        .count()
}

fn seal_of(run: &Run, id: CardId) -> Seal {
    run.card(id).seal
}

fn count_kind(run: &Run, set: ConsumableSet) -> usize {
    run.consumables
        .iter()
        .filter(|c| c.kind.set() == set)
        .count()
}

/// Whether the best hand in hand (as `planner::quick_best` finds it) beats what is left of the blind.
fn winning_in_hand(run: &Run) -> bool {
    if run.phase != Phase::Round || run.hand.is_empty() {
        return false;
    }
    let need = run.blind.as_ref().map_or(0.0, |b| b.chips) - run.round_chips;
    crate::planner::quick_best(run).0 >= need
}

fn masked(run: &Run, mask: u64) -> Vec<CardId> {
    (0..run.hand.len())
        .filter(|&i| mask >> i & 1 == 1)
        .map(|i| run.hand[i])
        .collect()
}

fn json_counts(c: &Counts) -> String {
    let parts: Vec<String> = c.iter().map(|(k, v)| format!("\"{k}\":{v}")).collect();
    format!("{{{}}}", parts.join(","))
}

/// Kinds that target cards in hand and whose target is worth checking.
fn targeted(kind: ConsumableKind) -> bool {
    use ConsumableKind::*;
    matches!(
        kind,
        DejaVu
            | Talisman
            | Medium
            | Trance
            | Aura
            | Cryptid
            | Death
            | Magician
            | Empress
            | Hierophant
            | Lovers
            | Chariot
            | Justice
            | Devil
            | Tower
            | Strength
            | HangedMan
            | Star
            | Moon
            | Sun
            | World
    )
}

impl Leaks {
    pub fn new() -> Leaks {
        Leaks::default()
    }

    /// One move: the position before it, the move, the position after, and what `finish_choice` noted while
    /// the move was decided (empty on a replay).
    pub fn observe(
        &mut self,
        before: &Run,
        mv: (MoveKind, u64),
        after: &Run,
        finish: &[FinishNote],
    ) {
        let (k, a) = mv;
        if self.sealed_by_ante4.is_none() && after.ante >= 4 {
            self.sealed_by_ante4 = Some(
                after
                    .playing_cards
                    .iter()
                    .any(|&id| matches!(seal_of(after, id), Seal::Blue | Seal::Purple)),
            );
        }
        if before.phase == Phase::Round {
            self.round_move(before, mv, after, finish);
        }
        if before.phase != Phase::Round && after.phase == Phase::Round {
            self.round_begins(after);
        }
        if before.phase == Phase::Shop {
            self.shop_move(before, mv);
        }
        if before.phase == Phase::Pack {
            self.pack_move(before, mv);
        } else {
            self.pack_counted = false;
        }
        match k {
            MoveKind::UseConsumable => {
                if let Some(c) = before.consumables.get((a & 0xFF) as usize) {
                    bump(&mut self.used, c.kind.name());
                    let mask = a >> 8;
                    if targeted(c.kind) && mask != 0 && before.phase == Phase::Round {
                        self.target(before, c.kind, mask);
                    }
                }
            }
            MoveKind::SellConsumable => {
                if let Some(c) = before.consumables.get(a as usize) {
                    bump(&mut self.sold, c.kind.name());
                }
            }
            MoveKind::SellJoker => {
                if let Some(j) = before.jokers.get(a as usize) {
                    self.jokers.push((before.ante, j.kind.name(), "sell"));
                }
            }
            MoveKind::RerollBoss => bump(&mut self.boss_rerolled_away, before.boss.name()),
            _ => {}
        }
    }

    fn target(&mut self, run: &Run, kind: ConsumableKind, mask: u64) {
        // Ranked among the cards that could take it: a seal Spectral's among the unsealed cards (and the target), an
        // Aura's among the cards without an edition; every card in hand otherwise.
        let eligible = |i: usize| {
            let c = run.card(run.hand[i]);
            mask >> i & 1 == 1
                || match kind {
                    ConsumableKind::DejaVu
                    | ConsumableKind::Talisman
                    | ConsumableKind::Trance
                    | ConsumableKind::Medium => c.seal == Seal::None,
                    ConsumableKind::Aura => c.edition == crate::card::Edition::None,
                    _ => true,
                }
        };
        let mut worth: Vec<(usize, f64)> = (0..run.hand.len())
            .filter(|&i| eligible(i))
            .map(|i| (i, crate::value::card_value(run, run.hand[i])))
            .collect();
        worth.sort_by(|x, y| y.1.total_cmp(&x.1).then(x.0.cmp(&y.0)));
        // Its rank: how many eligible cards are worth strictly more (cards of equal worth share a rank, so a tie is
        // never counted as a miss).
        let rank_of = |i: usize| {
            let w = worth
                .iter()
                .find(|&&(j, _)| j == i)
                .map_or(0.0, |&(_, w)| w);
            worth.iter().filter(|&&(_, x)| x > w + 1e-9).count()
        };
        let chosen: Vec<usize> = (0..run.hand.len())
            .filter(|&i| mask >> i & 1 == 1)
            .collect();
        self.targets.push(Target {
            kind: kind.name(),
            ranks: chosen.iter().map(|&i| rank_of(i)).collect(),
            sealed: chosen
                .iter()
                .map(|&i| seal_of(run, run.hand[i]) != Seal::None)
                .collect(),
            hand: run.hand.len(),
        });
    }

    /// A hand of type `h` played from `before`: Card Sharp's x3 fires when the type was already played this round.
    fn card_sharp(&mut self, before: &Run, h: crate::poker::HandType) {
        if !before.has_joker(JokerKind::CardSharp) {
            return;
        }
        bump_owned(&mut self.sharp, "plays_held".into());
        bump_owned(&mut self.sharp, format!("plays:{}", h.name()));
        if !self.sharp_round.0 {
            self.sharp_round.0 = true;
            bump_owned(&mut self.sharp, "rounds_held".into());
        }
        if before.hand_levels.get(h).played_this_round >= 1 {
            bump_owned(&mut self.sharp, format!("fired:{}", h.name()));
            if !self.sharp_round.1 {
                self.sharp_round.1 = true;
                bump_owned(&mut self.sharp, "rounds_fired".into());
            }
        }
    }

    fn round_begins(&mut self, run: &Run) {
        bump(&mut self.rounds, "begun");
        self.seen_round.clear();
        self.winning_with_room = None;
        self.had_winning = false;
        self.sharp_round = (false, false);
        self.start_kinds = run.consumables.iter().map(|c| c.kind).collect();
        for c in &run.consumables {
            bump(&mut self.held_at_start, c.kind.name());
        }
        self.see_hand(run);
    }

    fn see_hand(&mut self, run: &Run) {
        for &id in &run.hand {
            if self.seen_round.insert(id) {
                match seal_of(run, id) {
                    Seal::Blue => bump(&mut self.seals, "blue_drawn"),
                    Seal::Purple => bump(&mut self.seals, "purple_drawn"),
                    _ => {}
                }
            }
        }
    }

    fn round_move(
        &mut self,
        before: &Run,
        mv: (MoveKind, u64),
        after: &Run,
        finish: &[FinishNote],
    ) {
        let (k, a) = mv;
        let winning = winning_in_hand(before);
        let room = before.hands_left >= 2 || before.discards_left >= 1;
        if winning && !self.had_winning {
            self.had_winning = true;
            bump(&mut self.rounds, "with_winning_hand");
        }
        let first_winning = winning && room && self.winning_with_room.is_none();
        if first_winning {
            self.winning_with_room = Some((before.hands_left, before.discards_left));
        }
        // What finish_choice did at this decision: the outermost note whose move is the one made.
        if let Some(n) = finish.iter().rev().find(|n| n.chosen == mv) {
            bump(&mut self.rounds, "finish_decisions");
            for w in &n.ways {
                let tag = match w.outcome {
                    FarmOutcome::SameMove => "same_move",
                    FarmOutcome::Failed => "failed",
                    FarmOutcome::Passed => "passed",
                    FarmOutcome::Kept => "kept",
                };
                *self.ways.entry(format!("{}:{tag}", w.name)).or_insert(0) += 1;
            }
            // The way whose move was made: the last kept.
            if let Some(w) = n.ways.iter().rev().find(|w| w.outcome == FarmOutcome::Kept) {
                *self.ways.entry(format!("{}:made", w.name)).or_insert(0) += 1;
            }
        }
        let ended = after.phase != Phase::Round;
        let won = ended && after.phase != Phase::GameOver;
        match k {
            MoveKind::Discard => {
                let tarots = count_kind(after, ConsumableSet::Tarot) as i64
                    - count_kind(before, ConsumableSet::Tarot) as i64;
                let mut purple = 0i64;
                for id in masked(before, a) {
                    match seal_of(before, id) {
                        Seal::Blue => bump(
                            &mut self.seals,
                            if winning {
                                "blue_thrown_spare"
                            } else {
                                "blue_thrown_digging"
                            },
                        ),
                        Seal::Purple => {
                            purple += 1;
                            bump(
                                &mut self.seals,
                                if winning {
                                    "purple_thrown_spare"
                                } else {
                                    "purple_thrown_digging"
                                },
                            );
                        }
                        _ => {}
                    }
                }
                if purple > 0 {
                    let made = tarots.clamp(0, purple);
                    for _ in 0..made {
                        bump(&mut self.seals, "purple_tarot_made");
                    }
                    for _ in made..purple {
                        bump(&mut self.seals, "purple_tarot_missed");
                    }
                }
            }
            MoveKind::Play => {
                let played = masked(before, a);
                for &id in &played {
                    let s = seal_of(before, id);
                    let name = match (s, ended) {
                        (Seal::Blue, true) => "blue_played_win",
                        (Seal::Blue, false) => "blue_played",
                        (Seal::Purple, true) => "purple_played_win",
                        (Seal::Purple, false) => "purple_played",
                        _ => continue,
                    };
                    bump(&mut self.seals, name);
                    if s == Seal::Purple && before.discards_left > 0 {
                        bump(&mut self.seals, "purple_played_with_discards");
                        // Split by whether a consumable slot was free (thrown, it would have made a Tarot) or not.
                        let free =
                            before.consumables.len() < before.consumable_slots.max(0) as usize;
                        bump(
                            &mut self.seals,
                            if free {
                                "purple_played_with_discards_slot_free"
                            } else {
                                "purple_played_with_discards_slot_full"
                            },
                        );
                    }
                }
                if let Some(h) = after.last_hand {
                    bump(&mut self.hands, h.name());
                    self.card_sharp(before, h);
                }
                if won {
                    let held: Vec<CardId> = before
                        .hand
                        .iter()
                        .copied()
                        .filter(|id| !played.contains(id))
                        .collect();
                    let blue = held
                        .iter()
                        .filter(|&&id| seal_of(before, id) == Seal::Blue)
                        .count() as i64;
                    let purple_left = held
                        .iter()
                        .filter(|&&id| seal_of(before, id) == Seal::Purple)
                        .count() as u64;
                    if blue > 0 {
                        let planets = count_kind(after, ConsumableSet::Planet) as i64
                            - count_kind(before, ConsumableSet::Planet) as i64;
                        let made = planets.clamp(0, blue);
                        for _ in 0..blue {
                            bump(&mut self.seals, "blue_held_end");
                        }
                        for _ in 0..made {
                            bump(&mut self.seals, "blue_planet_made");
                        }
                        for _ in made..blue {
                            bump(&mut self.seals, "blue_planet_missed");
                        }
                    }
                    if purple_left > 0 && before.discards_left > 0 {
                        for _ in 0..purple_left {
                            bump(&mut self.seals, "purple_held_end_with_discards");
                        }
                    }
                    self.round_won(before, first_winning);
                }
            }
            _ => {}
        }
        if ended && !won {
            bump(&mut self.rounds, "lost");
            // B's bar (#27): a round lost after a winning hand was in hand.
            if self.had_winning {
                bump(&mut self.rounds, "lost_after_winning_hand");
            }
        }
        if ended {
            self.round_ends(after);
        } else {
            self.see_hand(after);
        }
    }

    fn round_won(&mut self, before: &Run, ended_on_first: bool) {
        bump(&mut self.rounds, "won");
        // Hands and discards left after the winning hand.
        *self
            .left_at_win
            .entry(((before.hands_left - 1).max(0), before.discards_left.max(0)))
            .or_insert(0) += 1;
        if let Some((h, d)) = self.winning_with_room {
            bump(&mut self.rounds, "won_with_room");
            if ended_on_first {
                bump(&mut self.rounds, "won_with_room_at_once");
            }
            if h >= 2 {
                bump(&mut self.rounds, "won_with_room_2plus_hands");
            }
            if d >= 1 {
                bump(&mut self.rounds, "won_with_room_discards");
            }
        }
        if before.discards_left > 0 {
            bump(&mut self.rounds, "won_with_discards_left");
            let purple_near = before
                .hand
                .iter()
                .chain(before.deck.iter())
                .any(|&id| seal_of(before, id) == Seal::Purple);
            if purple_near {
                bump(&mut self.rounds, "won_with_discards_left_purple_near");
            }
        }
        if before.hands_left >= 2 {
            let blue_to_draw = before
                .deck
                .iter()
                .any(|&id| seal_of(before, id) == Seal::Blue);
            if blue_to_draw {
                bump(&mut self.rounds, "won_with_hands_left_blue_to_draw");
            }
        }
    }

    fn round_ends(&mut self, after: &Run) {
        let full = after.consumables.len() as i32 >= after.consumable_slots;
        if full {
            bump(&mut self.rounds, "ended_slots_full");
        }
        // Kinds held at the start and still held now: held through the round, unused.
        let mut now: Vec<ConsumableKind> = after.consumables.iter().map(|c| c.kind).collect();
        for k in std::mem::take(&mut self.start_kinds) {
            if let Some(p) = now.iter().position(|&x| x == k) {
                now.swap_remove(p);
                bump(&mut self.held_through, k.name());
            }
        }
    }

    fn shop_move(&mut self, before: &Run, mv: (MoveKind, u64)) {
        let (k, a) = mv;
        let Some(shop) = before.shop.as_ref() else {
            return;
        };
        let key = (before.ante, behind(before));
        if self.shops_seen.insert(key) {
            bump(&mut self.shops, "visits");
            for b in shop.boosters.iter().flatten() {
                let kind = format!("{:?}", BOOSTERS[b.booster as usize].kind);
                bump_owned(&mut self.packs, format!("offered:{kind}"));
                if b.cost <= before.spendable() {
                    bump_owned(&mut self.packs, format!("affordable:{kind}"));
                }
            }
            for v in &shop.vouchers {
                bump(&mut self.vouchers_offered, v.voucher.name());
                if v.cost <= 0 || v.cost <= before.spendable() {
                    bump(&mut self.vouchers_affordable, v.voucher.name());
                }
            }
        }
        match k {
            MoveKind::Buy | MoveKind::BuyAndUse => match shop.row.get(a as usize).map(|r| &r.item) {
                Some(RowItem::Joker(j)) => self.jokers.push((before.ante, j.kind.name(), "buy")),
                Some(RowItem::Consumable(c)) => {
                    bump_owned(&mut self.packs, format!("bought:{:?}", c.kind.set()));
                    if k == MoveKind::BuyAndUse {
                        bump(&mut self.used_at_once, c.kind.name());
                    }
                }
                _ => {}
            },
            MoveKind::OpenBooster => {
                if let Some(Some(b)) = shop.boosters.get(a as usize) {
                    let kind = format!("{:?}", BOOSTERS[b.booster as usize].kind);
                    bump_owned(&mut self.packs, format!("opened:{kind}"));
                    bump_owned(&mut self.packs, format!("opened:{kind}:a{}", before.ante));
                }
            }
            MoveKind::RedeemVoucher => {
                if let Some(v) = shop.vouchers.get(a as usize) {
                    bump(&mut self.vouchers_redeemed, v.voucher.name());
                }
            }
            MoveKind::LeaveShop => {
                bump(&mut self.shops, "left");
                let cheapest = shop.boosters.iter().flatten().map(|b| b.cost).min();
                if let Some(c) = cheapest {
                    if before.money - c >= before.interest_cap {
                        bump(&mut self.shops, "left_rich_pack_unopened");
                    }
                }
                if before.money >= before.interest_cap {
                    bump(&mut self.shops, "left_at_or_above_interest_cap");
                }
                if crate::value::shops_left(before) <= 0.0 {
                    let bull = before.count_jokers(JokerKind::Bull)
                        + before.count_jokers(JokerKind::Bootstraps)
                        > 0;
                    self.last_shop = Some((before.money, bull));
                }
            }
            _ => {}
        }
    }

    fn pack_move(&mut self, before: &Run, mv: (MoveKind, u64)) {
        let (k, a) = mv;
        let Some(p) = before.pack.as_ref() else {
            return;
        };
        let kind = BOOSTERS[p.booster as usize].kind;
        let first_look =
            !self.pack_counted && matches!(k, MoveKind::PackChoose | MoveKind::PackSkip);
        if first_look {
            self.pack_counted = true;
        }
        if kind == PackKind::Spectral && first_look {
            for c in &p.cards {
                if let PackCard::Consumable(c) = c {
                    bump(&mut self.spectral_offered, c.kind.name());
                }
            }
            match k {
                MoveKind::PackChoose => {
                    if let Some(PackCard::Consumable(c)) = p.cards.get((a & 0xFF) as usize) {
                        bump(&mut self.spectral_taken, c.kind.name());
                    }
                }
                _ => bump(&mut self.spectral_taken, "skipped"),
            }
        }
        if k == MoveKind::PackChoose {
            match p.cards.get((a & 0xFF) as usize) {
                Some(PackCard::Joker(j)) => self.jokers.push((before.ante, j.kind.name(), "pack")),
                // A Tarot, Planet or Spectral taken from a pack is used at once (on the cards in hand).
                Some(PackCard::Consumable(c)) => {
                    bump(&mut self.used_at_once, c.kind.name());
                    if targeted(c.kind) && a >> 8 != 0 {
                        self.target(before, c.kind, a >> 8)
                    }
                }
                _ => {}
            }
        }
    }

    /// The tally as one JSON object (no trailing newline).
    pub fn json(&self) -> String {
        let left: Vec<String> = self
            .left_at_win
            .iter()
            .map(|((h, d), n)| format!("\"{h},{d}\":{n}"))
            .collect();
        let targets: Vec<String> = self
            .targets
            .iter()
            .map(|t| {
                format!(
                    "[\"{}\",{:?},{:?},{}]",
                    t.kind,
                    t.ranks,
                    t.sealed.iter().map(|&b| u8::from(b)).collect::<Vec<_>>(),
                    t.hand
                )
            })
            .collect();
        let jokers: Vec<String> = self
            .jokers
            .iter()
            .map(|(ante, j, how)| format!("[{ante},\"{j}\",\"{how}\"]"))
            .collect();
        let last = self.last_shop.map_or("null".to_string(), |(m, b)| {
            format!("{{\"money\":{m},\"bull_or_bootstraps\":{b}}}")
        });
        let by4 = self
            .sealed_by_ante4
            .map_or("null".to_string(), |b| b.to_string());
        format!(
            "{{\"seals\":{},\"rounds\":{},\"ways\":{},\"left_at_win\":{{{}}},\"held_at_start\":{},\
             \"held_through\":{},\"used\":{},\"sold\":{},\"targets\":[{}],\"spectral_offered\":{},\
             \"spectral_taken\":{},\"shops\":{},\"last_shop\":{},\"jokers\":[{}],\"vouchers_offered\":{},\
             \"vouchers_affordable\":{},\"vouchers_redeemed\":{},\"boss_rerolled_away\":{},\"hands\":{},\
             \"card_sharp\":{},\"used_at_once\":{},\"packs\":{},\"blue_or_purple_by_ante4\":{}}}",
            json_counts(&self.seals),
            json_counts(&self.rounds),
            {
                let parts: Vec<String> = self.ways.iter().map(|(k, v)| format!("\"{k}\":{v}")).collect();
                format!("{{{}}}", parts.join(","))
            },
            left.join(","),
            json_counts(&self.held_at_start),
            json_counts(&self.held_through),
            json_counts(&self.used),
            json_counts(&self.sold),
            targets.join(","),
            json_counts(&self.spectral_offered),
            json_counts(&self.spectral_taken),
            json_counts(&self.shops),
            last,
            jokers.join(","),
            json_counts(&self.vouchers_offered),
            json_counts(&self.vouchers_affordable),
            json_counts(&self.vouchers_redeemed),
            json_counts(&self.boss_rerolled_away),
            json_counts(&self.hands),
            {
                let parts: Vec<String> = self.sharp.iter().map(|(k, v)| format!("\"{k}\":{v}")).collect();
                format!("{{{}}}", parts.join(","))
            },
            json_counts(&self.used_at_once),
            {
                let parts: Vec<String> = self.packs.iter().map(|(k, v)| format!("\"{k}\":{v}")).collect();
                format!("{{{}}}", parts.join(","))
            },
            by4,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::env::apply;
    use crate::RunConfig;

    /// The greedy bot plays a run while the tally watches: every round begun ends won or lost, and the line holds
    /// what it counted.
    #[test]
    fn the_tally_counts_every_round_and_writes_one_line() {
        let mut run = Run::new(RunConfig::new("LEAKS1"));
        let mut t = Leaks::new();
        for step in 0..3000 {
            if run.phase == Phase::GameOver || run.won {
                break;
            }
            let before = run.clone();
            let m = crate::bots::greedy_move(&run, step);
            apply(&mut run, m.0, m.1).expect("the greedy bot's moves are legal");
            t.observe(&before, m, &run, &[]);
        }
        let begun = t.rounds.get("begun").copied().unwrap_or(0);
        let ended =
            t.rounds.get("won").copied().unwrap_or(0) + t.rounds.get("lost").copied().unwrap_or(0);
        assert!(begun >= 3, "rounds were played");
        assert!(
            ended == begun || ended + 1 == begun,
            "each round ends won or lost (the last may still be on)"
        );
        assert!(t.shops.get("visits").copied().unwrap_or(0) >= 1);
        let line = t.json();
        assert!(line.starts_with("{\"seals\":") && line.ends_with('}'));
        assert_eq!(line.matches('{').count(), line.matches('}').count());
        assert!(t.ways.is_empty(), "no finish notes on a replay");
    }

    /// A Purple Seal thrown with room makes a Tarot; with every slot full it is counted as missed.
    #[test]
    fn a_purple_seal_thrown_makes_a_tarot_or_is_counted_missed() {
        let mut run = Run::new(RunConfig::new("LEAKS2"));
        while run.phase != Phase::Round {
            let m = crate::bots::greedy_move(&run, 1);
            apply(&mut run, m.0, m.1).unwrap();
        }
        let id = run.hand[0];
        run.cards[id as usize].seal = Seal::Purple;
        for full in [false, true] {
            let mut r = run.clone();
            if full {
                r.consumable_slots = 0;
            }
            let mut t = Leaks::new();
            let before = r.clone();
            apply(&mut r, MoveKind::Discard, 1).unwrap();
            t.observe(&before, (MoveKind::Discard, 1), &r, &[]);
            let made = t.seals.get("purple_tarot_made").copied().unwrap_or(0);
            let missed = t.seals.get("purple_tarot_missed").copied().unwrap_or(0);
            assert_eq!(
                (made, missed),
                if full { (0, 1) } else { (1, 0) },
                "full {full}"
            );
        }
    }

    /// Card Sharp (lab #30): the first hand of a type in a round doesn't fire it, the same type again does.
    #[test]
    fn card_sharp_counts_the_rounds_and_hands_its_x3_fired_on() {
        let mut run = Run::new(RunConfig::new("LEAKS3"));
        while run.phase != Phase::Round {
            let m = crate::bots::greedy_move(&run, 1);
            apply(&mut run, m.0, m.1).unwrap();
        }
        run.add_joker(JokerKind::CardSharp);
        if let Some(b) = run.blind.as_mut() {
            b.chips = 1e12;
        }
        let mut t = Leaks::new();
        t.round_begins(&run);
        for _ in 0..2 {
            let before = run.clone();
            apply(&mut run, MoveKind::Play, 1).unwrap(); // one card: a High Card each time
            t.observe(&before, (MoveKind::Play, 1), &run, &[]);
        }
        let g = |k: &str| t.sharp.get(k).copied().unwrap_or(0);
        assert_eq!((g("plays_held"), g("rounds_held"), g("rounds_fired")), (2, 1, 1));
        assert_eq!((g("plays:High Card"), g("fired:High Card")), (2, 1));
        assert!(t.json().contains("\"card_sharp\":{"));
        assert!(t.json().contains("\"used_at_once\":{") && t.json().contains("\"packs\":{"));
    }
}
