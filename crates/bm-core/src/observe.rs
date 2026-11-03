//! The fair-play wall. The bot's input is built here, and only here, from
//! what the screen shows: never the seed, the random streams, the draw
//! order, creation order or anything the game hasn't revealed.
//!
//! [`screen`] copies the visible state into a [`Screen`]; [`encode`] turns
//! a screen into a fixed-length vector of floats. The leak test checks that
//! two runs with different seeds and deck orders but the same screen give
//! byte-identical vectors. [`fair_copy`] makes a copy for lookahead whose
//! hidden parts are reshuffled.

use bm_rng::{shuffle, LuaRandom, SeedSampler, Streams};

use crate::blinds::{BlindKind, BlindSlot, Boss, SlotState, N_BOSSES};
use crate::card::{Card, Edition, Enhancement, Rank, Seal, SortMode, Suit};
use crate::content::{
    BoosterId, ConsumableKind, PackKind, VoucherKind, BOOSTERS, N_CONSUMABLES, N_VOUCHERS,
};
use crate::joker_data::{JokerKind, N_JOKERS};
use crate::packs::PackCard;
use crate::poker::HandType;
use crate::run::joker_sell_value;
use crate::shop::RowItem;
use crate::state::{Phase, Run};
use crate::tags::{TagKind, N_TAGS};

/// Most hand cards and jokers the encoding has room for.
pub const MAX_HAND: usize = 24;
pub const MAX_JOKERS: usize = 16;

/// A card as the screen shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct CardView {
    /// None when face down.
    pub face: Option<(Rank, Suit, Enhancement, Edition, Seal)>,
    pub perma_bonus: i64,
    pub debuffed: bool,
    pub forced: bool,
}

/// A joker as the screen shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct JokerView {
    /// None when face down (Amber Acorn).
    pub kind: Option<JokerKind>,
    pub edition: Edition,
    pub eternal: bool,
    pub perishable: Option<i32>,
    pub rental: bool,
    pub debuffed: bool,
    pub sell_value: i64,
    /// The numbers its description shows (current mult, X-mult, chips,
    /// counter).
    pub mult: f64,
    pub x_mult: f64,
    pub chips: f64,
    pub counter: i64,
    /// To Do List's target hand.
    pub hand: Option<HandType>,
}

/// A shop, pack or slot item as the screen shows it.
#[derive(Clone, Debug, PartialEq)]
pub enum ItemView {
    Joker(JokerView),
    Consumable(ConsumableKind, Edition),
    Card(CardView),
}

/// The shop's cards, each with its price.
#[derive(Clone, Debug, PartialEq)]
pub struct ShopView {
    pub row: Vec<(ItemView, i64)>,
    pub vouchers: Vec<(VoucherKind, i64)>,
    pub boosters: Vec<Option<(BoosterId, i64)>>,
}

/// An open pack.
#[derive(Clone, Debug, PartialEq)]
pub struct PackView {
    pub booster: BoosterId,
    pub cards: Vec<ItemView>,
    pub choices_left: usize,
}

/// A visible hand type's row in the run info: level, chips, mult, times
/// played, times played this round.
pub type HandRow = (i32, f64, f64, u32, u32);

/// Everything the player can see.
#[derive(Clone, Debug, PartialEq)]
pub struct Screen {
    pub phase: Phase,
    pub ante: i32,
    pub round: i32,
    pub money: i64,
    pub hands_left: i32,
    pub discards_left: i32,
    pub hand_size: i32,
    pub joker_slots: i32,
    pub consumable_slots: i32,
    pub won: bool,
    pub sort_mode: SortMode,
    /// The blind being played (or on deck at blind select): kind, target,
    /// reward, disabled.
    pub blind: Option<(BlindKind, f64, i64, bool)>,
    pub round_chips: f64,
    pub slots: [SlotState; 3],
    pub boss: Boss,
    pub tags_offered: [TagKind; 2],
    /// The hand an offered Orbital Tag would level, per slot.
    pub orbital_offered: [Option<HandType>; 2],
    pub tags_held: Vec<(TagKind, Option<HandType>)>,
    /// Per hand type: level, chips, mult, played, played this round; only
    /// for visible hands.
    pub hands: [Option<HandRow>; 12],
    pub hand: Vec<CardView>,
    pub jokers: Vec<JokerView>,
    /// What the deck view shows: the cards left in the draw pile, as counts
    /// by (suit, rank), and how many of them carry each modifier. As in the
    /// game's view of the remaining deck, cards lying face down in hand are
    /// counted with them (the view marks how many with a "?"): it never tells
    /// which cards they are.
    pub deck_counts: [u8; 52],
    pub deck_enhancements: [u8; 9],
    pub deck_editions: [u8; 5],
    pub deck_seals: [u8; 5],
    pub deck_left: u32,
    pub deck_total: u32,
    /// Targets shown on held jokers: The Idol, Mail-In Rebate, Ancient
    /// Joker, Castle.
    pub idol: Option<(Rank, Suit)>,
    pub mail_rank: Option<Rank>,
    pub ancient_suit: Option<Suit>,
    pub castle_suit: Option<Suit>,
    /// The Ox's hand, shown while The Ox is the boss.
    pub ox_hand: Option<HandType>,
    pub skips: i32,
    pub hands_played_run: i32,
    pub unused_discards: i32,
    /// The cash-out screen's total.
    pub cash_out: Option<i64>,
    /// Held consumables: kind, edition, sell value.
    pub consumables: Vec<(ConsumableKind, Edition, i64)>,
    pub vouchers: [bool; N_VOUCHERS],
    pub shop: Option<ShopView>,
    pub pack: Option<PackView>,
    pub reroll_cost: i64,
    pub interest_cap: i64,
    pub joker_max: i32,
    pub tarots_used: u32,
    pub planets_used: u32,
    /// Distinct Planets used (Satellite shows it).
    pub planet_kinds_used: u32,
    /// The card The Fool would make.
    pub last_tarot_planet: Option<ConsumableKind>,
    /// Ectoplasm's hand-size cost (its description shows it).
    pub ecto_minus: i32,
}

/// A joker as its card and description show it now. `own` is its place in
/// the joker area (None for shop and pack cards). Per-frame values are
/// refreshed as the game does every frame; Loyalty Card shows how many hands
/// remain before its X4 (0 = the next hand).
fn joker_view(run: &Run, j: &crate::jokers::Joker, own: Option<usize>) -> JokerView {
    if j.face_down {
        return JokerView {
            kind: None,
            edition: Edition::None,
            eternal: false,
            perishable: None,
            rental: false,
            debuffed: false,
            sell_value: 0,
            mult: 0.0,
            x_mult: 0.0,
            chips: 0.0,
            counter: 0,
            hand: None,
        };
    }
    let mut v = j.clone();
    run.frame_refresh(&mut v, own);
    let counter = if v.kind == JokerKind::LoyaltyCard {
        (5 - (run.hands_played_run as i64 - v.counter)).rem_euclid(6)
    } else {
        v.counter
    };
    JokerView {
        kind: Some(v.kind),
        edition: v.edition,
        eternal: v.eternal,
        perishable: v.perishable.then_some(v.perish_tally),
        rental: v.rental,
        debuffed: v.debuffed,
        sell_value: joker_sell_value(&v),
        mult: v.mult,
        x_mult: v.x_mult,
        chips: v.chips,
        counter,
        hand: v.hand,
    }
}

fn card_view(c: &Card) -> CardView {
    CardView {
        face: if c.face_down {
            None
        } else {
            Some((c.rank, c.suit, c.enhancement, c.edition, c.seal))
        },
        perma_bonus: if c.face_down { 0 } else { c.perma_bonus },
        debuffed: c.debuffed && !c.face_down,
        forced: c.forced,
    }
}

fn suit_index(s: Suit) -> usize {
    s.index()
}

fn rank_index(r: Rank) -> usize {
    (r.id() - 2) as usize
}

/// The visible state of a run.
pub fn screen(run: &Run) -> Screen {
    let blind = match run.phase {
        Phase::Round | Phase::RoundEval => run
            .blind
            .as_ref()
            .map(|b| (b.kind, b.chips, b.dollars, b.disabled)),
        Phase::BlindSelect => {
            let k = run.blind_in_slot(run.on_deck());
            let scaling = crate::blinds::scaling_for_stake(run.cfg.stake);
            Some((
                k,
                crate::blinds::blind_target(run.ante, scaling, k, 1.0),
                k.reward(run.cfg.stake),
                false,
            ))
        }
        _ => None,
    };
    let mut hands = [None; 12];
    for h in HandType::ALL {
        let i = run.hand_levels.get(h);
        if i.visible {
            hands[h.index()] = Some((i.level, i.chips, i.mult, i.played, i.played_this_round));
        }
    }
    let mut deck_counts = [0u8; 52];
    let mut deck_enhancements = [0u8; 9];
    let mut deck_editions = [0u8; 5];
    let mut deck_seals = [0u8; 5];
    let face_down = run.hand.iter().filter(|&&id| run.card(id).face_down);
    for &id in run.deck.iter().chain(face_down) {
        let c = run.card(id);
        deck_counts[suit_index(c.suit) * 13 + rank_index(c.rank)] += 1;
        deck_enhancements[c.enhancement as usize] += 1;
        deck_editions[c.edition as usize] += 1;
        deck_seals[c.seal as usize] += 1;
    }
    let holds = |k: JokerKind| run.jokers.iter().any(|j| j.kind == k && !j.face_down);
    let offered_orbital = |s: BlindSlot| {
        if run.tags_offered[s.index()] == TagKind::Orbital {
            run.orbital_hand(s)
        } else {
            None
        }
    };
    let boss_now =
        run.blind
            .as_ref()
            .and_then(|b| b.boss())
            .or(if run.phase == Phase::BlindSelect {
                Some(run.boss)
            } else {
                None
            });
    Screen {
        phase: run.phase,
        ante: run.ante,
        round: run.round,
        money: run.money,
        hands_left: run.hands_left,
        discards_left: run.discards_left,
        hand_size: run.hand_size(),
        joker_slots: run.joker_slots,
        consumable_slots: run.consumable_slots,
        won: run.won,
        sort_mode: run.sort_mode,
        blind,
        round_chips: run.round_chips,
        slots: run.slots,
        boss: run.boss,
        tags_offered: run.tags_offered,
        orbital_offered: [
            offered_orbital(BlindSlot::Small),
            offered_orbital(BlindSlot::Big),
        ],
        tags_held: run.tags.iter().map(|t| (t.kind, t.orbital_hand)).collect(),
        hands,
        hand: run.hand.iter().map(|&id| card_view(run.card(id))).collect(),
        jokers: run
            .jokers
            .iter()
            .enumerate()
            .map(|(i, j)| joker_view(run, j, Some(i)))
            .collect(),
        deck_counts,
        deck_enhancements,
        deck_editions,
        deck_seals,
        deck_left: run.deck.len() as u32,
        deck_total: run.playing_cards.len() as u32,
        idol: holds(JokerKind::TheIdol).then_some(run.idol),
        mail_rank: holds(JokerKind::MailInRebate).then_some(run.mail_rank),
        ancient_suit: if holds(JokerKind::AncientJoker) {
            run.ancient_suit
        } else {
            None
        },
        castle_suit: holds(JokerKind::Castle).then_some(run.castle_suit),
        ox_hand: (boss_now == Some(Boss::Ox)).then_some(run.most_played),
        skips: run.skips,
        hands_played_run: run.hands_played_run,
        unused_discards: run.unused_discards,
        cash_out: if run.phase == Phase::RoundEval {
            run.cash_out.as_ref().map(|c| c.total)
        } else {
            None
        },
        consumables: run
            .consumables
            .iter()
            .map(|c| (c.kind, c.edition, c.sell_cost))
            .collect(),
        vouchers: run.vouchers,
        shop: run.shop.as_ref().map(|s| ShopView {
            row: s
                .row
                .iter()
                .map(|r| {
                    let v = match &r.item {
                        RowItem::Joker(j) => ItemView::Joker(joker_view(run, j, None)),
                        RowItem::Consumable(c) => ItemView::Consumable(c.kind, c.edition),
                        RowItem::Card(c) => ItemView::Card(card_view(c)),
                    };
                    (v, r.cost)
                })
                .collect(),
            vouchers: s.vouchers.iter().map(|v| (v.voucher, v.cost)).collect(),
            boosters: s
                .boosters
                .iter()
                .map(|b| b.as_ref().map(|b| (b.booster, b.cost)))
                .collect(),
        }),
        pack: run.pack.as_ref().map(|p| PackView {
            booster: p.booster,
            cards: p
                .cards
                .iter()
                .map(|c| match c {
                    PackCard::Joker(j) => ItemView::Joker(joker_view(run, j, None)),
                    PackCard::Consumable(c) => ItemView::Consumable(c.kind, c.edition),
                    PackCard::Card(c) => ItemView::Card(card_view(c)),
                })
                .collect(),
            choices_left: p.choices_left,
        }),
        reroll_cost: run.reroll_cost,
        interest_cap: run.interest_cap,
        joker_max: run.joker_max,
        tarots_used: run.usage_total.tarot,
        planets_used: run.usage_total.planet,
        planet_kinds_used: ConsumableKind::ALL[22..34]
            .iter()
            .filter(|c| run.usage[c.index()] > 0)
            .count() as u32,
        last_tarot_planet: run.last_tarot_planet,
        ecto_minus: run.ecto_minus,
    }
}

/// Floats per hand card and per joker in the encoding.
pub const CARD_FEATURES: usize = 1 + 13 + 4 + 9 + 5 + 5 + 3;
pub const JOKER_FEATURES: usize = 1 + 1 + 5 + 4 + 1 + 1 + 4 + 12;

/// Length of the encoded observation.
pub const OBS_LEN: usize = GLOBAL_FEATURES
    + 12 * 6
    + MAX_HAND * CARD_FEATURES
    + MAX_JOKERS * JOKER_FEATURES
    + DECK_FEATURES
    + ECON_FEATURES;

/// Room in the encoding for consumables, shop items and pack cards.
pub const MAX_CONSUMABLES: usize = 8;
pub const MAX_ROW: usize = 4;
pub const MAX_SHOP_VOUCHERS: usize = 3;
pub const MAX_PACK: usize = 5;
pub const CONSUMABLE_FEATURES: usize = 1 + N_CONSUMABLES + 2;
pub const ITEM_FEATURES: usize = 3 + JOKER_FEATURES + N_CONSUMABLES + 1 + CARD_FEATURES + 1;
const BOOSTER_FEATURES: usize = 9;
const ECON_FEATURES: usize = MAX_CONSUMABLES * CONSUMABLE_FEATURES
    + N_VOUCHERS
    + MAX_ROW * ITEM_FEATURES
    + MAX_SHOP_VOUCHERS * (2 + N_VOUCHERS)
    + 2 * BOOSTER_FEATURES
    + BOOSTER_FEATURES
    + 1
    + MAX_PACK * ITEM_FEATURES
    + 8
    + N_CONSUMABLES;

const GLOBAL_FEATURES: usize = 6 // phase
    + 12 // scalars
    + 2 // sort mode
    + 3 + 1 + N_BOSSES + 3 // blind kind, target, boss one-hot, reward/disabled/round chips
    + 3 * 5 // slot states
    + N_BOSSES // this ante's boss
    + 2 * N_TAGS + 2 * 12 // offered tags and their Orbital hands
    + N_TAGS // held tag counts
    + 13 + 4 + 13 + 4 + 4 + 12 // joker targets and The Ox's hand
    + 4; // skips, hands played, unused discards, cash-out
const DECK_FEATURES: usize = 52 + 9 + 5 + 5 + 2;

/// Signed log scale for big numbers (targets and chips reach 1e20+).
fn slog(x: f64) -> f32 {
    if x.is_nan() {
        return 0.0;
    }
    (x.signum() * (1.0 + x.abs()).ln()) as f32
}

fn one_hot(out: &mut Vec<f32>, n: usize, i: Option<usize>) {
    let start = out.len();
    out.resize(start + n, 0.0);
    if let Some(i) = i {
        if i < n {
            out[start + i] = 1.0;
        }
    }
}

/// The joker kind's index for an embedding (0 = empty slot, 1 = face down,
/// 2.. = kinds in game order).
pub fn joker_token(v: &JokerView) -> usize {
    match v.kind {
        Some(k) => 2 + k.index(),
        None => 1,
    }
}

/// Encode a screen as `OBS_LEN` floats.
pub fn encode(s: &Screen, out: &mut Vec<f32>) {
    out.clear();
    out.reserve(OBS_LEN);
    let phase = match s.phase {
        Phase::BlindSelect => 0,
        Phase::Round => 1,
        Phase::RoundEval => 2,
        Phase::Shop => 3,
        Phase::GameOver => 4,
        Phase::Pack => 5,
    };
    one_hot(out, 6, Some(phase));
    out.extend_from_slice(&[
        s.ante as f32 / 8.0,
        s.round as f32 / 24.0,
        slog(s.money as f64),
        s.money as f32 / 25.0,
        s.hands_left as f32 / 4.0,
        s.discards_left as f32 / 4.0,
        s.hand_size as f32 / 8.0,
        s.joker_slots as f32 / 5.0,
        s.consumable_slots as f32 / 2.0,
        f32::from(s.won),
        s.hand.len() as f32 / 8.0,
        s.jokers.len() as f32 / 5.0,
    ]);
    one_hot(out, 2, Some(s.sort_mode as usize));
    match s.blind {
        Some((kind, target, reward, disabled)) => {
            let (k, boss) = match kind {
                BlindKind::Small => (0, None),
                BlindKind::Big => (1, None),
                BlindKind::Boss(b) => (2, Some(b.index())),
            };
            one_hot(out, 3, Some(k));
            out.push(slog(target));
            one_hot(out, N_BOSSES, boss);
            out.extend_from_slice(&[
                reward as f32 / 5.0,
                f32::from(disabled),
                slog(s.round_chips),
            ]);
        }
        None => {
            one_hot(out, 3, None);
            out.push(0.0);
            one_hot(out, N_BOSSES, None);
            out.extend_from_slice(&[0.0, 0.0, slog(s.round_chips)]);
        }
    }
    for st in s.slots {
        let i = match st {
            SlotState::Upcoming => 0,
            SlotState::Select => 1,
            SlotState::Current => 2,
            SlotState::Defeated => 3,
            SlotState::Skipped => 4,
        };
        one_hot(out, 5, Some(i));
    }
    one_hot(out, N_BOSSES, Some(s.boss.index()));
    for i in 0..2 {
        one_hot(out, N_TAGS, Some(s.tags_offered[i].index()));
        one_hot(out, 12, s.orbital_offered[i].map(|h| h.index()));
    }
    let start = out.len();
    out.resize(start + N_TAGS, 0.0);
    for (t, _) in &s.tags_held {
        out[start + t.index()] += 1.0;
    }
    one_hot(out, 13, s.idol.map(|(r, _)| rank_index(r)));
    one_hot(out, 4, s.idol.map(|(_, su)| suit_index(su)));
    one_hot(out, 13, s.mail_rank.map(rank_index));
    one_hot(out, 4, s.ancient_suit.map(suit_index));
    one_hot(out, 4, s.castle_suit.map(suit_index));
    one_hot(out, 12, s.ox_hand.map(|h| h.index()));
    out.extend_from_slice(&[
        s.skips as f32 / 4.0,
        s.hands_played_run as f32 / 32.0,
        s.unused_discards as f32 / 16.0,
        s.cash_out.map_or(0.0, |c| c as f32 / 10.0),
    ]);
    debug_assert_eq!(out.len(), GLOBAL_FEATURES);

    for h in s.hands {
        match h {
            Some((level, chips, mult, played, this_round)) => out.extend_from_slice(&[
                1.0,
                level as f32 / 4.0,
                slog(chips),
                slog(mult),
                played as f32 / 8.0,
                this_round as f32 / 2.0,
            ]),
            None => out.extend_from_slice(&[0.0; 6]),
        }
    }

    for i in 0..MAX_HAND {
        let start = out.len();
        out.resize(start + CARD_FEATURES, 0.0);
        if let Some(c) = s.hand.get(i) {
            write_card(&mut out[start..], c);
        }
    }

    for i in 0..MAX_JOKERS {
        let start = out.len();
        out.resize(start + JOKER_FEATURES, 0.0);
        if let Some(j) = s.jokers.get(i) {
            write_joker(&mut out[start..], j);
        }
    }

    out.extend(s.deck_counts.iter().map(|&c| c as f32));
    out.extend(s.deck_enhancements.iter().map(|&c| c as f32 / 8.0));
    out.extend(s.deck_editions.iter().map(|&c| c as f32 / 8.0));
    out.extend(s.deck_seals.iter().map(|&c| c as f32 / 8.0));
    out.extend_from_slice(&[s.deck_left as f32 / 52.0, s.deck_total as f32 / 52.0]);

    // Consumables, vouchers, the shop and an open pack.
    for i in 0..MAX_CONSUMABLES {
        let start = out.len();
        out.resize(start + CONSUMABLE_FEATURES, 0.0);
        if let Some(&(k, ed, sell)) = s.consumables.get(i) {
            let f = &mut out[start..];
            f[0] = 1.0;
            f[1 + k.index()] = 1.0;
            f[1 + N_CONSUMABLES] = f32::from(ed == Edition::Negative);
            f[2 + N_CONSUMABLES] = sell as f32 / 5.0;
        }
    }
    out.extend(s.vouchers.iter().map(|&v| f32::from(v)));
    let shop = s.shop.as_ref();
    for i in 0..MAX_ROW {
        let start = out.len();
        out.resize(start + ITEM_FEATURES, 0.0);
        if let Some((item, cost)) = shop.and_then(|sh| sh.row.get(i)) {
            write_item(&mut out[start..], item, *cost);
        }
    }
    for i in 0..MAX_SHOP_VOUCHERS {
        let start = out.len();
        out.resize(start + 2 + N_VOUCHERS, 0.0);
        if let Some(&(v, cost)) = shop.and_then(|sh| sh.vouchers.get(i)) {
            let f = &mut out[start..];
            f[0] = 1.0;
            f[1 + v.index()] = 1.0;
            f[1 + N_VOUCHERS] = cost as f32 / 10.0;
        }
    }
    for i in 0..2 {
        let start = out.len();
        out.resize(start + BOOSTER_FEATURES, 0.0);
        if let Some(&Some((b, cost))) = shop.and_then(|sh| sh.boosters.get(i)) {
            write_booster(&mut out[start..], b, cost);
        }
    }
    let start = out.len();
    out.resize(start + BOOSTER_FEATURES + 1, 0.0);
    if let Some(p) = s.pack.as_ref() {
        write_booster(&mut out[start..], p.booster, 0);
        out[start + BOOSTER_FEATURES] = p.choices_left as f32 / 2.0;
    }
    for i in 0..MAX_PACK {
        let start = out.len();
        out.resize(start + ITEM_FEATURES, 0.0);
        if let Some(item) = s.pack.as_ref().and_then(|p| p.cards.get(i)) {
            write_item(&mut out[start..], item, 0);
        }
    }
    out.extend_from_slice(&[
        s.reroll_cost as f32 / 5.0,
        s.interest_cap as f32 / 25.0,
        s.joker_max as f32 / 2.0,
        s.tarots_used as f32 / 10.0,
        s.planets_used as f32 / 10.0,
        s.planet_kinds_used as f32 / 12.0,
        s.ecto_minus as f32,
        s.consumable_slots as f32 / 2.0,
    ]);
    one_hot(out, N_CONSUMABLES, s.last_tarot_planet.map(|k| k.index()));
    debug_assert_eq!(out.len(), OBS_LEN);
}

fn write_card(f: &mut [f32], c: &CardView) {
    f[0] = 1.0;
    if let Some((r, su, e, ed, seal)) = c.face {
        f[1 + rank_index(r)] = 1.0;
        f[14 + suit_index(su)] = 1.0;
        f[18 + e as usize] = 1.0;
        f[27 + ed as usize] = 1.0;
        f[32 + seal as usize] = 1.0;
    }
    f[37] = c.perma_bonus as f32 / 25.0;
    f[38] = f32::from(c.debuffed);
    f[39] = f32::from(c.forced);
}

fn write_joker(f: &mut [f32], j: &JokerView) {
    f[0] = 1.0;
    f[1] = joker_token(j) as f32 / (N_JOKERS + 2) as f32;
    f[2 + j.edition as usize] = 1.0;
    f[7] = f32::from(j.eternal);
    f[8] = j.perishable.map_or(0.0, |t| t as f32 / 5.0);
    f[9] = f32::from(j.rental);
    f[10] = f32::from(j.debuffed);
    f[11] = j.sell_value as f32 / 5.0;
    f[12] = f32::from(j.kind.is_none());
    f[13] = slog(j.mult);
    f[14] = slog(j.x_mult);
    f[15] = slog(j.chips);
    f[16] = slog(j.counter as f64);
    if let Some(h) = j.hand {
        f[17 + h.index()] = 1.0;
    }
}

/// type (3) | joker | consumable one-hot | card | price.
fn write_item(f: &mut [f32], item: &ItemView, cost: i64) {
    match item {
        ItemView::Joker(j) => {
            f[0] = 1.0;
            write_joker(&mut f[3..3 + JOKER_FEATURES], j);
        }
        ItemView::Consumable(k, ed) => {
            f[1] = 1.0;
            let o = 3 + JOKER_FEATURES;
            f[o + k.index()] = 1.0;
            f[o + N_CONSUMABLES] = f32::from(*ed == Edition::Negative);
        }
        ItemView::Card(c) => {
            f[2] = 1.0;
            let o = 4 + JOKER_FEATURES + N_CONSUMABLES;
            write_card(&mut f[o..o + CARD_FEATURES], c);
        }
    }
    f[ITEM_FEATURES - 1] = cost as f32 / 5.0;
}

fn write_booster(f: &mut [f32], b: BoosterId, cost: i64) {
    let info = &BOOSTERS[b as usize];
    f[0] = 1.0;
    let k = match info.kind {
        PackKind::Arcana => 0,
        PackKind::Celestial => 1,
        PackKind::Standard => 2,
        PackKind::Buffoon => 3,
        PackKind::Spectral => 4,
    };
    f[1 + k] = 1.0;
    f[6] = info.size as f32 / 5.0;
    f[7] = info.choose as f32 / 2.0;
    f[8] = cost as f32 / 5.0;
}

/// Where each part of `encode`'s output lies, for a network that reads the
/// screen as objects: (name, start, slots, floats per slot). Parts with one
/// slot are global features.
pub fn obs_layout() -> Vec<(&'static str, usize, usize, usize)> {
    let mut at = 0;
    let mut out = Vec::new();
    let mut part = |name: &'static str, slots: usize, width: usize| {
        out.push((name, at, slots, width));
        at += slots * width;
    };
    part("global", 1, GLOBAL_FEATURES);
    part("hand_levels", 12, 6);
    part("hand", MAX_HAND, CARD_FEATURES);
    part("jokers", MAX_JOKERS, JOKER_FEATURES);
    part("deck", 1, DECK_FEATURES);
    part("consumables", MAX_CONSUMABLES, CONSUMABLE_FEATURES);
    part("vouchers", 1, N_VOUCHERS);
    part("shop_row", MAX_ROW, ITEM_FEATURES);
    part("shop_vouchers", MAX_SHOP_VOUCHERS, 2 + N_VOUCHERS);
    part("boosters", 2, BOOSTER_FEATURES);
    part("pack_booster", 1, BOOSTER_FEATURES + 1);
    part("pack", MAX_PACK, ITEM_FEATURES);
    part("tail", 1, 8 + N_CONSUMABLES);
    debug_assert_eq!(at, OBS_LEN);
    out
}

/// Length of the value network's input (`encode_value`).
pub const VALUE_LEN: usize = OBS_LEN + 3 * N_JOKERS;

/// The value input's layout version: bumped whenever `encode` or
/// `encode_value` changes what a float means, so a network trained on one
/// layout is refused by another (`learned.rs`).
pub const VALUE_INPUT_VERSION: u32 = 1;

/// What the value network reads: `encode`'s floats, then the jokers held, in
/// the shop and in an open pack as counts by kind (`encode` gives a joker's
/// kind as one number, which a plain network can't take apart).
pub fn encode_value(s: &Screen, out: &mut Vec<f32>) {
    encode(s, out);
    let base = out.len();
    out.resize(base + 3 * N_JOKERS, 0.0);
    let mut count = |area: usize, j: &JokerView| {
        if let Some(k) = j.kind {
            out[base + area * N_JOKERS + k.index()] += 1.0;
        }
    };
    for j in &s.jokers {
        count(0, j);
    }
    for (item, _) in s.shop.iter().flat_map(|sh| &sh.row) {
        if let ItemView::Joker(j) = item {
            count(1, j);
        }
    }
    for item in s.pack.iter().flat_map(|p| &p.cards) {
        if let ItemView::Joker(j) = item {
            count(2, j);
        }
    }
    debug_assert_eq!(out.len(), VALUE_LEN);
}

/// The joker kinds in the screen's order, as embedding tokens (for a
/// network that embeds jokers rather than reading `encode`'s scalar).
pub fn joker_tokens(s: &Screen) -> [u16; MAX_JOKERS] {
    let mut t = [0u16; MAX_JOKERS];
    for (i, j) in s.jokers.iter().take(MAX_JOKERS).enumerate() {
        t[i] = joker_token(j) as u16;
    }
    t
}

/// Face-down cards in hand (The Mark, The Fish, The Wheel, The House) don't show which cards they are, and the
/// game's view of the remaining deck keeps hiding them (it counts them with the deck). What the player can still
/// read: where each one sits when the hand is sorted by rank and when it is sorted by suit (the game sorts them by
/// what they are, and sorting costs nothing), and under The Mark that each is a face card. In a fair copy each is
/// dealt again from the cards the player hasn't seen (the deck and the face-down cards themselves) that would sit in
/// the same places among the cards in sight, in both sorts, and fit the boss. The pool is ordered by the salt and
/// the cards alone, so two runs that differ only in which of those cards lie face down make the same copies.
fn redeal_face_down(c: &mut Run, order: u64) {
    let down: Vec<usize> = (0..c.hand.len())
        .filter(|&i| c.card(c.hand[i]).face_down)
        .collect();
    if down.is_empty() {
        return;
    }
    let mark = c.blind.as_ref().and_then(|b| b.active_boss()) == Some(Boss::Mark);
    let pareidolia = c.pareidolia();
    let sight: Vec<(f64, f64)> = c
        .hand
        .iter()
        .map(|&id| c.card(id))
        .filter(|card| !card.face_down)
        .map(|card| (card.sort_key(SortMode::Rank), card.sort_key(SortMode::Suit)))
        .collect();
    let place = |id: u32| -> Vec<(std::cmp::Ordering, std::cmp::Ordering)> {
        let card = c.card(id);
        let (r, s) = (card.sort_key(SortMode::Rank), card.sort_key(SortMode::Suit));
        sight
            .iter()
            .map(|&(or, os)| (r.total_cmp(&or), s.total_cmp(&os)))
            .collect()
    };
    let fits_boss = |id: u32| !mark || c.card(id).is_face_for_boss(pareidolia);
    let mut pool: Vec<u32> = c
        .deck
        .iter()
        .copied()
        .chain(down.iter().map(|&i| c.hand[i]))
        .collect();
    pool.sort_by_cached_key(|&id| card_key(order ^ 0xFACE_D0E5_CA2D_5EED, id as u64));
    let mut dealt = Vec::with_capacity(down.len());
    for &i in &down {
        let own = c.hand[i];
        let want = place(own);
        let k = pool
            .iter()
            .position(|&id| fits_boss(id) && place(id) == want)
            .or_else(|| pool.iter().position(|&id| id == own))
            .or_else(|| pool.iter().position(|&id| fits_boss(id)))
            .unwrap_or(0);
        dealt.push(pool.remove(k));
    }
    for (&i, id) in down.iter().zip(dealt) {
        c.hand[i] = id;
        c.card_mut(id).face_down = true;
    }
    for &id in &pool {
        c.card_mut(id).face_down = false;
    }
    pool.sort_by_cached_key(|&id| card_key(order, id as u64));
    c.deck = pool;
}

/// A card's place in a fair copy's draw order (a well-mixed hash of the salt and the card).
fn card_key(salt: u64, id: u64) -> u64 {
    let mut z = salt ^ id.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// A copy of the run for lookahead whose hidden parts are reshuffled from
/// what the player could know: the draw order, every future random event
/// (a fresh seed, so future bosses, tags, shops and rolls are new; the
/// copy's `cfg.seed` is that seed, so the real one isn't in it), the ante's
/// voucher while no shop has shown it yet, the order of face-down jokers and
/// which cards the face-down cards in hand are (`redeal_face_down`). `salt`
/// picks the reshuffle.
///
/// The draw order comes from the salt and each card alone, never from the
/// cards' real order: copies of positions that hold the same cards deal them
/// alike (so the options of one decision, and doing nothing, are all judged
/// on the same deals), and a card added or removed changes the deals only
/// where it comes in. Face-down jokers likewise: the player knows which
/// jokers lie face down but not where, so they are set out in creation order
/// (where Amber Acorn's own shuffles start) and shuffled from there; a copy
/// never starts from the real row, so two runs that differ only in where
/// those jokers lie make the same copies.
pub fn fair_copy(run: &Run, salt: u64) -> Run {
    let mut c = run.clone();
    let mut sampler = SeedSampler::new(salt);
    let seed = sampler.next_seed();
    c.rng = Streams::new(&seed);
    c.cfg.seed = seed;
    let mut g = LuaRandom::new(sampler.next_u64() as f64 / u64::MAX as f64);
    let order = sampler.next_u64();
    c.deck.sort_by_cached_key(|&id| card_key(order, id as u64));
    redeal_face_down(&mut c, order);
    let down: Vec<usize> = (0..c.jokers.len())
        .filter(|&i| c.jokers[i].face_down)
        .collect();
    if down.len() > 1 {
        let mut known = down.clone();
        known.sort_by_key(|&i| c.jokers[i].sort_id);
        shuffle(&mut known, &mut g);
        let moved: Vec<_> = known.iter().map(|&i| c.jokers[i].clone()).collect();
        for (slot, j) in down.iter().zip(moved) {
            c.jokers[*slot] = j;
        }
    }
    if c.shop_voucher.is_some() && !c.shop_voucher_shown {
        let key = format!("Voucher{}", c.ante);
        c.shop_voucher = Some(c.next_voucher(&key));
    }
    c
}
