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

/// Tries per plan. More is steadier and slower (`value::Scales::rounds`).
pub const TRIES: u64 = 10;

fn try_count(n: u64) -> u64 {
    crate::value::scale(n as usize, crate::value::scales().rounds) as u64
}
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
pub(crate) struct Seen {
    rank: u8,
    suit: Suit,
    stone: bool,
    wild: bool,
}

pub(crate) fn seen(run: &Run) -> Vec<Seen> {
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
    pub(crate) fn keep(self, hand: &[Seen]) -> u64 {
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
    pub(crate) fn options(hand: &[Seen]) -> Vec<Plan> {
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

/// The plans this hand allows, most promising first for this run: by how far each shape is built (as
/// `Plan::options`), and by what the jokers pay for in the cards it keeps (a flush in Ancient Joker's or
/// Bloodstone's suit, Kings with Baron, copies of The Idol's card...). For the value's quick players, so they
/// chase what the run's jokers pay for as a player would.
pub(crate) fn options_for(run: &Run) -> Vec<Plan> {
    let hand = seen(run);
    let mut plans = Plan::options(&hand);
    let built = |p: Plan| -> f64 {
        let n = p.keep(&hand).count_ones() as f64;
        match p {
            Plan::Best => 0.0,
            Plan::Flush(_) | Plan::Straight(_) => 2.0 * n,
            Plan::Kind(_) => 2.0 * n + 1.0,
            Plan::House(..) => 9.0,
        }
    };
    let pays = |p: Plan| -> f64 {
        if p == Plan::Best {
            return 0.0;
        }
        let keep = p.keep(&hand);
        (0..run.hand.len())
            .filter(|&i| keep >> i & 1 == 1)
            .map(|i| crate::value::joker_bonus(run, run.card(run.hand[i])) / 8.0)
            .sum()
    };
    let mut scored: Vec<(f64, Plan)> = plans.drain(..).map(|p| (built(p) + pays(p), p)).collect();
    scored.sort_by(|a, b| b.0.total_cmp(&a.0));
    scored.into_iter().map(|(_, p)| p).collect()
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
    // The Psychic scores only five-card hands: pad each with the lowest other cards.
    let psychic = run
        .blind
        .as_ref()
        .is_some_and(|b| b.active_boss() == Some(crate::blinds::Boss::Psychic));
    if psychic && n >= 5 {
        let mut low: Vec<usize> = (0..n).collect();
        low.sort_by_key(|&i| hand[i].rank);
        for c in &mut cands {
            for &i in &low {
                if c.count_ones() >= 5 {
                    break;
                }
                *c |= 1 << i;
            }
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
pub(crate) fn throw(run: &Run, hand: &[Seen], keep: u64) -> u64 {
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
pub(crate) fn plan_move(run: &Run, plan: Plan, best: (f64, u64)) -> (MoveKind, u64) {
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

/// How a try plays the rest of the round: towards a shape, or (the blind already beatable) taking value
/// first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Policy {
    Plan(Plan),
    Farm(Farm),
}

/// Ways to finish a round whose blind the hand in hand already beats.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Farm {
    /// Play the winning hand now (`four`: padded with spare cards to exactly four, for Square Joker).
    Win { four: bool },
    /// Throw spare cards (outside the winning hand) for what throwing them pays, while discards last, then
    /// win (`single`: one card on its own first, for Trading Card).
    Throw { single: bool },
    /// Play spare cards as extra hands while more than one hand is left (`cards` at a time: one for Sixth
    /// Sense or DNA, four for Square Joker, five to draw the most), then win.
    Extra { cards: u8 },
}

fn policy_move(run: &Run, policy: Policy) -> (MoveKind, u64) {
    match policy {
        Policy::Plan(p) => plan_move(run, p, quick_best(run)),
        Policy::Farm(f) => farm_move(run, f),
    }
}

/// One try of `policy` from `first` on a fair copy: (beat the blind, how close it got, what the round added
/// besides, when `reference` is given: see `value::round_gain`).
fn try_policy(
    run: &Run,
    policy: Policy,
    first: (MoveKind, u64),
    salt: u64,
    reference: Option<(f64, f64)>,
) -> (bool, f64, f64) {
    let target = run.blind.as_ref().map_or(1.0, |b| b.chips).max(1.0);
    let mut c = fair_copy(run, salt);
    // The first move is an extra hand when it is one of the policy's from this hand.
    let first_extra =
        matches!(policy, Policy::Farm(Farm::Extra { .. })) && policy_step(&c, policy).1;
    let ok = if first_extra {
        play_extra(&mut c, first)
    } else {
        apply(&mut c, first.0, first.1).is_ok()
    };
    if !ok {
        return (false, 0.0, 0.0);
    }
    let mut steps = 0;
    while c.phase == Phase::Round && steps < 40 {
        let (m, extra) = policy_step(&c, policy);
        let ok = if extra {
            play_extra(&mut c, m)
        } else {
            apply(&mut c, m.0, m.1).is_ok()
        };
        if !ok {
            break;
        }
        steps += 1;
    }
    let beat = c.phase == Phase::RoundEval || c.won;
    let close = if beat {
        1.0
    } else {
        (c.round_chips / target).min(1.0)
    };
    let gain = match reference {
        Some(r) if beat => {
            if c.phase == Phase::RoundEval {
                let _ = apply(&mut c, MoveKind::CashOut, 0);
            }
            crate::value::round_gain(run, &c, r)
        }
        _ => 0.0,
    };
    (beat, close, gain)
}

/// Whether rounds are judged by what they add as well as by beating the blind (the value teacher).
fn value_aware() -> bool {
    crate::teacher::by_value()
}

/// Chips and mult of a hand, to weigh what a round adds against (on a fair copy: chance effects must not
/// peek at the run's streams).
fn reference(run: &Run, mask: u64, salt: u64) -> (f64, f64) {
    let h = crate::bots::preview(&fair_copy(run, salt ^ 0x2EF), mask);
    (h.chips.max(1.0), h.mult.max(1.0))
}

/// The planner's move for a round where the best hand now doesn't beat the blind: the move of the plan that
/// beats it most often on fair copies (the value teacher: among plans that beat it equally often, the one
/// that adds the most to the run). `best` is the best hand now (from a full search).
pub fn plan(run: &Run, best: (f64, u64), salt: u64) -> (MoveKind, u64) {
    if run.forced_card().is_some() {
        return (MoveKind::Play, best.1); // Cerulean Bell: every selection holds its card; keep it simple
    }
    let reference = value_aware().then(|| reference(run, best.1, salt));
    let hand = seen(run);
    let mut choice: Option<((u64, f64), (MoveKind, u64))> = None;
    // Each plan from its own first move; with DNA on the first hand, the best-hand plan after copying the
    // most valuable card too (a hand played for DNA, judged like the rest).
    let mut tries: Vec<(Plan, Option<(MoveKind, u64)>)> = Plan::options(&hand)
        .into_iter()
        .map(|p| (p, None))
        .collect();
    let dna = run.hands_played_round == 0
        && run.hands_left > 1
        && run
            .jokers
            .iter()
            .any(|j| j.kind == crate::joker_data::JokerKind::DNA && !j.debuffed);
    if dna && value_aware() {
        if let Some(i) = dna_pick(run) {
            tries.push((Plan::Best, Some((MoveKind::Play, 1 << i))));
        }
    }
    for (n, (p, own)) in tries.into_iter().enumerate() {
        let first = own.unwrap_or_else(|| plan_move(run, p, best));
        if choice.as_ref().is_some_and(|(_, m)| *m == first) {
            continue; // the same move now as a better-scoring plan
        }
        let (mut beats, mut close, mut gain) = (0u64, 0.0, 0.0);
        for t in 0..try_count(TRIES) {
            let (beat, c, g) = try_policy(
                run,
                Policy::Plan(p),
                first,
                salt.wrapping_mul(0x9E37).wrapping_add((n as u64) << 32 | t),
                reference,
            );
            beats += u64::from(beat);
            close += c;
            gain += g;
        }
        // Beating the blind first; then what the round adds (when it is beaten at all), else how close.
        let key = (
            beats,
            if beats > 0 && reference.is_some() {
                gain / beats as f64
            } else {
                close
            },
        );
        if choice
            .as_ref()
            .is_none_or(|(k, _)| key.0 > k.0 || (key.0 == k.0 && key.1 > k.1))
        {
            choice = Some((key, first));
        }
    }
    choice.map_or((MoveKind::Play, best.1), |(_, m)| m)
}

/// How often the best-hand plan beats the blind from here, on fair copies (a quick read of a round's
/// difficulty).
pub fn beat_rate(run: &Run, salt: u64) -> f64 {
    beat_rate_with(run, salt, 6)
}

/// `beat_rate` from `n` tries (more for a steadier estimate).
pub fn beat_rate_with(run: &Run, salt: u64, n: u64) -> f64 {
    let fair = fair_copy(run, salt ^ 0xBEA7);
    let best = quick_best(&fair);
    let first = plan_move(&fair, Plan::Best, best);
    let n = n.max(1);
    let beats = (0..n)
        .filter(|&t| {
            try_policy(
                run,
                Policy::Plan(Plan::Best),
                first,
                salt.wrapping_add(0xB0 + t),
                None,
            )
            .0
        })
        .count();
    beats as f64 / n as f64
}

/// Tries per way of finishing a beatable round.
const FARM_TRIES: u64 = 6;
/// The least gain worth taking value before winning (below it, estimates differ by noise).
const FARM_MARGIN: f64 = 0.005;

/// Spare cards ordered for throwing: the ones throwing pays for first (Mail-In Rebate's rank, Purple Seals,
/// Jacks for Hit the Road, Castle's suit, face cards for Faceless Joker when three or more can go at once),
/// the ones worth more kept in hand last (Steel and Gold cards, Gold and Blue Seals, face cards for Reserved
/// Parking, Kings for Baron, Queens for Shoot the Moon), lowest rank first otherwise.
fn throw_order(run: &Run, spare: &[usize]) -> Vec<usize> {
    use crate::card::Seal;
    use crate::joker_data::JokerKind as J;
    let has = |k: J| run.count_jokers(k) > 0;
    let (mail, road, castle) = (has(J::MailInRebate), has(J::HitTheRoad), has(J::Castle));
    let (parking, baron, moon) = (has(J::ReservedParking), has(J::Baron), has(J::ShootTheMoon));
    let faces = spare
        .iter()
        .filter(|&&i| run.card(run.hand[i]).rank.is_face())
        .count();
    let faceless = has(J::FacelessJoker) && faces >= 3;
    let mut v = spare.to_vec();
    v.sort_by_key(|&i| {
        let c = run.card(run.hand[i]);
        let pays = (mail && c.rank == run.mail_rank)
            || c.seal == Seal::Purple
            || (road && c.rank == Rank::Jack)
            || (castle && c.suit == run.castle_suit)
            || (faceless && c.rank.is_face());
        let keep = matches!(c.enhancement, Enhancement::Steel | Enhancement::Gold)
            || matches!(c.seal, Seal::Blue | Seal::Gold)
            || (parking && c.rank.is_face())
            || (baron && c.rank == Rank::King)
            || (moon && c.rank == Rank::Queen);
        // Within each group, the cards worth least to this run first (jokers' builds included).
        let worth = (crate::value::card_value(run, run.hand[i]) * 10.0) as i64;
        (
            if pays {
                0
            } else if keep {
                2
            } else {
                1
            },
            worth,
        )
    });
    v
}

/// Spare cards to play as an extra hand: one card (a 6 for Sixth Sense, the best card for DNA, a Gold
/// Seal card that pays when it scores), or the lowest `cards` (the ones worth keeping in hand last).
fn extra_play(run: &Run, spare: &[usize], cards: usize) -> u64 {
    use crate::joker_data::JokerKind as J;
    let order = throw_order(run, spare);
    if cards == 1 {
        let first_hand = run.hands_played_round == 0;
        let gold = || {
            spare
                .iter()
                .copied()
                .find(|&i| run.card(run.hand[i]).seal == crate::card::Seal::Gold)
        };
        let pick = if first_hand && run.count_jokers(J::SixthSense) > 0 {
            spare
                .iter()
                .copied()
                .find(|&i| run.card(run.hand[i]).rank == Rank::Six)
                .or_else(gold)
        } else if first_hand && run.count_jokers(J::DNA) > 0 {
            dna_pick(run)
        } else {
            gold()
        };
        return 1 << pick.unwrap_or(order[0]);
    }
    order.into_iter().take(cards).fold(0u64, |m, i| m | 1 << i)
}

/// The card DNA should copy: the most valuable in hand to this run (its copy comes back to the hand, so a
/// winning hand keeps it).
fn dna_pick(run: &Run) -> Option<usize> {
    (0..run.hand.len())
        .filter(|&i| !run.card(run.hand[i]).face_down)
        .max_by(|&a, &b| {
            crate::value::card_value(run, run.hand[a])
                .total_cmp(&crate::value::card_value(run, run.hand[b]))
                .then(b.cmp(&a))
        })
}

/// The move of a way to finish a beatable round. If the hand no longer beats the blind (a boss took a card),
/// it plays to beat it like the planner's best-hand plan.
pub(crate) fn farm_move(run: &Run, farm: Farm) -> (MoveKind, u64) {
    farm_step(run, farm).0
}

/// `farm_move`, and whether the move is an extra hand (spare cards played, the round to go on).
fn farm_step(run: &Run, farm: Farm) -> ((MoveKind, u64), bool) {
    let plain = |m: (MoveKind, u64)| (m, false);
    let best = quick_best(run);
    let target = run.blind.as_ref().map_or(0.0, |b| b.chips);
    if best.0 < target - run.round_chips {
        return plain(plan_move(run, Plan::Best, best));
    }
    let keep = best.1;
    let spare: Vec<usize> = (0..run.hand.len())
        .filter(|&i| keep >> i & 1 == 0)
        .collect();
    match farm {
        Farm::Win { four } => {
            if four && keep.count_ones() < 4 && spare.len() + (keep.count_ones() as usize) >= 4 {
                let need = 4 - keep.count_ones() as usize;
                let padded = throw_order(run, &spare)
                    .into_iter()
                    .take(need)
                    .fold(keep, |m, i| m | 1 << i);
                if preview_score(run, padded) >= target - run.round_chips {
                    return plain((MoveKind::Play, padded));
                }
            }
            plain((MoveKind::Play, keep))
        }
        Farm::Throw { single } => {
            let first_throw = run.discards_used_round == 0;
            if run.discards_left > 0 && !spare.is_empty() && (!single || first_throw) {
                let n = if single { 1 } else { 5 };
                let out = throw_order(run, &spare)
                    .into_iter()
                    .take(n)
                    .fold(0u64, |m, i| m | 1 << i);
                plain((MoveKind::Discard, out))
            } else {
                plain((MoveKind::Play, keep))
            }
        }
        Farm::Extra { cards } => {
            let c = cards as usize;
            let gold_left = spare
                .iter()
                .any(|&i| run.card(run.hand[i]).seal == crate::card::Seal::Gold);
            let single_done = c == 1 && run.hands_played_round > 0 && !gold_left;
            if run.hands_left > 1 && spare.len() >= c && !single_done {
                ((MoveKind::Play, extra_play(run, &spare, c)), true)
            } else {
                plain((MoveKind::Play, keep))
            }
        }
    }
}

/// The value teacher's move when the hand in hand beats the blind: win now, or take value first (throwing
/// spare cards for what that pays, playing spare cards to grow jokers or for Sixth Sense or DNA, a fourth
/// card for Square Joker), whichever adds the most to the run on fair copies, among ways that still beat
/// the blind every time.
pub fn finish(run: &Run, best: (f64, u64), salt: u64) -> (MoveKind, u64) {
    finish_choice(run, best, salt).0
}

/// `finish`'s move, and whether it is an extra hand (played for what playing it gives, the round to go on).
pub(crate) fn finish_choice(run: &Run, best: (f64, u64), salt: u64) -> ((MoveKind, u64), bool) {
    use crate::joker_data::JokerKind as J;
    if run.forced_card().is_some() {
        return ((MoveKind::Play, best.1), false);
    }
    let has = |k: J| run.count_jokers(k) > 0;
    let reference = reference(run, best.1, salt);
    // Moves are worked out on a fair copy (the same cards in hand): never on the run's own streams.
    let fair = fair_copy(run, salt ^ 0xF00D);
    let mut options = vec![Farm::Win { four: false }];
    if has(J::SquareJoker) {
        options.push(Farm::Win { four: true });
    }
    if run.discards_left > 0 {
        options.push(Farm::Throw { single: false });
        if has(J::TradingCard) && run.discards_used_round == 0 {
            options.push(Farm::Throw { single: true });
        }
    }
    if run.hands_left > 1 {
        options.push(Farm::Extra { cards: 5 });
        if has(J::SquareJoker) {
            options.push(Farm::Extra { cards: 4 });
        }
        let gold_seal = run
            .hand
            .iter()
            .any(|&id| run.card(id).seal == crate::card::Seal::Gold);
        if gold_seal || (run.hands_played_round == 0 && (has(J::SixthSense) || has(J::DNA))) {
            options.push(Farm::Extra { cards: 1 });
        }
    }
    let mut chosen: Option<(f64, (MoveKind, u64), bool)> = None;
    for (n, f) in options.into_iter().enumerate() {
        let (first, extra) = farm_step(&fair, f);
        if chosen.as_ref().is_some_and(|(_, m, _)| *m == first) {
            continue;
        }
        let tries = if matches!(f, Farm::Win { .. }) {
            1
        } else {
            try_count(FARM_TRIES)
        };
        let (mut all, mut gain) = (true, 0.0);
        for t in 0..tries {
            let (beat, _, g) = try_policy(
                run,
                Policy::Farm(f),
                first,
                salt.wrapping_mul(0x51ED).wrapping_add((n as u64) << 32 | t),
                Some(reference),
            );
            all &= beat;
            gain += g;
        }
        if !all {
            continue;
        }
        let g = gain / tries as f64;
        let margin = if chosen.is_none() { 0.0 } else { FARM_MARGIN };
        if chosen.as_ref().is_none_or(|(b, _, _)| g > *b + margin) {
            chosen = Some((g, first, extra));
        }
    }
    chosen.map_or(((MoveKind::Play, best.1), false), |(_, m, e)| (m, e))
}

/// One move of `policy` in a try, and whether it is an extra hand (see `farm_step`).
fn policy_step(run: &Run, policy: Policy) -> ((MoveKind, u64), bool) {
    match policy {
        Policy::Farm(f) => farm_step(run, f),
        Policy::Plan(_) => (policy_move(run, policy), false),
    }
}

/// An extra hand in a try is played as the teacher plays it: with the jokers in the order that scores least,
/// so it doesn't end the round early; they go back to scoring order after it.
fn play_extra(c: &mut Run, m: (MoveKind, u64)) -> bool {
    if c.jokers.len() < 2 || c.jokers.iter().any(|j| j.face_down) {
        return apply(c, m.0, m.1).is_ok();
    }
    let low = crate::teacher::low_order(c);
    set_order(c, low);
    let ok = apply(c, m.0, m.1).is_ok();
    let back = crate::teacher::joker_order(c);
    set_order(c, back);
    ok
}

fn set_order(c: &mut Run, order: Vec<usize>) {
    c.jokers = order.iter().map(|&i| c.jokers[i].clone()).collect();
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
