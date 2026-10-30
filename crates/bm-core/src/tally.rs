//! What a run did, move by move, for the live numbers a job shows (`make watch`): the jokers it took, packs
//! opened, rerolls, skips, vouchers, blinds beaten, the hands it played and its biggest one, and how it
//! ended. `env::Env` keeps one per run and hands over a `RunEnd` when the run finishes, so the network's
//! training can show the same breakdown as `examples/baseline.rs` (whose counting this matches).

use crate::blinds::BlindKind;
use crate::env::MoveKind;
use crate::joker_data::JokerKind;
use crate::packs::PackCard;
use crate::shop::RowItem;
use crate::Run;

/// What ends a run cut short by its player's move limit.
pub const CUT: &str = "Too many moves";

/// A run's counts so far.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Tally {
    /// Moves that went through.
    pub moves: u64,
    /// Moves the game refused (they change nothing).
    pub refused: u64,
    pub rerolls: u64,
    pub skips: u64,
    pub packs: u64,
    pub vouchers: u64,
    /// Blinds beaten (cash-outs).
    pub blinds_beaten: u64,
    /// Jokers bought from the shop or taken from packs, in order.
    pub bought: Vec<JokerKind>,
    /// Hands played, by `HandType` index.
    pub hands: [u64; 12],
    /// The biggest single hand.
    pub best_hand: f64,
}

/// A move about to be made, read before it changes the run: the joker it would take, the score before.
#[derive(Clone, Copy, Debug)]
pub struct Pending {
    kind: MoveKind,
    joker: Option<JokerKind>,
    chips_before: f64,
}

impl Tally {
    pub fn before(run: &Run, kind: MoveKind, arg: u64) -> Pending {
        let joker = match kind {
            MoveKind::Buy | MoveKind::BuyAndUse => run
                .shop
                .as_ref()
                .and_then(|sh| sh.row.get(arg as usize))
                .and_then(|slot| match &slot.item {
                    RowItem::Joker(j) => Some(j.kind),
                    _ => None,
                }),
            MoveKind::PackChoose => run
                .pack
                .as_ref()
                .and_then(|p| p.cards.get((arg & 0xFF) as usize))
                .and_then(|c| match c {
                    PackCard::Joker(j) => Some(j.kind),
                    _ => None,
                }),
            _ => None,
        };
        Pending {
            kind,
            joker,
            chips_before: run.round_chips,
        }
    }

    /// Count a move after it was tried; a refused one counts only as refused.
    pub fn after(&mut self, run: &Run, p: Pending, refused: bool) {
        if refused {
            self.refused += 1;
            return;
        }
        self.moves += 1;
        match p.kind {
            MoveKind::Buy | MoveKind::BuyAndUse | MoveKind::PackChoose => {
                self.bought.extend(p.joker)
            }
            MoveKind::Reroll => self.rerolls += 1,
            MoveKind::SkipBlind => self.skips += 1,
            MoveKind::OpenBooster => self.packs += 1,
            MoveKind::RedeemVoucher => self.vouchers += 1,
            MoveKind::CashOut => self.blinds_beaten += 1,
            MoveKind::Play => {
                if let Some(h) = run.last_hand {
                    self.hands[h as usize] += 1;
                }
                let scored = if run.round_chips >= p.chips_before {
                    run.round_chips - p.chips_before
                } else {
                    run.round_chips
                };
                self.best_hand = self.best_hand.max(scored);
            }
            _ => {}
        }
    }
}

/// How a run ended, with its counts.
#[derive(Clone, Debug, PartialEq)]
pub struct RunEnd {
    pub won: bool,
    /// The ante it ended in (1 to 8), 9 for a win.
    pub ante: u8,
    /// What ended it: the blind it lost to, `CUT` for a run cut short, "" for a win.
    pub lost_to: &'static str,
    pub money: i64,
    /// The jokers it held at the end.
    pub held: Vec<JokerKind>,
    pub tally: Tally,
}

impl RunEnd {
    /// A run that is over: won, or lost to the blind it was in.
    pub fn new(run: &Run, tally: Tally) -> RunEnd {
        let lost_to = if run.won {
            ""
        } else {
            run.blind.as_ref().map_or("unknown", |b| blind_name(b.kind))
        };
        RunEnd::with(run, tally, lost_to)
    }

    /// A run its player's move limit cut short, counted as lost.
    pub fn cut(run: &Run, tally: Tally) -> RunEnd {
        RunEnd::with(run, tally, CUT)
    }

    fn with(run: &Run, tally: Tally, lost_to: &'static str) -> RunEnd {
        let won = run.won && lost_to.is_empty();
        RunEnd {
            won,
            ante: if won { 9 } else { run.ante.clamp(1, 8) as u8 },
            lost_to,
            money: run.money,
            held: run.jokers.iter().map(|j| j.kind).collect(),
            tally,
        }
    }
}

pub fn blind_name(kind: BlindKind) -> &'static str {
    match kind {
        BlindKind::Small => "Small Blind",
        BlindKind::Big => "Big Blind",
        BlindKind::Boss(b) => b.name(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bots::greedy_move;
    use crate::env::{Env, MoveKind};
    use crate::Phase;

    /// Greedy runs through `Env`: its tally of each finished run matches the moves made, as
    /// `examples/baseline.rs` counts them.
    #[test]
    fn a_finished_run_reports_its_moves_and_ending() {
        let mut env = Env::new(7, 1);
        for game in 0..3 {
            let mut want = Tally::default();
            let mut kinds = Vec::new();
            let mut step = 0u64;
            let end = loop {
                let (k, a) = greedy_move(&env.run, game << 20 | step);
                let joker = Tally::before(&env.run, k, a).joker;
                let info = env.step(k as i64, a);
                assert!(!info.refused, "the greedy bot only makes legal moves");
                kinds.push(k);
                want.bought.extend(joker);
                step += 1;
                if info.done {
                    break env
                        .ended
                        .take()
                        .expect("a finished run hands over its ending");
                }
                assert!(env.ended.is_none());
                assert!(step < 5000, "a greedy run ends");
            };
            let count = |m: MoveKind| kinds.iter().filter(|&&k| k == m).count() as u64;
            assert_eq!(end.tally.moves, kinds.len() as u64);
            assert_eq!(end.tally.refused, 0);
            assert_eq!(end.tally.rerolls, count(MoveKind::Reroll));
            assert_eq!(end.tally.skips, count(MoveKind::SkipBlind));
            assert_eq!(end.tally.packs, count(MoveKind::OpenBooster));
            assert_eq!(end.tally.vouchers, count(MoveKind::RedeemVoucher));
            assert_eq!(end.tally.blinds_beaten, count(MoveKind::CashOut));
            assert_eq!(end.tally.hands.iter().sum::<u64>(), count(MoveKind::Play));
            assert_eq!(end.tally.bought, want.bought);
            assert!(end.tally.best_hand > 0.0);
            if end.won {
                assert_eq!((end.ante, end.lost_to), (9, ""));
            } else {
                assert!(
                    (1..=8).contains(&end.ante) && !end.lost_to.is_empty() && end.lost_to != CUT
                );
            }
            assert_eq!(
                env.tally,
                Tally::default(),
                "the next run starts with a clean tally"
            );
            assert_eq!(env.run.phase, Phase::BlindSelect);
        }
    }

    #[test]
    fn a_refused_move_counts_as_refused_only() {
        let mut env = Env::new(3, 1);
        let info = env.step(MoveKind::Play as i64, 0b1);
        assert!(info.refused, "no playing before a blind is chosen");
        let info = env.step(99, 0);
        assert!(info.refused, "no such kind of move");
        assert_eq!(
            env.tally,
            Tally {
                refused: 2,
                ..Tally::default()
            }
        );
        let end = RunEnd::cut(&env.run, std::mem::take(&mut env.tally));
        assert_eq!((end.won, end.ante, end.lost_to), (false, 1, CUT));
        assert_eq!(end.tally.refused, 2);
    }
}
