//! Simple bots for demos, baselines and smoke tests. They decide from the
//! screen and from fair copies only (`observe::fair_copy`), never from the
//! real run's future.

use crate::env::MoveKind;
use crate::observe::fair_copy;
use crate::scoring::evaluate_play;
use crate::state::{Phase, Run};

/// All subsets of up to 5 hand positions that include `forced`, as masks.
pub fn candidate_masks(n: usize, forced: Option<usize>) -> Vec<u64> {
    let mut out = Vec::new();
    for mask in 1u64..(1u64 << n.min(20)) {
        if mask.count_ones() > 5 {
            continue;
        }
        if let Some(f) = forced {
            if mask & (1 << f) == 0 {
                continue;
            }
        }
        out.push(mask);
    }
    out
}

/// What playing these hand positions would score now, judged on a fair copy.
pub fn preview_score(fair: &Run, mask: u64) -> f64 {
    let mut sim = fair.clone();
    let mut chosen = Vec::new();
    for (i, &id) in fair.hand.iter().enumerate() {
        if mask & (1 << i) != 0 {
            chosen.push(id);
        }
    }
    sim.hand.retain(|id| !chosen.contains(id));
    sim.play = chosen;
    sim.hands_left -= 1;
    evaluate_play(&mut sim).score
}

/// The greedy bot: always takes the blind, plays the highest-scoring hand
/// when it clears the blind or no discard is left, and otherwise discards
/// the lowest cards that are not part of its best hand.
pub fn greedy_move(run: &Run, salt: u64) -> (MoveKind, u64) {
    match run.phase {
        Phase::BlindSelect => (MoveKind::SelectBlind, 0),
        Phase::RoundEval => (MoveKind::CashOut, 0),
        Phase::Shop => (MoveKind::LeaveShop, 0),
        Phase::Pack => (MoveKind::PackSkip, 0),
        Phase::GameOver => (MoveKind::SelectBlind, 0),
        Phase::Round => {
            let fair = fair_copy(run, salt);
            let n = run.hand.len();
            let forced = run.forced_card();
            let mut best = (f64::MIN, 1u64);
            for mask in candidate_masks(n, forced) {
                let s = preview_score(&fair, mask);
                // Prefer fewer cards on ties: they leave more in hand.
                if s > best.0 || (s == best.0 && mask.count_ones() < best.1.count_ones()) {
                    best = (s, mask);
                }
            }
            let target = run.blind.as_ref().map_or(0.0, |b| b.chips);
            let clears = run.round_chips + best.0 >= target;
            if clears || run.discards_left < 1 || run.hands_left <= 1 {
                return (MoveKind::Play, best.1);
            }
            // Discard up to 5 of the lowest cards outside the best hand.
            let mut rest: Vec<usize> = (0..n).filter(|i| best.1 & (1 << i) == 0).collect();
            rest.sort_by_key(|&i| run.card(run.hand[i]).rank.id());
            let mut mask = 0u64;
            for &i in rest.iter().take(5) {
                mask |= 1 << i;
            }
            if let Some(f) = forced {
                if mask & (1 << f) == 0 {
                    if mask.count_ones() == 5 {
                        let drop = rest[..rest.len().min(5)].last().copied().unwrap_or(0);
                        mask &= !(1 << drop);
                    }
                    mask |= 1 << f;
                }
            }
            if mask == 0 {
                (MoveKind::Play, best.1)
            } else {
                (MoveKind::Discard, mask)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::env::apply;
    use crate::state::RunConfig;

    #[test]
    fn greedy_beats_the_first_blinds_often() {
        let mut seeds = bm_rng::SeedSampler::new(5);
        let mut cleared = 0;
        for i in 0..40 {
            let mut run = Run::new(RunConfig::new(&seeds.next_seed()));
            for step in 0..60 {
                if run.phase == Phase::GameOver || run.round >= 3 && run.phase == Phase::BlindSelect
                {
                    break;
                }
                let (k, a) = greedy_move(&run, i * 100 + step);
                apply(&mut run, k, a).unwrap();
            }
            if run.round >= 3 {
                cleared += 1;
            }
        }
        assert!(
            cleared >= 30,
            "greedy cleared both ante-1 blinds in only {cleared} of 40 runs"
        );
    }
}
