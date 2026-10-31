//! Simple bots for demos, baselines and smoke tests. They decide from the
//! screen and from fair copies only (`observe::fair_copy`), never from the
//! real run's future.

use crate::card::Edition;
use crate::consumables::UseFrom;
use crate::content::ConsumableSet;
use crate::env::MoveKind;
use crate::observe::fair_copy;
use crate::packs::PackCard;
use crate::scoring::evaluate_play;
use crate::shop::RowItem;
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
    preview(fair, mask).score
}

thread_local! {
    /// The copy each preview plays its hand on, kept so that copying into it reuses its memory.
    static PREVIEW: std::cell::RefCell<Option<Run>> = const { std::cell::RefCell::new(None) };
}

/// The whole evaluation of playing these hand positions now (hand type, chips, mult, score), on a copy.
pub fn preview(fair: &Run, mask: u64) -> crate::scoring::HandScore {
    PREVIEW.with(|cell| {
        let mut kept = cell.borrow_mut();
        let sim = match kept.as_mut() {
            Some(sim) => {
                sim.clone_from(fair);
                sim
            }
            None => kept.insert(fair.clone()),
        };
        sim.play.clear();
        sim.hand.clear();
        for (i, &id) in fair.hand.iter().enumerate() {
            if mask & (1 << i) != 0 {
                sim.play.push(id);
            } else {
                sim.hand.push(id);
            }
        }
        sim.hands_left -= 1;
        evaluate_play(sim)
    })
}

fn joker_room(run: &Run, e: Edition) -> bool {
    let need = i32::from(e != Edition::Negative);
    run.jokers.len() as i32 + need <= run.joker_slots
}

fn consumable_room(run: &Run, e: Edition) -> bool {
    let need = i32::from(e != Edition::Negative);
    run.consumables.len() as i32 + need <= run.consumable_slots
}

/// A held Planet to use now, if any.
fn planet_to_use(run: &Run) -> Option<(MoveKind, u64)> {
    run.consumables
        .iter()
        .position(|c| c.kind.set() == ConsumableSet::Planet)
        .map(|i| (MoveKind::UseConsumable, i as u64))
}

/// Shopping: the first joker it can afford and hold, then the first Planet
/// (used at once), then a pack while keeping $10; otherwise leave.
fn shop_move(run: &Run) -> (MoveKind, u64) {
    let afford = |cost: i64| cost <= 0 || cost <= run.spendable();
    if let Some(shop) = run.shop.as_ref() {
        for (i, r) in shop.row.iter().enumerate() {
            let buy = match &r.item {
                RowItem::Joker(j) => afford(r.cost) && joker_room(run, j.edition),
                RowItem::Consumable(c) => {
                    c.kind.set() == ConsumableSet::Planet
                        && afford(r.cost)
                        && consumable_room(run, c.edition)
                }
                RowItem::Card(_) => false,
            };
            if buy {
                return (MoveKind::Buy, i as u64);
            }
        }
        if let Some(m) = planet_to_use(run) {
            return m;
        }
        for (i, b) in shop.boosters.iter().enumerate() {
            if let Some(b) = b {
                if afford(b.cost) && run.spendable() - b.cost >= 10 {
                    return (MoveKind::OpenBooster, i as u64);
                }
            }
        }
    }
    (MoveKind::LeaveShop, 0)
}

/// In an open pack: the first card it can take without selecting hand
/// cards, else skip.
fn pack_move(run: &Run) -> (MoveKind, u64) {
    if let Some(p) = run.pack.as_ref() {
        for (i, c) in p.cards.iter().enumerate() {
            let ok = match c {
                PackCard::Joker(j) => joker_room(run, j.edition),
                PackCard::Consumable(c) => run.can_use_consumable(c, &[], UseFrom::Pack),
                PackCard::Card(_) => true,
            };
            if ok {
                return (MoveKind::PackChoose, i as u64);
            }
        }
    }
    (MoveKind::PackSkip, 0)
}

/// The greedy bot: always takes the blind, plays the highest-scoring hand
/// when it clears the blind or no discard is left, and otherwise discards
/// the lowest cards that are not part of its best hand. It uses Planets as
/// soon as it holds them, buys jokers and Planets it can afford, opens packs
/// while it keeps $10, and takes the first pack card that needs no hand
/// selection.
pub fn greedy_move(run: &Run, salt: u64) -> (MoveKind, u64) {
    match run.phase {
        Phase::BlindSelect => (MoveKind::SelectBlind, 0),
        Phase::RoundEval => (MoveKind::CashOut, 0),
        Phase::Shop => shop_move(run),
        Phase::Pack => pack_move(run),
        Phase::GameOver => (MoveKind::SelectBlind, 0),
        Phase::Round if planet_to_use(run).is_some() => planet_to_use(run).expect("checked"),
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

/// Where quick playouts from a position end: how the run is going.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Projection {
    pub playouts: u32,
    /// Runs that end at each ante (index 1 to 8 = lost at that ante, 9 =
    /// won); index 0 unused.
    pub final_ante: [u32; 10],
    /// Playouts in which the blind being played (or the next one) is beaten.
    pub cleared_blind: u32,
}

impl Projection {
    pub fn mean_ante(&self) -> f64 {
        let n: u32 = self.final_ante.iter().sum();
        if n == 0 {
            return 0.0;
        }
        let s: u64 = self
            .final_ante
            .iter()
            .enumerate()
            .map(|(a, &c)| a as u64 * c as u64)
            .sum();
        s as f64 / n as f64
    }

    pub fn p_win(&self) -> f64 {
        self.final_ante[9] as f64 / self.playouts.max(1) as f64
    }

    pub fn p_clear(&self) -> f64 {
        self.cleared_blind as f64 / self.playouts.max(1) as f64
    }
}

/// Quick playouts by the greedy bot from fair copies of `run` (hidden parts
/// reshuffled, fresh random streams), until the run is lost or won: at most
/// `n`, and no more than `budget` of time once 8 are done. For the viewer's
/// outlook chart; never shown to a bot.
pub fn projection(run: &Run, n: usize, budget: std::time::Duration, salt: u64) -> Projection {
    let start = std::time::Instant::now();
    let mut p = Projection::default();
    for i in 0..n as u64 {
        if i >= 8 && start.elapsed() > budget {
            break;
        }
        let mut sim = fair_copy(run, salt.wrapping_mul(1_000_003).wrapping_add(i));
        let round = sim.round;
        let in_round = sim.phase == Phase::Round;
        let mut cleared = None;
        for step in 0..4000u64 {
            if sim.phase == Phase::GameOver || sim.won {
                break;
            }
            let (k, a) = greedy_move(&sim, salt ^ (i << 20) ^ step);
            if crate::env::apply(&mut sim, k, a).is_err() {
                break;
            }
            // The blind being played (or, between blinds, the next one played).
            let target_round = if in_round { round } else { round + 1 };
            if cleared.is_none() && sim.round == target_round && sim.phase == Phase::RoundEval {
                cleared = Some(true);
            }
            if cleared.is_none() && (sim.round > target_round || sim.phase == Phase::GameOver) {
                cleared = Some(sim.round > target_round);
            }
        }
        let ante = if sim.won {
            9
        } else {
            sim.ante.clamp(1, 8) as usize
        };
        p.final_ante[ante] += 1;
        if cleared == Some(true) || sim.won {
            p.cleared_blind += 1;
        }
        p.playouts += 1;
    }
    p
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
            for step in 0..300 {
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

    /// The old way of previewing: a fresh copy of the run for each hand.
    fn preview_on_a_fresh_copy(fair: &Run, mask: u64) -> crate::scoring::HandScore {
        let mut sim = fair.clone();
        let chosen: Vec<_> = (0..fair.hand.len())
            .filter(|i| mask & (1 << i) != 0)
            .map(|i| fair.hand[i])
            .collect();
        sim.hand.retain(|id| !chosen.contains(id));
        sim.play = chosen;
        sim.hands_left -= 1;
        evaluate_play(&mut sim)
    }

    /// Positions from greedy runs that get on (jokers, levels, changed cards): in rounds, or in every phase.
    fn positions(n: usize, rounds_only: bool) -> Vec<Run> {
        let mut seeds = bm_rng::SeedSampler::new(41);
        let mut out = Vec::new();
        for i in 0..6 {
            let mut run = Run::new(RunConfig::new(&seeds.next_seed()));
            for step in 0..3000 {
                if run.phase == Phase::GameOver || run.won || out.len() >= n * (i + 1) / 6 {
                    break;
                }
                if (run.phase == Phase::Round || !rounds_only) && step % 3 == 0 {
                    out.push(run.clone());
                }
                let (k, a) = greedy_move(&run, i as u64 * 10_000 + step);
                apply(&mut run, k, a).unwrap();
            }
        }
        out
    }

    #[test]
    fn a_preview_on_the_kept_copy_scores_as_on_a_fresh_one() {
        let runs = positions(60, true);
        assert!(runs.len() >= 40 && runs.iter().any(|r| r.jokers.len() >= 3));
        // Each position's hands in turn, and positions interleaved, so nothing carries over between them.
        for (i, run) in runs.iter().enumerate() {
            let other = &runs[(i * 7 + 3) % runs.len()];
            for mask in candidate_masks(run.hand.len(), None).into_iter().step_by(7) {
                assert_eq!(
                    preview(run, mask),
                    preview_on_a_fresh_copy(run, mask),
                    "position {i}, mask {mask:b}"
                );
                let m = mask & ((1 << other.hand.len()) - 1);
                if m != 0 && m.count_ones() <= 5 {
                    assert_eq!(preview(other, m), preview_on_a_fresh_copy(other, m));
                }
            }
        }
    }

    #[test]
    fn copying_into_a_kept_run_gives_the_same_run_as_a_fresh_copy() {
        let runs = positions(90, false);
        for phase in [Phase::Round, Phase::Shop, Phase::Pack, Phase::BlindSelect] {
            assert!(runs.iter().any(|r| r.phase == phase), "{phase:?}");
        }
        let mut kept = Run::new(RunConfig::new("KEPTRUN1"));
        for run in runs.iter().chain(runs.iter().rev()) {
            kept.clone_from(run);
            assert_eq!(format!("{kept:?}"), format!("{run:?}"));
            assert_eq!(format!("{:?}", run.clone()), format!("{run:?}"));
        }
    }

    #[test]
    fn projections_are_deterministic_and_sensible() {
        let run = Run::new(RunConfig::new("PROJECT1"));
        let a = projection(&run, 16, std::time::Duration::from_secs(60), 7);
        let b = projection(&run, 16, std::time::Duration::from_secs(60), 7);
        assert_eq!(a, b);
        assert_eq!(a.playouts, 16);
        assert_eq!(a.final_ante.iter().sum::<u32>(), 16);
        assert!(a.mean_ante() >= 1.0 && a.mean_ante() <= 9.0);
        assert!(
            a.p_clear() > 0.5,
            "greedy clears the first Small Blind most of the time"
        );
    }

    #[test]
    fn greedy_moves_are_always_legal() {
        let mut seeds = bm_rng::SeedSampler::new(8);
        let (mut bought, mut opened, mut used) = (0, 0, 0);
        for i in 0..12 {
            let mut run = Run::new(RunConfig::new(&seeds.next_seed()));
            for step in 0..3000 {
                if run.phase == Phase::GameOver || run.won {
                    break;
                }
                let (k, a) = greedy_move(&run, i * 10_000 + step);
                match k {
                    MoveKind::Buy => bought += 1,
                    MoveKind::OpenBooster => opened += 1,
                    MoveKind::UseConsumable => used += 1,
                    _ => {}
                }
                apply(&mut run, k, a).unwrap_or_else(|e| panic!("{k:?} {a}: {e}"));
            }
        }
        assert!(
            bought > 0 && opened > 0 && used > 0,
            "{bought} {opened} {used}"
        );
    }
}
