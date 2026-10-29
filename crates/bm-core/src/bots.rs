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
