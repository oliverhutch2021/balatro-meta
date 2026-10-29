//! Owned jokers: their state and the effects the engine has implemented so
//! far. Each effect follows the game's joker routine branch by branch
//! (`rules/scoring.md`); jokers not yet implemented do nothing and are
//! listed by [`is_implemented`].

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
    /// A counter whose meaning depends on the joker (Seltzer's hands left,
    /// Rocket's payout, Loyalty Card's start, Turtle Bean's hand size).
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

/// Jokers whose effects the engine implements so far. Others can be held but
/// do nothing yet; studies must not use them until phase 2 adds them.
pub fn is_implemented(kind: JokerKind) -> bool {
    use JokerKind as J;
    matches!(
        kind,
        J::Joker
            | J::GreedyJoker
            | J::LustyJoker
            | J::WrathfulJoker
            | J::GluttonousJoker
            | J::JollyJoker
            | J::ZanyJoker
            | J::MadJoker
            | J::CrazyJoker
            | J::DrollJoker
            | J::SlyJoker
            | J::WilyJoker
            | J::CleverJoker
            | J::DeviousJoker
            | J::CraftyJoker
            | J::HalfJoker
            | J::FourFingers
            | J::Mime
            | J::CreditCard
            | J::Banner
            | J::MysticSummit
            | J::Misprint
            | J::Dusk
            | J::RaisedFist
            | J::Fibonacci
            | J::ScaryFace
            | J::AbstractJoker
            | J::Hack
            | J::Pareidolia
            | J::GrosMichel
            | J::EvenSteven
            | J::OddTodd
            | J::Scholar
            | J::BusinessCard
            | J::Supernova
            | J::RideTheBus
            | J::SpaceJoker
            | J::Runner
            | J::IceCream
            | J::Splash
            | J::BlueJoker
            | J::Hiker
            | J::GreenJoker
            | J::Cavendish
            | J::CardSharp
            | J::SquareJoker
            | J::Vampire
            | J::Shortcut
            | J::Baron
            | J::Obelisk
            | J::MidasMask
            | J::Photograph
            | J::Erosion
            | J::ReservedParking
            | J::Juggler
            | J::Drunkard
            | J::GoldenJoker
            | J::LuckyCat
            | J::BaseballCard
            | J::Bull
            | J::Popcorn
            | J::SpareTrousers
            | J::AncientJoker
            | J::WalkieTalkie
            | J::Seltzer
            | J::SmileyFace
            | J::GoldenTicket
            | J::MrBones
            | J::Acrobat
            | J::SockAndBuskin
            | J::SmearedJoker
            | J::HangingChad
            | J::RoughGem
            | J::Bloodstone
            | J::Arrowhead
            | J::OnyxAgate
            | J::Blueprint
            | J::MerryAndy
            | J::OopsAllSixes
            | J::TheIdol
            | J::Matador
            | J::TheDuo
            | J::TheTrio
            | J::TheFamily
            | J::TheOrder
            | J::TheTribe
            | J::Stuntman
            | J::Brainstorm
            | J::ShootTheMoon
            | J::Bootstraps
            | J::Triboulet
            | J::Chicot
            | J::Troubadour
            | J::TurtleBean
    )
}
