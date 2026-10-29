//! Playing cards: suits, ranks, enhancements, editions, seals, and the
//! orderings the game uses for them (`rules/hands.md`, `rules/scoring.md`,
//! `rules/run_setup.md`).

use std::fmt;

/// A suit. The declaration order is the order flushes are checked in
/// (Spades, Hearts, Clubs, Diamonds).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(u8)]
pub enum Suit {
    Spades = 0,
    Hearts = 1,
    Clubs = 2,
    Diamonds = 3,
}

impl Suit {
    /// Flush check order.
    pub const ALL: [Suit; 4] = [Suit::Spades, Suit::Hearts, Suit::Clubs, Suit::Diamonds];

    /// Hearts and Diamonds are one colour, Spades and Clubs the other.
    pub fn is_red(self) -> bool {
        matches!(self, Suit::Hearts | Suit::Diamonds)
    }

    /// Letter used in card keys (`C_2`, `S_A`, ...).
    pub fn letter(self) -> char {
        match self {
            Suit::Spades => 'S',
            Suit::Hearts => 'H',
            Suit::Clubs => 'C',
            Suit::Diamonds => 'D',
        }
    }

    /// Tie-break value in sort keys: Diamonds 0.01, Clubs 0.02, Hearts 0.03, Spades 0.04.
    pub fn sort_value(self) -> f64 {
        match self {
            Suit::Diamonds => 0.01,
            Suit::Clubs => 0.02,
            Suit::Hearts => 0.03,
            Suit::Spades => 0.04,
        }
    }

    /// Original-suit value: a tenth of [`Suit::sort_value`].
    pub fn original_value(self) -> f64 {
        match self {
            Suit::Diamonds => 0.001,
            Suit::Clubs => 0.002,
            Suit::Hearts => 0.003,
            Suit::Spades => 0.004,
        }
    }

    pub fn index(self) -> usize {
        self as usize
    }
}

/// A rank. The discriminant is the game's rank id (Ace high = 14).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(u8)]
pub enum Rank {
    Two = 2,
    Three = 3,
    Four = 4,
    Five = 5,
    Six = 6,
    Seven = 7,
    Eight = 8,
    Nine = 9,
    Ten = 10,
    Jack = 11,
    Queen = 12,
    King = 13,
    Ace = 14,
}

impl Rank {
    pub const ALL: [Rank; 13] = [
        Rank::Two,
        Rank::Three,
        Rank::Four,
        Rank::Five,
        Rank::Six,
        Rank::Seven,
        Rank::Eight,
        Rank::Nine,
        Rank::Ten,
        Rank::Jack,
        Rank::Queen,
        Rank::King,
        Rank::Ace,
    ];

    /// The game's rank id: 2 to 10, Jack 11, Queen 12, King 13, Ace 14.
    pub fn id(self) -> u8 {
        self as u8
    }

    pub fn from_id(id: u8) -> Option<Rank> {
        match id {
            2..=14 => Some(Rank::ALL[(id - 2) as usize]),
            _ => None,
        }
    }

    /// Chips when scored: 2 to 10 their number, J/Q/K 10, Ace 11.
    pub fn chips(self) -> i64 {
        match self {
            Rank::Jack | Rank::Queen | Rank::King => 10,
            Rank::Ace => 11,
            r => r.id() as i64,
        }
    }

    /// Jack, Queen or King (before Pareidolia).
    pub fn is_face(self) -> bool {
        matches!(self, Rank::Jack | Rank::Queen | Rank::King)
    }

    /// Face bonus in sort keys: J 0.1, Q 0.2, K 0.3, A 0.4.
    pub fn face_value(self) -> f64 {
        match self {
            Rank::Jack => 0.1,
            Rank::Queen => 0.2,
            Rank::King => 0.3,
            Rank::Ace => 0.4,
            _ => 0.0,
        }
    }

    /// Letter used in card keys: 2-9, T, J, Q, K, A.
    pub fn letter(self) -> char {
        match self {
            Rank::Ten => 'T',
            Rank::Jack => 'J',
            Rank::Queen => 'Q',
            Rank::King => 'K',
            Rank::Ace => 'A',
            r => (b'0' + r.id()) as char,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Enhancement {
    #[default]
    None,
    Bonus,
    Mult,
    Wild,
    Glass,
    Steel,
    Stone,
    Gold,
    Lucky,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Edition {
    #[default]
    None,
    Foil,
    Holographic,
    Polychrome,
    Negative,
}

impl Edition {
    /// Additive chips of the edition (Foil 50).
    pub fn chips(self) -> f64 {
        if self == Edition::Foil {
            50.0
        } else {
            0.0
        }
    }

    /// Additive mult of the edition (Holographic 10).
    pub fn mult(self) -> f64 {
        if self == Edition::Holographic {
            10.0
        } else {
            0.0
        }
    }

    /// Multiplicative mult of the edition (Polychrome 1.5), or 1.
    pub fn x_mult(self) -> f64 {
        if self == Edition::Polychrome {
            1.5
        } else {
            1.0
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Seal {
    #[default]
    None,
    Gold,
    Red,
    Blue,
    Purple,
}

/// The hand's sort mode: by rank (the default) or by suit, both descending.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum SortMode {
    #[default]
    Rank,
    Suit,
}

/// A playing card. `sort_id` is its place in the game's global creation
/// order, which decides every shuffle and every random pick among cards.
#[derive(Clone, Debug, PartialEq)]
pub struct Card {
    pub sort_id: u64,
    pub suit: Suit,
    pub rank: Rank,
    /// The suit the card had when first made (survives suit changes).
    pub original_suit: Suit,
    pub enhancement: Enhancement,
    pub edition: Edition,
    pub seal: Seal,
    /// Permanent extra chips (Hiker).
    pub perma_bonus: i64,
    pub debuffed: bool,
    pub face_down: bool,
    /// Played earlier in this ante (read by The Pillar).
    pub played_this_ante: bool,
    pub times_played: u32,
    /// Forced into the selection by Cerulean Bell.
    pub forced: bool,
}

impl Card {
    pub fn new(sort_id: u64, suit: Suit, rank: Rank) -> Self {
        Card {
            sort_id,
            suit,
            rank,
            original_suit: suit,
            enhancement: Enhancement::None,
            edition: Edition::None,
            seal: Seal::None,
            perma_bonus: 0,
            debuffed: false,
            face_down: false,
            played_this_ante: false,
            times_played: 0,
            forced: false,
        }
    }

    pub fn is_stone(&self) -> bool {
        self.enhancement == Enhancement::Stone
    }

    /// The rank as poker-hand detection sees it: none for a Stone Card.
    /// Debuffs don't change it.
    pub fn detect_rank(&self) -> Option<Rank> {
        if self.is_stone() {
            None
        } else {
            Some(self.rank)
        }
    }

    /// Flush test (`rules/hands.md` section 2).
    pub fn counts_for_flush(&self, suit: Suit, smeared: bool) -> bool {
        if self.is_stone() {
            return false;
        }
        if self.enhancement == Enhancement::Wild && !self.debuffed {
            return true;
        }
        if smeared {
            return self.suit.is_red() == suit.is_red();
        }
        self.suit == suit
    }

    /// Suit test used by jokers during scoring (`rules/scoring.md` 12.8).
    pub fn has_suit(&self, suit: Suit, smeared: bool) -> bool {
        if self.debuffed || self.is_stone() {
            return false;
        }
        if self.enhancement == Enhancement::Wild {
            return true;
        }
        if smeared {
            return self.suit.is_red() == suit.is_red();
        }
        self.suit == suit
    }

    /// Suit test used by boss blinds: ignores debuff (so Wild cards have
    /// every suit even when debuffed); Stone cards have none.
    pub fn has_suit_for_boss(&self, suit: Suit, smeared: bool) -> bool {
        if self.is_stone() {
            return false;
        }
        if self.enhancement == Enhancement::Wild {
            return true;
        }
        if smeared {
            return self.suit.is_red() == suit.is_red();
        }
        self.suit == suit
    }

    /// Face test used by jokers: a debuffed card is never a face card; with
    /// Pareidolia every card is (Stone cards included).
    pub fn is_face(&self, pareidolia: bool) -> bool {
        if self.debuffed {
            return false;
        }
        pareidolia || (!self.is_stone() && self.rank.is_face())
    }

    /// Face test used by bosses (ignores debuff).
    pub fn is_face_for_boss(&self, pareidolia: bool) -> bool {
        pareidolia || (!self.is_stone() && self.rank.is_face())
    }

    /// Rank id as jokers compare it: none for a Stone Card (the game returns a
    /// fresh random negative number, which never matches).
    pub fn rank_id(&self) -> Option<u8> {
        self.detect_rank().map(Rank::id)
    }

    /// Chips this card adds when scored once (not debuffed).
    pub fn chip_bonus(&self) -> i64 {
        match self.enhancement {
            Enhancement::Stone => 50 + self.perma_bonus,
            Enhancement::Bonus => self.rank.chips() + 30 + self.perma_bonus,
            _ => self.rank.chips() + self.perma_bonus,
        }
    }

    /// Key for sorting the hand (descending), without the game's final
    /// creation-order term, which [`sort_hand`] applies as a tie-break.
    pub fn sort_key(&self, mode: SortMode) -> f64 {
        let m = if self.is_stone() {
            -1000.0
        } else {
            match mode {
                SortMode::Rank => 1.0,
                SortMode::Suit => 1000.0,
            }
        };
        self.rank.chips() as f64
            + self.suit.sort_value() * m
            + self.original_suit.original_value() * 0.0001 * m
            + self.rank.face_value()
    }

    /// The card's key, e.g. `H_K`.
    pub fn key(&self) -> String {
        format!("{}_{}", self.suit.letter(), self.rank.letter())
    }
}

impl fmt::Display for Card {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}{}", self.rank.letter(), self.suit.letter())?;
        if self.enhancement != Enhancement::None {
            write!(f, "[{:?}]", self.enhancement)?;
        }
        if self.edition != Edition::None {
            write!(f, "<{:?}>", self.edition)?;
        }
        if self.seal != Seal::None {
            write!(f, "{{{:?}}}", self.seal)?;
        }
        Ok(())
    }
}

/// Sort cards the way the game sorts the hand: descending by key, equal keys
/// keeping creation order (older first). `key_of` gives each id's
/// ([`Card::sort_key`], `sort_id`).
pub fn sort_hand<F>(ids: &mut [u32], key_of: F)
where
    F: Fn(u32) -> (f64, u64),
{
    ids.sort_by(|&a, &b| {
        let (ka, sa) = key_of(a);
        let (kb, sb) = key_of(b);
        kb.partial_cmp(&ka)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(sa.cmp(&sb))
    });
}

/// The 52 standard card keys in the game's sorted order: suits C, D, H, S;
/// within a suit the rank letters byte-sorted: 2-9, A, J, K, Q, T. This is
/// the starting deck's creation order and the order of every pick of a card
/// key (`front`, `erratic`, ...).
pub fn standard_card_order() -> Vec<(Suit, Rank)> {
    let suits = [Suit::Clubs, Suit::Diamonds, Suit::Hearts, Suit::Spades];
    let ranks = [
        Rank::Two,
        Rank::Three,
        Rank::Four,
        Rank::Five,
        Rank::Six,
        Rank::Seven,
        Rank::Eight,
        Rank::Nine,
        Rank::Ace,
        Rank::Jack,
        Rank::King,
        Rank::Queen,
        Rank::Ten,
    ];
    let mut out = Vec::with_capacity(52);
    for s in suits {
        for r in ranks {
            out.push((s, r));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn card_key_order_is_byte_sorted() {
        let keys: Vec<String> = standard_card_order()
            .iter()
            .map(|(s, r)| format!("{}_{}", s.letter(), r.letter()))
            .collect();
        let mut sorted = keys.clone();
        sorted.sort();
        assert_eq!(keys, sorted);
        assert_eq!(keys[0], "C_2");
        assert_eq!(keys[8], "C_A");
        assert_eq!(keys[12], "C_T");
        assert_eq!(keys[51], "S_T");
    }

    #[test]
    fn chips_and_bonuses() {
        let mut c = Card::new(1, Suit::Hearts, Rank::King);
        assert_eq!(c.chip_bonus(), 10);
        c.enhancement = Enhancement::Bonus;
        assert_eq!(c.chip_bonus(), 40);
        c.enhancement = Enhancement::Stone;
        c.perma_bonus = 5;
        assert_eq!(c.chip_bonus(), 55);
        assert_eq!(Card::new(2, Suit::Spades, Rank::Ace).chip_bonus(), 11);
    }

    #[test]
    fn rank_sort_puts_stone_last_and_spades_first_within_rank() {
        let mut cards = [
            Card::new(1, Suit::Diamonds, Rank::King),
            Card::new(2, Suit::Spades, Rank::King),
            Card::new(3, Suit::Hearts, Rank::Two),
            Card::new(4, Suit::Clubs, Rank::Ace),
        ];
        cards[2].enhancement = Enhancement::Stone;
        let mut ids: Vec<u32> = (0..4).collect();
        sort_hand(&mut ids, |i| {
            let c = &cards[i as usize];
            (c.sort_key(SortMode::Rank), c.sort_id)
        });
        let order: Vec<String> = ids.iter().map(|&i| cards[i as usize].key()).collect();
        assert_eq!(order, ["C_A", "S_K", "D_K", "H_2"]);
    }

    #[test]
    fn suit_sort_groups_by_suit_spades_first() {
        let cards = [
            Card::new(1, Suit::Diamonds, Rank::Ace),
            Card::new(2, Suit::Spades, Rank::Two),
            Card::new(3, Suit::Hearts, Rank::King),
            Card::new(4, Suit::Spades, Rank::Queen),
        ];
        let mut ids: Vec<u32> = (0..4).collect();
        sort_hand(&mut ids, |i| {
            let c = &cards[i as usize];
            (c.sort_key(SortMode::Suit), c.sort_id)
        });
        let order: Vec<String> = ids.iter().map(|&i| cards[i as usize].key()).collect();
        assert_eq!(order, ["S_Q", "S_2", "H_K", "D_A"]);
    }
}
