//! Consumables, vouchers and booster packs: keys, pool order, costs and the
//! numbers their effects use (`rules/consumables.md`, `rules/vouchers_tags.md`,
//! `rules/shop.md` section 3). Numbers only, from the game's data.

use crate::card::{Enhancement, Seal, Suit};
use crate::poker::HandType;

/// A consumable's set.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ConsumableSet {
    Tarot,
    Planet,
    Spectral,
}

/// Every consumable: the 22 Tarots, 12 Planets and 18 Spectrals, each set in
/// its pool order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(u8)]
pub enum ConsumableKind {
    Fool,
    Magician,
    HighPriestess,
    Empress,
    Emperor,
    Hierophant,
    Lovers,
    Chariot,
    Justice,
    Hermit,
    WheelOfFortune,
    Strength,
    HangedMan,
    Death,
    Temperance,
    Devil,
    Tower,
    Star,
    Moon,
    Sun,
    Judgement,
    World,
    Mercury,
    Venus,
    Earth,
    Mars,
    Jupiter,
    Saturn,
    Uranus,
    Neptune,
    Pluto,
    PlanetX,
    Ceres,
    Eris,
    Familiar,
    Grim,
    Incantation,
    Talisman,
    Aura,
    Wraith,
    Sigil,
    Ouija,
    Ectoplasm,
    Immolate,
    Ankh,
    DejaVu,
    Hex,
    Trance,
    Medium,
    Cryptid,
    Soul,
    BlackHole,
}

pub const N_CONSUMABLES: usize = 52;

const CONSUMABLE_KEYS: [(&str, &str); N_CONSUMABLES] = [
    ("c_fool", "The Fool"),
    ("c_magician", "The Magician"),
    ("c_high_priestess", "The High Priestess"),
    ("c_empress", "The Empress"),
    ("c_emperor", "The Emperor"),
    ("c_heirophant", "The Hierophant"),
    ("c_lovers", "The Lovers"),
    ("c_chariot", "The Chariot"),
    ("c_justice", "Justice"),
    ("c_hermit", "The Hermit"),
    ("c_wheel_of_fortune", "The Wheel of Fortune"),
    ("c_strength", "Strength"),
    ("c_hanged_man", "The Hanged Man"),
    ("c_death", "Death"),
    ("c_temperance", "Temperance"),
    ("c_devil", "The Devil"),
    ("c_tower", "The Tower"),
    ("c_star", "The Star"),
    ("c_moon", "The Moon"),
    ("c_sun", "The Sun"),
    ("c_judgement", "Judgement"),
    ("c_world", "The World"),
    ("c_mercury", "Mercury"),
    ("c_venus", "Venus"),
    ("c_earth", "Earth"),
    ("c_mars", "Mars"),
    ("c_jupiter", "Jupiter"),
    ("c_saturn", "Saturn"),
    ("c_uranus", "Uranus"),
    ("c_neptune", "Neptune"),
    ("c_pluto", "Pluto"),
    ("c_planet_x", "Planet X"),
    ("c_ceres", "Ceres"),
    ("c_eris", "Eris"),
    ("c_familiar", "Familiar"),
    ("c_grim", "Grim"),
    ("c_incantation", "Incantation"),
    ("c_talisman", "Talisman"),
    ("c_aura", "Aura"),
    ("c_wraith", "Wraith"),
    ("c_sigil", "Sigil"),
    ("c_ouija", "Ouija"),
    ("c_ectoplasm", "Ectoplasm"),
    ("c_immolate", "Immolate"),
    ("c_ankh", "Ankh"),
    ("c_deja_vu", "Deja Vu"),
    ("c_hex", "Hex"),
    ("c_trance", "Trance"),
    ("c_medium", "Medium"),
    ("c_cryptid", "Cryptid"),
    ("c_soul", "The Soul"),
    ("c_black_hole", "Black Hole"),
];

impl ConsumableKind {
    pub const ALL: [ConsumableKind; N_CONSUMABLES] = {
        use ConsumableKind::*;
        [
            Fool,
            Magician,
            HighPriestess,
            Empress,
            Emperor,
            Hierophant,
            Lovers,
            Chariot,
            Justice,
            Hermit,
            WheelOfFortune,
            Strength,
            HangedMan,
            Death,
            Temperance,
            Devil,
            Tower,
            Star,
            Moon,
            Sun,
            Judgement,
            World,
            Mercury,
            Venus,
            Earth,
            Mars,
            Jupiter,
            Saturn,
            Uranus,
            Neptune,
            Pluto,
            PlanetX,
            Ceres,
            Eris,
            Familiar,
            Grim,
            Incantation,
            Talisman,
            Aura,
            Wraith,
            Sigil,
            Ouija,
            Ectoplasm,
            Immolate,
            Ankh,
            DejaVu,
            Hex,
            Trance,
            Medium,
            Cryptid,
            Soul,
            BlackHole,
        ]
    };

    pub fn index(self) -> usize {
        self as usize
    }

    pub fn key(self) -> &'static str {
        CONSUMABLE_KEYS[self.index()].0
    }

    pub fn name(self) -> &'static str {
        CONSUMABLE_KEYS[self.index()].1
    }

    pub fn from_key(key: &str) -> Option<ConsumableKind> {
        CONSUMABLE_KEYS
            .iter()
            .position(|k| k.0 == key)
            .map(|i| ConsumableKind::ALL[i])
    }

    pub fn set(self) -> ConsumableSet {
        match self.index() {
            0..=21 => ConsumableSet::Tarot,
            22..=33 => ConsumableSet::Planet,
            _ => ConsumableSet::Spectral,
        }
    }

    /// All cards of a set, in pool order.
    pub fn of_set(set: ConsumableSet) -> &'static [ConsumableKind] {
        match set {
            ConsumableSet::Tarot => &Self::ALL[0..22],
            ConsumableSet::Planet => &Self::ALL[22..34],
            ConsumableSet::Spectral => &Self::ALL[34..52],
        }
    }

    /// Base cost: Tarot and Planet 3, Spectral 4.
    pub fn base_cost(self) -> i64 {
        if self.set() == ConsumableSet::Spectral {
            4
        } else {
            3
        }
    }

    /// The hand a Planet levels.
    pub fn planet_hand(self) -> Option<HandType> {
        use ConsumableKind::*;
        Some(match self {
            Mercury => HandType::Pair,
            Venus => HandType::ThreeOfAKind,
            Earth => HandType::FullHouse,
            Mars => HandType::FourOfAKind,
            Jupiter => HandType::Flush,
            Saturn => HandType::Straight,
            Uranus => HandType::TwoPair,
            Neptune => HandType::StraightFlush,
            Pluto => HandType::HighCard,
            PlanetX => HandType::FiveOfAKind,
            Ceres => HandType::FlushHouse,
            Eris => HandType::FlushFive,
            _ => return None,
        })
    }

    /// The Planet for a hand.
    pub fn planet_for(h: HandType) -> ConsumableKind {
        *Self::ALL[22..34]
            .iter()
            .find(|c| c.planet_hand() == Some(h))
            .expect("every hand has a planet")
    }

    /// (min, max) selected hand cards for cards that act on the selection.
    pub fn selection_limits(self) -> Option<(usize, usize)> {
        use ConsumableKind::*;
        Some(match self {
            Magician | Empress | Hierophant | Strength | HangedMan => (1, 2),
            Lovers | Chariot | Justice | Devil | Tower => (1, 1),
            Death => (2, 2),
            Star | Moon | Sun | World => (1, 3),
            Talisman | DejaVu | Trance | Medium | Cryptid => (1, 1),
            _ => return None,
        })
    }

    /// The enhancement a Tarot gives its selected cards.
    pub fn tarot_enhancement(self) -> Option<Enhancement> {
        use ConsumableKind::*;
        Some(match self {
            Magician => Enhancement::Lucky,
            Empress => Enhancement::Mult,
            Hierophant => Enhancement::Bonus,
            Lovers => Enhancement::Wild,
            Chariot => Enhancement::Steel,
            Justice => Enhancement::Glass,
            Devil => Enhancement::Gold,
            Tower => Enhancement::Stone,
            _ => return None,
        })
    }

    /// The suit a Tarot gives its selected cards.
    pub fn tarot_suit(self) -> Option<Suit> {
        use ConsumableKind::*;
        Some(match self {
            Star => Suit::Diamonds,
            Moon => Suit::Clubs,
            Sun => Suit::Hearts,
            World => Suit::Spades,
            _ => return None,
        })
    }

    /// The seal a Spectral gives its selected card.
    pub fn spectral_seal(self) -> Option<Seal> {
        use ConsumableKind::*;
        Some(match self {
            Talisman => Seal::Gold,
            DejaVu => Seal::Red,
            Trance => Seal::Blue,
            Medium => Seal::Purple,
            _ => return None,
        })
    }
}

/// Every voucher, in pool order (tier 1 odd, tier 2 even).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(u8)]
pub enum VoucherKind {
    Overstock,
    OverstockPlus,
    ClearanceSale,
    Liquidation,
    Hone,
    GlowUp,
    RerollSurplus,
    RerollGlut,
    CrystalBall,
    OmenGlobe,
    Telescope,
    Observatory,
    Grabber,
    NachoTong,
    Wasteful,
    Recyclomancy,
    TarotMerchant,
    TarotTycoon,
    PlanetMerchant,
    PlanetTycoon,
    SeedMoney,
    MoneyTree,
    Blank,
    Antimatter,
    MagicTrick,
    Illusion,
    Hieroglyph,
    Petroglyph,
    DirectorsCut,
    Retcon,
    PaintBrush,
    Palette,
}

pub const N_VOUCHERS: usize = 32;

const VOUCHER_KEYS: [(&str, &str); N_VOUCHERS] = [
    ("v_overstock_norm", "Overstock"),
    ("v_overstock_plus", "Overstock Plus"),
    ("v_clearance_sale", "Clearance Sale"),
    ("v_liquidation", "Liquidation"),
    ("v_hone", "Hone"),
    ("v_glow_up", "Glow Up"),
    ("v_reroll_surplus", "Reroll Surplus"),
    ("v_reroll_glut", "Reroll Glut"),
    ("v_crystal_ball", "Crystal Ball"),
    ("v_omen_globe", "Omen Globe"),
    ("v_telescope", "Telescope"),
    ("v_observatory", "Observatory"),
    ("v_grabber", "Grabber"),
    ("v_nacho_tong", "Nacho Tong"),
    ("v_wasteful", "Wasteful"),
    ("v_recyclomancy", "Recyclomancy"),
    ("v_tarot_merchant", "Tarot Merchant"),
    ("v_tarot_tycoon", "Tarot Tycoon"),
    ("v_planet_merchant", "Planet Merchant"),
    ("v_planet_tycoon", "Planet Tycoon"),
    ("v_seed_money", "Seed Money"),
    ("v_money_tree", "Money Tree"),
    ("v_blank", "Blank"),
    ("v_antimatter", "Antimatter"),
    ("v_magic_trick", "Magic Trick"),
    ("v_illusion", "Illusion"),
    ("v_hieroglyph", "Hieroglyph"),
    ("v_petroglyph", "Petroglyph"),
    ("v_directors_cut", "Director's Cut"),
    ("v_retcon", "Retcon"),
    ("v_paint_brush", "Paint Brush"),
    ("v_palette", "Palette"),
];

impl VoucherKind {
    pub fn all() -> impl Iterator<Item = VoucherKind> {
        (0..N_VOUCHERS).map(VoucherKind::from_index)
    }

    pub fn from_index(i: usize) -> VoucherKind {
        assert!(i < N_VOUCHERS);
        // SAFETY: repr(u8), discriminants 0..32 in declaration order.
        unsafe { std::mem::transmute::<u8, VoucherKind>(i as u8) }
    }

    pub fn index(self) -> usize {
        self as usize
    }

    pub fn key(self) -> &'static str {
        VOUCHER_KEYS[self.index()].0
    }

    pub fn name(self) -> &'static str {
        VOUCHER_KEYS[self.index()].1
    }

    pub fn from_key(key: &str) -> Option<VoucherKind> {
        VOUCHER_KEYS
            .iter()
            .position(|k| k.0 == key)
            .map(VoucherKind::from_index)
    }

    /// The tier-1 voucher a tier-2 voucher needs.
    pub fn requires(self) -> Option<VoucherKind> {
        let i = self.index();
        (i % 2 == 1).then(|| VoucherKind::from_index(i - 1))
    }

    pub fn base_cost(self) -> i64 {
        10
    }
}

/// The five kinds of booster pack.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PackKind {
    Arcana,
    Celestial,
    Standard,
    Buffoon,
    Spectral,
}

/// One booster pack (art variants are separate entries, as in the pool).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BoosterInfo {
    pub key: &'static str,
    pub kind: PackKind,
    pub weight: f64,
    /// Cards shown.
    pub size: usize,
    /// Cards that can be taken.
    pub choose: usize,
    pub cost: i64,
}

macro_rules! booster {
    ($key:expr, $kind:ident, $w:expr, $size:expr, $choose:expr, $cost:expr) => {
        BoosterInfo {
            key: $key,
            kind: PackKind::$kind,
            weight: $w,
            size: $size,
            choose: $choose,
            cost: $cost,
        }
    };
}

/// The 32 boosters in pool order.
pub const BOOSTERS: [BoosterInfo; 32] = [
    booster!("p_arcana_normal_1", Arcana, 1.0, 3, 1, 4),
    booster!("p_arcana_normal_2", Arcana, 1.0, 3, 1, 4),
    booster!("p_arcana_normal_3", Arcana, 1.0, 3, 1, 4),
    booster!("p_arcana_normal_4", Arcana, 1.0, 3, 1, 4),
    booster!("p_arcana_jumbo_1", Arcana, 1.0, 5, 1, 6),
    booster!("p_arcana_jumbo_2", Arcana, 1.0, 5, 1, 6),
    booster!("p_arcana_mega_1", Arcana, 0.25, 5, 2, 8),
    booster!("p_arcana_mega_2", Arcana, 0.25, 5, 2, 8),
    booster!("p_celestial_normal_1", Celestial, 1.0, 3, 1, 4),
    booster!("p_celestial_normal_2", Celestial, 1.0, 3, 1, 4),
    booster!("p_celestial_normal_3", Celestial, 1.0, 3, 1, 4),
    booster!("p_celestial_normal_4", Celestial, 1.0, 3, 1, 4),
    booster!("p_celestial_jumbo_1", Celestial, 1.0, 5, 1, 6),
    booster!("p_celestial_jumbo_2", Celestial, 1.0, 5, 1, 6),
    booster!("p_celestial_mega_1", Celestial, 0.25, 5, 2, 8),
    booster!("p_celestial_mega_2", Celestial, 0.25, 5, 2, 8),
    booster!("p_standard_normal_1", Standard, 1.0, 3, 1, 4),
    booster!("p_standard_normal_2", Standard, 1.0, 3, 1, 4),
    booster!("p_standard_normal_3", Standard, 1.0, 3, 1, 4),
    booster!("p_standard_normal_4", Standard, 1.0, 3, 1, 4),
    booster!("p_standard_jumbo_1", Standard, 1.0, 5, 1, 6),
    booster!("p_standard_jumbo_2", Standard, 1.0, 5, 1, 6),
    booster!("p_standard_mega_1", Standard, 0.25, 5, 2, 8),
    booster!("p_standard_mega_2", Standard, 0.25, 5, 2, 8),
    booster!("p_buffoon_normal_1", Buffoon, 0.6, 2, 1, 4),
    booster!("p_buffoon_normal_2", Buffoon, 0.6, 2, 1, 4),
    booster!("p_buffoon_jumbo_1", Buffoon, 0.6, 4, 1, 6),
    booster!("p_buffoon_mega_1", Buffoon, 0.15, 4, 2, 8),
    booster!("p_spectral_normal_1", Spectral, 0.3, 2, 1, 4),
    booster!("p_spectral_normal_2", Spectral, 0.3, 2, 1, 4),
    booster!("p_spectral_jumbo_1", Spectral, 0.3, 4, 1, 6),
    booster!("p_spectral_mega_1", Spectral, 0.07, 4, 2, 8),
];

/// Index into [`BOOSTERS`].
pub type BoosterId = u8;

pub fn booster_by_key(key: &str) -> Option<BoosterId> {
    BOOSTERS
        .iter()
        .position(|b| b.key == key)
        .map(|i| i as BoosterId)
}

/// The enhancement pool in its order (Standard packs, Illusion, Familiar
/// and friends use it without Stone).
pub const ENHANCEMENT_POOL: [Enhancement; 8] = [
    Enhancement::Bonus,
    Enhancement::Mult,
    Enhancement::Wild,
    Enhancement::Glass,
    Enhancement::Steel,
    Enhancement::Stone,
    Enhancement::Gold,
    Enhancement::Lucky,
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tables_line_up() {
        for (i, c) in ConsumableKind::ALL.iter().enumerate() {
            assert_eq!(c.index(), i);
            assert_eq!(ConsumableKind::from_key(c.key()), Some(*c));
        }
        for v in VoucherKind::all() {
            assert_eq!(VoucherKind::from_key(v.key()), Some(v));
        }
        assert_eq!(
            VoucherKind::OverstockPlus.requires(),
            Some(VoucherKind::Overstock)
        );
        assert_eq!(
            VoucherKind::Palette.requires(),
            Some(VoucherKind::PaintBrush)
        );
        assert_eq!(VoucherKind::Blank.requires(), None);
        let w: f64 = BOOSTERS.iter().map(|b| b.weight).sum();
        assert!((w - 22.42).abs() < 1e-9);
        for h in HandType::ALL {
            assert_eq!(ConsumableKind::planet_for(h).planet_hand(), Some(h));
        }
    }
}
