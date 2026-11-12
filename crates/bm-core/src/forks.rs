//! Forks: what the teacher's options at a decision lead to, found by playing each one out.
//!
//! A recording labels every position with how its run ended: one result for the two hundred decisions that
//! led to it, most of it luck, which says what a winning position looks like but never which of the options
//! here was the better one. A fork asks that directly. At a shop's or an open pack's first decision, the
//! options the teacher weighs (`teacher::candidates`: its best few by the quick value, and doing nothing; the
//! teacher's own choice and the move the run made, if they are others) are each applied to fair copies of the
//! run, and the teacher plays each copy out to the end of the run. The copies share their fresh seeds across
//! the options, so every option faces the same hidden futures as far as its play allows (a paired comparison:
//! far less noise than fresh seeds for each). What comes out, for each option and copy: the position the option
//! leaves (the value network's input, as `baseline --record` keeps positions) and how the play-out ended (won,
//! and the blinds behind the run when it ended). A network trained on them (`train_value.py --forks`) learns
//! to put the options here in the right order, which is the only thing the teacher asks of it.
//!
//! Nothing hidden is used: a fair copy's hidden parts are reshuffled (`observe::fair_copy`), and the forks are
//! training data, never the bot's play.

use crate::env::{apply, MoveKind};
use crate::learned::blinds_behind;
use crate::observe::{encode_value, fair_copy, screen, VALUE_LEN};
use crate::state::{Phase, Run};
use crate::teacher::{candidates, decision, new_run, reorders, teacher_move};

/// Moves a play-out may take before it is given up (as `baseline` caps a run).
pub const MAX_STEPS: u64 = 3000;

/// How a play-out ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Outcome {
    pub won: bool,
    /// Blinds behind the run when it ended: 24 for a win, else as `learned::blinds_behind` counts them.
    pub stage: u32,
}

impl Outcome {
    /// Better outcomes compare higher: a win over any loss, a loss by the blinds it got past.
    fn key(&self) -> u32 {
        if self.won {
            24
        } else {
            self.stage
        }
    }
}

/// One option at a fork: its moves, whose choice it was, and for each copy the position it left and how the
/// play-out from there ended.
#[derive(Clone, Debug)]
pub struct ForkOption {
    pub moves: Vec<(MoveKind, u64)>,
    /// The teacher's own choice here, decided now (with the options in force).
    pub chosen: bool,
    /// The move the run made here (a recording's), when the fork is on a replay.
    pub made: bool,
    /// The quick value the teacher's first look gave it (NaN for an option it didn't weigh).
    pub quick: f64,
    pub rows: Vec<Vec<f32>>,
    pub ends: Vec<Outcome>,
}

impl ForkOption {
    /// How many of its play-outs won.
    pub fn win_rate(&self) -> f64 {
        let n = self.ends.len().max(1) as f64;
        self.ends.iter().filter(|e| e.won).count() as f64 / n
    }
}

/// A decision forked: its options, each played out on the same `copies` fair copies, and where it was.
#[derive(Clone, Debug)]
pub struct Fork {
    pub options: Vec<ForkOption>,
    pub copies: usize,
    pub ante: i32,
    pub phase: Phase,
}

impl Fork {
    /// The option whose play-outs went best: most wins, then the furthest losses (the first on a tie).
    pub fn best(&self) -> usize {
        let score = |o: &ForkOption| o.ends.iter().map(|e| u64::from(e.key())).sum::<u64>();
        (0..self.options.len())
            .max_by(|&a, &b| {
                let (oa, ob) = (&self.options[a], &self.options[b]);
                oa.ends
                    .iter()
                    .filter(|e| e.won)
                    .count()
                    .cmp(&ob.ends.iter().filter(|e| e.won).count())
                    .then(score(oa).cmp(&score(ob)))
                    .then(b.cmp(&a))
            })
            .unwrap_or(0)
    }

    /// The teacher's own choice among the options, if it was one.
    pub fn chosen(&self) -> Option<usize> {
        self.options.iter().position(|o| o.chosen)
    }

    /// What the teacher's choice gave away: the best option's win rate less its own (0 when it chose best).
    pub fn regret(&self) -> Option<f64> {
        let c = self.chosen()?;
        Some(self.options[self.best()].win_rate() - self.options[c].win_rate())
    }

    /// How the options compare copy by copy: the share of copies where `a` ended better than `b` (half for alike),
    /// the pair's target for the network.
    pub fn share_better(&self, a: usize, b: usize) -> f64 {
        let (oa, ob) = (&self.options[a], &self.options[b]);
        let n = oa.ends.len().min(ob.ends.len()).max(1) as f64;
        oa.ends
            .iter()
            .zip(&ob.ends)
            .map(|(x, y)| match x.key().cmp(&y.key()) {
                std::cmp::Ordering::Greater => 1.0,
                std::cmp::Ordering::Equal => 0.5,
                std::cmp::Ordering::Less => 0.0,
            })
            .sum::<f64>()
            / n
    }
}

/// Teacher options the play-outs add to the ones in force (`set_play_options`: "light", say, for play-outs at
/// half the shop budget; every option of a fork gets the same continuation, so the comparison stays fair).
static PLAY_OPTIONS: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();

/// Set the options the play-outs add (once, before any fork). An unknown option is an error.
pub fn set_play_options(names: &[String]) -> Result<(), String> {
    let mut all = crate::teacher::options();
    all.extend(names.iter().cloned());
    crate::teacher::check_options(&all)?;
    PLAY_OPTIONS
        .set(all)
        .map_err(|_| "the play-outs' options are already set".to_string())
}

/// The teacher plays this run to its end (or `MAX_STEPS` moves), forgetting first what it remembered of the
/// run it was in, so a play-out is the same on any thread.
pub fn play_out(run: Run, salt: u64) -> Outcome {
    match PLAY_OPTIONS.get() {
        Some(names) => {
            let names: Vec<&str> = names.iter().map(String::as_str).collect();
            crate::teacher::with_options(&names, || play_out_plain(run, salt))
        }
        None => play_out_plain(run, salt),
    }
}

fn play_out_plain(run: Run, salt: u64) -> Outcome {
    play_out_watched(run, salt, None).0
}

/// A play-out that also says whether the run held joker `watch` when it first left the shop (or pack) it
/// started in, and when it ended.
fn play_out_watched(
    mut run: Run,
    salt: u64,
    watch: Option<crate::joker_data::JokerKind>,
) -> (Outcome, bool, bool) {
    new_run();
    let holds = |r: &Run| watch.is_some_and(|w| r.count_jokers(w) > 0);
    let mut kept_shop: Option<bool> = None;
    let mut step = 0u64;
    while run.phase != Phase::GameOver && !run.won && step < MAX_STEPS {
        if kept_shop.is_none() && !matches!(run.phase, Phase::Shop | Phase::Pack) {
            kept_shop = Some(holds(&run));
        }
        let (k, a) = teacher_move(&run, salt << 20 | step);
        if apply(&mut run, k, a).is_err() {
            break;
        }
        step += 1;
    }
    let out = Outcome {
        won: run.won,
        stage: if run.won {
            24
        } else {
            blinds_behind(&run) as u32
        },
    };
    let end = holds(&run);
    (out, kept_shop.unwrap_or(end), end)
}

/// How one line of a forced fork went on one copy: the play-out's outcome, and whether the watched joker was
/// held as the shop closed and at the end.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LineEnd {
    pub outcome: Outcome,
    pub kept_shop: bool,
    pub kept_end: bool,
}

/// A decision forked between lines named from outside (the forced-buy forks, lab #20): each line's moves applied
/// to the same `copies` fair copies, then the teacher plays each copy to the end, as `play_out` does (with the
/// play-outs' options). The copies' hidden futures and the play-outs' own salts are the same for every line, so
/// the lines are compared pair by pair. None if a line's moves can't be made on a copy (the first copy decides:
/// the copies differ only in what is hidden). `watch`: a joker whose keeping each play-out reports.
pub fn forced_fork(
    run: &Run,
    lines: &[Vec<(MoveKind, u64)>],
    watch: Option<crate::joker_data::JokerKind>,
    salt: u64,
    copies: usize,
) -> Option<Vec<Vec<LineEnd>>> {
    let mut out = vec![Vec::with_capacity(copies); lines.len()];
    for c in 0..copies {
        let base = fair_copy(run, salt ^ (0xF0_4000 + c as u64));
        for (i, moves) in lines.iter().enumerate() {
            let mut copy = base.clone();
            if moves.iter().any(|&(k, a)| apply(&mut copy, k, a).is_err()) {
                new_run();
                return None;
            }
            let ps = (salt << 8 | c as u64) & ((1 << 44) - 1);
            let (outcome, kept_shop, kept_end) = match PLAY_OPTIONS.get() {
                Some(names) => {
                    let names: Vec<&str> = names.iter().map(String::as_str).collect();
                    crate::teacher::with_options(&names, || play_out_watched(copy, ps, watch))
                }
                None => play_out_watched(copy, ps, watch),
            };
            out[i].push(LineEnd {
                outcome,
                kept_shop,
                kept_end,
            });
        }
    }
    new_run();
    Some(out)
}

/// Fork the decision here: the `best` options by the teacher's quick look, doing nothing, the teacher's own choice
/// and the move `made` (if others), each on `copies` fair copies played out by the teacher. None where the
/// teacher has no options to weigh (outside a shop or an open pack), or fewer than two options could be made.
/// `salt` tells the copies and the play-outs apart from other forks'; the copies' seeds are the same for every
/// option. A teacher's choice that only puts jokers in another order (on its way out of the shop) counts as doing
/// nothing.
pub fn fork(
    run: &Run,
    made: Option<(MoveKind, u64)>,
    salt: u64,
    best: usize,
    copies: usize,
) -> Option<Fork> {
    let mut lists = candidates(run, salt << 20, best);
    if lists.len() < 2 {
        return None;
    }
    let nothing = lists.len() - 1;
    let quick = quick_values(run, salt << 20, &lists);
    let own = decision(run, salt << 20).2;
    let own = if reorders(own.0) {
        lists[nothing][0]
    } else {
        own
    };
    let made = made.map(|m| if reorders(m.0) { lists[nothing][0] } else { m });
    for extra in [Some(own), made].into_iter().flatten() {
        if !lists.iter().any(|l| l[0] == extra) {
            lists.push(vec![extra]);
        }
    }
    let mut options = Vec::with_capacity(lists.len());
    let mut input = Vec::with_capacity(VALUE_LEN);
    for (i, moves) in lists.into_iter().enumerate() {
        let mut rows = Vec::with_capacity(copies);
        let mut ends = Vec::with_capacity(copies);
        for c in 0..copies {
            let mut copy = fair_copy(run, salt ^ (0xF0_4000 + c as u64));
            if moves.iter().any(|&(k, a)| apply(&mut copy, k, a).is_err()) {
                break;
            }
            encode_value(&screen(&copy), &mut input);
            rows.push(input.clone());
            ends.push(play_out(
                copy,
                (salt << 8 | (i as u64) << 4 | c as u64) & ((1 << 44) - 1),
            ));
        }
        if ends.len() < copies {
            continue;
        }
        options.push(ForkOption {
            chosen: moves[0] == own,
            made: made == Some(moves[0]),
            quick: quick.get(i).copied().unwrap_or(f64::NAN),
            moves,
            rows,
            ends,
        });
    }
    new_run();
    (options.len() >= 2).then_some(Fork {
        options,
        copies,
        ante: run.ante,
        phase: run.phase,
    })
}

/// The quick value the teacher's first look gives each option (its position after the moves on a fair copy), in
/// the options' order, as `candidates` sorted them.
fn quick_values(run: &Run, salt: u64, lists: &[Vec<(MoveKind, u64)>]) -> Vec<f64> {
    lists
        .iter()
        .map(|moves| {
            let mut c = fair_copy(run, salt ^ 0x5EED);
            if moves.iter().any(|&(k, a)| apply(&mut c, k, a).is_err()) {
                return f64::NAN;
            }
            crate::value::quick(&c, salt)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::RunConfig;

    /// A run played by the teacher to its first shop.
    fn at_first_shop(seed: &str) -> Run {
        new_run();
        let mut run = Run::new(RunConfig::new(seed));
        let mut step = 0;
        while run.phase != Phase::Shop && step < 400 {
            let (k, a) = teacher_move(&run, step);
            apply(&mut run, k, a).expect("the teacher's moves are legal");
            step += 1;
        }
        assert_eq!(run.phase, Phase::Shop, "{seed}: reached a shop");
        run
    }

    #[test]
    fn a_fork_plays_each_option_out_on_the_same_copies_and_says_which_went_best() {
        let run = crate::teacher::with_options(&["light"], || at_first_shop("FORK1"));
        let f = crate::teacher::with_options(&["light"], || fork(&run, None, 7, 2, 2))
            .expect("a shop has options");
        assert!(f.options.len() >= 2 && f.options.len() <= 4);
        assert_eq!((f.copies, f.ante, f.phase), (2, 1, Phase::Shop));
        for o in &f.options {
            assert_eq!((o.rows.len(), o.ends.len()), (2, 2));
            assert!(o.rows.iter().all(|r| r.len() == VALUE_LEN));
            assert!(o.ends.iter().all(|e| e.won == (e.stage == 24)));
        }
        // Doing nothing (leaving) is the last of the teacher's own candidates.
        let leave = f
            .options
            .iter()
            .position(|o| o.moves == [(MoveKind::LeaveShop, 0)])
            .expect("leaving is an option");
        assert!(f.options[leave].rows[0] != f.options[0].rows[0] || leave == 0);
        assert_eq!(f.options.iter().filter(|o| o.chosen).count(), 1);
        assert!(f.options.iter().all(|o| !o.made));
        let b = f.best();
        assert!(f
            .options
            .iter()
            .all(|o| o.win_rate() <= f.options[b].win_rate()));
        assert!(f.regret().is_some_and(|r| r >= 0.0));
        assert!((f.share_better(0, 1) + f.share_better(1, 0) - 1.0).abs() < 1e-12);
        // The same salt forks the same way.
        let g = crate::teacher::with_options(&["light"], || fork(&run, None, 7, 2, 2)).unwrap();
        assert_eq!(
            g.options.iter().map(|o| o.ends.clone()).collect::<Vec<_>>(),
            f.options.iter().map(|o| o.ends.clone()).collect::<Vec<_>>()
        );
    }

    #[test]
    fn the_move_the_run_made_is_an_option_too_and_is_marked() {
        let run = crate::teacher::with_options(&["light"], || at_first_shop("FORK2"));
        let f = crate::teacher::with_options(&["light"], || {
            fork(&run, Some((MoveKind::LeaveShop, 0)), 3, 1, 1)
        })
        .expect("a fork");
        let made: Vec<_> = f.options.iter().filter(|o| o.made).collect();
        assert_eq!(made.len(), 1);
        assert_eq!(made[0].moves, [(MoveKind::LeaveShop, 0)]);
    }

    #[test]
    fn a_forced_fork_plays_named_lines_on_shared_copies() {
        let run = crate::teacher::with_options(&["light"], || at_first_shop("FORK4"));
        let lines = vec![
            vec![(MoveKind::LeaveShop, 0)],
            vec![(MoveKind::LeaveShop, 0)],
        ];
        let ends =
            crate::teacher::with_options(&["light"], || forced_fork(&run, &lines, None, 5, 2))
                .expect("leaving can be made");
        assert_eq!(ends.len(), 2);
        assert!(ends.iter().all(|l| l.len() == 2));
        // The same line on the same copies and salts plays out the same.
        assert_eq!(ends[0], ends[1]);
        // A line that can't be made gives no fork.
        let bad = vec![vec![(MoveKind::LeaveShop, 0)], vec![(MoveKind::Buy, 99)]];
        assert!(
            crate::teacher::with_options(&["light"], || forced_fork(&run, &bad, None, 5, 1))
                .is_none()
        );
    }

    #[test]
    fn no_fork_outside_a_shop_or_a_pack() {
        let run = Run::new(RunConfig::new("FORK3"));
        assert_eq!(run.phase, Phase::BlindSelect);
        assert!(fork(&run, None, 1, 2, 1).is_none());
    }
}
