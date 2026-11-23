//! A training environment around [`Run`]: moves as (kind, argument) pairs,
//! fresh seeds for every episode, and a random legal move for benchmarks
//! and baselines. Python wraps this (`balatro_meta._engine.VecEnv`).

use bm_rng::SeedSampler;

use crate::blinds::BlindSlot;
use crate::card::SortMode;
use crate::observe::{encode, obs_layout, screen, OBS_LEN};
use crate::state::{Phase, Run, RunConfig};
use crate::tally::{RunEnd, Tally};

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
    /// The run's counts so far (`tally::Tally`).
    pub tally: Tally,
    /// How the last finished run ended, until the caller takes it.
    pub ended: Option<RunEnd>,
    /// The run's high-water mark of position (`position`): what the learner's potential has already paid for.
    pub high: i32,
    /// The learner's engine (lab #38, Stage 0): moves with no decision in them are made here, never asked for (the
    /// only kind the learner may choose, cash-out, the jokers' order by `teacher::joker_order`), and a move that
    /// changes nothing on the screen counts as refused. Off by default (the other users of `Env`).
    pub auto: bool,
    /// Whether the learner may skip blinds (off from Stage 0: `learner_kinds`).
    pub skips: bool,
}

/// Kinds the learner never chooses (train_policy.py's NEVER): the sorts, buy-and-use, the boss reroll.
pub const LEARNER_NEVER: [MoveKind; 4] =
    [MoveKind::SortRank, MoveKind::SortSuit, MoveKind::BuyAndUse, MoveKind::RerollBoss];
/// Moves the engine makes by itself in a row at most (a guard; a few are ever needed).
const AUTO_MAX: usize = 32;

/// The learner's own inputs (lab #38), after the screen's (`observe::encode`, which the teacher's value networks
/// also read and so never changes): a block of LEARNER_FEATURES floats. Column 0: blinds already paid for ahead of
/// where the run stands, the high-water mark less the position, in antes (after Hieroglyph or Petroglyph the
/// potential pays nothing until the run gets back: B). The others are kept for Stage 0's next inputs (capacity
/// against the coming blinds) and are 0 until then.
pub const LEARNER_FEATURES: usize = 32;
/// The learner's whole input: the screen, then the learner's block.
pub const POLICY_OBS_LEN: usize = OBS_LEN + LEARNER_FEATURES;

/// The learner's input layout: the screen's parts, then "learner".
pub fn policy_layout() -> Vec<(&'static str, usize, usize, usize)> {
    let mut l = obs_layout();
    l.push(("learner", OBS_LEN, 1, LEARNER_FEATURES));
    l
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
    /// Where the run stands after the move (before a new run starts at its end): blinds beaten so far, 3 an ante
    /// before this one plus this ante's beaten (skipped blinds don't count); 24 at a win. The learner's potential
    /// takes its high-water mark, so an ante sent back (Hieroglyph, Petroglyph) never pays twice (lab #38).
    pub position: i32,
    /// In a round, the share of the blind's target scored so far (0 to 1); 0 elsewhere.
    pub progress: f32,
}

/// `StepInfo::position` of a run.
pub fn position(run: &Run) -> i32 {
    if run.won {
        return 24;
    }
    let beaten = run.slots.iter().filter(|s| matches!(s, crate::blinds::SlotState::Defeated)).count() as i32;
    3 * (run.ante.max(1) - 1) + beaten
}

/// `StepInfo::progress` of a run: in a round, and in the round a run was lost in (its blind and score survive the
/// loss: B, lab #38, so a near miss and a blowout don't end on the same return).
pub fn progress(run: &Run) -> f32 {
    match (&run.blind, run.phase) {
        (Some(b), Phase::Round | Phase::GameOver) if b.chips > 0.0 => {
            (run.round_chips / b.chips).clamp(0.0, 1.0) as f32
        }
        _ => 0.0,
    }
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
            tally: Tally::default(),
            ended: None,
            high: 0,
            auto: false,
            skips: true,
        }
    }

    /// The kinds the learner may choose here: the game's legal kinds less `LEARNER_NEVER`; with `auto`, less joker
    /// moves (the engine orders the jokers); without `skips`, less a blind skip.
    pub fn learner_kinds(&self) -> [bool; N_MOVE_KINDS] {
        let mut k = legal_kinds(&self.run);
        if self.auto {
            for m in LEARNER_NEVER {
                k[m as usize] = false;
            }
            k[MoveKind::MoveJoker as usize] = false;
        }
        if !self.skips {
            k[MoveKind::SkipBlind as usize] = false;
        }
        k
    }

    /// With `auto`: make the moves that hold no decision, in a row, until the learner has one: cash-out; the jokers
    /// into `teacher::joker_order`; the only kind left that takes no choice of argument (Select Blind with skips off,
    /// Leave Shop or Skip Pack with nothing else allowed). Returns how many it made.
    pub fn auto_steps(&mut self) -> usize {
        if !self.auto {
            return 0;
        }
        let mut made = 0;
        while made < AUTO_MAX && self.run.phase != Phase::GameOver && !self.run.won {
            let m = if self.run.phase == Phase::RoundEval {
                Some((MoveKind::CashOut, 0))
            } else if let Some(m) = crate::teacher::reorder(&self.run).filter(|m| m.0 == MoveKind::MoveJoker) {
                Some(m)
            } else {
                let k = self.learner_kinds();
                let open: Vec<usize> = (0..N_MOVE_KINDS).filter(|&i| k[i]).collect();
                match open.as_slice() {
                    [i] if matches!(
                        MoveKind::from_index(*i as i64),
                        Some(MoveKind::SelectBlind | MoveKind::LeaveShop | MoveKind::PackSkip | MoveKind::CashOut)
                    ) =>
                    {
                        MoveKind::from_index(*i as i64).map(|m| (m, 0))
                    }
                    _ => None,
                }
            };
            let Some((kind, arg)) = m else { break };
            if apply(&mut self.run, kind, arg).is_err() {
                break;
            }
            made += 1;
        }
        made
    }

    pub fn reset(&mut self) {
        let mut cfg = RunConfig::new(&self.seeds.next_seed());
        cfg.stake = self.stake;
        self.run = Run::new(cfg);
        self.episodes += 1;
        self.tally = Tally::default();
        self.high = 0;
    }

    /// Start a new run on this seed (to play chosen seeds, such as another bot's).
    pub fn start(&mut self, seed: &str) {
        let mut cfg = RunConfig::new(seed);
        cfg.stake = self.stake;
        self.run = Run::new(cfg);
        self.tally = Tally::default();
        self.high = 0;
        self.auto_steps();
    }

    /// Apply a move; at the end of an episode (lost, or won by beating
    /// ante 8) report it and start a new one. A refused move changes nothing.
    pub fn step(&mut self, kind: i64, arg: u64) -> StepInfo {
        let Some(k) = MoveKind::from_index(kind) else {
            self.tally.refused += 1;
            return StepInfo {
                refused: true,
                ante: self.run.ante,
                position: position(&self.run),
                progress: progress(&self.run),
                ..StepInfo::default()
            };
        };
        let in_round = self.run.phase == Phase::Round;
        let pending = Tally::before(&self.run, k, arg);
        // With `auto`, a move that leaves the screen as it was is a move with nothing in it (re-sorting a sorted hand,
        // a card moved onto itself): it counts as refused (lab #38, B's test 3).
        let before = self.auto.then(|| {
            let mut v = Vec::new();
            encode(&screen(&self.run), &mut v);
            v
        });
        let mut refused = apply(&mut self.run, k, arg).is_err();
        if let Some(b) = before.filter(|_| !refused) {
            let mut v = Vec::new();
            encode(&screen(&self.run), &mut v);
            refused = v == b;
        }
        self.tally.after(&self.run, pending, refused);
        let won = self.run.won;
        let done = won || self.run.phase == Phase::GameOver;
        let info = StepInfo {
            reward: if won { 1.0 } else { 0.0 },
            done,
            won,
            refused,
            ante: self.run.ante,
            cleared: in_round && (won || self.run.phase == Phase::RoundEval),
            position: position(&self.run),
            progress: progress(&self.run),
        };
        self.high = self.high.max(info.position);
        if done {
            self.ended = Some(RunEnd::new(&self.run, std::mem::take(&mut self.tally)));
            self.reset();
        }
        self.auto_steps();
        info
    }

    /// The learner's input: the screen (`observe::encode`), then its own block (`LEARNER_FEATURES`).
    pub fn observe(&self, out: &mut Vec<f32>) {
        encode(&screen(&self.run), out);
        let start = out.len();
        out.resize(start + LEARNER_FEATURES, 0.0);
        out[start] = (self.high - position(&self.run)).max(0) as f32 / 3.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The learner's input is the screen then its block; column 0 is the blinds paid for ahead (0 on a fresh run).
    #[test]
    fn the_learner_input_is_the_screen_then_its_block() {
        let e = Env::new(3, 1);
        let mut v = Vec::new();
        e.observe(&mut v);
        assert_eq!(v.len(), POLICY_OBS_LEN);
        let mut s = Vec::new();
        encode(&screen(&e.run), &mut s);
        assert_eq!(&v[..OBS_LEN], &s[..]);
        assert!(v[OBS_LEN..].iter().all(|&x| x == 0.0));
        let mut e = e;
        e.high = 6;
        let mut v = Vec::new();
        e.observe(&mut v);
        assert_eq!(v[OBS_LEN], 2.0, "six blinds paid ahead of a fresh run: two antes");
        assert_eq!(policy_layout().last().map(|p| (p.0, p.1, p.3)), Some(("learner", OBS_LEN, LEARNER_FEATURES)));
    }

    /// Lab #38: each step reports where the run stands (blinds beaten so far, 24 at a win; skipped blinds never
    /// count) and, in a round, the share of the target scored.
    #[test]
    fn a_step_reports_position_and_progress() {
        let mut e = Env::new(7, 3);
        let mut beaten = 0;
        for k in 0..2000u64 {
            let (kind, arg) = crate::bots::greedy_move(&e.run, k);
            let info = e.step(kind as i64, arg);
            assert!((0.0..=1.0).contains(&info.progress));
            if info.cleared {
                beaten += 1;
            }
            if info.done {
                assert_eq!(info.position, if info.won { 24 } else { beaten });
                if !info.won {
                    assert!(info.progress > 0.0 && info.progress < 1.0, "a loss keeps its round's progress");
                }
                return;
            }
            assert_eq!(info.position, beaten, "with no skips, position counts the blinds beaten");
            assert_eq!(info.position, position(&e.run));
        }
    }
}

#[cfg(test)]
mod learner_tests {
    use super::*;

    /// Lab #38 (B's test 4): the learner's input is the screen's and its own block's only: two fair copies of a
    /// position (the deck's order and every random stream drawn afresh, the screen the same) give the same input,
    /// along a run, in every phase.
    #[test]
    fn fair_copies_give_the_learner_the_same_input() {
        let mut e = Env::new(42, 3);
        e.auto = true;
        e.skips = false;
        e.auto_steps();
        let mut rng = XorShift(9);
        let mut checked = 0;
        for step in 0..1200u64 {
            let copy = |salt: u64| {
                let mut c = Env::new(1, 3);
                c.run = crate::observe::fair_copy(&e.run, salt);
                c.high = e.high;
                let mut v = Vec::new();
                c.observe(&mut v);
                v
            };
            let (a, b) = (copy(step * 2 + 1), copy(step * 2 + 2));
            let mut mine = Vec::new();
            e.observe(&mut mine);
            assert_eq!(a, b, "two fair copies, one screen, at step {step} in {:?}", e.run.phase);
            assert_eq!(a, mine, "a fair copy shows what the run shows");
            checked += 1;
            let k = e.learner_kinds();
            let mut m = random_move(&e.run, &mut rng);
            for _ in 0..20 {
                if k[m.0 as usize] {
                    break;
                }
                m = random_move(&e.run, &mut rng);
            }
            if k[m.0 as usize] {
                e.step(m.0 as i64, m.1);
            }
        }
        assert!(checked > 1000);
    }

    /// Lab #38 (Stage 0): with `auto`, the learner is never asked for a move with no decision in it: no cash-out
    /// screen, no blind choice with skips off, the jokers always in the teacher's order; and a move that changes
    /// nothing on the screen (B's test 3) is refused.
    #[test]
    fn the_learner_is_only_asked_for_decisions() {
        for seed in 0..6u64 {
            let mut e = Env::new(100 + seed, 3);
            e.auto = true;
            e.skips = false;
            e.auto_steps();
            let mut rng = XorShift(seed + 1);
            for _ in 0..1500 {
                assert_ne!(e.run.phase, Phase::RoundEval, "cash-out is the engine's");
                let open = e.learner_kinds().iter().filter(|&&x| x).count();
                assert!(e.run.phase != Phase::BlindSelect || open > 1, "a blind choice with nothing else to do is the engine's");
                assert!(crate::teacher::reorder(&e.run).is_none(), "the jokers are in the teacher's order");
                let k = e.learner_kinds();
                assert!(!k[MoveKind::MoveJoker as usize] && !k[MoveKind::SkipBlind as usize]);
                assert!(LEARNER_NEVER.iter().all(|m| !k[*m as usize]));
                // A card moved onto itself changes nothing: refused.
                if e.run.phase == Phase::Round && e.run.hand.len() > 1 {
                    let mut c = Env { run: e.run.clone(), seeds: SeedSampler::new(1), stake: 3, episodes: 0,
                        tally: Tally::default(), ended: None, high: e.high, auto: true, skips: false };
                    assert!(c.step(MoveKind::MoveCard as i64, 0).refused, "a card moved onto itself");
                }
                // A random legal move among the learner's kinds.
                let mut m = random_move(&e.run, &mut rng);
                for _ in 0..20 {
                    if k[m.0 as usize] {
                        break;
                    }
                    m = random_move(&e.run, &mut rng);
                }
                if !k[m.0 as usize] {
                    continue;
                }
                e.step(m.0 as i64, m.1);
            }
        }
    }
}
