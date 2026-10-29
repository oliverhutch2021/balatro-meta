//! A training environment around [`Run`]: moves as (kind, argument) pairs,
//! fresh seeds for every episode, and a random legal move for benchmarks
//! and baselines. Python wraps this (`balatro_meta._engine.VecEnv`).

use bm_rng::SeedSampler;

use crate::blinds::BlindSlot;
use crate::card::SortMode;
use crate::observe::{encode, screen};
use crate::state::{Phase, Run, RunConfig};

/// Move kinds. The argument is a hand bitmask for Play and Discard, a
/// position for SellJoker, and `from | to << 8` for the two moves.
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
}

pub const N_MOVE_KINDS: usize = 11;

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
    match kind {
        MoveKind::SelectBlind => run.select_blind(),
        MoveKind::SkipBlind => run.skip_blind(),
        MoveKind::CashOut => run.cash_out().map(|_| ()),
        MoveKind::LeaveShop => run.leave_shop(),
        MoveKind::SortRank => run.sort_hand_by(SortMode::Rank),
        MoveKind::SortSuit => run.sort_hand_by(SortMode::Suit),
        MoveKind::Play => run.play(&mask_positions(arg)).map(|_| ()),
        MoveKind::Discard => run.discard(&mask_positions(arg)),
        MoveKind::MoveCard => {
            run.move_hand_card((arg & 0xff) as usize, ((arg >> 8) & 0xff) as usize)
        }
        MoveKind::MoveJoker => run.move_joker((arg & 0xff) as usize, ((arg >> 8) & 0xff) as usize),
        MoveKind::SellJoker => run.sell_joker(arg as usize).map(|_| ()),
    }
}

/// Which move kinds are legal now (arguments still have to be valid).
pub fn legal_kinds(run: &Run) -> [bool; N_MOVE_KINDS] {
    let mut k = [false; N_MOVE_KINDS];
    match run.phase {
        Phase::BlindSelect => {
            k[MoveKind::SelectBlind as usize] = true;
            k[MoveKind::SkipBlind as usize] = run.on_deck() != BlindSlot::Boss;
        }
        Phase::Round => {
            k[MoveKind::Play as usize] = !run.hand.is_empty();
            k[MoveKind::Discard as usize] = !run.hand.is_empty() && run.discards_left > 0;
            k[MoveKind::SortRank as usize] = true;
            k[MoveKind::SortSuit as usize] = true;
            k[MoveKind::MoveCard as usize] = run.hand.len() > 1;
        }
        Phase::RoundEval => k[MoveKind::CashOut as usize] = true,
        Phase::Shop => k[MoveKind::LeaveShop as usize] = true,
        Phase::GameOver => {}
    }
    if run.phase != Phase::GameOver {
        k[MoveKind::MoveJoker as usize] = run.jokers.len() > 1;
        k[MoveKind::SellJoker as usize] = run.jokers.iter().any(|j| !j.eternal);
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
/// sometimes reorder, select or skip blinds, cash out and leave the shop.
/// Never sells (so random runs keep their jokers).
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
            let k = 1 + rng.below(n.min(5));
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
            if run.discards_left > 0 && rng.below(3) == 0 {
                (MoveKind::Discard, mask)
            } else {
                (MoveKind::Play, mask)
            }
        }
        Phase::RoundEval => (MoveKind::CashOut, 0),
        Phase::Shop => (MoveKind::LeaveShop, 0),
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

/// What one step returned.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct StepInfo {
    pub reward: f32,
    pub done: bool,
    pub won: bool,
    pub refused: bool,
    /// Antes cleared in the episode that just ended (or so far).
    pub ante: i32,
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
        let refused = apply(&mut self.run, k, arg).is_err();
        let won = self.run.won;
        let done = won || self.run.phase == Phase::GameOver;
        let info = StepInfo {
            reward: if won { 1.0 } else { 0.0 },
            done,
            won,
            refused,
            ante: self.run.ante,
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
