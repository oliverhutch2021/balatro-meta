//! The multiplier table (S, lab #33, 18 and 20 November): what one change to a build does to its scoring power.
//!
//! A build (a run at the first blind choice of an ante) is copied, changed once, and measured two ways against the
//! build as it stands: `power`, the value's own measure (the mean log total of the next round played out with no
//! target, `value::outlook`, the figure `--calibrate` records as `at_start.power`) and the same against this ante's
//! boss with its effect (`boss_power`); and the bench (`bench::ante_score`, the ante's three blinds with the real
//! boss). Every variant is read on the same salt, so the differences are not luck. The run itself is never changed:
//! a diagnostic only, never play or training.
//!
//! The changes are S's list: the focus hand three and five levels up (and the hand that scores the most, where it
//! differs); the deck made one suit, and that with its Flush three levels up; one rank doubled to eight copies (the
//! deck's size kept); thinned to 40 by the worst cards; the best card, and the best four, given a Red Seal, made Glass,
//! made Steel; each retrigger joker, Blueprint and Brainstorm in place of the weakest joker (the one whose loss costs
//! least power; added if a slot is free); the best common, uncommon and rare joker for the build in that place; one
//! more hand, discard, card in hand; and the best pair of the single changes.

use crate::blinds::{blind_target, scaling_for_stake, BlindKind};
use crate::card::{Enhancement, Rank, Seal, Suit};
use crate::joker_data::JokerKind;
use crate::poker::HandType;
use crate::state::Run;

/// Deals behind a power read (as `--calibrate`), and behind a joker search's screening read.
pub const SAMPLES: usize = 12;
const SCREEN_SAMPLES: usize = 6;

/// A change: its name, its group (two changes of one group never pair), and what it does to a copy.
struct Change {
    name: String,
    group: &'static str,
    apply: Box<dyn Fn(&mut Run) + Sync>,
}

fn change(name: impl Into<String>, group: &'static str, apply: impl Fn(&mut Run) + Sync + 'static) -> Change {
    Change { name: name.into(), group, apply: Box::new(apply) }
}

/// The value's power read: (next round, this ante's boss with its effect), each a mean log total.
pub fn power(run: &Run, salt: u64, samples: usize) -> Option<(f64, f64)> {
    let o = crate::value::outlook(run, salt, samples)?;
    let m = |v: &[f64]| if v.is_empty() { f64::NAN } else { v.iter().sum::<f64>() / v.len() as f64 };
    Some((m(&o.next), m(&o.boss)))
}

/// Put the jokers in the order the teacher would hold them (`teacher::joker_order`).
fn order_jokers(run: &mut Run) {
    let order = crate::teacher::joker_order(run);
    let ids: Vec<u64> = order.iter().map(|&i| run.jokers[i].sort_id).collect();
    for (to, id) in ids.iter().enumerate() {
        if let Some(from) = run.jokers.iter().position(|j| j.sort_id == *id) {
            if from != to {
                let _ = run.move_joker(from, to);
            }
        }
    }
}

/// Put `kind` in place of the joker at `weakest` (or in a free slot), then order the row. A copying joker (Blueprint,
/// Brainstorm) copies by where it stands, so it is tried in every place and left where the power is highest.
fn swap_in(run: &mut Run, weakest: Option<usize>, kind: JokerKind, salt: u64) {
    if (run.jokers.len() as i32) >= run.joker_slots {
        if let Some(i) = weakest {
            run.remove_joker(i);
        }
    }
    run.add_joker(kind);
    order_jokers(run);
    if matches!(kind, JokerKind::Blueprint | JokerKind::Brainstorm) {
        let Some(at) = run.jokers.iter().position(|j| j.kind == kind) else { return };
        let best = (0..run.jokers.len())
            .filter_map(|to| {
                let mut c = run.clone();
                c.move_joker(at, to).ok()?;
                power(&c, salt, SCREEN_SAMPLES).map(|p| (to, p.0))
            })
            .max_by(|a, b| a.1.total_cmp(&b.1));
        if let Some((to, _)) = best {
            let _ = run.move_joker(at, to);
        }
    }
}

/// The joker whose loss costs the least power (None with no jokers).
fn weakest_joker(run: &Run, salt: u64) -> Option<usize> {
    (0..run.jokers.len())
        .filter_map(|i| {
            let mut c = run.clone();
            c.remove_joker(i);
            power(&c, salt, SCREEN_SAMPLES).map(|p| (i, p.0))
        })
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .map(|x| x.0)
}

/// The deck's cards by worth, best first (Stone cards last).
fn by_worth(run: &Run) -> Vec<crate::state::CardId> {
    let mut ids = run.playing_cards.clone();
    ids.sort_by(|&a, &b| crate::value::card_value(run, b).total_cmp(&crate::value::card_value(run, a)));
    ids
}

fn most_common_suit(run: &Run) -> Suit {
    let mut n = [0usize; 4];
    for &id in &run.playing_cards {
        let c = run.card(id);
        if c.enhancement != Enhancement::Stone {
            n[c.suit as usize] += 1;
        }
    }
    let i = (0..4).max_by_key(|&i| (n[i], std::cmp::Reverse(i))).unwrap_or(0);
    [Suit::Spades, Suit::Hearts, Suit::Clubs, Suit::Diamonds][i]
}

/// The rank held most (the higher on a tie).
fn most_common_rank(run: &Run) -> Rank {
    let mut n = [0usize; 15];
    for &id in &run.playing_cards {
        let c = run.card(id);
        if c.enhancement != Enhancement::Stone {
            n[c.rank.id() as usize] += 1;
        }
    }
    let r = (2..=14usize).max_by_key(|&r| (n[r], r)).unwrap_or(14);
    Rank::ALL[r - 2]
}

fn remove_card(run: &mut Run, id: crate::state::CardId) {
    run.playing_cards.retain(|&x| x != id);
    run.deck.retain(|&x| x != id);
    run.hand.retain(|&x| x != id);
    run.discard.retain(|&x| x != id);
}

/// The single changes for this build (some found by search: the weakest joker, the best of each rarity).
fn changes(run: &Run, salt: u64) -> Vec<Change> {
    let mut out = Vec::new();
    // Hand levels: the focus hand (the bot's own, `teacher::focus_hand`), else the hand that scores the most; and the
    // scoring hand separately where it differs (S: both columns).
    let scoring = crate::value::outlook(run, salt, SCREEN_SAMPLES).map(|o| o.main);
    let focus = crate::teacher::focus_hand(run).or(scoring);
    if let Some(h) = focus {
        for n in [3, 5] {
            out.push(change(format!("levels+{n}"), "levels", move |r: &mut Run| r.hand_levels.level_up(h, n)));
        }
    }
    if let Some(h) = scoring.filter(|&s| Some(s) != focus) {
        out.push(change("scoring-levels+3", "levels", move |r: &mut Run| r.hand_levels.level_up(h, 3)));
    }
    // The deck.
    let suit = most_common_suit(run);
    let one_suit = move |r: &mut Run| {
        for id in r.playing_cards.clone() {
            let c = r.card_mut(id);
            if c.enhancement != Enhancement::Stone {
                c.suit = suit;
            }
        }
    };
    out.push(change("one-suit", "suit", one_suit));
    out.push(change("one-suit+flush+3", "suit", move |r: &mut Run| {
        one_suit(r);
        r.hand_levels.level_up(HandType::Flush, 3);
    }));
    let rank = most_common_rank(run);
    out.push(change("rank-x8", "deck", move |r: &mut Run| {
        let have = r.playing_cards.iter().filter(|&&id| r.card(id).rank == rank).count();
        let mut low: Vec<_> = r
            .playing_cards
            .iter()
            .copied()
            .filter(|&id| r.card(id).rank != rank && r.card(id).enhancement != Enhancement::Stone)
            .collect();
        low.sort_by_key(|&id| r.card(id).rank.id());
        for id in low.into_iter().take(8usize.saturating_sub(have)) {
            r.card_mut(id).rank = rank;
        }
    }));
    out.push(change("thin-to-40", "deck", |r: &mut Run| {
        let ids = by_worth(r);
        for &id in ids.iter().skip(40) {
            remove_card(r, id);
        }
    }));
    // The best card, and the best four.
    for (name, group, f) in [
        ("red-seal", "seal", (|c: &mut crate::card::Card| c.seal = Seal::Red) as fn(&mut crate::card::Card)),
        ("glass", "enh", |c: &mut crate::card::Card| c.enhancement = Enhancement::Glass),
        ("steel", "enh", |c: &mut crate::card::Card| c.enhancement = Enhancement::Steel),
    ] {
        for n in [1usize, 4] {
            let label = if n == 1 { format!("best-card-{name}") } else { format!("best-4-{name}") };
            out.push(change(label, group, move |r: &mut Run| {
                for id in by_worth(r).into_iter().take(n) {
                    f(r.card_mut(id));
                }
            }));
        }
    }
    // Jokers in place of the weakest.
    let weakest = weakest_joker(run, salt);
    use JokerKind as J;
    for k in [J::HangingChad, J::SockAndBuskin, J::Dusk, J::Mime, J::Hack, J::Seltzer, J::Blueprint, J::Brainstorm] {
        if !run.jokers.iter().any(|j| j.kind == k) {
            out.push(change(format!("joker:{}", k.name()), "joker", move |r: &mut Run| swap_in(r, weakest, k, salt)));
        }
    }
    for (rarity, label) in [(1u8, "best-common"), (2, "best-uncommon"), (3, "best-rare")] {
        let best = JokerKind::ALL
            .iter()
            .copied()
            .filter(|k| k.rarity() == rarity && !run.jokers.iter().any(|j| j.kind == *k))
            .filter_map(|k| {
                let mut c = run.clone();
                swap_in(&mut c, weakest, k, salt);
                power(&c, salt, SCREEN_SAMPLES).map(|p| (k, p.0))
            })
            .max_by(|a, b| a.1.total_cmp(&b.1));
        if let Some((k, _)) = best {
            out.push(change(format!("{label}:{}", k.name()), "joker", move |r: &mut Run| swap_in(r, weakest, k, salt)));
        }
    }
    // Hands, discards, hand size.
    out.push(change("hand+1", "hands", |r: &mut Run| r.hands_per_round += 1));
    out.push(change("discard+1", "discards", |r: &mut Run| r.discards_per_round += 1));
    out.push(change("hand-size+1", "size", |r: &mut Run| r.hand_size_real += 1));
    out
}

/// One build's line: the build as it stands and after each change, by power (next round, boss) and the bench
/// ([small, big, boss, ante]), the asks of the ante's three blinds (ln), and the best pair. None if the run is over.
pub fn line(run: &Run, salt: u64, bench_tries: u64) -> Option<String> {
    let ante = run.ante;
    let read = |r: &Run| -> Option<(String, f64)> {
        let (p, pb) = power(r, salt, SAMPLES)?;
        let b = crate::bench::ante_score(r, ante, Some(run.boss), bench_tries, salt ^ 0xBE4C)?;
        Some((
            format!(
                "{{\"power\":{p:.4},\"boss_power\":{pb:.4},\"bench\":[{:.4},{:.4},{:.4},{:.4}]}}",
                b[0], b[1], b[2], b[3]
            ),
            p,
        ))
    };
    let before = read(run)?.0;
    let changes = changes(run, salt);
    let mut singles: Vec<(usize, f64)> = Vec::new();
    let mut after = Vec::new();
    for (i, ch) in changes.iter().enumerate() {
        let mut c = run.clone();
        (ch.apply)(&mut c);
        if let Some((rec, p)) = read(&c) {
            singles.push((i, p));
            after.push(format!("\"{}\":{rec}", ch.name));
        }
    }
    // The best pair: the two best single changes by power, of different groups.
    singles.sort_by(|a, b| b.1.total_cmp(&a.1));
    let pair = singles.first().and_then(|&(a, _)| {
        singles.iter().find(|&&(b, _)| changes[b].group != changes[a].group).map(|&(b, _)| (a, b))
    });
    if let Some((a, b)) = pair {
        let mut c = run.clone();
        (changes[a].apply)(&mut c);
        (changes[b].apply)(&mut c);
        if let Some((rec, _)) = read(&c) {
            after.push(format!("\"pair:{}+{}\":{rec}", changes[a].name, changes[b].name));
        }
    }
    let scaling = scaling_for_stake(run.cfg.stake);
    let ask = |k: BlindKind| blind_target(ante, scaling, k, 1.0).max(1.0).ln();
    let jokers: Vec<String> = run.jokers.iter().map(|j| format!("\"{}\"", j.kind.name())).collect();
    Some(format!(
        "{{\"ante\":{ante},\"boss\":\"{}\",\"focus\":{},\"ask\":[{:.4},{:.4},{:.4}],\"jokers\":[{}],\"deck\":{},\
         \"before\":{before},\"after\":{{{}}}}}",
        run.boss.name(),
        crate::teacher::focus_hand(run).map_or("null".to_string(), |h| format!("\"{}\"", h.name())),
        ask(BlindKind::Small),
        ask(BlindKind::Big),
        ask(BlindKind::Boss(run.boss)),
        jokers.join(","),
        run.playing_cards.len(),
        after.join(",")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A greedy run at ante 2's first blind choice (the first of these seeds whose run gets there).
    fn at_blind_choice(seed: &str) -> Run {
        for k in 0..20 {
            let mut run = Run::new(crate::RunConfig::new(&format!("{seed}{k}")));
            let mut step = 0;
            while !(run.phase == crate::state::Phase::BlindSelect && run.ante == 2)
                && run.phase != crate::state::Phase::GameOver
                && step < 600
            {
                let m = crate::bots::greedy_move(&run, step);
                if crate::env::apply(&mut run, m.0, m.1).is_err() {
                    break;
                }
                step += 1;
            }
            if run.phase == crate::state::Phase::BlindSelect && run.ante == 2 {
                return run;
            }
        }
        panic!("no greedy run reached ante 2")
    }

    /// Each change changes what it names and nothing of the run itself; the deck's size is kept by "rank-x8".
    #[test]
    fn the_changes_do_what_they_name_on_a_copy() {
        let run = at_blind_choice("MULTA");
        assert_eq!(run.phase, crate::state::Phase::BlindSelect);
        let chs = changes(&run, 7);
        let get = |name: &str| chs.iter().find(|c| c.name == name).unwrap_or_else(|| panic!("{name}"));
        let mut c = run.clone();
        (get("one-suit").apply)(&mut c);
        let suits: std::collections::HashSet<_> = c
            .playing_cards
            .iter()
            .filter(|&&id| c.card(id).enhancement != Enhancement::Stone)
            .map(|&id| c.card(id).suit as usize)
            .collect();
        assert_eq!(suits.len(), 1);
        let mut c = run.clone();
        (get("rank-x8").apply)(&mut c);
        assert_eq!(c.playing_cards.len(), run.playing_cards.len(), "the deck's size kept");
        let r = most_common_rank(&run);
        assert!(c.playing_cards.iter().filter(|&&id| c.card(id).rank == r).count() >= 8);
        let mut c = run.clone();
        (get("thin-to-40").apply)(&mut c);
        assert_eq!(c.playing_cards.len(), 40);
        assert_eq!(c.deck.len(), 40);
        let mut c = run.clone();
        (get("hand-size+1").apply)(&mut c);
        assert_eq!(c.hand_size(), run.hand_size() + 1);
        let mut c = run.clone();
        (get("joker:Blueprint").apply)(&mut c);
        assert!(c.jokers.iter().any(|j| j.kind == JokerKind::Blueprint));
        assert!(c.jokers.len() as i32 <= c.joker_slots);
        assert!(chs.iter().any(|c| c.name.starts_with("best-rare:")));
    }

    /// A line reads: the build before, each change after, the asks, the best pair.
    #[test]
    fn a_line_has_before_after_and_the_asks() {
        let run = at_blind_choice("MULTB");
        let l = line(&run, 3, 2).expect("a line");
        for key in ["\"before\":{\"power\":", "\"hand+1\":{\"power\":", "\"ask\":[", "\"pair:", "\"one-suit+flush+3\""] {
            assert!(l.contains(key), "{key} in {l}");
        }
        assert_eq!(l.matches('{').count(), l.matches('}').count());
    }
}
