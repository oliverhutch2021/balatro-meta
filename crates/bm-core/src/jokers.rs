//! Owned jokers: their state, starting values and fixed numbers. Their
//! effects live in `scoring.rs` (the scoring contexts) and
//! `joker_events.rs` (blind start, discards, the shop, packs, sales), each
//! following the game's joker routine branch by branch (`rules/scoring.md`,
//! `rules/jokers_1_76.md`, `rules/jokers_77_150.md`).

use crate::card::Edition;
use crate::joker_data::JokerKind;
use crate::poker::HandType;

/// An owned joker.
#[derive(Clone, Debug, PartialEq)]
pub struct Joker {
    pub kind: JokerKind,
    /// Creation order (decides picks among jokers).
    pub sort_id: u64,
    pub edition: Edition,
    pub eternal: bool,
    pub perishable: bool,
    /// Rounds left before a Perishable joker is debuffed.
    pub perish_tally: i32,
    pub rental: bool,
    pub debuffed: bool,
    /// Face down (Amber Acorn): hidden from the player.
    pub face_down: bool,
    /// Whether its add-to-deck effects (hand size, discards, ...) are applied.
    pub added: bool,
    /// Extra sell value (Egg, Gift Card).
    pub extra_value: i64,
    /// Price and stored sell value (`Card:set_cost`).
    pub cost: i64,
    pub sell_cost: i64,
    /// `ability.mult`: a joker's own growing or fixed mult.
    pub mult: f64,
    /// `ability.x_mult`: a joker's own growing or fixed X-mult.
    pub x_mult: f64,
    /// Growing chips (Runner, Ice Cream, Square Joker, Castle, Wee Joker).
    pub chips: f64,
    /// A counter whose meaning depends on the joker: Seltzer's hands left,
    /// Rocket's payout, Loyalty Card's start (hands played when it was
    /// made), Turtle Bean's hand size, Yorick's discards left, Invisible
    /// Joker's rounds, and the per-frame deck tallies of Steel Joker, Stone
    /// Joker, Driver's License and Cloud 9.
    pub counter: i64,
    /// A target hand (To Do List).
    pub hand: Option<HandType>,
    /// Destroyed by its own effect (Ice Cream, Seltzer, Popcorn, Gros
    /// Michel, Mr. Bones, ...): leaves once the current step is over.
    pub removing: bool,
    /// Marked for destruction at this blind start (Ceremonial Dagger,
    /// Madness): does nothing more at it.
    pub sliced: bool,
}

impl Joker {
    pub fn new(kind: JokerKind, sort_id: u64) -> Joker {
        use JokerKind as J;
        let mult = match kind {
            J::Joker => 4.0,
            J::Popcorn => 20.0,
            J::Swashbuckler => 1.0,
            _ => 0.0,
        };
        let x_mult = match kind {
            J::TheDuo | J::TheTribe | J::Ramen => 2.0,
            J::TheTrio | J::TheOrder => 3.0,
            J::TheFamily => 4.0,
            _ => 1.0,
        };
        let chips = match kind {
            J::IceCream => 100.0,
            _ => 0.0,
        };
        let counter = match kind {
            J::Seltzer => 10,
            J::Rocket => 1,
            J::TurtleBean => 5,
            J::Yorick => 23,
            _ => 0,
        };
        Joker {
            kind,
            sort_id,
            edition: Edition::None,
            eternal: false,
            perishable: false,
            perish_tally: 5,
            rental: false,
            debuffed: false,
            face_down: false,
            added: false,
            extra_value: 0,
            cost: kind.info().cost,
            sell_cost: (kind.info().cost / 2).max(1),
            mult,
            x_mult,
            chips,
            counter,
            hand: None,
            removing: false,
            sliced: false,
        }
    }

    pub fn rarity(&self) -> u8 {
        self.kind.rarity()
    }
}

/// The hand type a type-joker needs in the played hand (`ability.type`).
pub fn joker_hand_type(kind: JokerKind) -> Option<HandType> {
    use JokerKind as J;
    Some(match kind {
        J::JollyJoker | J::SlyJoker | J::TheDuo => HandType::Pair,
        J::ZanyJoker | J::WilyJoker | J::TheTrio => HandType::ThreeOfAKind,
        J::MadJoker | J::CleverJoker => HandType::TwoPair,
        J::CrazyJoker | J::DeviousJoker | J::TheOrder => HandType::Straight,
        J::DrollJoker | J::CraftyJoker | J::TheTribe => HandType::Flush,
        J::TheFamily => HandType::FourOfAKind,
        _ => return None,
    })
}

/// `ability.t_mult`: +mult when the played hand contains the joker's type.
pub fn joker_t_mult(kind: JokerKind) -> f64 {
    use JokerKind as J;
    match kind {
        J::JollyJoker => 8.0,
        J::ZanyJoker | J::CrazyJoker => 12.0,
        J::MadJoker | J::DrollJoker => 10.0,
        _ => 0.0,
    }
}

/// `ability.t_chips`: +chips when the played hand contains the joker's type.
pub fn joker_t_chips(kind: JokerKind) -> f64 {
    use JokerKind as J;
    match kind {
        J::SlyJoker => 50.0,
        J::WilyJoker | J::DeviousJoker => 100.0,
        J::CleverJoker | J::CraftyJoker => 80.0,
        _ => 0.0,
    }
}

/// `ability.h_size`, `ability.d_size` and the named add-to-deck effects that
/// change round resources: (hand size, discards per round, hands per round).
pub fn joker_resource_change(j: &Joker) -> (i32, i32, i32) {
    use JokerKind as J;
    match j.kind {
        J::Juggler => (1, 0, 0),
        J::Drunkard => (0, 1, 0),
        J::MerryAndy => (-1, 3, 0),
        J::Stuntman => (-2, 0, 0),
        J::Troubadour => (2, 0, -1),
        J::TurtleBean => (j.counter as i32, 0, 0),
        _ => (0, 0, 0),
    }
}

/// Whether the engine implements this joker's effects. Every one of the
/// 150 is implemented; kept so callers can still ask.
pub fn is_implemented(_kind: JokerKind) -> bool {
    true
}
