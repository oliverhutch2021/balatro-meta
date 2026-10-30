//! The hand planner: what to build towards this round, and so which cards to keep, play or throw now.
//!
//! A plan is a hand to build: a flush in one suit, three or four of a kind on one rank, a full house on two
//! ranks, a straight on five ranks, or simply playing the best hand every time. Each plan says, for any
//! hand, what to keep and what to throw: the throw is a discard while discards last, and after that, with
//! hands to spare, a hand played only to draw new cards (digging). Whatever the plan, a hand that beats the
//! blind is played as soon as it is there. The last hand is played once the discards are gone, never before
//! (with one hand, as against The Needle, the discards are the only way to a better hand).
//!
//! The planner tries every plan the hand allows many times on fair copies of the round: the deck's hidden
//! order is reshuffled in each, so it never learns the real draws, only the odds a player can work out from
//! the cards left in the deck. It picks the plan that beats the blind most often (then the one that gets
//! closest), and returns that plan's move now.

use crate::bots::preview_score;
use crate::card::{Enhancement, Rank, Suit};
use crate::env::{apply, MoveKind};
use crate::observe::fair_copy;
use crate::state::{Phase, Run};

/// Tries per plan. More is steadier and slower.
pub const TRIES: u64 = 10;
/// Most plans tried in one decision (the best-looking ones first).
const MAX_PLANS: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Plan {
    /// Play the best hand each time (discarding the cards outside it while discards last).
    Best,
    Flush(Suit),
    /// Three or four of a kind.
    Kind(Rank),
    /// A full house on two ranks (three of the first).
    House(Rank, Rank),
    /// A straight on the five ranks from `low` up (1 = Ace low).
    Straight(u8),
}

/// A hand card as plans see it: rank id (1..=14 where needed), suit, and whether it counts for either.
#[derive(Clone, Copy)]
struct Seen {
    rank: u8,
    suit: Suit,
    stone: bool,
    wild: bool,
}

fn seen(run: &Run) -> Vec<Seen> {
    run.hand
        .iter()
        .map(|&id| {
            let c = run.card(id);
            Seen {
                rank: c.rank.id(),
                suit: c.suit,
                stone: c.enhancement == Enhancement::Stone || c.face_down,
                wild: c.enhancement == Enhancement::Wild,
            }
        })
        .collect()
}

fn in_window(rank: u8, low: u8) -> bool {
    let r = if low == 1 && rank == 14 { 1 } else { rank };
    (low..low + 5).contains(&r)
}

impl Plan {
    /// Hand positions this plan keeps.
    fn keep(self, hand: &[Seen]) -> u64 {
        let mut mask = 0u64;
        match self {
            Plan::Best => {}
            Plan::Flush(s) => {
                for (i, c) in hand.iter().enumerate() {
                    if !c.stone && (c.suit == s || c.wild) {
                        mask |= 1 << i;
                    }
                }
            }
            Plan::Kind(r) => {
                for (i, c) in hand.iter().enumerate() {
                    if !c.stone && c.rank == r.id() {
                        mask |= 1 << i;
                    }
                }
            }
            Plan::House(a, b) => {
                for (i, c) in hand.iter().enumerate() {
                    if !c.stone && (c.rank == a.id() || c.rank == b.id()) {
                        mask |= 1 << i;
                    }
                }
            }
            Plan::Straight(low) => {
                let mut got = 0u32;
                for (i, c) in hand.iter().enumerate() {
                    let bit = if low == 1 && c.rank == 14 { 1 } else { c.rank };
                    if !c.stone && in_window(c.rank, low) && got & (1 << bit) == 0 {
                        got |= 1 << bit;
                        mask |= 1 << i;
                    }
                }
            }
        }
        mask
    }

    /// The plans this hand allows (Best always), most promising first.
    fn options(hand: &[Seen]) -> Vec<Plan> {
        let mut by_rank = [0u8; 15];
        let mut by_suit = [0u8; 4];
        let mut wild = 0u8;
        for c in hand.iter().filter(|c| !c.stone) {
            by_rank[c.rank as usize] += 1;
            if c.wild {
                wild += 1;
            } else {
                by_suit[c.suit as usize] += 1;
            }
        }
        let mut out = vec![(0u8, Plan::Best)];
        for s in Suit::ALL {
            let n = by_suit[s as usize] + wild;
            if n >= 3 {
                out.push((n * 2, Plan::Flush(s)));
            }
        }
        let mut pairs: Vec<Rank> = Rank::ALL
            .iter()
            .rev()
            .copied()
            .filter(|r| by_rank[r.id() as usize] >= 2)
            .collect();
        for &r in &pairs {
            out.push((by_rank[r.id() as usize] * 2 + 1, Plan::Kind(r)));
        }
        if pairs.len() >= 2 {
            pairs.sort_by_key(|r| std::cmp::Reverse((by_rank[r.id() as usize], r.id())));
            out.push((9, Plan::House(pairs[0], pairs[1])));
        }
        for low in 1..=10u8 {
            let have = (low..low + 5)
                .filter(|&r| by_rank[if r == 1 { 14 } else { r as usize }] > 0)
                .count() as u8;
            if have >= 3 {
                out.push((have * 2, Plan::Straight(low)));
            }
        }
        out.sort_by_key(|(k, _)| std::cmp::Reverse(*k));
        let mut plans: Vec<Plan> = out.into_iter().map(|(_, p)| p).collect();
        plans.truncate(MAX_PLANS);
        if !plans.contains(&Plan::Best) {
            plans.push(Plan::Best);
        }
        plans
    }
}

/// The best hand among the usual shapes (each pair, trips, two pair, full house, flush, straight, the high
/// card), scored as the game would score it now: (score, mask). Quicker than trying every subset.
pub fn quick_best(run: &Run) -> (f64, u64) {
    let hand = seen(run);
    let n = hand.len();
    let mut cands: Vec<u64> = Vec::with_capacity(16);
    let mut high = (0u8, 0usize);
    let mut by_rank: [u64; 15] = [0; 15];
    let mut by_suit: [Vec<usize>; 4] = Default::default();
    for (i, c) in hand.iter().enumerate() {
        if c.stone {
            continue;
        }
        if c.rank > high.0 {
            high = (c.rank, i);
        }
        by_rank[c.rank as usize] |= 1 << i;
        for s in Suit::ALL {
            if c.suit == s || c.wild {
                by_suit[s as usize].push(i);
            }
        }
    }
    if n > 0 {
        cands.push(1 << high.1);
    }
    let groups: Vec<u64> = (2..=14)
        .rev()
        .map(|r| by_rank[r])
        .filter(|m| m.count_ones() >= 2)
        .collect();
    for &g in &groups {
        cands.push(g & mask_first(g, 4));
    }
    if groups.len() >= 2 {
        cands.push(mask_first(groups[0], 2) | mask_first(groups[1], 2));
    }
    if let Some(&t) = groups.iter().find(|g| g.count_ones() >= 3) {
        if let Some(&p) = groups.iter().find(|&&g| g != t) {
            cands.push(mask_first(t, 3) | mask_first(p, 2));
        }
    }
    for s in Suit::ALL {
        let mut cards = by_suit[s as usize].clone();
        if cards.len() >= 5 {
            cards.sort_by_key(|&i| std::cmp::Reverse(hand[i].rank));
            cands.push(cards.iter().take(5).fold(0, |m, &i| m | 1 << i));
        }
    }
    for low in 1..=10u8 {
        let mut mask = 0u64;
        let mut ok = true;
        for r in low..low + 5 {
            let rid = if r == 1 { 14 } else { r as usize };
            let m = by_rank[rid];
            if m == 0 {
                ok = false;
                break;
            }
            mask |= m & m.wrapping_neg(); // one card of the rank
        }
        if ok {
            cands.push(mask);
        }
    }
    if let Some(f) = run.forced_card() {
        for c in &mut cands {
            *c |= 1 << f;
        }
    }
    let mut best = (f64::MIN, cands.first().copied().unwrap_or(1));
    for &m in &cands {
        if m.count_ones() == 0 || m.count_ones() > 5 {
            continue;
        }
        let s = preview_score(run, m);
        if s > best.0 {
            best = (s, m);
        }
    }
    best
}

/// The first `k` set bits of `mask`.
fn mask_first(mask: u64, k: u32) -> u64 {
    let mut out = 0u64;
    let mut left = mask;
    for _ in 0..k {
        if left == 0 {
            break;
        }
        let bit = left & left.wrapping_neg();
        out |= bit;
        left &= !bit;
    }
    out
}

/// Up to five hand positions to throw: those outside `keep`, lowest rank first (Stone cards first of all).
fn throw(run: &Run, hand: &[Seen], keep: u64) -> u64 {
    let mut out: Vec<usize> = (0..hand.len()).filter(|&i| keep >> i & 1 == 0).collect();
    out.sort_by_key(|&i| (!hand[i].stone, hand[i].rank));
    let mut mask = out.into_iter().take(5).fold(0u64, |m, i| m | 1 << i);
    if let Some(f) = run.forced_card() {
        if mask >> f & 1 == 0 {
            if mask.count_ones() == 5 {
                let top = 63 - mask.leading_zeros();
                mask &= !(1 << top);
            }
            mask |= 1 << f;
        }
    }
    mask
}

/// The plan's move now, given `best` (the best hand now) and whether it beats the blind.
fn plan_move(run: &Run, plan: Plan, best: (f64, u64)) -> (MoveKind, u64) {
    let target = run.blind.as_ref().map_or(0.0, |b| b.chips);
    let last = run.hands_left <= 1;
    if best.0 >= target - run.round_chips || (last && run.discards_left == 0) {
        return (MoveKind::Play, best.1);
    }
    let hand = seen(run);
    let keep = match plan {
        Plan::Best => best.1,
        p => p.keep(&hand),
    };
    let formed = match plan {
        Plan::Best => true,
        Plan::Flush(_) | Plan::Straight(_) => keep.count_ones() >= 5,
        Plan::Kind(_) => keep.count_ones() >= 4,
        Plan::House(..) => keep.count_ones() >= 5,
    };
    if plan != Plan::Best && formed {
        return (MoveKind::Play, best.1); // built: score with it
    }
    let out = throw(run, &hand, keep);
    if out == 0 {
        return (MoveKind::Play, best.1);
    }
    if run.discards_left > 0 {
        (MoveKind::Discard, out)
    } else if plan == Plan::Best || last {
        (MoveKind::Play, best.1)
    } else {
        (MoveKind::Play, out) // dig: a hand played to draw new cards
    }
}

/// One try of `plan` from `first` on a fair copy: (beat the blind, how close it got).
fn try_plan(run: &Run, plan: Plan, first: (MoveKind, u64), salt: u64) -> (bool, f64) {
    let target = run.blind.as_ref().map_or(1.0, |b| b.chips).max(1.0);
    let mut c = fair_copy(run, salt);
    if apply(&mut c, first.0, first.1).is_err() {
        return (false, 0.0);
    }
    let mut steps = 0;
    while c.phase == Phase::Round && steps < 40 {
        let m = plan_move(&c, plan, quick_best(&c));
        if apply(&mut c, m.0, m.1).is_err() {
            break;
        }
        steps += 1;
    }
    let beat = c.phase == Phase::RoundEval || c.won;
    (
        beat,
        if beat {
            1.0
        } else {
            (c.round_chips / target).min(1.0)
        },
    )
}

/// The planner's move for a round where the best hand now doesn't beat the blind: the move of the plan that
/// beats it most often on fair copies. `best` is the best hand now (from a full search).
pub fn plan(run: &Run, best: (f64, u64), salt: u64) -> (MoveKind, u64) {
    if run.forced_card().is_some() {
        return (MoveKind::Play, best.1); // Cerulean Bell: every selection holds its card; keep it simple
    }
    let hand = seen(run);
    let mut choice: Option<(f64, (MoveKind, u64))> = None;
    for (n, p) in Plan::options(&hand).into_iter().enumerate() {
        let first = plan_move(run, p, best);
        if choice.as_ref().is_some_and(|(_, m)| *m == first) {
            continue; // the same move now as a better-scoring plan
        }
        let mut score = 0.0;
        for t in 0..TRIES {
            let (beat, close) = try_plan(
                run,
                p,
                first,
                salt.wrapping_mul(0x9E37).wrapping_add((n as u64) << 32 | t),
            );
            score += if beat { 1.0 } else { 0.0 } + 0.01 * close;
        }
        if choice.as_ref().is_none_or(|(s, _)| score > *s) {
            choice = Some((score, first));
        }
    }
    choice.map_or((MoveKind::Play, best.1), |(_, m)| m)
}

/// Every subset-based best hand, for the planner's tests.
#[cfg(test)]
pub(crate) fn full_best(run: &Run) -> (f64, u64) {
    let mut best = (f64::MIN, 1u64);
    for mask in crate::bots::candidate_masks(run.hand.len(), run.forced_card()) {
        let s = preview_score(run, mask);
        if s > best.0 {
            best = (s, mask);
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(rank: u8, suit: Suit) -> Seen {
        Seen {
            rank,
            suit,
            stone: false,
            wild: false,
        }
    }

    #[test]
    fn plans_keep_what_they_build() {
        use Suit::*;
        let hand = [
            s(14, Hearts),
            s(9, Hearts),
            s(4, Hearts),
            s(4, Spades),
            s(10, Clubs),
            s(11, Hearts),
            s(12, Diamonds),
            s(2, Clubs),
        ];
        assert_eq!(Plan::Flush(Hearts).keep(&hand), 0b0010_0111);
        assert_eq!(Plan::Kind(Rank::Four).keep(&hand), 0b0000_1100);
        // 10, J, Q, (K missing), A: one card per rank in the window 10..14.
        assert_eq!(Plan::Straight(10).keep(&hand), 0b0111_0001);
        let opts = Plan::options(&hand);
        assert!(
            opts.contains(&Plan::Best)
                && opts.contains(&Plan::Flush(Hearts))
                && opts.contains(&Plan::Kind(Rank::Four))
        );
        assert!(opts.contains(&Plan::Straight(10)));
        assert!(
            !opts.contains(&Plan::Flush(Clubs)),
            "two clubs make no flush plan"
        );
    }

    #[test]
    fn the_quick_best_hand_matches_the_full_search_on_plain_hands() {
        let mut seeds = crate::bm_rng::SeedSampler::new(31);
        let mut same = 0;
        let mut total = 0;
        for _ in 0..40 {
            let mut run = Run::new(crate::RunConfig::new(&seeds.next_seed()));
            apply(&mut run, MoveKind::SelectBlind, 0).unwrap();
            let (q, _) = quick_best(&run);
            let (f, _) = full_best(&run);
            assert!(q <= f + 1e-9, "the quick search can't beat the full one");
            same += usize::from((q - f).abs() < 1e-9);
            total += 1;
        }
        assert!(same * 10 >= total * 9, "{same} of {total} the same");
    }
}
