//! Poker-hand detection: the hand type, the hand types a play contains, and
//! which played cards score (`rules/hands.md`).

use crate::card::{Card, Suit};

/// The 12 hand types, weakest first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(u8)]
pub enum HandType {
    HighCard = 0,
    Pair = 1,
    TwoPair = 2,
    ThreeOfAKind = 3,
    Straight = 4,
    Flush = 5,
    FullHouse = 6,
    FourOfAKind = 7,
    StraightFlush = 8,
    FiveOfAKind = 9,
    FlushHouse = 10,
    FlushFive = 11,
}

impl HandType {
    /// Weakest first.
    pub const ALL: [HandType; 12] = [
        HandType::HighCard,
        HandType::Pair,
        HandType::TwoPair,
        HandType::ThreeOfAKind,
        HandType::Straight,
        HandType::Flush,
        HandType::FullHouse,
        HandType::FourOfAKind,
        HandType::StraightFlush,
        HandType::FiveOfAKind,
        HandType::FlushHouse,
        HandType::FlushFive,
    ];

    pub fn index(self) -> usize {
        self as usize
    }

    pub fn from_index(i: usize) -> HandType {
        HandType::ALL[i]
    }

    /// The game's listing order: Flush Five = 1 ... High Card = 12.
    pub fn game_order(self) -> u8 {
        12 - self as u8
    }

    pub fn name(self) -> &'static str {
        match self {
            HandType::HighCard => "High Card",
            HandType::Pair => "Pair",
            HandType::TwoPair => "Two Pair",
            HandType::ThreeOfAKind => "Three of a Kind",
            HandType::Straight => "Straight",
            HandType::Flush => "Flush",
            HandType::FullHouse => "Full House",
            HandType::FourOfAKind => "Four of a Kind",
            HandType::StraightFlush => "Straight Flush",
            HandType::FiveOfAKind => "Five of a Kind",
            HandType::FlushHouse => "Flush House",
            HandType::FlushFive => "Flush Five",
        }
    }
}

/// A set of hand types (bit i = `HandType::ALL[i]`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct HandSet(pub u16);

impl HandSet {
    pub fn contains(self, h: HandType) -> bool {
        self.0 & (1 << h as u16) != 0
    }

    pub fn insert(&mut self, h: HandType) {
        self.0 |= 1 << h as u16;
    }

    /// The strongest hand type in the set.
    pub fn best(self) -> Option<HandType> {
        if self.0 == 0 {
            None
        } else {
            Some(HandType::from_index(15 - self.0.leading_zeros() as usize))
        }
    }
}

/// Jokers that change detection (active = owned and not debuffed).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DetectRules {
    pub four_fingers: bool,
    pub shortcut: bool,
    pub smeared: bool,
    pub splash: bool,
}

/// What detection found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Detection {
    pub hand: HandType,
    pub contains: HandSet,
    /// Positions in the played list (play order), ascending.
    pub scoring: Vec<usize>,
}

/// Groups of exactly `n` cards of one rank, highest rank first; each group's
/// positions in play order.
fn groups(members: &[Vec<usize>; 15], n: usize) -> Vec<&Vec<usize>> {
    (2..=14)
        .rev()
        .map(|r| &members[r])
        .filter(|m| m.len() == n)
        .collect()
}

/// The game's sort value for the highest-card pick (without the age term,
/// which `detect` applies as a tie-break).
fn nominal(c: &Card) -> f64 {
    let m = if c.is_stone() { -1000.0 } else { 1.0 };
    c.rank.chips() as f64
        + c.rank.face_value()
        + c.suit.sort_value() * m
        + c.original_suit.original_value() * 0.0001 * m
}

/// Detect the hand formed by `played` (1 to 5 cards, in play order).
pub fn detect(played: &[&Card], rules: DetectRules) -> Detection {
    let n = played.len();
    let mut members: [Vec<usize>; 15] = Default::default();
    for (i, c) in played.iter().enumerate() {
        if let Some(r) = c.detect_rank() {
            members[r.id() as usize].push(i);
        }
    }
    let g5 = groups(&members, 5);
    let g4 = groups(&members, 4);
    let g3 = groups(&members, 3);
    let g2 = groups(&members, 2);

    let need = if rules.four_fingers { 4 } else { 5 };
    let sized = n <= 5 && n >= need;

    let flush: Option<Vec<usize>> = if !sized {
        None
    } else {
        Suit::ALL.iter().find_map(|&s| {
            let col: Vec<usize> = (0..n)
                .filter(|&i| played[i].counts_for_flush(s, rules.smeared))
                .collect();
            (col.len() >= need).then_some(col)
        })
    };

    let straight: Option<Vec<usize>> = if !sized {
        None
    } else {
        let mut run = 0usize;
        let mut collected: Vec<usize> = Vec::new();
        let mut found = false;
        let mut skipped = false;
        for j in 1..=14usize {
            let bucket = &members[if j == 1 { 14 } else { j }];
            if !bucket.is_empty() {
                run += 1;
                skipped = false;
                collected.extend_from_slice(bucket);
            } else if rules.shortcut && !skipped && j != 14 {
                skipped = true;
            } else {
                run = 0;
                skipped = false;
                if found {
                    break;
                }
                collected.clear();
            }
            if run >= need {
                found = true;
            }
        }
        found.then_some(collected)
    };

    let mut highest: Option<usize> = None;
    for i in 0..n {
        let better = match highest {
            None => true,
            Some(h) => {
                let (a, b) = (nominal(played[i]), nominal(played[h]));
                a > b || (a == b && played[i].sort_id < played[h].sort_id)
            }
        };
        if better {
            highest = Some(i);
        }
    }

    let mut contains = HandSet::default();
    let has_flush = flush.is_some();
    let has_straight = straight.is_some();
    if !g5.is_empty() && has_flush {
        contains.insert(HandType::FlushFive);
    }
    if !g3.is_empty() && !g2.is_empty() && has_flush {
        contains.insert(HandType::FlushHouse);
    }
    if !g5.is_empty() {
        contains.insert(HandType::FiveOfAKind);
    }
    if has_flush && has_straight {
        contains.insert(HandType::StraightFlush);
    }
    if !g4.is_empty() || !g5.is_empty() {
        contains.insert(HandType::FourOfAKind);
    }
    if !g3.is_empty() && !g2.is_empty() {
        contains.insert(HandType::FullHouse);
    }
    if has_flush {
        contains.insert(HandType::Flush);
    }
    if has_straight {
        contains.insert(HandType::Straight);
    }
    if !g3.is_empty() || !g4.is_empty() || !g5.is_empty() {
        contains.insert(HandType::ThreeOfAKind);
    }
    if g2.len() == 2 || (g3.len() == 1 && g2.len() == 1) {
        contains.insert(HandType::TwoPair);
    }
    if !g2.is_empty() || !g3.is_empty() || !g4.is_empty() || !g5.is_empty() {
        contains.insert(HandType::Pair);
    }
    if n >= 1 {
        contains.insert(HandType::HighCard);
    }

    let hand = contains.best().unwrap_or(HandType::HighCard);
    let mut scoring: Vec<usize> = match hand {
        HandType::FlushFive | HandType::FiveOfAKind => g5[0].clone(),
        HandType::FlushHouse | HandType::FullHouse => {
            g3[0].iter().chain(g2[0].iter()).copied().collect()
        }
        HandType::StraightFlush => {
            let f = flush.clone().unwrap();
            let mut v = f.clone();
            for i in straight.clone().unwrap() {
                if !f.contains(&i) {
                    v.push(i);
                }
            }
            v
        }
        HandType::FourOfAKind => g4
            .first()
            .or(g5.first())
            .map(|g| (*g).clone())
            .unwrap_or_default(),
        HandType::Flush => flush.clone().unwrap(),
        HandType::Straight => straight.clone().unwrap(),
        HandType::ThreeOfAKind => g3
            .first()
            .or(g4.first())
            .or(g5.first())
            .map(|g| (*g).clone())
            .unwrap_or_default(),
        HandType::TwoPair => {
            let second = g2.get(1).or(g3.first()).unwrap();
            g2[0].iter().chain(second.iter()).copied().collect()
        }
        HandType::Pair => g2
            .first()
            .or(g3.first())
            .map(|g| (*g).clone())
            .unwrap_or_default(),
        HandType::HighCard => highest.into_iter().collect(),
    };
    if rules.splash {
        scoring = (0..n).collect();
    } else {
        for (i, c) in played.iter().enumerate() {
            if c.is_stone() && !scoring.contains(&i) {
                scoring.push(i);
            }
        }
    }
    scoring.sort_unstable();
    scoring.dedup();
    Detection {
        hand,
        contains,
        scoring,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::card::{Enhancement, Rank};

    fn cards(spec: &str) -> Vec<Card> {
        spec.split_whitespace()
            .enumerate()
            .map(|(i, t)| {
                let b = t.as_bytes();
                let rank = match b[0] {
                    b'T' => Rank::Ten,
                    b'J' => Rank::Jack,
                    b'Q' => Rank::Queen,
                    b'K' => Rank::King,
                    b'A' => Rank::Ace,
                    d => Rank::from_id(d - b'0').unwrap(),
                };
                let suit = match b[1] {
                    b'S' => Suit::Spades,
                    b'H' => Suit::Hearts,
                    b'C' => Suit::Clubs,
                    _ => Suit::Diamonds,
                };
                let mut c = Card::new(i as u64 + 1, suit, rank);
                if b.len() > 2 {
                    c.enhancement = match b[2] {
                        b'w' => Enhancement::Wild,
                        b's' => Enhancement::Stone,
                        _ => Enhancement::None,
                    };
                }
                c
            })
            .collect()
    }

    fn det(spec: &str, rules: DetectRules) -> Detection {
        let cs = cards(spec);
        let refs: Vec<&Card> = cs.iter().collect();
        detect(&refs, rules)
    }

    const NONE: DetectRules = DetectRules {
        four_fingers: false,
        shortcut: false,
        smeared: false,
        splash: false,
    };

    #[test]
    fn basic_types_and_scoring_cards() {
        let d = det("KS KH 2C 5D 9S", NONE);
        assert_eq!(d.hand, HandType::Pair);
        assert_eq!(d.scoring, vec![0, 1]);
        let d = det("KS 2C KH 2D 9S", NONE);
        assert_eq!(d.hand, HandType::TwoPair);
        assert_eq!(d.scoring, vec![0, 1, 2, 3]);
        let d = det("AS 2C 3H 4D 5S", NONE);
        assert_eq!(d.hand, HandType::Straight);
        let d = det("TS JC QH KD AS", NONE);
        assert_eq!(d.hand, HandType::Straight);
        let d = det("QS KC AH 2D 3S", NONE);
        assert_eq!(d.hand, HandType::HighCard, "no wrap-around");
        assert_eq!(d.scoring, vec![2], "the Ace is the high card");
    }

    #[test]
    fn contained_hands() {
        let d = det("KS KH KC 2D 2S", NONE);
        assert_eq!(d.hand, HandType::FullHouse);
        for h in [
            HandType::TwoPair,
            HandType::ThreeOfAKind,
            HandType::Pair,
            HandType::HighCard,
        ] {
            assert!(d.contains.contains(h), "{h:?}");
        }
        let d = det("KS KH KC KD 2S", NONE);
        assert_eq!(d.hand, HandType::FourOfAKind);
        assert!(!d.contains.contains(HandType::TwoPair));
        assert!(d.contains.contains(HandType::ThreeOfAKind) && d.contains.contains(HandType::Pair));
        assert_eq!(d.scoring, vec![0, 1, 2, 3]);
    }

    #[test]
    fn four_fingers_and_shortcut() {
        let ff = DetectRules {
            four_fingers: true,
            ..NONE
        };
        let d = det("2S 3S 4S 9S 5H", ff);
        assert_eq!(
            d.hand,
            HandType::StraightFlush,
            "flush and straight on different cards"
        );
        assert_eq!(d.scoring, vec![0, 1, 2, 3, 4]);
        let d = det("5S 6H 7C 8D 8S", ff);
        assert_eq!(d.hand, HandType::Straight);
        assert_eq!(d.scoring, vec![0, 1, 2, 3, 4], "both 8s score");
        let d = det("KS KH 2S 5S 9S", ff);
        assert_eq!(d.hand, HandType::Flush);
        assert!(d.contains.contains(HandType::Pair));
        assert_eq!(d.scoring, vec![0, 2, 3, 4]);
        let sc = DetectRules {
            shortcut: true,
            ..NONE
        };
        assert_eq!(det("2S 4H 6C 8D TS", sc).hand, HandType::Straight);
        assert_eq!(det("AS 3H 5C 7D 9S", sc).hand, HandType::Straight);
        assert_eq!(det("2S 3H 5C 6D 9S", sc).hand, HandType::HighCard);
        let both = DetectRules {
            shortcut: true,
            four_fingers: true,
            ..NONE
        };
        assert_eq!(det("2S 3H 4C 5D 7S", both).scoring, vec![0, 1, 2, 3, 4]);
        assert_eq!(det("2S 3H 4C 5D 7S", ff).scoring, vec![0, 1, 2, 3]);
    }

    #[test]
    fn wild_smeared_stone_splash() {
        let d = det("2H 5H 9Hw KS 7H", NONE);
        assert_eq!(d.hand, HandType::HighCard, "wild 9 is a heart but KS isn't");
        let d = det("2H 5H 9Sw KH 7H", NONE);
        assert_eq!(d.hand, HandType::Flush);
        let mut cs = cards("2H 5H 9Sw KH 7H");
        cs[2].debuffed = true;
        let refs: Vec<&Card> = cs.iter().collect();
        assert_eq!(
            detect(&refs, NONE).hand,
            HandType::HighCard,
            "a debuffed wild card is not wild"
        );
        let sm = DetectRules {
            smeared: true,
            ..NONE
        };
        assert_eq!(det("2H 5D 9H KD 7H", sm).hand, HandType::Flush);
        let d = det("KS KH 3Cs 9D", NONE);
        assert_eq!(d.hand, HandType::Pair);
        assert_eq!(d.scoring, vec![0, 1, 2], "stone cards always score");
        let d = det(
            "KS KH 3C 9D",
            DetectRules {
                splash: true,
                ..NONE
            },
        );
        assert_eq!(d.scoring, vec![0, 1, 2, 3]);
        let d = det("3Cs 4Ds", NONE);
        assert_eq!(d.hand, HandType::HighCard);
        assert_eq!(d.scoring, vec![0, 1]);
    }

    /// Independent reference: classic poker rules on standard cards.
    fn reference(cs: &[&Card]) -> (HandType, Vec<usize>) {
        let n = cs.len();
        let mut count = [0usize; 15];
        for c in cs {
            count[c.rank.id() as usize] += 1;
        }
        let mut by_count: Vec<(usize, usize)> = (2..=14)
            .filter(|&r| count[r] > 0)
            .map(|r| (count[r], r))
            .collect();
        by_count.sort_by(|a, b| b.cmp(a));
        let flush = n == 5 && cs.iter().all(|c| c.suit == cs[0].suit);
        let mut ranks: Vec<usize> = cs.iter().map(|c| c.rank.id() as usize).collect();
        ranks.sort();
        let straight = n == 5
            && by_count.len() == 5
            && (ranks[4] - ranks[0] == 4 || ranks == vec![2, 3, 4, 5, 14]);
        let pos_of = |r: usize| -> Vec<usize> {
            (0..n).filter(|&i| cs[i].rank.id() as usize == r).collect()
        };
        let all: Vec<usize> = (0..n).collect();
        match (by_count[0].0, by_count.get(1).map(|x| x.0)) {
            _ if straight && flush => (HandType::StraightFlush, all),
            (4, _) => (HandType::FourOfAKind, pos_of(by_count[0].1)),
            (3, Some(2)) => (HandType::FullHouse, all),
            _ if flush => (HandType::Flush, all),
            _ if straight => (HandType::Straight, all),
            (3, _) => (HandType::ThreeOfAKind, pos_of(by_count[0].1)),
            (2, Some(2)) => {
                let mut v = pos_of(by_count[0].1);
                v.extend(pos_of(by_count[1].1));
                v.sort();
                (HandType::TwoPair, v)
            }
            (2, _) => (HandType::Pair, pos_of(by_count[0].1)),
            _ => (HandType::HighCard, pos_of(*ranks.last().unwrap())),
        }
    }

    #[test]
    fn exhaustive_against_reference_on_standard_cards() {
        let deck: Vec<Card> = crate::card::standard_card_order()
            .into_iter()
            .enumerate()
            .map(|(i, (s, r))| Card::new(i as u64, s, r))
            .collect();
        let mut checked = 0usize;
        let mut idx = [0usize; 5];
        for k in 1..=5usize {
            // all k-subsets of 52, in increasing index order
            for (j, slot) in idx.iter_mut().enumerate().take(k) {
                *slot = j;
            }
            loop {
                let hand: Vec<&Card> = idx[..k].iter().map(|&i| &deck[i]).collect();
                let got = detect(&hand, NONE);
                let (want, want_scoring) = reference(&hand);
                assert_eq!(
                    got.hand,
                    want,
                    "{:?}",
                    hand.iter().map(|c| c.key()).collect::<Vec<_>>()
                );
                assert_eq!(got.scoring, want_scoring);
                checked += 1;
                // next combination
                let mut i = k;
                loop {
                    if i == 0 {
                        break;
                    }
                    i -= 1;
                    if idx[i] < 52 - k + i {
                        idx[i] += 1;
                        for j in i + 1..k {
                            idx[j] = idx[j - 1] + 1;
                        }
                        i = usize::MAX;
                        break;
                    }
                }
                if i != usize::MAX {
                    break;
                }
            }
        }
        assert_eq!(checked, 52 + 1326 + 22100 + 270725 + 2598960);
    }
}
