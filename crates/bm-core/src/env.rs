//! A training environment around [`Run`]: moves as (kind, argument) pairs,
//! fresh seeds for every episode, and a random legal move for benchmarks
//! and baselines. Python wraps this (`balatro_meta._engine.VecEnv`).

use bm_rng::SeedSampler;

use crate::blinds::BlindSlot;
use crate::card::SortMode;
use crate::observe::{encode, screen};
use crate::state::{Phase, Run, RunConfig};

/// Move kinds. The argument is:
/// - a hand bitmask for Play and Discard;
/// - `from | to << 8` for MoveCard and MoveJoker;
/// - a slot index for SellJoker, SellConsumable, Buy, BuyAndUse,
///   RedeemVoucher and OpenBooster;
/// - `index | hand mask << 8` for UseConsumable and PackChoose.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum MoveKind {
    SelectBlind = 0,
    SkipBlind = 1,
    CashOut = 2,
    LeaveShop = 3,
    SortRank = 4,
    SortSuit = 5,
    Play = 6,
    Discard = 7,
    MoveCard = 8,
    MoveJoker = 9,
    SellJoker = 10,
    Reroll = 11,
    Buy = 12,
    BuyAndUse = 13,
    RedeemVoucher = 14,
    OpenBooster = 15,
    SellConsumable = 16,
    UseConsumable = 17,
    PackChoose = 18,
    PackSkip = 19,
    RerollBoss = 20,
}

pub const N_MOVE_KINDS: usize = 21;

impl MoveKind {
    pub fn from_index(i: i64) -> Option<MoveKind> {
        use MoveKind::*;
        Some(match i {
            0 => SelectBlind,
            1 => SkipBlind,
            2 => CashOut,
            3 => LeaveShop,
            4 => SortRank,
            5 => SortSuit,
            6 => Play,
            7 => Discard,
            8 => MoveCard,
            9 => MoveJoker,
            10 => SellJoker,
            11 => Reroll,
            12 => Buy,
            13 => BuyAndUse,
            14 => RedeemVoucher,
            15 => OpenBooster,
            16 => SellConsumable,
            17 => UseConsumable,
            18 => PackChoose,
            19 => PackSkip,
            20 => RerollBoss,
            _ => return None,
        })
    }
}

/// Hand positions in a bitmask, ascending.
pub fn mask_positions(mask: u64) -> Vec<usize> {
    (0..64).filter(|i| mask & (1u64 << i) != 0).collect()
}

/// Apply one move. `Err` if the game refuses it (the run is unchanged).
pub fn apply(run: &mut Run, kind: MoveKind, arg: u64) -> Result<(), crate::Refused> {
    let idx = (arg & 0xff) as usize;
    let hand = mask_positions(arg >> 8);
    match kind {
        MoveKind::SelectBlind => run.select_blind(),
        MoveKind::SkipBlind => run.skip_blind(),
        MoveKind::CashOut => run.cash_out().map(|_| ()),
        MoveKind::LeaveShop => run.leave_shop(),
        MoveKind::SortRank => run.sort_hand_by(SortMode::Rank),
        MoveKind::SortSuit => run.sort_hand_by(SortMode::Suit),
        MoveKind::Play => run.play(&mask_positions(arg)).map(|_| ()),
        MoveKind::Discard => run.discard(&mask_positions(arg)),
        MoveKind::MoveCard => run.move_hand_card(idx, ((arg >> 8) & 0xff) as usize),
        MoveKind::MoveJoker => run.move_joker(idx, ((arg >> 8) & 0xff) as usize),
        MoveKind::SellJoker => run.sell_joker(idx).map(|_| ()),
        MoveKind::Reroll => run.reroll_shop(),
        MoveKind::Buy => run.buy(idx),
        MoveKind::BuyAndUse => run.buy_and_use(idx),
        MoveKind::RedeemVoucher => run.redeem_voucher(idx),
        MoveKind::OpenBooster => run.open_booster(idx),
        MoveKind::SellConsumable => run.sell_consumable(idx).map(|_| ()),
        MoveKind::UseConsumable => run.use_consumable(idx, &hand),
        MoveKind::PackChoose => run.pack_choose(idx, &hand),
        MoveKind::PackSkip => run.pack_skip(),
        MoveKind::RerollBoss => run.reroll_boss(),
    }
}

/// Which move kinds are legal now (arguments still have to be valid).
pub fn legal_kinds(run: &Run) -> [bool; N_MOVE_KINDS] {
    let mut k = [false; N_MOVE_KINDS];
    match run.phase {
        Phase::BlindSelect => {
            k[MoveKind::SelectBlind as usize] = true;
            k[MoveKind::SkipBlind as usize] = run.on_deck() != BlindSlot::Boss;
            k[MoveKind::RerollBoss as usize] = run.can_reroll_boss();
        }
        Phase::Round => {
            k[MoveKind::Play as usize] = !run.hand.is_empty();
            k[MoveKind::Discard as usize] = !run.hand.is_empty() && run.discards_left > 0;
            k[MoveKind::SortRank as usize] = true;
            k[MoveKind::SortSuit as usize] = true;
            k[MoveKind::MoveCard as usize] = run.hand.len() > 1;
        }
        Phase::RoundEval => k[MoveKind::CashOut as usize] = true,
        Phase::Shop => {
            k[MoveKind::LeaveShop as usize] = true;
            k[MoveKind::Reroll as usize] =
                run.reroll_cost == 0 || run.spendable() - run.reroll_cost >= 0;
            if let Some(s) = run.shop.as_ref() {
                k[MoveKind::Buy as usize] = !s.row.is_empty();
                k[MoveKind::BuyAndUse as usize] = s
                    .row
                    .iter()
                    .any(|r| matches!(r.item, crate::shop::RowItem::Consumable(_)));
                k[MoveKind::RedeemVoucher as usize] = !s.vouchers.is_empty();
                k[MoveKind::OpenBooster as usize] = s.boosters.iter().any(|b| b.is_some());
            }
        }
        Phase::Pack => {
            k[MoveKind::PackChoose as usize] =
                run.pack.as_ref().is_some_and(|p| !p.cards.is_empty());
            k[MoveKind::PackSkip as usize] = true;
            k[MoveKind::MoveCard as usize] = run.hand.len() > 1;
        }
        Phase::GameOver => {}
    }
    if run.phase != Phase::GameOver {
        k[MoveKind::MoveJoker as usize] = run.jokers.len() > 1;
        k[MoveKind::SellJoker as usize] = run.jokers.iter().any(|j| !j.eternal);
        k[MoveKind::SellConsumable as usize] = !run.consumables.is_empty();
        k[MoveKind::UseConsumable as usize] = !run.consumables.is_empty();
    }
    k
}

/// A small fast generator for the env's own choices (never the game's).
#[derive(Clone, Debug)]
pub struct XorShift(pub u64);

impl XorShift {
    pub fn below(&mut self, n: usize) -> usize {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 % n.max(1) as u64) as usize
    }
}

/// A random legal move: play or discard 1-5 random cards most of the time,
/// sometimes reorder, select or skip blinds, buy, reroll, open packs and
/// use consumables. Never sells (so random runs keep their jokers). The
/// move may still be refused (e.g. too expensive); callers retry or fall
/// back to a safe move.
pub fn random_move(run: &Run, rng: &mut XorShift) -> (MoveKind, u64) {
    match run.phase {
        Phase::BlindSelect => {
            if run.on_deck() != BlindSlot::Boss && rng.below(4) == 0 {
                (MoveKind::SkipBlind, 0)
            } else {
                (MoveKind::SelectBlind, 0)
            }
        }
        Phase::Round => {
            let n = run.hand.len();
            if n > 1 && rng.below(8) == 0 {
                let (a, b) = (rng.below(n), rng.below(n));
                return (MoveKind::MoveCard, (a | b << 8) as u64);
            }
            if !run.consumables.is_empty() && rng.below(10) == 0 {
                let i = rng.below(run.consumables.len());
                let k = 1 + rng.below(n.clamp(1, 3));
                let mask = random_mask(run, rng, k);
                let sel: Vec<u32> = mask_positions(mask).iter().map(|&p| run.hand[p]).collect();
                if run.can_use_consumable(
                    &run.consumables[i],
                    &sel,
                    crate::consumables::UseFrom::Slots,
                ) {
                    return (MoveKind::UseConsumable, i as u64 | mask << 8);
                }
            }
            let k = 1 + rng.below(n.min(5));
            let mask = random_mask(run, rng, k);
            if run.discards_left > 0 && rng.below(3) == 0 {
                (MoveKind::Discard, mask)
            } else {
                (MoveKind::Play, mask)
            }
        }
        Phase::RoundEval => (MoveKind::CashOut, 0),
        Phase::Shop => {
            let Some(s) = run.shop.as_ref() else {
                return (MoveKind::LeaveShop, 0);
            };
            let afford = |c: i64| c <= 0 || c <= run.spendable();
            if !run.consumables.is_empty() && rng.below(4) == 0 {
                let i = rng.below(run.consumables.len());
                if run.can_use_consumable(
                    &run.consumables[i],
                    &[],
                    crate::consumables::UseFrom::Slots,
                ) {
                    return (MoveKind::UseConsumable, i as u64);
                }
            }
            match rng.below(6) {
                0 if !s.row.is_empty() => {
                    let i = rng.below(s.row.len());
                    let r = &s.row[i];
                    let room = match &r.item {
                        crate::shop::RowItem::Joker(_) => {
                            (run.jokers.len() as i32) < run.joker_slots
                        }
                        crate::shop::RowItem::Consumable(_) => {
                            run.consumables.len() < run.consumable_slots as usize
                        }
                        crate::shop::RowItem::Card(_) => true,
                    };
                    if afford(r.cost) && room {
                        (MoveKind::Buy, i as u64)
                    } else {
                        (MoveKind::LeaveShop, 0)
                    }
                }
                1 => {
                    let open: Vec<usize> = (0..s.boosters.len())
                        .filter(|&i| s.boosters[i].as_ref().is_some_and(|b| afford(b.cost)))
                        .collect();
                    if open.is_empty() {
                        (MoveKind::LeaveShop, 0)
                    } else {
                        (MoveKind::OpenBooster, open[rng.below(open.len())] as u64)
                    }
                }
                2 if s
                    .vouchers
                    .first()
                    .is_some_and(|v| v.cost <= run.spendable()) =>
                {
                    (MoveKind::RedeemVoucher, 0)
                }
                3 if run.reroll_cost == 0 || run.spendable() - run.reroll_cost >= 0 => {
                    (MoveKind::Reroll, 0)
                }
                _ => (MoveKind::LeaveShop, 0),
            }
        }
        Phase::Pack => {
            let Some(p) = run.pack.as_ref() else {
                return (MoveKind::PackSkip, 0);
            };
            if p.cards.is_empty() || rng.below(4) == 0 {
                return (MoveKind::PackSkip, 0);
            }
            let i = rng.below(p.cards.len());
            let k = 1 + rng.below(run.hand.len().clamp(1, 2));
            let mask = if run.hand.is_empty() {
                0
            } else {
                random_mask(run, rng, k)
            };
            let ok = match &p.cards[i] {
                crate::packs::PackCard::Joker(j) => {
                    j.edition == crate::card::Edition::Negative
                        || (run.jokers.len() as i32) < run.joker_slots
                }
                crate::packs::PackCard::Consumable(c) => {
                    let sel: Vec<u32> = mask_positions(mask).iter().map(|&p| run.hand[p]).collect();
                    run.can_use_consumable(c, &sel, crate::consumables::UseFrom::Pack)
                }
                crate::packs::PackCard::Card(_) => true,
            };
            if ok {
                (MoveKind::PackChoose, i as u64 | mask << 8)
            } else {
                (MoveKind::PackSkip, 0)
            }
        }
        Phase::GameOver => (MoveKind::SelectBlind, 0),
    }
}

/// `k` random distinct hand positions (always including Cerulean Bell's card).
fn random_mask(run: &Run, rng: &mut XorShift, k: usize) -> u64 {
    let n = run.hand.len();
    if n == 0 {
        return 0;
    }
    let k = k.min(n);
    let mut mask = 0u64;
    let mut chosen = 0;
    if let Some(f) = run.forced_card() {
        mask |= 1 << f;
        chosen = 1;
    }
    while chosen < k {
        let i = rng.below(n);
        if mask & (1 << i) == 0 {
            mask |= 1 << i;
            chosen += 1;
        }
    }
    mask
}

/// A safe move that is always accepted in this phase (falls back from a
/// refused random move).
pub fn safe_move(run: &Run) -> (MoveKind, u64) {
    match run.phase {
        Phase::BlindSelect => (MoveKind::SelectBlind, 0),
        Phase::Round => {
            let mut mask = 1u64;
            if let Some(f) = run.forced_card() {
                mask = 1 << f;
            }
            (MoveKind::Play, mask)
        }
        Phase::RoundEval => (MoveKind::CashOut, 0),
        Phase::Shop => (MoveKind::LeaveShop, 0),
        Phase::Pack => (MoveKind::PackSkip, 0),
        Phase::GameOver => (MoveKind::SelectBlind, 0),
    }
}

/// One environment: a run that restarts on a fresh seed when it ends.
#[derive(Clone, Debug)]
pub struct Env {
    pub run: Run,
    seeds: SeedSampler,
    pub stake: u8,
    pub episodes: u64,
}

/// The seed of run `i` of a job with seed base `base` (the baseline job's runs, so other bots can play the
/// very same seeds).
pub fn run_seed(base: u64, i: u64) -> String {
    SeedSampler::new(base.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ i).next_seed()
}

/// What one step returned.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct StepInfo {
    pub reward: f32,
    pub done: bool,
    pub won: bool,
    pub refused: bool,
    /// Antes cleared in the episode that just ended (or so far).
    pub ante: i32,
    /// The move beat a blind.
    pub cleared: bool,
}

impl Env {
    pub fn new(seed: u64, stake: u8) -> Env {
        let mut seeds = SeedSampler::new(seed);
        let mut cfg = RunConfig::new(&seeds.next_seed());
        cfg.stake = stake;
        Env {
            run: Run::new(cfg),
            seeds,
            stake,
            episodes: 0,
        }
    }

    pub fn reset(&mut self) {
        let mut cfg = RunConfig::new(&self.seeds.next_seed());
        cfg.stake = self.stake;
        self.run = Run::new(cfg);
        self.episodes += 1;
    }

    /// Start a new run on this seed (to play chosen seeds, such as another bot's).
    pub fn start(&mut self, seed: &str) {
        let mut cfg = RunConfig::new(seed);
        cfg.stake = self.stake;
        self.run = Run::new(cfg);
    }

    /// Apply a move; at the end of an episode (lost, or won by beating
    /// ante 8) report it and start a new one. A refused move changes nothing.
    pub fn step(&mut self, kind: i64, arg: u64) -> StepInfo {
        let Some(k) = MoveKind::from_index(kind) else {
            return StepInfo {
                refused: true,
                ante: self.run.ante,
                ..StepInfo::default()
            };
        };
        let in_round = self.run.phase == Phase::Round;
        let refused = apply(&mut self.run, k, arg).is_err();
        let won = self.run.won;
        let done = won || self.run.phase == Phase::GameOver;
        let info = StepInfo {
            reward: if won { 1.0 } else { 0.0 },
            done,
            won,
            refused,
            ante: self.run.ante,
            cleared: in_round && (won || self.run.phase == Phase::RoundEval),
        };
        if done {
            self.reset();
        }
        info
    }

    pub fn observe(&self, out: &mut Vec<f32>) {
        encode(&screen(&self.run), out);
    }
}
