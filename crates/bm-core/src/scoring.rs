//! Scoring a played hand, and the joker routine every joker effect goes
//! through (`rules/scoring.md`). The evaluation runs in one step in the
//! game's order: detection, the boss's hand check, 'before' effects, base
//! chips and mult, played cards with retriggers, held cards, jokers then
//! consumables, destruction, the floored score, then 'after' effects.

use crate::blinds::Boss;
use crate::card::{Edition, Enhancement, Seal, Suit};
use crate::joker_data::JokerKind as J;
use crate::jokers::{joker_hand_type, joker_t_chips, joker_t_mult};
use crate::poker::{detect, DetectRules, HandSet, HandType};
use crate::state::{CardId, Run};

/// What a joker (or a card) returns from one evaluation: the game's result
/// table. `None` from a routine means it returned nothing at all.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Effect {
    /// Per-card +chips.
    pub chips: f64,
    /// Per-card +mult.
    pub mult: f64,
    /// Per-card or held X-mult.
    pub x_mult: Option<f64>,
    /// Money.
    pub dollars: i64,
    /// Held +mult.
    pub h_mult: f64,
    pub repetitions: u32,
    /// Main-effect +chips, +mult, X-mult.
    pub chip_mod: f64,
    pub mult_mod: f64,
    pub xmult_mod: Option<f64>,
    /// 'Before': level up the played hand.
    pub level_up: bool,
    /// End of round: Mr. Bones saved the run.
    pub saved: bool,
}

/// The played hand as the joker routine sees it.
#[derive(Clone, Debug, Default)]
pub struct PlayedHand {
    /// All played cards in play order.
    pub full: Vec<CardId>,
    /// The scoring cards in play order.
    pub scoring: Vec<CardId>,
    pub hand: Option<HandType>,
    pub contains: HandSet,
}

/// Which question a joker is being asked.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Ctx {
    Before,
    /// A scoring card's per-card effect.
    PlayCard(CardId),
    /// A held card's effect during scoring.
    HeldCard(CardId),
    PlayRepetition(CardId),
    /// A held card's retriggers; the flag says whether it did something.
    HeldRepetition(CardId, bool),
    Main,
    /// Joker-on-joker: the joker at this index is being scored.
    OtherJoker(usize),
    After,
    DebuffedHand,
    EndOfRound {
        game_over: bool,
    },
    /// End of round, a held card's retriggers (Mime).
    EndOfRoundRepetition(CardId, bool),
    PreDiscard {
        hook: bool,
    },
    /// A discarded card; `last` = the last card of the discard.
    Discard {
        card: CardId,
        last: bool,
        hook: bool,
    },
    SettingBlind,
}

/// The joker routine for the joker at `idx` (`Card:calculate_joker`).
/// `depth` counts Blueprint/Brainstorm hops (0 = asked directly).
pub fn calculate_joker(
    run: &mut Run,
    idx: usize,
    ctx: Ctx,
    h: &PlayedHand,
    depth: u32,
) -> Option<Effect> {
    if run.jokers[idx].debuffed {
        return None;
    }
    let kind = run.jokers[idx].kind;
    // Copy jokers run the copied joker's routine at their own position.
    let copied = match kind {
        J::Blueprint => Some(idx + 1),
        J::Brainstorm => Some(0),
        _ => None,
    };
    if let Some(other) = copied {
        if other < run.jokers.len() && other != idx {
            let d = depth + 1;
            if d as usize > run.jokers.len() + 1 {
                return None;
            }
            if let Some(e) = calculate_joker(run, other, ctx, h, d) {
                return Some(e);
            }
        }
        return None;
    }
    let bp = depth > 0;
    match ctx {
        Ctx::Before => before(run, idx, h, bp),
        Ctx::PlayCard(c) => play_card(run, idx, c, h, bp),
        Ctx::HeldCard(c) => held_card(run, idx, c),
        Ctx::PlayRepetition(c) => play_repetition(run, idx, c, h),
        Ctx::HeldRepetition(_, did) | Ctx::EndOfRoundRepetition(_, did) => (kind == J::Mime && did)
            .then(|| Effect {
                repetitions: 1,
                ..Effect::default()
            }),
        Ctx::Main => main_effect(run, idx, h, bp),
        Ctx::OtherJoker(other) => {
            let o = &run.jokers[other];
            (kind == J::BaseballCard && o.rarity() == 2 && other != idx).then(|| Effect {
                xmult_mod: Some(1.5),
                ..Effect::default()
            })
        }
        Ctx::After => after(run, idx, bp),
        Ctx::DebuffedHand => {
            if kind == J::Matador && run.blind.as_ref().is_some_and(|b| b.triggered) {
                run.pending_money += 8;
                return Some(Effect {
                    dollars: 8,
                    ..Effect::default()
                });
            }
            None
        }
        Ctx::EndOfRound { game_over } => end_of_round(run, idx, game_over, bp),
        Ctx::PreDiscard { .. } => None,
        Ctx::Discard { last, .. } => {
            if kind == J::GreenJoker && !bp && last {
                let j = &mut run.jokers[idx];
                let prev = j.mult;
                j.mult = (j.mult - 1.0).max(0.0);
                if j.mult != prev {
                    return Some(Effect::default());
                }
            }
            None
        }
        Ctx::SettingBlind => {
            if kind == J::Chicot
                && !bp
                && run
                    .blind
                    .as_ref()
                    .is_some_and(|b| b.boss().is_some() && !b.disabled)
            {
                run.disable_blind();
            }
            None
        }
    }
}

fn roll(run: &mut Run, key: &str, odds: f64) -> bool {
    run.rng.random(key) < run.probability / odds
}

fn some() -> Option<Effect> {
    Some(Effect::default())
}

fn before(run: &mut Run, idx: usize, h: &PlayedHand, bp: bool) -> Option<Effect> {
    let kind = run.jokers[idx].kind;
    match kind {
        J::SpareTrousers
            if !bp
                && (h.contains.contains(HandType::TwoPair)
                    || h.contains.contains(HandType::FullHouse)) =>
        {
            run.jokers[idx].mult += 2.0;
            some()
        }
        J::SpaceJoker if roll(run, "space", 4.0) => Some(Effect {
            level_up: true,
            ..Effect::default()
        }),
        J::SquareJoker if h.full.len() == 4 && !bp => {
            run.jokers[idx].chips += 4.0;
            some()
        }
        J::Runner if h.contains.contains(HandType::Straight) && !bp => {
            run.jokers[idx].chips += 15.0;
            some()
        }
        J::MidasMask if !bp => {
            let par = run.pareidolia();
            let mut any = false;
            for &c in &h.scoring {
                if run.card(c).is_face(par) {
                    run.set_enhancement(c, Enhancement::Gold);
                    any = true;
                }
            }
            any.then(Effect::default)
        }
        J::Vampire if !bp => {
            let mut n = 0;
            for &c in &h.scoring {
                let card = run.card(c);
                if card.enhancement != Enhancement::None && !card.debuffed {
                    run.set_enhancement(c, Enhancement::None);
                    n += 1;
                }
            }
            if n > 0 {
                run.jokers[idx].x_mult += 0.1 * n as f64;
                return some();
            }
            None
        }
        J::RideTheBus if !bp => {
            let par = run.pareidolia();
            let faces = h.scoring.iter().any(|&c| run.card(c).is_face(par));
            let j = &mut run.jokers[idx];
            if faces {
                let last = j.mult;
                j.mult = 0.0;
                if last > 0.0 {
                    return some();
                }
            } else {
                j.mult += 1.0;
            }
            None
        }
        J::Obelisk if !bp => {
            let ht = h.hand?;
            let played = run.hand_levels.get(ht).played;
            let reset = !HandType::ALL.iter().any(|&o| {
                let i = run.hand_levels.get(o);
                o != ht && i.played >= played && i.visible
            });
            let j = &mut run.jokers[idx];
            if reset {
                if j.x_mult > 1.0 {
                    j.x_mult = 1.0;
                    return some();
                }
            } else {
                j.x_mult += 0.2;
            }
            None
        }
        J::GreenJoker if !bp => {
            run.jokers[idx].mult += 1.0;
            some()
        }
        _ => None,
    }
}

fn play_card(run: &mut Run, idx: usize, c: CardId, h: &PlayedHand, bp: bool) -> Option<Effect> {
    let kind = run.jokers[idx].kind;
    let smeared = run.smeared();
    let par = run.pareidolia();
    let card = run.card(c).clone();
    let id = card.rank_id();
    let is = |s: Suit| card.has_suit(s, smeared);
    let face = card.is_face(par);
    let fx = |chips: f64, mult: f64| {
        Some(Effect {
            chips,
            mult,
            ..Effect::default()
        })
    };
    let xm = |x: f64| {
        Some(Effect {
            x_mult: Some(x),
            ..Effect::default()
        })
    };
    match kind {
        J::Hiker => {
            run.card_mut(c).perma_bonus += 5;
            some()
        }
        J::LuckyCat if card.lucky_trigger && !bp => {
            run.jokers[idx].x_mult += 0.25;
            some()
        }
        J::WeeJoker if id == Some(2) && !bp => {
            run.jokers[idx].chips += 8.0;
            some()
        }
        J::Photograph => {
            let first_face = h
                .scoring
                .iter()
                .copied()
                .find(|&s| run.card(s).is_face(par));
            if first_face == Some(c) {
                xm(2.0)
            } else {
                None
            }
        }
        J::TheIdol if id == Some(run.idol.0.id()) && is(run.idol.1) => xm(2.0),
        J::ScaryFace if face => fx(30.0, 0.0),
        J::SmileyFace if face => fx(0.0, 5.0),
        J::GoldenTicket if card.enhancement == Enhancement::Gold => {
            run.pending_money += 4;
            Some(Effect {
                dollars: 4,
                ..Effect::default()
            })
        }
        J::Scholar if id == Some(14) => fx(20.0, 4.0),
        J::WalkieTalkie if id == Some(10) || id == Some(4) => fx(10.0, 4.0),
        J::BusinessCard if face && roll(run, "business", 2.0) => {
            run.pending_money += 2;
            Some(Effect {
                dollars: 2,
                ..Effect::default()
            })
        }
        J::Fibonacci if matches!(id, Some(2 | 3 | 5 | 8 | 14)) => fx(0.0, 8.0),
        J::EvenSteven if matches!(id, Some(r) if r <= 10 && r % 2 == 0) => fx(0.0, 4.0),
        J::OddTodd if matches!(id, Some(r) if (r <= 10 && r % 2 == 1) || r == 14) => fx(31.0, 0.0),
        J::GreedyJoker if is(Suit::Diamonds) => fx(0.0, 3.0),
        J::LustyJoker if is(Suit::Hearts) => fx(0.0, 3.0),
        J::WrathfulJoker if is(Suit::Spades) => fx(0.0, 3.0),
        J::GluttonousJoker if is(Suit::Clubs) => fx(0.0, 3.0),
        J::RoughGem if is(Suit::Diamonds) => {
            run.pending_money += 1;
            Some(Effect {
                dollars: 1,
                ..Effect::default()
            })
        }
        J::OnyxAgate if is(Suit::Clubs) => fx(0.0, 7.0),
        J::Arrowhead if is(Suit::Spades) => fx(50.0, 0.0),
        J::Bloodstone if is(Suit::Hearts) && roll(run, "bloodstone", 2.0) => xm(1.5),
        J::AncientJoker if run.ancient_suit.is_some_and(is) => xm(1.5),
        J::Triboulet if id == Some(12) || id == Some(13) => xm(2.0),
        _ => None,
    }
}

fn held_card(run: &mut Run, idx: usize, c: CardId) -> Option<Effect> {
    let kind = run.jokers[idx].kind;
    let card = run.card(c).clone();
    let id = card.rank_id();
    match kind {
        J::ShootTheMoon if id == Some(12) => {
            if card.debuffed {
                some()
            } else {
                Some(Effect {
                    h_mult: 13.0,
                    ..Effect::default()
                })
            }
        }
        J::Baron if id == Some(13) => {
            if card.debuffed {
                some()
            } else {
                Some(Effect {
                    x_mult: Some(1.5),
                    ..Effect::default()
                })
            }
        }
        J::ReservedParking if card.is_face(run.pareidolia()) && roll(run, "parking", 2.0) => {
            run.pending_money += 1;
            Some(Effect {
                dollars: 1,
                ..Effect::default()
            })
        }
        J::RaisedFist => {
            // The rightmost non-Stone card with the lowest rank.
            let mut lowest: Option<(u8, CardId)> = None;
            for &hc in &run.hand {
                let k = run.card(hc);
                if !k.is_stone() && lowest.is_none_or(|(r, _)| r >= k.rank.id()) {
                    lowest = Some((k.rank.id(), hc));
                }
            }
            match lowest {
                Some((_, low)) if low == c => {
                    if card.debuffed {
                        some()
                    } else {
                        Some(Effect {
                            h_mult: 2.0 * card.rank.chips() as f64,
                            ..Effect::default()
                        })
                    }
                }
                _ => None,
            }
        }
        _ => None,
    }
}

fn play_repetition(run: &mut Run, idx: usize, c: CardId, h: &PlayedHand) -> Option<Effect> {
    let kind = run.jokers[idx].kind;
    let card = run.card(c);
    let reps = |n| {
        Some(Effect {
            repetitions: n,
            ..Effect::default()
        })
    };
    match kind {
        J::SockAndBuskin if card.is_face(run.pareidolia()) => reps(1),
        J::HangingChad if h.scoring.first() == Some(&c) => reps(2),
        J::Dusk if run.hands_left == 0 => reps(1),
        J::Seltzer => reps(1),
        J::Hack if matches!(card.rank_id(), Some(2..=5)) => reps(1),
        _ => None,
    }
}

fn main_effect(run: &mut Run, idx: usize, h: &PlayedHand, _bp: bool) -> Option<Effect> {
    let j = run.jokers[idx].clone();
    let kind = j.kind;
    let has = |t: HandType| h.contains.contains(t);
    let mm = |m: f64| {
        Some(Effect {
            mult_mod: m,
            ..Effect::default()
        })
    };
    let cm = |c: f64| {
        Some(Effect {
            chip_mod: c,
            ..Effect::default()
        })
    };
    let xm = |x: f64| {
        Some(Effect {
            xmult_mod: Some(x),
            ..Effect::default()
        })
    };
    // The generic branches come first, in the game's order.
    let jtype = joker_hand_type(kind);
    if kind != J::SeeingDouble && j.x_mult > 1.0 && jtype.is_none_or(has) {
        return xm(j.x_mult);
    }
    let t_mult = joker_t_mult(kind);
    if t_mult > 0.0 && jtype.is_some_and(has) {
        return mm(t_mult);
    }
    let t_chips = joker_t_chips(kind);
    if t_chips > 0.0 && jtype.is_some_and(has) {
        return cm(t_chips);
    }
    match kind {
        J::HalfJoker if h.full.len() <= 3 => mm(20.0),
        J::AbstractJoker => mm(3.0 * run.jokers.len() as f64),
        J::Acrobat if run.hands_left == 0 => xm(3.0),
        J::MysticSummit if run.discards_left == 0 => mm(15.0),
        J::Misprint => {
            let m = run.rng.random_range("misprint", 0, 23);
            mm(m as f64)
        }
        J::Banner if run.discards_left > 0 => cm(30.0 * run.discards_left as f64),
        J::Stuntman => cm(250.0),
        J::Matador => {
            if run.blind.as_ref().is_some_and(|b| b.triggered) {
                run.pending_money += 8;
                return Some(Effect {
                    dollars: 8,
                    ..Effect::default()
                });
            }
            None
        }
        J::Supernova => {
            let n = h.hand.map_or(0, |t| run.hand_levels.get(t).played);
            mm(n as f64)
        }
        J::WeeJoker => cm(j.chips),
        J::BlueJoker if !run.deck.is_empty() => cm(2.0 * run.deck.len() as f64),
        J::Erosion => {
            let lost = 52 - run.playing_cards.len() as i64;
            if lost > 0 {
                mm(4.0 * lost as f64)
            } else {
                None
            }
        }
        J::SquareJoker | J::Runner | J::IceCream => cm(j.chips),
        J::Bull => {
            let m = run.money_with_pending();
            if m > 0 {
                cm(2.0 * m.max(0) as f64)
            } else {
                None
            }
        }
        J::Joker => mm(j.mult),
        J::SpareTrousers | J::RideTheBus | J::Popcorn | J::GreenJoker if j.mult > 0.0 => mm(j.mult),
        J::GrosMichel => mm(15.0),
        J::Cavendish => xm(3.0),
        J::CardSharp => {
            let again = h
                .hand
                .is_some_and(|t| run.hand_levels.get(t).played_this_round > 1);
            if again {
                xm(3.0)
            } else {
                None
            }
        }
        J::Bootstraps => {
            let n = run.money_with_pending().div_euclid(5);
            if n >= 1 {
                mm(2.0 * n as f64)
            } else {
                None
            }
        }
        _ => None,
    }
}

fn after(run: &mut Run, idx: usize, bp: bool) -> Option<Effect> {
    let kind = run.jokers[idx].kind;
    match kind {
        J::IceCream if !bp => {
            let j = &mut run.jokers[idx];
            if j.chips - 5.0 <= 0.0 {
                j.removing = true;
            } else {
                j.chips -= 5.0;
            }
            some()
        }
        J::Seltzer if !bp => {
            let j = &mut run.jokers[idx];
            if j.counter - 1 <= 0 {
                j.removing = true;
            } else {
                j.counter -= 1;
            }
            some()
        }
        _ => None,
    }
}

fn end_of_round(run: &mut Run, idx: usize, game_over: bool, bp: bool) -> Option<Effect> {
    if bp {
        return None;
    }
    let kind = run.jokers[idx].kind;
    match kind {
        J::Popcorn => {
            let j = &mut run.jokers[idx];
            if j.mult - 4.0 <= 0.0 {
                j.removing = true;
            } else {
                j.mult -= 4.0;
            }
            some()
        }
        J::GrosMichel => {
            if roll(run, "gros_michel", 6.0) {
                run.jokers[idx].removing = true;
                run.gros_michel_extinct = true;
            }
            some()
        }
        J::Cavendish => {
            if roll(run, "cavendish", 1000.0) {
                run.jokers[idx].removing = true;
            }
            some()
        }
        J::TurtleBean => {
            let j = &mut run.jokers[idx];
            if j.counter - 1 <= 0 {
                j.removing = true;
            } else {
                j.counter -= 1;
                run.change_hand_size(-1);
            }
            some()
        }
        J::MrBones => {
            let target = run.blind.as_ref().map_or(0.0, |b| b.chips);
            if game_over && run.round_chips / target >= 0.25 {
                run.jokers[idx].removing = true;
                return Some(Effect {
                    saved: true,
                    ..Effect::default()
                });
            }
            None
        }
        _ => None,
    }
}

/// Money a joker pays at cash-out (`Card:calculate_dollar_bonus`).
pub fn joker_cash_out(run: &Run, idx: usize) -> Option<i64> {
    let j = &run.jokers[idx];
    if j.debuffed {
        return None;
    }
    match j.kind {
        J::GoldenJoker => Some(4),
        _ => None,
    }
}

/// The outcome of one evaluated hand.
#[derive(Clone, Debug, PartialEq)]
pub struct HandScore {
    pub hand: HandType,
    /// Scoring cards in play order.
    pub scoring: Vec<CardId>,
    pub chips: f64,
    pub mult: f64,
    /// floor(chips x mult) added to the round total.
    pub score: f64,
    /// The boss made the hand score nothing.
    pub not_allowed: bool,
    /// Scoring cards destroyed (shattered Glass).
    pub destroyed: Vec<CardId>,
}

/// The rules that change detection, from the active jokers.
pub fn detect_rules(run: &Run) -> DetectRules {
    DetectRules {
        four_fingers: run.has_joker(J::FourFingers),
        shortcut: run.has_joker(J::Shortcut),
        smeared: run.has_joker(J::SmearedJoker),
        splash: run.has_joker(J::Splash),
    }
}

/// The boss's hand check (`Blind:debuff_hand`), with its side effects.
fn boss_hand_check(run: &mut Run, n_played: usize, hand: HandType) -> bool {
    let Some(b) = run.blind.as_mut() else {
        return false;
    };
    if b.disabled {
        return false;
    }
    b.triggered = false;
    match b.boss() {
        Some(Boss::Psychic) if n_played < 5 => {
            b.triggered = true;
            return true;
        }
        Some(Boss::Eye) => {
            if b.eye_hands.contains(hand) {
                b.triggered = true;
                return true;
            }
            b.eye_hands.insert(hand);
        }
        Some(Boss::Mouth) => {
            if b.mouth_hand.is_some_and(|m| m != hand) {
                b.triggered = true;
                return true;
            }
            b.mouth_hand = Some(hand);
        }
        Some(Boss::Arm) => {
            if run.hand_levels.get(hand).level > 1 {
                b.triggered = true;
                run.hand_levels.level_up(hand, -1);
            }
        }
        Some(Boss::Ox) if hand == run.most_played => {
            b.triggered = true;
            run.money = 0;
        }
        _ => {}
    }
    false
}

/// Evaluate the hand in the play area (`G.FUNCS.evaluate_play`). Money
/// earned is left in `pending_money` for the caller to add afterwards.
pub fn evaluate_play(run: &mut Run) -> HandScore {
    let full = run.play.clone();
    let rules = detect_rules(run);
    let det = {
        let cards: Vec<&crate::card::Card> = full.iter().map(|&id| run.card(id)).collect();
        detect(&cards, rules)
    };
    let hand = det.hand;
    {
        let info = run.hand_levels.get_mut(hand);
        info.played += 1;
        info.played_this_round += 1;
        info.visible = true;
    }
    run.last_hand = Some(hand);
    let scoring: Vec<CardId> = det.scoring.iter().map(|&i| full[i]).collect();
    let ph = PlayedHand {
        full: full.clone(),
        scoring: scoring.clone(),
        hand: Some(hand),
        contains: det.contains,
    };

    let mut chips;
    let mut mult;
    let mut destroyed = Vec::new();
    let not_allowed = boss_hand_check(run, full.len(), hand);
    if !not_allowed {
        // 'Before' effects (Space Joker levels the hand up here).
        for i in 0..run.jokers.len() {
            if let Some(e) = calculate_joker(run, i, Ctx::Before, &ph, 0) {
                if e.level_up {
                    run.hand_levels.level_up(hand, 1);
                }
            }
        }
        let info = run.hand_levels.get(hand);
        mult = info.mult;
        chips = info.chips;
        if run
            .blind
            .as_ref()
            .is_some_and(|b| b.active_boss() == Some(Boss::Flint))
        {
            mult = (mult * 0.5 + 0.5).floor().max(1.0);
            chips = (chips * 0.5 + 0.5).floor().max(0.0);
            if let Some(b) = run.blind.as_mut() {
                b.triggered = true;
            }
        }

        // Played cards, in play order.
        for &c in &scoring {
            if run.card(c).debuffed {
                if let Some(b) = run.blind.as_mut() {
                    b.triggered = true;
                }
                continue;
            }
            let mut reps = 1;
            if run.card(c).seal == Seal::Red {
                reps += 1;
            }
            for k in 0..run.jokers.len() {
                if let Some(e) = calculate_joker(run, k, Ctx::PlayRepetition(c), &ph, 0) {
                    reps += e.repetitions;
                }
            }
            for _ in 0..reps {
                let own = own_play_effect(run, c);
                let mut joker_fx = Vec::new();
                for k in 0..run.jokers.len() {
                    if let Some(e) = calculate_joker(run, k, Ctx::PlayCard(c), &ph, 0) {
                        joker_fx.push(e);
                    }
                }
                run.card_mut(c).lucky_trigger = false;
                // The card's own effect: chips, mult, money, X-mult, edition.
                chips += own.chips;
                mult += own.mult;
                if let Some(x) = own.x_mult {
                    mult *= x;
                }
                if let Some((ec, em, ex)) = own.edition {
                    chips += ec;
                    mult += em;
                    mult *= ex;
                }
                for e in joker_fx {
                    chips += e.chips;
                    mult += e.mult;
                    if let Some(x) = e.x_mult {
                        mult *= x;
                    }
                }
            }
        }

        // Held cards, left to right.
        let held = run.hand.clone();
        for &c in &held {
            let mut reps = 1usize;
            let mut pass = 0usize;
            while pass < reps {
                let own = own_held_effect(run, c);
                let mut joker_fx = Vec::new();
                for k in 0..run.jokers.len() {
                    if let Some(e) = calculate_joker(run, k, Ctx::HeldCard(c), &ph, 0) {
                        joker_fx.push(e);
                    }
                }
                if pass == 0 {
                    let did = own.is_some() || !joker_fx.is_empty();
                    let card = run.card(c);
                    if card.seal == Seal::Red && !card.debuffed && did {
                        reps += 1;
                    }
                    for k in 0..run.jokers.len() {
                        if let Some(e) =
                            calculate_joker(run, k, Ctx::HeldRepetition(c, did), &ph, 0)
                        {
                            reps += e.repetitions as usize;
                        }
                    }
                }
                if let Some(x) = own {
                    mult *= x;
                }
                for e in joker_fx {
                    mult += e.h_mult;
                    if let Some(x) = e.x_mult {
                        mult *= x;
                    }
                }
                pass += 1;
            }
        }

        // Jokers left to right, then consumables.
        for i in 0..run.jokers.len() {
            let (debuffed, edition) = (run.jokers[i].debuffed, run.jokers[i].edition);
            if !debuffed {
                chips += edition.chips();
                mult += edition.mult();
            }
            if let Some(e) = calculate_joker(run, i, Ctx::Main, &ph, 0) {
                mult += e.mult_mod;
                chips += e.chip_mod;
                if let Some(x) = e.xmult_mod {
                    mult *= x;
                }
            }
            for v in 0..run.jokers.len() {
                if let Some(e) = calculate_joker(run, v, Ctx::OtherJoker(i), &ph, 0) {
                    mult += e.mult_mod;
                    chips += e.chip_mod;
                    if let Some(x) = e.xmult_mod {
                        mult *= x;
                    }
                }
            }
            if !debuffed && edition == Edition::Polychrome {
                mult *= 1.5;
            }
        }

        // Destruction: Glass cards roll to shatter.
        for &c in &scoring {
            let card = run.card(c);
            if card.enhancement == Enhancement::Glass && !card.debuffed && roll(run, "glass", 4.0) {
                destroyed.push(c);
            }
        }
    } else {
        chips = 0.0;
        mult = 0.0;
        for i in 0..run.jokers.len() {
            calculate_joker(run, i, Ctx::DebuffedHand, &ph, 0);
        }
    }

    let score = (chips * mult).floor();
    run.round_chips += score;

    for i in 0..run.jokers.len() {
        calculate_joker(run, i, Ctx::After, &ph, 0);
    }

    HandScore {
        hand,
        scoring,
        chips,
        mult,
        score,
        not_allowed,
        destroyed,
    }
}

/// A scored card's own effect for one trigger.
struct OwnPlay {
    chips: f64,
    mult: f64,
    x_mult: Option<f64>,
    edition: Option<(f64, f64, f64)>,
}

fn own_play_effect(run: &mut Run, c: CardId) -> OwnPlay {
    let card = run.card(c).clone();
    let chips = card.chip_bonus() as f64;
    let mut mult = 0.0;
    match card.enhancement {
        Enhancement::Mult => mult = 4.0,
        Enhancement::Lucky if roll(run, "lucky_mult", 5.0) => {
            run.card_mut(c).lucky_trigger = true;
            mult = 20.0;
        }
        _ => {}
    }
    let x_mult = (card.enhancement == Enhancement::Glass).then_some(2.0);
    let mut dollars = 0;
    if card.seal == Seal::Gold {
        dollars += 3;
    }
    if card.enhancement == Enhancement::Lucky && roll(run, "lucky_money", 15.0) {
        run.card_mut(c).lucky_trigger = true;
        dollars += 20;
    }
    run.pending_money += dollars;
    let edition = match card.edition {
        Edition::None => None,
        e => Some((e.chips(), e.mult(), e.x_mult())),
    };
    OwnPlay {
        chips,
        mult,
        x_mult,
        edition,
    }
}

/// A held card's own effect during scoring: Steel X1.5 unless debuffed.
fn own_held_effect(run: &Run, c: CardId) -> Option<f64> {
    let card = run.card(c);
    (card.enhancement == Enhancement::Steel && !card.debuffed).then_some(1.5)
}
