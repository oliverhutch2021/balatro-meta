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
}
