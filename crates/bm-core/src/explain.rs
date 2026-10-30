//! The teacher's plan in words, for the viewer: what its value model sees in a position, said plainly. Every
//! sentence comes from the model's own numbers (its chance of beating each blind ahead, the hands the deck
//! deals, what the jokers pay for, the money and what comes in each round); nothing here decides anything,
//! and nothing here is the game's text.

use crate::blinds::{BlindSlot, Boss, SlotState};
use crate::card::{Card, Enhancement, Rank, Suit};
use crate::joker_data::JokerKind;
use crate::poker::HandType;
use crate::state::Run;
use crate::value::{breakdown, dealt_hands, flows, one_joker_bonus, Breakdown, DECK_DEALS};

/// What a boss does, in a few words of our own.
pub fn boss_effect(b: Boss) -> &'static str {
    match b {
        Boss::Arm => "each hand played loses a level",
        Boss::Club => "Clubs score nothing",
        Boss::Eye => "no hand type may be played twice",
        Boss::Fish => "cards drawn after a hand come face down",
        Boss::Flint => "base chips and mult are halved",
        Boss::Goad => "Spades score nothing",
        Boss::Head => "Hearts score nothing",
        Boss::Hook => "two cards in hand are thrown after each hand",
        Boss::House => "the first hand comes face down",
        Boss::Manacle => "one card less in hand",
        Boss::Mark => "face cards come face down",
        Boss::Mouth => "one hand type for the whole round",
        Boss::Needle => "a single hand",
        Boss::Ox => "the most played hand costs all the money",
        Boss::Pillar => "cards already played this ante score nothing",
        Boss::Plant => "face cards score nothing",
        Boss::Psychic => "five-card hands only",
        Boss::Serpent => "only three cards drawn after each hand or throw",
        Boss::Tooth => "$1 lost for each card played",
        Boss::Wall => "a very large target",
        Boss::Water => "no discards",
        Boss::Wheel => "some cards come face down",
        Boss::Window => "Diamonds score nothing",
        Boss::AmberAcorn => "the jokers are flipped and shuffled",
        Boss::CeruleanBell => "one card is always selected",
        Boss::CrimsonHeart => "a joker is switched off each hand",
        Boss::VerdantLeaf => "no card scores until a joker is sold",
        Boss::VioletVessel => "an enormous target",
    }
}

/// A number in a short form (4.2k, 1.3M).
fn short(x: f64) -> String {
    let a = x.abs();
    if a >= 1e9 {
        format!("{:.1}B", x / 1e9)
    } else if a >= 1e6 {
        format!("{:.1}M", x / 1e6)
    } else if a >= 1e4 {
        format!("{:.0}k", x / 1e3)
    } else if a >= 1e3 {
        format!("{:.1}k", x / 1e3)
    } else {
        format!("{x:.0}")
    }
}

fn pct(p: f64) -> String {
    format!("{:.0}%", 100.0 * p)
}

/// Names for the blinds `value::blinds_ahead` weighs, in the same order.
fn blind_names(run: &Run) -> Vec<String> {
    let mut out = Vec::new();
    for slot in BlindSlot::ALL {
        if matches!(
            run.slots[slot.index()],
            SlotState::Defeated | SlotState::Skipped
        ) {
            continue;
        }
        out.push(match slot {
            BlindSlot::Small => "the Small Blind".to_string(),
            BlindSlot::Big => "the Big Blind".to_string(),
            BlindSlot::Boss => format!("{} ({})", run.boss.name(), boss_effect(run.boss)),
        });
    }
    if run.ante < run.win_ante {
        for s in [
            "next ante's Small Blind",
            "next ante's Big Blind",
            "next ante's boss",
        ] {
            out.push(s.to_string());
        }
    }
    out
}

fn rank_name(r: u8) -> &'static str {
    match r {
        14 => "Aces",
        13 => "Kings",
        12 => "Queens",
        11 => "Jacks",
        10 => "10s",
        9 => "9s",
        8 => "8s",
        7 => "7s",
        6 => "6s",
        5 => "5s",
        4 => "4s",
        3 => "3s",
        _ => "2s",
    }
}

fn suit_name(s: Suit) -> &'static str {
    match s {
        Suit::Hearts => "Hearts",
        Suit::Clubs => "Clubs",
        Suit::Diamonds => "Diamonds",
        Suit::Spades => "Spades",
    }
}

fn enh_name(e: Enhancement) -> &'static str {
    match e {
        Enhancement::Steel => "Steel cards",
        Enhancement::Gold => "Gold cards",
        Enhancement::Glass => "Glass cards",
        Enhancement::Lucky => "Lucky cards",
        Enhancement::Stone => "Stone cards",
        Enhancement::Wild => "Wild cards",
        Enhancement::Mult => "Mult cards",
        Enhancement::Bonus => "Bonus cards",
        Enhancement::None => "plain cards",
    }
}

/// What one joker pays for in playing cards, in words (from the same table the teacher builds by), or None.
fn wants(run: &Run, kind: JokerKind) -> Option<String> {
    let card = |r: Rank, s: Suit, e: Enhancement| {
        let mut c = Card::new(0, s, r);
        c.enhancement = e;
        one_joker_bonus(run, kind, &c)
    };
    let mut parts: Vec<String> = Vec::new();
    // Ranks (a plain Spade of each), unless every rank pays the same.
    let ranks: Vec<(u8, f64)> = Rank::ALL
        .iter()
        .map(|&r| (r.id(), card(r, Suit::Spades, Enhancement::None)))
        .collect();
    let low = ranks.iter().map(|r| r.1).fold(f64::MAX, f64::min);
    let paid: Vec<u8> = ranks.iter().filter(|r| r.1 > low).map(|r| r.0).collect();
    if !paid.is_empty() {
        let mut ids = paid.clone();
        ids.sort_unstable();
        let faces = ids == [11, 12, 13];
        let run_of = ids.len() > 2 && ids.windows(2).all(|w| w[1] == w[0] + 1);
        parts.push(if faces {
            "face cards".to_string()
        } else if run_of {
            format!("{} to {}", rank_name(ids[0]), rank_name(ids[ids.len() - 1]))
        } else {
            let names: Vec<&str> = ids.iter().rev().map(|&r| rank_name(r)).collect();
            names.join(", ")
        });
    }
    // Suits (a 7 of each), unless every suit pays the same.
    let suits: Vec<(Suit, f64)> = Suit::ALL
        .iter()
        .map(|&s| (s, card(Rank::Seven, s, Enhancement::None)))
        .collect();
    let low = suits.iter().map(|s| s.1).fold(f64::MAX, f64::min);
    let paid: Vec<&str> = suits
        .iter()
        .filter(|s| s.1 > low)
        .map(|s| suit_name(s.0))
        .collect();
    if !paid.is_empty() && paid.len() < 4 {
        parts.push(paid.join(" and "));
    }
    // Enhancements, beyond what a plain 7 gets.
    let plain = card(Rank::Seven, Suit::Spades, Enhancement::None);
    for e in [
        Enhancement::Steel,
        Enhancement::Gold,
        Enhancement::Glass,
        Enhancement::Lucky,
        Enhancement::Stone,
    ] {
        if card(Rank::Seven, Suit::Spades, e) > plain + 1e-9 {
            parts.push(enh_name(e).to_string());
        }
    }
    (!parts.is_empty()).then(|| parts.join(", "))
}

/// The plan in a few sentences, or None once the run is over.
pub fn plan(run: &Run, salt: u64) -> Option<Vec<String>> {
    let b = breakdown(run, salt)?;
    Some(plan_from(run, salt, &b))
}

/// The plan from a breakdown already made (the viewer shows both).
pub fn plan_from(run: &Run, salt: u64, b: &Breakdown) -> Vec<String> {
    let mut out = Vec::new();

    // The blinds ahead: the next one, and the riskiest if it isn't comfortable.
    let names = blind_names(run);
    if let Some(&(target, p, _)) = b.blinds.first() {
        let name = names
            .first()
            .cloned()
            .unwrap_or_else(|| "the next blind".into());
        out.push(format!(
            "Beats {name} {} of the time: a round scores about {} against {}.",
            pct(p),
            short(b.next_total),
            short(target)
        ));
    }
    if let Some((i, &(target, p, _))) = b
        .blinds
        .iter()
        .enumerate()
        .skip(1)
        .min_by(|a, b| a.1 .1.total_cmp(&b.1 .1))
    {
        if p < 0.85 {
            let name = names
                .get(i)
                .cloned()
                .unwrap_or_else(|| "a later blind".into());
            out.push(format!(
                "The risk is {name}: {} against {}, so scoring power comes before money.",
                pct(p),
                short(target)
            ));
        }
    }

    // The hands this deck deals, and the one that scores most.
    let dealt = dealt_hands(run, salt ^ 0x9A1, DECK_DEALS);
    if !dealt.is_empty() {
        let n = dealt.len() as f64;
        let share = |h: HandType| dealt.iter().filter(|d| d.0 == h).count() as f64 / n;
        let level = run.hand_levels.get(b.main).level;
        let mut common: Vec<(HandType, f64)> = HandType::ALL
            .iter()
            .map(|&h| (h, share(h)))
            .filter(|x| x.1 > 0.0)
            .collect();
        common.sort_by(|a, b| b.1.total_cmp(&a.1));
        let others: Vec<String> = common
            .iter()
            .filter(|x| x.0 != b.main)
            .take(2)
            .map(|x| format!("{} {}", x.0.name(), pct(x.1)))
            .collect();
        let often = if share(b.main) < 0.05 {
            "rarely gives it".to_string()
        } else {
            format!("gives it {} of the time", pct(share(b.main)))
        };
        out.push(format!(
            "Scores most with {} (level {level}); dealt with a couple of throws, the deck {often}{}.",
            b.main.name(),
            if others.is_empty() {
                String::new()
            } else {
                format!(" ({})", others.join(", "))
            }
        ));
    }

    // What the jokers pay for in the deck.
    let mut pays: Vec<String> = Vec::new();
    for j in run.jokers.iter().filter(|j| !j.debuffed) {
        if let Some(w) = wants(run, j.kind) {
            pays.push(format!("{} pays for {w}", j.kind.name()));
        }
    }
    if !pays.is_empty() {
        pays.truncate(3);
        out.push(format!("Build: {}.", pays.join("; ")));
    }

    // Growth and income.
    let mut growing = Vec::new();
    let mut earning = Vec::new();
    // Each joker's flows beyond what the deck brings by itself (Gold cards, Blue Seals...).
    let mut bare = run.clone();
    bare.jokers.clear();
    let base = flows(&bare, b.main);
    for j in run.jokers.iter().filter(|j| !j.debuffed) {
        let mut one = bare.clone();
        one.jokers = vec![j.clone()];
        let f = flows(&one, b.main);
        let grows = f.chips > base.chips
            || f.mult > base.mult
            || f.levels > base.levels
            || f.xmult.iter().any(|&(_, rate)| rate > 0.0);
        if grows {
            growing.push(j.kind.name());
        }
        if f.money > base.money + 0.5
            || f.tarots > base.tarots + 0.1
            || f.spectrals > base.spectrals + 0.1
        {
            earning.push(j.kind.name());
        }
    }
    let interest = if run.interest_amount > 0 {
        run.interest_amount * (run.money.max(0) / 5).min(run.interest_cap / 5)
    } else {
        0
    };
    let mut money = format!("Money ${}, interest ${interest} a round", run.money);
    if !earning.is_empty() {
        money += &format!(", income from {}", earning.join(", "));
    }
    let at_risk = b.blinds.iter().take(3).any(|x| x.1 < 0.85);
    money += if at_risk {
        "; spends it on scoring while the blinds ahead are at risk."
    } else if run.money < run.interest_cap && run.interest_amount > 0 {
        "; with the blinds safe, money towards full interest is worth more than a small buy."
    } else {
        "; at full interest: a buy has to beat what the money is worth later."
    };
    out.push(money);
    if !growing.is_empty() {
        out.push(format!("Growing each round: {}.", growing.join(", ")));
    }

    // What it lacks.
    let xmult = run.jokers.iter().any(|j| j.x_mult > 1.0 && !j.debuffed);
    let free = run.joker_slots - run.jokers.len() as i32;
    let mut lacks = Vec::new();
    if !xmult && run.ante >= 3 {
        lacks.push("an x-mult joker (the later blinds grow faster than adders can)".to_string());
    }
    if free > 0 && run.ante >= 2 {
        lacks.push(format!(
            "{free} empty joker slot{}",
            if free > 1 { "s" } else { "" }
        ));
    }
    if run.hand_levels.get(b.main).level + 1 < run.ante {
        lacks.push(format!("Planets for {}", b.main.name()));
    }
    if !lacks.is_empty() {
        out.push(format!("Looking for {}.", lacks.join(", ")));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blinds::BlindKind;

    #[test]
    fn the_plan_says_what_the_model_sees() {
        let mut run = Run::scenario("PLANTEXT", BlindKind::Small);
        run.phase = crate::state::Phase::BlindSelect;
        run.hand.clear();
        run.blind = None;
        run.add_joker(JokerKind::Baron);
        run.add_joker(JokerKind::OnyxAgate);
        run.add_joker(JokerKind::GreenJoker);
        let lines = plan(&run, 5).expect("a plan");
        let text = lines.join(" ");
        assert!(text.contains("Beats the Small Blind"), "{text}");
        assert!(text.contains("Baron pays for Kings"), "{text}");
        assert!(text.contains("Onyx Agate pays for Clubs"), "{text}");
        assert!(text.contains("Growing each round: Green Joker"), "{text}");
        assert!(text.contains("Scores most with"), "{text}");
    }

    #[test]
    fn ranges_and_faces_read_naturally() {
        let run = Run::scenario("PLANTEXT", BlindKind::Small);
        assert_eq!(wants(&run, JokerKind::Hack).as_deref(), Some("2s to 5s"));
        assert_eq!(
            wants(&run, JokerKind::SmileyFace).as_deref(),
            Some("face cards")
        );
        assert_eq!(
            wants(&run, JokerKind::SteelJoker).as_deref(),
            Some("Steel cards")
        );
        assert_eq!(wants(&run, JokerKind::Joker), None);
    }
}
