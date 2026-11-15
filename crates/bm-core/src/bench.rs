//! The build bench (S, lab #33, Move 1): a build scored against Green's late blinds without playing a run.
//!
//! A build is a position (`Run`): its jokers, deck, hand levels, hand size, hands and discards, vouchers. For an ante
//! A it is put on fair copies at the start of each of A's three blinds (the deck reshuffled, the round fresh, the
//! blind's target that ante's), and the planner's own read of a round's difficulty, `planner::beat_rate_with` (the
//! best-hand plan played out from the deal), says how often it beats each. The ante's score is the product of the
//! three: the chance of clearing the ante with the build as it stands. The boss is the one given (a real run's) or, for
//! a made-up build, each boss the ante can hold in turn, averaged. No seed is read: the copies' streams are fresh.
//!
//! It scores ceilings, not what a run reaches: the build is frozen, so nothing grows, is bought or is lost between the
//! blinds. Its validation (`baseline --bench FILE`, `scripts/bench_check.py`) asks whether the score predicts which
//! of the bot's runs clear the antes they enter.

use crate::blinds::{Boss, BlindKind};
use crate::state::{Phase, Run};

/// Tries per blind (fair copies, each played out by the planner).
pub const TRIES: u64 = 8;

/// The chance the build in `run` beats `kind` at ante `ante` (`tries` fair copies). None if the run is over.
pub fn blind_chance(run: &Run, ante: i32, kind: BlindKind, tries: u64, salt: u64) -> Option<f64> {
    // A finished run's build (lost or won) is put back at a blind's selection, so its final build can be scored.
    let revived;
    let run = if matches!(run.phase, Phase::GameOver) || run.won {
        let mut r = run.clone();
        r.phase = Phase::BlindSelect;
        r.won = false;
        r.blind = None;
        r.slots = [crate::blinds::SlotState::Select, crate::blinds::SlotState::Upcoming, crate::blinds::SlotState::Upcoming];
        r.hand.clear();
        let all = r.playing_cards.clone();
        r.deck = all;
        r.play.clear();
        r.discard.clear();
        revived = r;
        &revived
    } else {
        run
    };
    let mut c = crate::value::next_round(run, salt ^ 0xBE4C)?;
    if c.phase != Phase::Round {
        return None;
    }
    c.ante = ante;
    c.round_chips = 0.0;
    c.hands_left = c.hands_per_round.max(1);
    c.discards_left = c.discards_per_round.max(0);
    c.hands_played_round = 0;
    c.discards_used_round = 0;
    c.force_blind(kind);
    Some(planned_chance(&c, salt, tries))
}

/// How often the planner beats the blind in `c` (a round at its start): on each of `deals` fair deals, every plan the
/// hand offers is tried `PLAN_TRIES` times from that hand and the best plan's rate counts (the planner chooses its plan
/// from the hand by the same tries); the mean over deals.
fn planned_chance(c: &Run, salt: u64, deals: u64) -> f64 {
    use crate::planner::{options_for, plan_move, quick_best, try_policy, Plan, Policy};
    let deals = deals.max(1);
    let mut sum = 0.0;
    for k in 0..deals {
        let d = crate::observe::fair_copy(c, salt.wrapping_add(0xD0 + k));
        let best = quick_best(&d);
        let mut plans = options_for(&d);
        if !plans.contains(&Plan::Best) {
            plans.push(Plan::Best);
        }
        let rate = plans
            .iter()
            .map(|&p| {
                let first = plan_move(&d, p, best);
                (0..PLAN_TRIES)
                    .filter(|&t| try_policy(&d, Policy::Plan(p), first, salt ^ (k << 16 | t), None).0)
                    .count() as f64
                    / PLAN_TRIES as f64
            })
            .fold(0.0, f64::max);
        sum += rate;
    }
    sum / deals as f64
}

/// Tries per plan on one deal.
const PLAN_TRIES: u64 = 4;

/// The bosses ante `ante` can hold (the showdown bosses at the winning ante), for a made-up build.
pub fn bosses_for(ante: i32, win_ante: i32) -> Vec<Boss> {
    Boss::ALL
        .iter()
        .copied()
        .filter(|b| if ante >= win_ante { b.is_showdown() } else { !b.is_showdown() && b.min_ante() <= ante })
        .collect()
}

/// One ante's score: the chances of the Small, the Big and the boss, and their product. With `boss` None, the boss's
/// chance is the mean over the bosses the ante can hold.
pub fn ante_score(run: &Run, ante: i32, boss: Option<Boss>, tries: u64, salt: u64) -> Option<[f64; 4]> {
    let small = blind_chance(run, ante, BlindKind::Small, tries, salt)?;
    let big = blind_chance(run, ante, BlindKind::Big, tries, salt ^ 1)?;
    let boss_p = match boss {
        Some(b) => blind_chance(run, ante, BlindKind::Boss(b), tries, salt ^ 2)?,
        None => {
            let all = bosses_for(ante, run.win_ante);
            let ps: Vec<f64> = all
                .iter()
                .enumerate()
                .filter_map(|(i, &b)| blind_chance(run, ante, BlindKind::Boss(b), tries.div_ceil(2), salt ^ (16 + i as u64)))
                .collect();
            if ps.is_empty() {
                return None;
            }
            ps.iter().sum::<f64>() / ps.len() as f64
        }
    };
    Some([small, big, boss_p, small * big * boss_p])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::env::apply;
    use crate::joker_data::JokerKind;
    use crate::RunConfig;

    fn first_shop(seed: &str) -> Run {
        let mut run = Run::new(RunConfig::at_stake(seed, 3));
        while run.phase != Phase::Shop {
            let m = crate::bots::greedy_move(&run, 1);
            apply(&mut run, m.0, m.1).unwrap();
        }
        run
    }

    /// A bare build clears ante 1 more often than ante 6; a strong x-mult row clears ante 6 more often than a bare one;
    /// the score is a product of three chances and is the same from the same salt.
    #[test]
    fn the_bench_ranks_builds_and_antes_as_it_should() {
        let bare = first_shop("BENCH1");
        let a1 = ante_score(&bare, 1, Some(Boss::Hook), TRIES, 7).unwrap();
        let a6 = ante_score(&bare, 6, Some(Boss::Hook), TRIES, 7).unwrap();
        assert!(a1[3] > a6[3], "ante 1 {a1:?} against ante 6 {a6:?}");
        assert!((a1[3] - a1[0] * a1[1] * a1[2]).abs() < 1e-12);
        let mut strong = bare.clone();
        for k in [JokerKind::Cavendish, JokerKind::Triboulet, JokerKind::Joker, JokerKind::Baron] {
            strong.add_joker(k);
        }
        let s6 = ante_score(&strong, 6, Some(Boss::Hook), TRIES, 7).unwrap();
        assert!(s6[3] >= a6[3], "a strong row {s6:?} against bare {a6:?}");
        assert_eq!(ante_score(&strong, 6, Some(Boss::Hook), TRIES, 7), Some(s6), "the same salt, the same score");
        assert!(bosses_for(8, 8).iter().all(|b| b.is_showdown()) && !bosses_for(5, 8).is_empty());
    }
}
