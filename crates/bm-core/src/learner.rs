//! The learner's computed inputs (lab #38, Stage 0): what the engine can tell the network cheaply, so it need not be
//! rediscovered from outcomes.
//!
//! `fast_capacity`: the build's scoring capacity, the best of the usual hand shapes (`planner::quick_best`) on k
//! fair deals of the coming round, times the round's hands (antezero's preview, `PREVIEW_HANDS` x `PREVIEW_PLAYS`,
//! made over our own planner), set against the asks of this ante's next blind and boss and of the next two antes
//! (S1: nothing in antezero's inputs looks past the current ante). The build is frozen, so the far asks say how far
//! today's build would carry. Everything is read from what the screen shows: the deals are fair copies whose salt
//! comes from the screen itself, so two positions that look the same get the same reading (B's test 4).

use crate::blinds::{blind_target, scaling_for_stake, BlindKind, Boss};
use crate::state::{Phase, Run};

/// Fair deals behind a reading.
pub const DEALS: u64 = 4;
/// The asks a reading is set against: this ante's next blind, this ante's boss, the next ante's Small, Big and
/// boss, and the boss two antes on.
pub const ASKS: usize = 6;
/// The learner block's columns `fast_capacity` fills (after column 0, `env`'s blinds paid ahead): ln capacity /
/// 20, then for each ask its log margin / 4 (clamped to +-1) and the share of deals that clear it.
pub const CAPACITY_COLUMNS: usize = 1 + 2 * ASKS;

/// A capacity reading: ln of the round's capacity (mean over deals), and against each ask its mean log margin and
/// the share of deals whose capacity clears it.
#[derive(Clone, Debug, PartialEq)]
pub struct Capacity {
    pub ln: f64,
    pub margin: [f64; ASKS],
    pub clear: [f64; ASKS],
}

/// Against every ask the read is the same (the coming round's deal, with no boss effect): the boss's effect on a deal
/// already made isn't reproduced by `quick_best` (debuffs are set as cards are drawn), so this ante's boss is read as
/// its target only. Its rank agreement with the value's power read (`multipliers::power`, whole rounds played) was
/// 0.90 on 371 shop states of light-teacher runs at antes 2 and up; against the boss with its effect, 0.39.
///
/// The asks (chips) for `run` as it stands: (this ante's next blind, its boss, the next ante's three, the boss two
/// antes on). A boss not yet known counts as a plain boss (x2).
pub fn asks(run: &Run) -> [f64; ASKS] {
    let sc = scaling_for_stake(run.cfg.stake);
    let a = run.ante.max(1);
    let next = match (&run.blind, run.phase) {
        (Some(b), Phase::Round) => b.chips,
        _ => {
            let kind = match run.on_deck() {
                crate::blinds::BlindSlot::Small => BlindKind::Small,
                crate::blinds::BlindSlot::Big => BlindKind::Big,
                crate::blinds::BlindSlot::Boss => BlindKind::Boss(run.boss),
            };
            blind_target(a, sc, kind, 1.0)
        }
    };
    let plain = BlindKind::Boss(Boss::Hook);
    [
        next,
        blind_target(a, sc, BlindKind::Boss(run.boss), 1.0),
        blind_target(a + 1, sc, BlindKind::Small, 1.0),
        blind_target(a + 1, sc, BlindKind::Big, 1.0),
        blind_target(a + 1, sc, plain, 1.0),
        blind_target(a + 2, sc, plain, 1.0),
    ]
}

/// The build's capacity on `deals` fair deals of its coming round (salt from the screen: `screen_salt`).
pub fn fast_capacity(run: &Run, salt: u64, deals: u64) -> Option<Capacity> {
    if run.phase == Phase::GameOver || run.won {
        return None;
    }
    let asks = asks(run);
    let mut lns = Vec::with_capacity(deals as usize);
    for k in 0..deals {
        let mut c = next_round_fair(run, salt.wrapping_add(k.wrapping_mul(0x9E37_79B9)))?;
        while let Some(m) = crate::teacher::reorder(&c) {
            if crate::env::apply(&mut c, m.0, m.1).is_err() {
                break;
            }
        }
        let (best, _) = crate::planner::quick_best(&c);
        let hands = c.hands_left.max(1) as f64;
        lns.push((best.max(1.0) * hands).ln());
    }
    if lns.is_empty() {
        return None;
    }
    let n = lns.len() as f64;
    let ln = lns.iter().sum::<f64>() / n;
    let mut margin = [0.0; ASKS];
    let mut clear = [0.0; ASKS];
    for (i, &ask) in asks.iter().enumerate() {
        let l = ask.max(1.0).ln();
        margin[i] = lns.iter().map(|x| x - l).sum::<f64>() / n;
        clear[i] = lns.iter().filter(|&&x| x >= l).count() as f64 / n;
    }
    Some(Capacity { ln, margin, clear })
}

/// A fair copy moved on to the start of its coming round (`value::next_round`), or the round it is in.
fn next_round_fair(run: &Run, salt: u64) -> Option<Run> {
    crate::value::next_round(run, salt)
}

/// A salt from what the screen shows (its encoding), so the same screen always reads the same.
pub fn screen_salt(screen_floats: &[f32]) -> u64 {
    let mut h: u64 = 0xCBF2_9CE4_8422_2325;
    for &x in screen_floats {
        h ^= u64::from(x.to_bits());
        h = h.wrapping_mul(0x0100_0000_01B3);
    }
    h
}

/// The learner block's capacity columns, written into `out` (CAPACITY_COLUMNS floats; zeros when the run is over).
pub fn write_capacity(out: &mut [f32], c: Option<&Capacity>) {
    let Some(c) = c else { return };
    out[0] = (c.ln / 20.0) as f32;
    for i in 0..ASKS {
        out[1 + 2 * i] = (c.margin[i] / 4.0).clamp(-1.0, 1.0) as f32;
        out[2 + 2 * i] = c.clear[i] as f32;
    }
}

/// Play candidates (lab #38, A's queue after the exchange evaluation): with the network's shops, the teacher's play
/// took ante-1 deaths from 17% to 2%, and the network alone almost never found a Flush or a Straight (0.4% of hands),
/// which a free pick of cards has to assemble card by card. So the engine offers its own best plays as choices, each
/// with what it scores, and the network can still pick its own cards (the last slot).
///
/// In a round with a hand left: the planner's hand shapes (`planner::shape_candidates`: each pair, trips, two pair,
/// full house, flush, straight, high card, Cerulean Bell's card and The Psychic's padding), each scored on a fair copy
/// whose salt is the screen's, so a chance effect (Lucky cards, Misprint) is never read off the run's own streams,
/// the best `PLAY_CANDS` by score, best first.
pub const PLAY_CANDS: usize = 8;
/// The candidates' slots: `PLAY_CANDS` candidates, then the free pick (present whenever a play is).
pub const PLAY_SLOTS: usize = PLAY_CANDS + 1;
/// Floats per candidate slot (`write_candidates`): 0 present; 1 to 12 the hand type; 13 ln score / 20; 14 log margin
/// against the chips still to score this round / 4 (+-1); 15 it clears the blind now; 16 log margin of its score
/// times the hands left against the chips still to score / 4 (+-1); 17 cards / 5; 18 its place in the list /
/// PLAY_CANDS; 19 ln chips / 10; 20 ln mult / 10; 21 the boss makes it score nothing; 22 scoring cards / 5; 23 the
/// free pick's flag; then `MAX_HAND` columns: the hand positions it plays (1 or 0).
pub const CAND_FEATURES: usize = 24 + crate::observe::MAX_HAND;
/// The first of the card columns.
pub const CAND_CARDS: usize = 24;

/// A candidate: the hand positions played and what the play scores.
#[derive(Clone, Debug)]
pub struct Candidate {
    pub mask: u64,
    pub score: crate::scoring::HandScore,
}

/// The play candidates of `run` (empty outside a round or with no hand left), best first, at most `PLAY_CANDS`.
pub fn play_candidates(run: &Run, salt: u64) -> Vec<Candidate> {
    if run.phase != Phase::Round || run.hands_left <= 0 || run.hand.is_empty() || run.won {
        return Vec::new();
    }
    let fair = crate::observe::fair_copy(run, salt ^ 0xC4_4D5);
    let mut seen: Vec<u64> = Vec::new();
    let mut out: Vec<Candidate> = Vec::new();
    for m in crate::planner::shape_candidates(&fair) {
        if m == 0 || m.count_ones() > 5 || seen.contains(&m) {
            continue;
        }
        seen.push(m);
        out.push(Candidate { mask: m, score: crate::bots::preview(&fair, m) });
    }
    // Best first; equal scores keep the planner's order (stable).
    out.sort_by(|a, b| b.score.score.partial_cmp(&a.score.score).unwrap_or(std::cmp::Ordering::Equal));
    out.truncate(PLAY_CANDS);
    out
}

/// The candidates' block, written into `out` (PLAY_SLOTS x CAND_FEATURES floats, zeros outside a round).
pub fn write_candidates(out: &mut [f32], run: &Run, cands: &[Candidate]) {
    if run.phase != Phase::Round || run.hands_left <= 0 || run.hand.is_empty() || run.won {
        return;
    }
    let left = run
        .blind
        .as_ref()
        .map_or(1.0, |b| (b.chips - run.round_chips).max(1.0));
    let hands = run.hands_left.max(1) as f64;
    let lm = |x: f64| ((x.max(1.0).ln() - left.ln()) / 4.0).clamp(-1.0, 1.0) as f32;
    for (k, c) in cands.iter().enumerate().take(PLAY_CANDS) {
        let row = &mut out[k * CAND_FEATURES..(k + 1) * CAND_FEATURES];
        let s = &c.score;
        let score = if s.not_allowed { 0.0 } else { s.score };
        row[0] = 1.0;
        row[1 + s.hand as usize] = 1.0;
        row[13] = ((score + 1.0).ln() / 20.0) as f32;
        row[14] = lm(score);
        row[15] = f32::from(score >= left);
        row[16] = lm(score * hands);
        row[17] = c.mask.count_ones() as f32 / 5.0;
        row[18] = k as f32 / PLAY_CANDS as f32;
        row[19] = (s.chips.max(1.0).ln() / 10.0) as f32;
        row[20] = (s.mult.max(1.0).ln() / 10.0) as f32;
        row[21] = f32::from(s.not_allowed);
        row[22] = s.scoring.len() as f32 / 5.0;
        for i in 0..crate::observe::MAX_HAND {
            if c.mask & (1 << i) != 0 {
                row[CAND_CARDS + i] = 1.0;
            }
        }
    }
    let free = &mut out[PLAY_CANDS * CAND_FEATURES..PLAY_SLOTS * CAND_FEATURES];
    free[0] = 1.0;
    free[23] = 1.0;
}

/// Discard candidates (lab #38, smoke-3): the planner's own plans for the hand (`planner::options_for`: a flush in a
/// suit, a kind, a full house, a straight, the best hand), each as the cards it would throw (`planner::throw`, at
/// most 5, the cheapest first, Cerulean Bell's card kept in), with what the throw is for: the cards it keeps, the
/// cards it still needs, how many cards left in the draw pile would help (the deck view's counts: the pile and the
/// face-down cards in hand), and the chance a throw of that size draws what it needs (hypergeometric). Read on a
/// fair copy (the best hand's mask is scored there). The network can still pick its own cards (the last slot).
pub const DISCARD_CANDS: usize = 8;
/// The discard candidates' slots: `DISCARD_CANDS`, then the free pick (present whenever a discard is legal).
pub const DISCARD_SLOTS: usize = DISCARD_CANDS + 1;
/// Floats per discard slot (`write_discards`): 0 present; 1 to 5 the plan (best, flush, kind, full house,
/// straight); 6 cards kept / 8; 7 cards thrown / 5; 8 cards still needed / 5; 9 helping cards / cards in the pile;
/// 10 expected helping cards drawn / 5; 11 the chance of drawing all it needs; 12 discards left / 5; 13 hands left
/// / 5; 14 its place in the list / DISCARD_CANDS; 15 the free pick's flag; then `MAX_HAND` columns: the hand
/// positions it throws.
pub const DCAND_FEATURES: usize = 16 + crate::observe::MAX_HAND;
/// The first of the discard tokens' card columns.
pub const DCAND_CARDS: usize = 16;

/// A discard candidate: the hand positions thrown and what for.
#[derive(Clone, Debug)]
pub struct DiscardCand {
    pub mask: u64,
    /// 0 best, 1 flush, 2 kind, 3 full house, 4 straight.
    pub plan: u8,
    pub kept: u32,
    pub need: u32,
    pub outs: u32,
    pub pile: u32,
}

/// The chance that `draws` cards from a pile of `pile` holding `outs` helpers bring at least `need` of them.
pub fn chance_at_least(pile: u32, outs: u32, draws: u32, need: u32) -> f64 {
    if need == 0 {
        return 1.0;
    }
    let draws = draws.min(pile);
    if outs < need || draws < need || pile == 0 {
        return 0.0;
    }
    // ln C(n, k)
    let lc = |n: u32, k: u32| -> f64 {
        if k > n {
            return f64::NEG_INFINITY;
        }
        (1..=k).map(|i| ((n - k + i) as f64).ln() - (i as f64).ln()).sum()
    };
    let total = lc(pile, draws);
    (need..=draws.min(outs))
        .map(|k| (lc(outs, k) + lc(pile - outs, draws - k) - total).exp())
        .sum::<f64>()
        .min(1.0)
}

fn discard_live(run: &Run) -> bool {
    run.phase == Phase::Round && run.discards_left > 0 && run.hands_left > 0 && !run.hand.is_empty() && !run.won
}

/// The discard candidates of `run` (empty when no discard is legal), in the planner's order, at most
/// `DISCARD_CANDS`, each throw distinct.
pub fn discard_candidates(run: &Run, salt: u64) -> Vec<DiscardCand> {
    use crate::planner::Plan;
    if !discard_live(run) {
        return Vec::new();
    }
    let fair = crate::observe::fair_copy(run, salt ^ 0xD15C);
    let hand = crate::planner::seen(&fair);
    // The deck view's counts: the draw pile and the face-down cards in hand.
    let pile_cards: Vec<&crate::card::Card> = fair
        .deck
        .iter()
        .chain(fair.hand.iter().filter(|&&id| fair.card(id).face_down))
        .map(|&id| fair.card(id))
        .collect();
    let pile = pile_cards.len() as u32;
    let usable = |c: &crate::card::Card| c.enhancement != crate::card::Enhancement::Stone;
    let best = crate::planner::quick_best(&fair);
    let mut out: Vec<DiscardCand> = Vec::new();
    for plan in crate::planner::options_for(&fair) {
        let keep = match plan {
            Plan::Best => best.1,
            p => p.keep(&hand),
        };
        let mask = crate::planner::throw(&fair, &hand, keep);
        if mask == 0 || out.iter().any(|d| d.mask == mask) {
            continue;
        }
        let kept = keep.count_ones();
        let (code, need, outs) = match plan {
            Plan::Best => (0u8, 0u32, 0u32),
            Plan::Flush(s) => {
                let n = pile_cards
                    .iter()
                    .filter(|c| usable(c) && (c.suit == s || c.enhancement == crate::card::Enhancement::Wild))
                    .count();
                (1, 5u32.saturating_sub(kept), n as u32)
            }
            Plan::Kind(r) => {
                let n = pile_cards.iter().filter(|c| usable(c) && c.rank == r).count();
                (2, 4u32.saturating_sub(kept), n as u32)
            }
            Plan::House(a, b) => {
                let n = pile_cards.iter().filter(|c| usable(c) && (c.rank == a || c.rank == b)).count();
                (3, 5u32.saturating_sub(kept), n as u32)
            }
            Plan::Straight(low) => {
                // Ranks of the window not yet held.
                let held: Vec<u8> = (0..hand.len())
                    .filter(|&i| keep >> i & 1 == 1)
                    .map(|i| fair.card(fair.hand[i]).rank.id())
                    .map(|r| if low == 1 && r == 14 { 1 } else { r })
                    .collect();
                let n = pile_cards
                    .iter()
                    .filter(|c| usable(c))
                    .filter(|c| {
                        let r = c.rank.id();
                        let r = if low == 1 && r == 14 { 1 } else { r };
                        (low..low + 5).contains(&r) && !held.contains(&r)
                    })
                    .count();
                (4, 5u32.saturating_sub(kept), n as u32)
            }
        };
        out.push(DiscardCand { mask, plan: code, kept, need, outs, pile });
        if out.len() == DISCARD_CANDS {
            break;
        }
    }
    out
}

/// The discard candidates' block, written into `out` (DISCARD_SLOTS x DCAND_FEATURES floats, zeros when no discard
/// is legal).
pub fn write_discards(out: &mut [f32], run: &Run, cands: &[DiscardCand]) {
    if !discard_live(run) {
        return;
    }
    for (k, d) in cands.iter().enumerate().take(DISCARD_CANDS) {
        let row = &mut out[k * DCAND_FEATURES..(k + 1) * DCAND_FEATURES];
        let thrown = d.mask.count_ones();
        row[0] = 1.0;
        row[1 + d.plan as usize] = 1.0;
        row[6] = d.kept as f32 / 8.0;
        row[7] = thrown as f32 / 5.0;
        row[8] = d.need as f32 / 5.0;
        row[9] = if d.pile > 0 { d.outs as f32 / d.pile as f32 } else { 0.0 };
        row[10] = if d.pile > 0 { (thrown.min(d.pile) as f32 * d.outs as f32 / d.pile as f32) / 5.0 } else { 0.0 };
        row[11] = if d.plan == 0 { 0.0 } else { chance_at_least(d.pile, d.outs, thrown, d.need) as f32 };
        row[12] = run.discards_left as f32 / 5.0;
        row[13] = run.hands_left as f32 / 5.0;
        row[14] = k as f32 / DISCARD_CANDS as f32;
        for i in 0..crate::observe::MAX_HAND {
            if d.mask & (1 << i) != 0 {
                row[DCAND_CARDS + i] = 1.0;
            }
        }
    }
    let free = &mut out[DISCARD_CANDS * DCAND_FEATURES..DISCARD_SLOTS * DCAND_FEATURES];
    free[0] = 1.0;
    free[15] = 1.0;
    free[12] = run.discards_left as f32 / 5.0;
    free[13] = run.hands_left as f32 / 5.0;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The asks grow with the antes ahead, so the margins fall and the clears don't rise; the same salt reads the same.
    #[test]
    fn a_reading_falls_against_the_antes_ahead() {
        let run = Run::new(crate::RunConfig::new("CAPA1"));
        let a = asks(&run);
        assert!(a[2] > a[0] && a[4] > a[3] && a[5] > a[4], "{a:?}");
        let c = fast_capacity(&run, 7, DEALS).expect("a reading");
        assert!(c.ln > 0.0);
        assert!(c.margin[5] < c.margin[4] && c.margin[4] < c.margin[2] && c.margin[2] < c.margin[0]);
        assert!(c.clear[5] <= c.clear[2] && c.clear[2] <= c.clear[0]);
        assert_eq!(fast_capacity(&run, 7, DEALS), Some(c));
        let mut out = [0.0f32; CAPACITY_COLUMNS];
        write_capacity(&mut out, fast_capacity(&run, 7, DEALS).as_ref());
        assert!(out.iter().all(|x| (-1.0..=1.0).contains(x)));
    }

    /// In a round the candidates are distinct plays of 1 to 5 cards in hand, best first, the same for the same salt,
    /// and none outside a round; the block marks them and the free pick, and each card column is the play's mask.
    #[test]
    fn play_candidates_are_the_best_plays_in_hand() {
        let mut run = Run::new(crate::RunConfig::new("CANDS1"));
        let mut empty = vec![0.0f32; PLAY_SLOTS * CAND_FEATURES];
        assert!(play_candidates(&run, 3).is_empty());
        write_candidates(&mut empty, &run, &[]);
        assert!(empty.iter().all(|&x| x == 0.0), "nothing outside a round");
        crate::env::apply(&mut run, crate::env::MoveKind::SelectBlind, 0).unwrap();
        assert_eq!(run.phase, Phase::Round);
        let c = play_candidates(&run, 3);
        assert!(!c.is_empty() && c.len() <= PLAY_CANDS);
        let n = run.hand.len() as u32;
        for (i, x) in c.iter().enumerate() {
            assert!((1..=5).contains(&x.mask.count_ones()) && x.mask >> n == 0, "{:b}", x.mask);
            assert!(c[..i].iter().all(|y| y.mask != x.mask && y.score.score >= x.score.score));
        }
        let again = play_candidates(&run, 3);
        assert_eq!(c.iter().map(|x| x.mask).collect::<Vec<_>>(), again.iter().map(|x| x.mask).collect::<Vec<_>>());
        let mut out = vec![0.0f32; PLAY_SLOTS * CAND_FEATURES];
        write_candidates(&mut out, &run, &c);
        for (k, x) in c.iter().enumerate() {
            let row = &out[k * CAND_FEATURES..(k + 1) * CAND_FEATURES];
            assert_eq!(row[0], 1.0);
            let bits = (0..crate::observe::MAX_HAND).fold(0u64, |m, i| m | (u64::from(row[CAND_CARDS + i] > 0.5) << i));
            assert_eq!(bits, x.mask);
            assert_eq!(row[1..13].iter().sum::<f32>(), 1.0);
        }
        let free = &out[PLAY_CANDS * CAND_FEATURES..];
        assert!(free[0] == 1.0 && free[23] == 1.0);
        // The best candidate scores at least what the planner's quick best does on the same fair copy.
        let fair = crate::observe::fair_copy(&run, 3 ^ 0xC4_4D5);
        assert!(c[0].score.score >= crate::planner::quick_best(&fair).0 - 1e-9);
    }

    /// The hypergeometric chance: certain with nothing needed, impossible without enough helpers or draws, and
    /// the textbook value for one helper drawn from four in a pile of ten with three draws.
    #[test]
    fn the_chance_of_drawing_what_a_throw_needs() {
        assert_eq!(chance_at_least(40, 0, 5, 0), 1.0);
        assert_eq!(chance_at_least(40, 1, 5, 2), 0.0);
        assert_eq!(chance_at_least(40, 9, 1, 2), 0.0);
        // P(at least 1 of 4 in 3 draws from 10) = 1 - C(6,3)/C(10,3) = 1 - 20/120.
        assert!((chance_at_least(10, 4, 3, 1) - (1.0 - 20.0 / 120.0)).abs() < 1e-12);
        let all: f64 = (0..=3).map(|k| chance_at_least(10, 4, 3, k) - chance_at_least(10, 4, 3, k + 1)).sum();
        assert!((all - 1.0).abs() < 1e-12);
    }

    /// In a round with discards the candidates are distinct throws of 1 to 5 cards in hand, the same for the same
    /// salt, none without a discard left; the block marks them and the free pick, each card column is the throw.
    #[test]
    fn discard_candidates_are_the_plans_throws() {
        let mut run = Run::new(crate::RunConfig::new("DCANDS1"));
        assert!(discard_candidates(&run, 3).is_empty());
        crate::env::apply(&mut run, crate::env::MoveKind::SelectBlind, 0).unwrap();
        let d = discard_candidates(&run, 3);
        assert!(!d.is_empty() && d.len() <= DISCARD_CANDS);
        let n = run.hand.len() as u32;
        for (i, x) in d.iter().enumerate() {
            assert!((1..=5).contains(&x.mask.count_ones()) && x.mask >> n == 0, "{:b}", x.mask);
            assert!(d[..i].iter().all(|y| y.mask != x.mask));
            assert!(x.outs <= x.pile && x.plan <= 4);
            assert!(crate::env::apply(&mut run.clone(), crate::env::MoveKind::Discard, x.mask).is_ok());
        }
        let again = discard_candidates(&run, 3);
        assert_eq!(d.iter().map(|x| x.mask).collect::<Vec<_>>(), again.iter().map(|x| x.mask).collect::<Vec<_>>());
        let mut out = vec![0.0f32; DISCARD_SLOTS * DCAND_FEATURES];
        write_discards(&mut out, &run, &d);
        for (k, x) in d.iter().enumerate() {
            let row = &out[k * DCAND_FEATURES..(k + 1) * DCAND_FEATURES];
            let bits = (0..crate::observe::MAX_HAND).fold(0u64, |m, i| m | (u64::from(row[DCAND_CARDS + i] > 0.5) << i));
            assert_eq!(bits, x.mask);
            assert_eq!(row[1..6].iter().sum::<f32>(), 1.0);
            assert!((0.0..=1.0).contains(&row[11]));
        }
        let free = &out[DISCARD_CANDS * DCAND_FEATURES..];
        assert!(free[0] == 1.0 && free[15] == 1.0);
        // No discard left: no candidates, and the block stays zero.
        run.discards_left = 0;
        assert!(discard_candidates(&run, 3).is_empty());
        let mut none = vec![0.0f32; DISCARD_SLOTS * DCAND_FEATURES];
        write_discards(&mut none, &run, &[]);
        assert!(none.iter().all(|&x| x == 0.0));
    }
}
