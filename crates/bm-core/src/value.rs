//! The teacher's value of a position: one number that every decision between rounds tries to raise. It
//! weighs scoring power against the blinds coming up, what the run will gain by itself as it goes on, money
//! and held cards. Nothing in it says what to buy or how to build: buying a joker, using a Tarot on some
//! cards, keeping $25 or opening a pack is worth whatever it does to this number.
//!
//! - **Scoring power is measured, not assumed.** The coming round is played out on fair copies (hidden
//!   cards reshuffled, fresh random streams) with the real engine: the run's own deck, jokers, levels,
//!   enhancements, editions and seals, every hand played and no target, so each sample is what that deal
//!   can score. Synergies, the chips / mult / x-mult balance, deck fixing and hand levels all show up in
//!   the totals. This ante's boss is played out too, with its effect, when it isn't the next blind.
//! - **The blinds ahead.** Each blind left in this ante and the next ante's three is weighed by the chance
//!   of beating it: the sample totals, grown by what the run gains by itself until then, against the
//!   blind's target. So when a blind is at risk, power is worth much more than money.
//! - **Growth** (`flows`): what jokers and cards add each round by themselves, from what they do and the
//!   run they're in: growing jokers' chips, mult and x-mult, levels of the main hand (Burnt Joker, Space
//!   Joker, Blue Seals), money (econ jokers, Gold cards and Seals, interest), Tarots and Spectrals made.
//! - **Money** is scoring power not bought yet: it can only help the blinds after the next shop, so it
//!   counts inside their chances (worth nothing if the run dies first), with less for each dollar the more
//!   is held, plus the interest and income it brings in on the way. Held Tarots and Spectrals count like
//!   money.
//! - **The long run**: scoring power a few antes on, so a joker that grows is worth buying before it is
//!   needed.
//!
//! Every estimate uses the caller's salt for its fair copies, so options compared in one decision see the
//! same deals (common random numbers): their differences are their own, not luck.

use std::cell::RefCell;
use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};

use crate::blinds::{
    base_amount, blind_target, scaling_for_stake, BlindKind, BlindSlot, SlotState,
};
use crate::bots::preview;
use crate::card::{Enhancement, Rank, Seal, Suit};
use crate::content::ConsumableSet;
use crate::env::{apply, MoveKind};
use crate::hand_levels::base_values;
use crate::joker_data::JokerKind;
use crate::observe::fair_copy;
use crate::planner::{plan_move, quick_best, seen, Plan};
use crate::poker::HandType;
use crate::state::{Phase, Run};

/// Deals played out per estimate (each by two simple players, the better kept).
pub const SAMPLES: usize = 6;
/// Deals for a quick first look (screening options before the full estimate).
pub const QUICK: usize = 2;
/// How sharply a blind's chance turns with the log margin of a round's total over its target, and the
/// teacher's planner's edge over the simple players here (a log factor): fitted to the teacher's own
/// rounds (120 shop positions, 6 rounds each).
const SHARPNESS: f64 = 3.5;
const PLANNER_EDGE: f64 = 0.5;
/// Weight of the long run (log scoring power some rounds on).
const LONG_RUN: f64 = 0.5;
/// Rounds the long run looks ahead.
const LONG_ROUNDS: f64 = 9.0;
/// Log scoring power a dollar buys in a shop at ante 1 (falling with the ante: late shops add less to a
/// big score), and the money at which each further dollar is worth half as much.
const BUYING_POWER: f64 = 0.03;
const MONEY_SCALE: f64 = 25.0;
/// Held Tarots and Spectrals, in dollars.
const TAROT: f64 = 3.0;
const SPECTRAL: f64 = 5.0;

/// What playing the coming rounds out showed.
#[derive(Clone, Debug)]
pub struct Outlook {
    /// ln of each sample round's total (every hand played, no target), for the next blind.
    pub next: Vec<f64>,
    /// The same for this ante's boss with its effect, when the boss isn't next (else empty).
    pub boss: Vec<f64>,
    /// Mean chips and mult of the hands played, for the growth arithmetic.
    pub chips: f64,
    pub mult: f64,
    /// The hand type that scored the most points.
    pub main: HandType,
}

/// A fair copy of the run, moved on to the start of its next round (closing a pack, cashing out, leaving
/// the shop, choosing the blind), or None if the run is over. In a round it is that round.
pub fn next_round(run: &Run, salt: u64) -> Option<Run> {
    let mut c = fair_copy(run, salt);
    for _ in 0..12 {
        let step = match c.phase {
            Phase::Round => return Some(c),
            Phase::GameOver => return None,
            Phase::Pack => MoveKind::PackSkip,
            Phase::RoundEval => MoveKind::CashOut,
            Phase::Shop => MoveKind::LeaveShop,
            Phase::BlindSelect => MoveKind::SelectBlind,
        };
        apply(&mut c, step, 0).ok()?;
    }
    None
}

/// One hand played in a sample round: (type, chips, mult, score).
pub type Played = (HandType, f64, f64, f64);

/// The round from `start` played to its end with no target, by one of two simple players: keep the best
/// hand and throw the rest while discards last (`shape` false), or build the most promising shape.
pub fn play_out(start: &Run, shape: bool) -> (f64, Vec<Played>) {
    let mut c = start.clone();
    let mut hands = Vec::new();
    let mut steps = 0;
    // The teacher sets its jokers in scoring order before it plays.
    while let Some(m) = crate::teacher::reorder(&c) {
        if apply(&mut c, m.0, m.1).is_err() || steps > 20 {
            break;
        }
        steps += 1;
    }
    steps = 0;
    while c.phase == Phase::Round && steps < 40 {
        // Verdant Leaf: a joker is sold so the cards count (as the teacher does).
        if let Some(m) = crate::teacher::verdant_sale(&c) {
            if apply(&mut c, m.0, m.1).is_err() {
                break;
            }
            steps += 1;
            continue;
        }
        let best = quick_best(&c);
        let mv = if c.discards_left > 0 {
            let plan = if shape {
                Plan::options(&seen(&c))
                    .first()
                    .copied()
                    .unwrap_or(Plan::Best)
            } else {
                Plan::Best
            };
            plan_move(&c, plan, best)
        } else {
            (MoveKind::Play, best.1)
        };
        if mv.0 == MoveKind::Play {
            let h = preview(&c, mv.1);
            hands.push((h.hand, h.chips, h.mult, h.score));
        }
        if apply(&mut c, mv.0, mv.1).is_err() {
            break;
        }
        steps += 1;
    }
    (c.round_chips, hands)
}

/// Both players on one deal; the better total and its hands.
fn sample_round(start: &Run) -> (f64, Vec<Played>) {
    let a = play_out(start, false);
    let b = play_out(start, true);
    if b.0 > a.0 {
        b
    } else {
        a
    }
}

fn take_target_away(c: &mut Run) {
    if let Some(b) = c.blind.as_mut() {
        b.chips = f64::INFINITY;
    }
}

/// Play the coming round (and this ante's boss, if it isn't next) out on `samples` deals.
pub fn outlook(run: &Run, salt: u64, samples: usize) -> Option<Outlook> {
    let mut next = Vec::with_capacity(samples);
    let mut boss = Vec::new();
    let (mut chips, mut mult, mut n) = (0.0, 0.0, 0.0);
    let mut points = [0.0f64; 12];
    let boss_ahead = run.phase != Phase::Round
        && !matches!(run.slots[BlindSlot::Boss.index()], SlotState::Defeated)
        && !run.slots[..2]
            .iter()
            .all(|s| matches!(s, SlotState::Defeated | SlotState::Skipped));
    for k in 0..samples {
        let mut c = next_round(run, salt.wrapping_add(k as u64 * 0x9E37_79B9))?;
        let mut b = boss_ahead.then(|| {
            let mut b = c.clone();
            b.force_blind(BlindKind::Boss(run.boss));
            b
        });
        take_target_away(&mut c);
        let (total, hands) = sample_round(&c);
        next.push(total.max(1.0).ln());
        for (h, ch, mu, sc) in hands {
            chips += ch;
            mult += mu;
            n += 1.0;
            points[h as usize] += sc;
        }
        if let Some(b) = b.as_mut() {
            take_target_away(b);
            boss.push(sample_round(b).0.max(1.0).ln());
        }
    }
    let main = HandType::ALL
        .iter()
        .copied()
        .max_by(|a, b| points[*a as usize].total_cmp(&points[*b as usize]))
        .unwrap_or(HandType::Pair);
    Some(Outlook {
        next,
        boss,
        chips: if n > 0.0 { chips / n } else { 10.0 },
        mult: if n > 0.0 { mult / n } else { 2.0 },
        main,
    })
}

/// What the run gains by itself per round, from what its jokers and cards do in the run they're in.
#[derive(Clone, Debug, Default)]
pub struct Flows {
    /// Chips added to a hand's chips per round (growing jokers, Hiker's cards).
    pub chips: f64,
    /// Mult added (before the x-mults) per round.
    pub mult: f64,
    /// Each growing x-mult joker: (its x-mult now, change per round).
    pub xmult: Vec<(f64, f64)>,
    /// Levels of the main hand per round.
    pub levels: f64,
    /// Dollars per round (not counting interest and blind rewards).
    pub money: f64,
    /// Tarots and Spectrals made per round.
    pub tarots: f64,
    pub spectrals: f64,
    /// Log scoring power lost per round (jokers destroyed or wearing out).
    pub drain: f64,
}

/// The run's per-round flows. Rates are rough judgements of what each effect yields per round in a run
/// like this one (its deck, its main hand), to be checked against the engine's own runs.
pub fn flows(run: &Run, main: HandType) -> Flows {
    use JokerKind::*;
    let deck = run.playing_cards.len().max(1) as f64;
    let count = |pred: &dyn Fn(&crate::card::Card) -> bool| {
        run.playing_cards
            .iter()
            .filter(|&&id| pred(run.card(id)))
            .count() as f64
    };
    let faces = count(&|c| c.rank.is_face()) / deck;
    let enhanced = count(&|c| c.enhancement != Enhancement::None);
    let lucky = count(&|c| c.enhancement == Enhancement::Lucky);
    let glass = count(&|c| c.enhancement == Enhancement::Glass);
    let gold_cards = count(&|c| c.enhancement == Enhancement::Gold);
    let of_rank = |r: Rank| count(&|c| c.rank == r);
    let diamonds = count(&|c| c.suit == Suit::Diamonds) / deck;
    let hands = run.hands_per_round.max(1) as f64;
    let discards = run.discards_per_round.max(0) as f64;
    // Cards scored and thrown per round, and the chance a card is in hand when the round ends.
    let scored = hands * 3.5;
    let thrown = discards * 3.0;
    let held_at_end = (run.hand_size().max(0) as f64 / deck).min(1.0);
    let n_jokers = run.jokers.len() as f64;
    let is = |h: &[HandType]| h.contains(&main);
    let mut f = Flows::default();
    for j in &run.jokers {
        if j.debuffed {
            continue;
        }
        let x = j.x_mult.max(1.0);
        match j.kind {
            // Growing chips and mult.
            GreenJoker => f.mult += hands - 0.5 * discards,
            RideTheBus => f.mult += if faces < 0.12 { 2.0 } else { 0.4 },
            Supernova => f.mult += hands * 0.7,
            SquareJoker => {
                f.chips += if is(&[HandType::TwoPair, HandType::FourOfAKind]) {
                    4.0 * hands * 0.7
                } else {
                    4.0
                }
            }
            Runner => {
                f.chips += if is(&[HandType::Straight]) {
                    15.0 * hands * 0.7
                } else {
                    3.0
                }
            }
            WeeJoker => f.chips += 8.0 * scored * of_rank(Rank::Two) / deck,
            Hiker => f.chips += 5.0 * scored * 5.0 / deck,
            Castle => f.chips += 3.0 * thrown / 4.0,
            SpareTrousers => {
                f.mult += if is(&[HandType::TwoPair, HandType::FullHouse]) {
                    2.0 * hands * 0.7
                } else {
                    0.5
                }
            }
            FlashCard => f.mult += 1.0,
            RedCard => f.mult += 0.5,
            FortuneTeller => f.mult += 0.5,
            IceCream => f.chips -= 5.0 * hands,
            Popcorn => f.mult -= 4.0,
            GrosMichel => f.mult -= 15.0 / 6.0,
            // Growing x-mult.
            Constellation => f.xmult.push((x, 0.08)),
            Hologram => f.xmult.push((x, 0.06)),
            Vampire => f.xmult.push((x, 0.1 * (scored * enhanced / deck).min(4.0))),
            LuckyCat => f.xmult.push((x, 0.25 * scored * lucky / deck / 5.0)),
            GlassJoker => f.xmult.push((x, 0.75 * scored * glass / deck / 4.0)),
            Campfire => f.xmult.push((x, 0.05)),
            Obelisk => f.xmult.push((x, 0.1)),
            Madness => {
                f.xmult.push((x, 0.33));
                f.drain += 0.03; // it destroys a random joker at every Small and Big Blind
            }
            Yorick => f.xmult.push((x, thrown / 23.0)),
            Ramen => f.xmult.push((j.x_mult, -0.01 * thrown)),
            // Levels of the main hand.
            BurntJoker => f.levels += 0.7,
            SpaceJoker => f.levels += hands * 0.25 * 0.7,
            // Money.
            GoldenJoker => f.money += 4.0,
            Egg => f.money += 3.0,
            GiftCard => f.money += n_jokers + run.consumables.len() as f64,
            Cloud9 => f.money += of_rank(Rank::Nine),
            Rocket => f.money += j.counter.max(1) as f64,
            DelayedGratification => f.money += discards,
            Satellite => {
                f.money += HandType::ALL
                    .iter()
                    .filter(|&&h| run.hand_levels.get(h).level > 1)
                    .count() as f64
            }
            BusinessCard => f.money += scored * faces,
            ReservedParking => f.money += hands * 3.0 * faces * 0.5,
            FacelessJoker => f.money += 1.0,
            MailInRebate => f.money += 5.0 * thrown / 13.0,
            TradingCard => f.money += 3.0,
            GoldenTicket => f.money += 4.0 * scored * gold_cards / deck,
            RoughGem => f.money += scored * diamonds,
            Matador => f.money += 1.3,
            MidasMask => f.money += 1.0,
            // Cards made.
            Cartomancer => f.tarots += 1.0,
            Perkeo => f.tarots += 1.0,
            Hallucination => f.tarots += 0.3,
            EightBall => f.tarots += scored * of_rank(Rank::Eight) / deck / 4.0,
            Vagabond => f.tarots += 0.2,
            Superposition => f.tarots += if is(&[HandType::Straight]) { 0.5 } else { 0.0 },
            SixthSense => f.spectrals += 0.2,
            Seance => f.spectrals += 0.05,
            _ => {}
        }
    }
    // Cards: Blue Seals held at the end make a Planet for the hand last played; Gold Seals pay when
    // played; Gold cards pay when held at the end; Lucky cards sometimes pay; Purple Seals make a Tarot
    // when thrown.
    let mut blue = 0.0;
    let mut purple = 0.0;
    let mut gold_seal = 0.0;
    for &id in &run.playing_cards {
        match run.card(id).seal {
            Seal::Blue => blue += 1.0,
            Seal::Purple => purple += 1.0,
            Seal::Gold => gold_seal += 1.0,
            _ => {}
        }
    }
    f.levels += blue * held_at_end;
    f.money += 3.0 * gold_seal * scored / deck
        + 3.0 * gold_cards * held_at_end
        + 20.0 / 15.0 * lucky * scored / deck;
    f.tarots += purple * thrown / deck;
    f
}

/// Log growth of a round's score over `rounds` rounds from the flows.
fn growth(f: &Flows, o: &Outlook, run: &Run, rounds: f64) -> f64 {
    if rounds <= 0.0 {
        return 0.0;
    }
    let (_, _, level_chips, level_mult) = base_values(o.main);
    let x_now: f64 = run
        .jokers
        .iter()
        .filter(|j| !j.debuffed && j.x_mult > 1.0)
        .map(|j| j.x_mult)
        .product::<f64>()
        .max(1.0);
    let chips = o.chips.max(1.0);
    let added_mult = (o.mult / x_now).max(1.0);
    let more_chips = (f.chips + f.levels * level_chips) * rounds;
    let more_mult = (f.mult + f.levels * level_mult) * rounds;
    let mut g = ((chips + more_chips).max(1.0) / chips).ln()
        + ((added_mult + more_mult).max(1.0) / added_mult).ln();
    for &(x, rate) in &f.xmult {
        let x = x.max(0.1);
        g += ((x + rate * rounds).max(0.1) / x).ln();
    }
    g - f.drain * rounds
}

fn sigmoid(z: f64) -> f64 {
    1.0 / (1.0 + (-z).exp())
}

/// The chance of beating `target`, from sample log totals moved by `shift`.
fn chance(samples: &[f64], shift: f64, target: f64) -> f64 {
    if samples.is_empty() {
        return 0.5;
    }
    let t = target.max(1.0).ln();
    samples
        .iter()
        .map(|&s| sigmoid(SHARPNESS * (s + shift + PLANNER_EDGE - t)))
        .sum::<f64>()
        / samples.len() as f64
}

/// The blinds ahead: (target chips, rounds from now, weight, this ante's boss).
fn blinds_ahead(run: &Run) -> Vec<(f64, f64, f64, bool)> {
    let scaling = scaling_for_stake(run.cfg.stake);
    let mut out = Vec::new();
    let mut n = 0.0;
    for slot in BlindSlot::ALL {
        if matches!(
            run.slots[slot.index()],
            SlotState::Defeated | SlotState::Skipped
        ) {
            continue;
        }
        let kind = match slot {
            BlindSlot::Small => BlindKind::Small,
            BlindSlot::Big => BlindKind::Big,
            BlindSlot::Boss => BlindKind::Boss(run.boss),
        };
        out.push((
            blind_target(run.ante, scaling, kind, 1.0),
            n,
            1.0,
            slot == BlindSlot::Boss,
        ));
        n += 1.0;
    }
    if run.ante < run.win_ante {
        for mult in [1.0, 1.5, 2.0] {
            out.push((base_amount(run.ante + 1, scaling) * mult, n, 0.7, false));
            n += 1.0;
        }
    }
    out
}

/// Rounds left in the run (to the winning boss).
fn rounds_left(run: &Run) -> f64 {
    let done = run.slots[..]
        .iter()
        .filter(|s| matches!(s, SlotState::Defeated | SlotState::Skipped))
        .count() as f64;
    ((run.win_ante - run.ante).max(0) as f64 * 3.0 + 3.0 - done).max(1.0)
}

/// Log scoring power a dollar buys now: less late (a shop adds less to a big score), more with vouchers
/// that make shops cheaper or wider, less with no joker slot free (a new joker then replaces one).
pub fn buying_power(run: &Run) -> f64 {
    use crate::content::VoucherKind::*;
    let has = |v: crate::content::VoucherKind| run.vouchers[v.index()];
    let mut shops = 1.0;
    for (v, f) in [
        (ClearanceSale, 1.25),
        (Liquidation, 1.3),
        (Overstock, 1.15),
        (OverstockPlus, 1.12),
        (RerollSurplus, 1.05),
        (RerollGlut, 1.05),
        (Hone, 1.03),
        (GlowUp, 1.03),
        (CrystalBall, 1.03),
        (TarotMerchant, 1.02),
        (TarotTycoon, 1.02),
        (PlanetMerchant, 1.02),
        (PlanetTycoon, 1.02),
        (OmenGlobe, 1.02),
        (MagicTrick, 1.01),
        (Illusion, 1.02),
        (Telescope, 1.03),
    ] {
        if has(v) {
            shops *= f;
        }
    }
    let used = run
        .jokers
        .iter()
        .filter(|j| j.edition != crate::card::Edition::Negative)
        .count() as i32;
    let free = (run.joker_slots - used).max(0) as f64;
    let slots = 0.65 + 0.35 * free.min(1.0) + 0.05 * (free - 1.0).max(0.0);
    BUYING_POWER * (1.0 - 0.08 * (run.ante.clamp(1, 8) - 1) as f64) * shops * slots
}

/// Scoring power that `dollars` will buy (each further dollar buying less).
fn bought(run: &Run, dollars: f64) -> f64 {
    buying_power(run) * MONEY_SCALE * (1.0 + dollars.max(0.0) / MONEY_SCALE).ln()
}

/// A held tag in dollars: what it will give (a free joker in the next shop, money after the boss...).
/// Tags that act at once (money, levels, packs, jokers) are never held, so they count where they land.
fn tag_dollars(t: crate::tags::TagKind) -> f64 {
    use crate::tags::TagKind::*;
    match t {
        Uncommon => 6.0,
        Rare => 9.0,
        Negative => 12.0,
        Foil => 5.0,
        Holographic => 7.0,
        Polychrome => 10.0,
        Investment => 25.0,
        Voucher => 10.0,
        Coupon => 12.0,
        D6 => 5.0,
        Juggle => 2.0,
        Double => 6.0,
        Boss => 2.0,
        _ => 3.0,
    }
}

/// Money now, with held Tarots, Spectrals and tags counted as dollars.
fn money_now(run: &Run) -> f64 {
    let held: f64 = run
        .consumables
        .iter()
        .map(|c| match c.kind.set() {
            ConsumableSet::Spectral => SPECTRAL,
            _ => TAROT,
        })
        .sum();
    let tags: f64 = run.tags.iter().map(|t| tag_dollars(t.kind)).sum();
    run.money.max(0) as f64 + held + tags
}

/// Dollars coming in per round beyond what every round pays (blind rewards are the same whatever is
/// decided, so they are left out): interest on what is held, the run's income, cards it makes.
fn income(run: &Run, f: &Flows) -> f64 {
    let interest = if run.interest_amount > 0 {
        (run.interest_amount * (run.money.max(0) / 5).min(run.interest_cap / 5)) as f64
    } else {
        0.0
    };
    interest + f.money + f.tarots * TAROT + f.spectrals * SPECTRAL
}

fn mean(v: &[f64]) -> f64 {
    if v.is_empty() {
        0.0
    } else {
        v.iter().sum::<f64>() / v.len() as f64
    }
}

/// The value of a position between rounds (shop, pack, blind choice, cash-out). Higher is better.
pub fn value(run: &Run, salt: u64) -> f64 {
    value_with(run, salt, SAMPLES)
}

/// A quicker, rougher value (fewer deals; the same first deals as `value`), for screening options.
pub fn quick(run: &Run, salt: u64) -> f64 {
    value_with(run, salt, QUICK)
}

/// The value of a position in a round, judged as if the round began again with the same deck, jokers and
/// blind: for decisions whose effect outlasts the round (changing or adding cards, using held cards).
pub fn value_in_round(run: &Run, salt: u64, samples: usize) -> f64 {
    if run.phase != Phase::Round {
        return value_with(run, salt, samples);
    }
    let mut c = run.clone();
    let back: Vec<_> = c
        .hand
        .drain(..)
        .chain(c.play.drain(..))
        .chain(c.discard.drain(..))
        .collect();
    c.deck.extend(back);
    c.round_chips = 0.0;
    for slot in BlindSlot::ALL {
        if c.slots[slot.index()] == SlotState::Current {
            c.slots[slot.index()] = SlotState::Select;
        }
    }
    c.blind = None;
    c.phase = Phase::BlindSelect;
    value_with(&c, salt, samples)
}

pub fn value_with(run: &Run, salt: u64, samples: usize) -> f64 {
    if run.won {
        return 1e6;
    }
    let key = (state_key(run) ^ samples as u64, salt);
    if let Some(v) = CACHE.with(|c| c.borrow().get(&key).copied()) {
        return v;
    }
    let v = breakdown_with(run, salt, samples).map_or(-1e6, |b| b.total);
    CACHE.with(|c| {
        let mut c = c.borrow_mut();
        if c.len() > 4096 {
            c.clear();
        }
        c.insert(key, v);
    });
    v
}

/// How a position's value is made up (for checking the model and for showing a decision's reasons).
#[derive(Clone, Debug)]
pub struct Breakdown {
    /// Each blind ahead: (target, chance of beating it, weight).
    pub blinds: Vec<(f64, f64, f64)>,
    /// Median sample total for the next blind.
    pub next_total: f64,
    pub main: HandType,
    pub long_run: f64,
    pub money: f64,
    pub assets: f64,
    pub flows: Flows,
    pub total: f64,
}

pub fn breakdown(run: &Run, salt: u64) -> Option<Breakdown> {
    breakdown_with(run, salt, SAMPLES)
}

fn breakdown_with(run: &Run, salt: u64, samples: usize) -> Option<Breakdown> {
    let o = outlook(run, salt, samples)?;
    let f = flows(run, o.main);
    let money = money_now(run);
    let per_round = income(run, &f);
    let mut blinds = Vec::new();
    let mut v = 0.0;
    for (target, n, w, boss) in blinds_ahead(run) {
        let samples = if boss && !o.boss.is_empty() {
            &o.boss
        } else {
            &o.next
        };
        // Money helps only once a shop has been passed: every blind after the next.
        let shopping = if n >= 1.0 {
            bought(run, money + per_round * n)
        } else {
            0.0
        };
        let p = chance(samples, growth(&f, &o, run, n) + shopping, target);
        v += w * p.max(1e-6).ln();
        blinds.push((target, p, w));
    }
    let long = LONG_ROUNDS.min(rounds_left(run));
    let shopping = bought(run, money + per_round * long);
    let long_run = LONG_RUN * (mean(&o.next) + growth(&f, &o, run, long) + shopping);
    let mut sorted = o.next.clone();
    sorted.sort_by(f64::total_cmp);
    Some(Breakdown {
        blinds,
        next_total: sorted[sorted.len() / 2].exp(),
        main: o.main,
        long_run,
        money: LONG_RUN * shopping,
        assets: 0.0,
        total: v + long_run,
        flows: f,
    })
}

/// What cards and money gained between two positions are worth (Tarots or Spectrals made as the shop
/// closes or the blind starts...), in the value's units.
pub fn holdings_gain(start: &Run, end: &Run) -> f64 {
    let scale = buying_power(start) * MONEY_SCALE;
    scale
        * ((1.0 + money_now(end) / MONEY_SCALE).ln() - (1.0 + money_now(start) / MONEY_SCALE).ln())
}

/// Weight of growth a round adds (it lasts the rest of the run), against the value's other terms.
const GROWTH_WEIGHT: f64 = 1.0;

/// What a round's play added to the run besides beating the blind, in the value's units (log scoring
/// power): money and cards made (cash-out included, so unused hands, interest and payouts count), jokers
/// grown or worn down, hand levels gained, cards improved. `start` is the position the round was planned
/// from, `end` the same run after the round (cashed out if it was won); `reference` is the chips and mult
/// of a typical hand now, to weigh added chips and mult against.
pub fn round_gain(start: &Run, end: &Run, reference: (f64, f64)) -> f64 {
    let scale = buying_power(start) * MONEY_SCALE;
    let money = scale
        * ((1.0 + money_now(end) / MONEY_SCALE).ln() - (1.0 + money_now(start) / MONEY_SCALE).ln());
    let x_now: f64 = start
        .jokers
        .iter()
        .filter(|j| j.x_mult > 1.0)
        .map(|j| j.x_mult)
        .product::<f64>()
        .max(1.0);
    let chips = reference.0.max(1.0);
    let added_mult = (reference.1 / x_now).max(1.0);
    let (mut dc, mut dm, mut gx) = (0.0, 0.0, 0.0);
    for j1 in &end.jokers {
        let Some(j0) = start.jokers.iter().find(|j| j.sort_id == j1.sort_id) else {
            continue;
        };
        dc += j1.chips - j0.chips;
        dm += j1.mult - j0.mult;
        if j0.x_mult > 0.0 && j1.x_mult > 0.0 {
            gx += (j1.x_mult / j0.x_mult).ln();
        }
        if j1.kind == JokerKind::Supernova {
            // Its mult is the play count of the hand played.
            let most = HandType::ALL
                .iter()
                .copied()
                .max_by_key(|&h| start.hand_levels.get(h).played)
                .unwrap_or(HandType::Pair);
            dm += (end.hand_levels.get(most).played - start.hand_levels.get(most).played) as f64;
        }
    }
    // Jokers lost (destroyed) count against the round.
    for j0 in &start.jokers {
        if !end.jokers.iter().any(|j| j.sort_id == j0.sort_id) {
            gx -= 0.15;
        }
    }
    // Levels: those of the most played hands count in full, others a little.
    let mut levels = 0.0;
    for h in HandType::ALL {
        let d = (end.hand_levels.get(h).level - start.hand_levels.get(h).level) as f64;
        if d != 0.0 {
            let (_, _, lc, lm) = base_values(h);
            let weight = if start.hand_levels.get(h).played > 0 {
                1.0
            } else {
                0.3
            };
            levels += weight
                * (((chips + d * lc).max(1.0) / chips).ln()
                    + ((added_mult + d * lm).max(1.0) / added_mult).ln());
        }
    }
    // Hiker's chips on cards, spread over the deck.
    let bonus = |r: &Run| -> f64 {
        r.playing_cards
            .iter()
            .map(|&id| r.card(id).perma_bonus as f64)
            .sum::<f64>()
            / r.playing_cards.len().max(1) as f64
    };
    dc += 5.0 * (bonus(end) - bonus(start));
    let growth =
        ((chips + dc).max(1.0) / chips).ln() + ((added_mult + dm).max(1.0) / added_mult).ln() + gx;
    GROWTH_WEIGHT * (growth + levels) + money
}

thread_local! {
    static CACHE: RefCell<HashMap<(u64, u64), f64>> = RefCell::new(HashMap::new());
}

/// A key for everything `value` depends on (not the shop's or a pack's contents, which it never sees).
fn state_key(run: &Run) -> u64 {
    let mut h = DefaultHasher::new();
    (
        run.phase as u8,
        run.ante,
        run.round,
        run.money,
        run.boss as u8,
    )
        .hash(&mut h);
    run.slots.iter().for_each(|s| (*s as u8).hash(&mut h));
    (
        run.hands_per_round,
        run.discards_per_round,
        run.hand_size(),
        run.joker_slots,
        run.consumable_slots,
        run.interest_cap,
        run.interest_amount,
    )
        .hash(&mut h);
    run.vouchers.hash(&mut h);
    for j in &run.jokers {
        (
            j.kind as u16,
            j.edition as u8,
            j.sort_id,
            j.eternal,
            j.debuffed,
            j.counter,
            j.extra_value,
        )
            .hash(&mut h);
        (j.mult.to_bits(), j.x_mult.to_bits(), j.chips.to_bits()).hash(&mut h);
    }
    for c in &run.consumables {
        (c.kind as u16, c.edition as u8).hash(&mut h);
    }
    for &id in &run.playing_cards {
        let c = run.card(id);
        (
            c.rank as u8,
            c.suit as u8,
            c.enhancement as u8,
            c.edition as u8,
            c.seal as u8,
            c.perma_bonus,
        )
            .hash(&mut h);
    }
    for t in HandType::ALL {
        let i = run.hand_levels.get(t);
        (i.level, i.played).hash(&mut h);
    }
    for t in &run.tags {
        (t.kind as u8).hash(&mut h);
    }
    h.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Run, RunConfig};

    /// A run at its first shop (the teacher played the first blind).
    fn first_shop(seed: &str) -> Run {
        let mut run = Run::new(RunConfig::new(seed));
        let mut step = 0;
        while run.phase != Phase::Shop && step < 200 {
            let (k, a) = crate::teacher::teacher_move(&run, step);
            apply(&mut run, k, a).unwrap();
            step += 1;
        }
        assert_eq!(run.phase, Phase::Shop);
        run
    }

    #[test]
    fn a_good_joker_is_worth_more_than_its_price_early() {
        let run = first_shop("VALUE1");
        let base = value(&run, 7);
        let mut with = run.clone();
        with.add_joker(JokerKind::Joker); // +4 mult
        let mut strong = run.clone();
        strong.add_joker(JokerKind::Cavendish);
        let (v1, v2) = (value(&with, 7), value(&strong, 7));
        assert!(v1 > base, "+4 mult is worth something: {base} -> {v1}");
        assert!(v2 > v1, "X3 is worth more than +4 mult: {v1} vs {v2}");
        let mut poorer = with.clone();
        poorer.money -= 4;
        assert!(value(&poorer, 7) > base, "early, +4 mult is worth $4");
    }

    #[test]
    fn money_counts_more_at_an_interest_step() {
        let run = first_shop("VALUE2");
        let at = |m: i64| {
            let mut r = run.clone();
            r.money = m;
            value(&r, 3)
        };
        assert!(at(20) > at(19) && at(25) > at(24));
        assert!(
            at(25) - at(24) > at(24) - at(23),
            "the $25 step pays $1 a round more"
        );
    }

    #[test]
    fn levels_of_the_hand_played_count() {
        let run = first_shop("VALUE3");
        let o = outlook(&run, 5, SAMPLES).unwrap();
        let base = value(&run, 5);
        let mut lv = run.clone();
        lv.hand_levels.level_up(o.main, 3);
        assert!(value(&lv, 5) > base, "three levels of {:?}", o.main);
    }

    #[test]
    fn growth_is_counted_for_a_growing_joker() {
        let run = first_shop("VALUE4");
        let mut g = run.clone();
        g.add_joker(JokerKind::GreenJoker);
        let o = outlook(&g, 9, SAMPLES).unwrap();
        let f = flows(&g, o.main);
        assert!(f.mult > 0.0 && growth(&f, &o, &g, 6.0) > 0.0);
    }
}
