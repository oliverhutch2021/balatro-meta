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
use crate::joker_data::{JokerKind, N_JOKERS};
use crate::poker::HandType;
use crate::run::joker_sell_value;
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
    /// by (suit, rank), and how many of them carry each modifier.
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
    for &id in &run.deck {
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
            .map(|j| {
                if j.face_down {
                    JokerView {
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
                    }
                } else {
                    JokerView {
                        kind: Some(j.kind),
                        edition: j.edition,
                        eternal: j.eternal,
                        perishable: j.perishable.then_some(j.perish_tally),
                        rental: j.rental,
                        debuffed: j.debuffed,
                        sell_value: joker_sell_value(j),
                        mult: j.mult,
                        x_mult: j.x_mult,
                        chips: j.chips,
                        counter: j.counter,
                    }
                }
            })
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
    }
}

/// Floats per hand card and per joker in the encoding.
pub const CARD_FEATURES: usize = 1 + 13 + 4 + 9 + 5 + 5 + 3;
pub const JOKER_FEATURES: usize = 1 + 1 + 5 + 4 + 1 + 1 + 4;

/// Length of the encoded observation.
pub const OBS_LEN: usize = GLOBAL_FEATURES
    + 12 * 6
    + MAX_HAND * CARD_FEATURES
    + MAX_JOKERS * JOKER_FEATURES
    + DECK_FEATURES;

const GLOBAL_FEATURES: usize = 5 // phase
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
    };
    one_hot(out, 5, Some(phase));
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
        let Some(c) = s.hand.get(i) else { continue };
        let f = &mut out[start..];
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

    for i in 0..MAX_JOKERS {
        let start = out.len();
        out.resize(start + JOKER_FEATURES, 0.0);
        let Some(j) = s.jokers.get(i) else { continue };
        let f = &mut out[start..];
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
    }

    out.extend(s.deck_counts.iter().map(|&c| c as f32));
    out.extend(s.deck_enhancements.iter().map(|&c| c as f32 / 8.0));
    out.extend(s.deck_editions.iter().map(|&c| c as f32 / 8.0));
    out.extend(s.deck_seals.iter().map(|&c| c as f32 / 8.0));
    out.extend_from_slice(&[s.deck_left as f32 / 52.0, s.deck_total as f32 / 52.0]);
    debug_assert_eq!(out.len(), OBS_LEN);
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

/// A copy of the run for lookahead whose hidden parts are reshuffled from
/// what the player could know: the draw order, every future random event
/// (a fresh seed, so future bosses, tags, shops and rolls are new) and the
/// order of face-down jokers. `salt` picks the reshuffle.
pub fn fair_copy(run: &Run, salt: u64) -> Run {
    let mut c = run.clone();
    let mut sampler = SeedSampler::new(salt);
    let seed = sampler.next_seed();
    c.rng = Streams::new(&seed);
    let mut g = LuaRandom::new(sampler.next_u64() as f64 / u64::MAX as f64);
    shuffle(&mut c.deck, &mut g);
    let down: Vec<usize> = (0..c.jokers.len())
        .filter(|&i| c.jokers[i].face_down)
        .collect();
    if down.len() > 1 {
        let mut order = down.clone();
        shuffle(&mut order, &mut g);
        let moved: Vec<_> = order.iter().map(|&i| c.jokers[i].clone()).collect();
        for (slot, j) in down.iter().zip(moved) {
            c.jokers[*slot] = j;
        }
    }
    c
}
