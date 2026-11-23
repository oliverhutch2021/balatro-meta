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
}
