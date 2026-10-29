//! Blinds and bosses: chip targets, rewards and the boss draw
//! (`rules/blinds.md`).

use bm_rng::{pick_index, Streams};

/// The three blind slots of an ante, in the order they are faced.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum BlindSlot {
    Small = 0,
    Big = 1,
    Boss = 2,
}

impl BlindSlot {
    pub const ALL: [BlindSlot; 3] = [BlindSlot::Small, BlindSlot::Big, BlindSlot::Boss];

    pub fn index(self) -> usize {
        self as usize
    }
}

/// A slot's state on the blind-select screen.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum SlotState {
    #[default]
    Upcoming,
    /// On deck: the one that can be selected (or skipped).
    Select,
    Current,
    Defeated,
    Skipped,
}

/// Every boss blind. Regular bosses come first, then the five showdown
/// bosses; within each group the order is the byte order of the game's keys,
/// which is the order the `boss` pick sees them in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(u8)]
pub enum Boss {
    Arm,
    Club,
    Eye,
    Fish,
    Flint,
    Goad,
    Head,
    Hook,
    House,
    Manacle,
    Mark,
    Mouth,
    Needle,
    Ox,
    Pillar,
    Plant,
    Psychic,
    Serpent,
    Tooth,
    Wall,
    Water,
    Wheel,
    Window,
    AmberAcorn,
    CeruleanBell,
    CrimsonHeart,
    VerdantLeaf,
    VioletVessel,
}

pub const N_BOSSES: usize = 28;

impl Boss {
    pub const ALL: [Boss; N_BOSSES] = [
        Boss::Arm,
        Boss::Club,
        Boss::Eye,
        Boss::Fish,
        Boss::Flint,
        Boss::Goad,
        Boss::Head,
        Boss::Hook,
        Boss::House,
        Boss::Manacle,
        Boss::Mark,
        Boss::Mouth,
        Boss::Needle,
        Boss::Ox,
        Boss::Pillar,
        Boss::Plant,
        Boss::Psychic,
        Boss::Serpent,
        Boss::Tooth,
        Boss::Wall,
        Boss::Water,
        Boss::Wheel,
        Boss::Window,
        Boss::AmberAcorn,
        Boss::CeruleanBell,
        Boss::CrimsonHeart,
        Boss::VerdantLeaf,
        Boss::VioletVessel,
    ];

    pub fn index(self) -> usize {
        self as usize
    }

    pub fn from_index(i: usize) -> Boss {
        Boss::ALL[i]
    }

    /// The game's key.
    pub fn key(self) -> &'static str {
        match self {
            Boss::Arm => "bl_arm",
            Boss::Club => "bl_club",
            Boss::Eye => "bl_eye",
            Boss::Fish => "bl_fish",
            Boss::Flint => "bl_flint",
            Boss::Goad => "bl_goad",
            Boss::Head => "bl_head",
            Boss::Hook => "bl_hook",
            Boss::House => "bl_house",
            Boss::Manacle => "bl_manacle",
            Boss::Mark => "bl_mark",
            Boss::Mouth => "bl_mouth",
            Boss::Needle => "bl_needle",
            Boss::Ox => "bl_ox",
            Boss::Pillar => "bl_pillar",
            Boss::Plant => "bl_plant",
            Boss::Psychic => "bl_psychic",
            Boss::Serpent => "bl_serpent",
            Boss::Tooth => "bl_tooth",
            Boss::Wall => "bl_wall",
            Boss::Water => "bl_water",
            Boss::Wheel => "bl_wheel",
            Boss::Window => "bl_window",
            Boss::AmberAcorn => "bl_final_acorn",
            Boss::CeruleanBell => "bl_final_bell",
            Boss::CrimsonHeart => "bl_final_heart",
            Boss::VerdantLeaf => "bl_final_leaf",
            Boss::VioletVessel => "bl_final_vessel",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Boss::Arm => "The Arm",
            Boss::Club => "The Club",
            Boss::Eye => "The Eye",
            Boss::Fish => "The Fish",
            Boss::Flint => "The Flint",
            Boss::Goad => "The Goad",
            Boss::Head => "The Head",
            Boss::Hook => "The Hook",
            Boss::House => "The House",
            Boss::Manacle => "The Manacle",
            Boss::Mark => "The Mark",
            Boss::Mouth => "The Mouth",
            Boss::Needle => "The Needle",
            Boss::Ox => "The Ox",
            Boss::Pillar => "The Pillar",
            Boss::Plant => "The Plant",
            Boss::Psychic => "The Psychic",
            Boss::Serpent => "The Serpent",
            Boss::Tooth => "The Tooth",
            Boss::Wall => "The Wall",
            Boss::Water => "The Water",
            Boss::Wheel => "The Wheel",
            Boss::Window => "The Window",
            Boss::AmberAcorn => "Amber Acorn",
            Boss::CeruleanBell => "Cerulean Bell",
            Boss::CrimsonHeart => "Crimson Heart",
            Boss::VerdantLeaf => "Verdant Leaf",
            Boss::VioletVessel => "Violet Vessel",
        }
    }

    pub fn is_showdown(self) -> bool {
        self >= Boss::AmberAcorn
    }

    /// Lowest ante at which a regular boss can be drawn.
    pub fn min_ante(self) -> i32 {
        match self {
            Boss::Club
            | Boss::Goad
            | Boss::Head
            | Boss::Hook
            | Boss::Manacle
            | Boss::Pillar
            | Boss::Psychic
            | Boss::Window => 1,
            Boss::Eye | Boss::Tooth => 3,
            Boss::Plant => 4,
            Boss::Serpent => 5,
            Boss::Ox => 6,
            b if b.is_showdown() => 10,
            _ => 2,
        }
    }

    /// Chip-target multiplier.
    pub fn mult(self) -> f64 {
        match self {
            Boss::Wall => 4.0,
            Boss::Needle => 1.0,
            Boss::VioletVessel => 6.0,
            _ => 2.0,
        }
    }

    /// Reward for beating it.
    pub fn reward(self) -> i64 {
        if self.is_showdown() {
            8
        } else {
            5
        }
    }
}

/// Which blind is being faced or shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BlindKind {
    Small,
    Big,
    Boss(Boss),
}

impl BlindKind {
    pub fn mult(self) -> f64 {
        match self {
            BlindKind::Small => 1.0,
            BlindKind::Big => 1.5,
            BlindKind::Boss(b) => b.mult(),
        }
    }

    /// Reward at the given stake (Red Stake and up: the Small Blind pays nothing).
    pub fn reward(self, stake: u8) -> i64 {
        match self {
            BlindKind::Small => {
                if stake >= 2 {
                    0
                } else {
                    3
                }
            }
            BlindKind::Big => 4,
            BlindKind::Boss(b) => b.reward(),
        }
    }

    pub fn boss(self) -> Option<Boss> {
        match self {
            BlindKind::Boss(b) => Some(b),
            _ => None,
        }
    }

    pub fn slot(self) -> BlindSlot {
        match self {
            BlindKind::Small => BlindSlot::Small,
            BlindKind::Big => BlindSlot::Big,
            BlindKind::Boss(_) => BlindSlot::Boss,
        }
    }
}

/// Which base-amount table a stake uses: 1 (White, Red), 2 (Green, Black,
/// Blue) or 3 (Purple, Orange, Gold).
pub fn scaling_for_stake(stake: u8) -> u8 {
    match stake {
        0..=2 => 1,
        3..=5 => 2,
        _ => 3,
    }
}

const BASE_AMOUNTS: [[f64; 8]; 3] = [
    [
        300.0, 800.0, 2000.0, 5000.0, 11000.0, 20000.0, 35000.0, 50000.0,
    ],
    [
        300.0, 900.0, 2600.0, 8000.0, 20000.0, 36000.0, 60000.0, 100000.0,
    ],
    [
        300.0, 1000.0, 3200.0, 9000.0, 25000.0, 60000.0, 110000.0, 200000.0,
    ],
];

/// Lua's `a % b` for numbers: `a - floor(a / b) * b`.
#[inline]
pub fn lua_mod(a: f64, b: f64) -> f64 {
    a - (a / b).floor() * b
}

/// The ante's base chip amount for a scaling table (1, 2 or 3).
pub fn base_amount(ante: i32, scaling: u8) -> f64 {
    let table = &BASE_AMOUNTS[(scaling.clamp(1, 3) - 1) as usize];
    if ante < 1 {
        return 100.0;
    }
    if ante <= 8 {
        return table[(ante - 1) as usize];
    }
    let a = table[7];
    let c = (ante - 8) as f64;
    let d = 1.0 + 0.2 * c;
    let raw = (a * (1.6 + (0.75 * c).powf(d)).powf(c)).floor();
    let step = 10f64.powf((raw.log10() - 1.0).floor());
    raw - lua_mod(raw, step)
}

/// A blind's chip target: base amount x blind multiplier x the deck's ante
/// scaling (Plasma Deck 2).
pub fn blind_target(ante: i32, scaling: u8, kind: BlindKind, deck_ante_scaling: f64) -> f64 {
    base_amount(ante, scaling) * kind.mult() * deck_ante_scaling
}

/// Use counts of every boss, for the least-used rule.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BossPool {
    pub used: [u32; N_BOSSES],
    /// Bosses banned by a challenge.
    pub banned: [bool; N_BOSSES],
}

impl BossPool {
    /// Bosses that may be drawn at `ante` before the least-used rule
    /// (`rules/blinds.md` section 6), in pick order.
    pub fn eligible(&self, ante: i32, win_ante: i32) -> Vec<Boss> {
        let a1 = ante.max(1);
        let showdown_ante = ante.rem_euclid(win_ante) == 0 && ante >= 2;
        let regular_ok = a1.rem_euclid(win_ante) != 0 || ante < 2;
        Boss::ALL
            .iter()
            .copied()
            .filter(|b| !self.banned[b.index()])
            .filter(|b| {
                if b.is_showdown() {
                    showdown_ante
                } else {
                    regular_ok && b.min_ante() <= a1
                }
            })
            .collect()
    }

    /// Draw the next boss on stream `boss` and count it as used.
    pub fn draw(&mut self, rng: &mut Streams, ante: i32, win_ante: i32) -> Boss {
        let eligible = self.eligible(ante, win_ante);
        // The game starts its minimum search at 100 and keeps bosses used at
        // most that often; nothing in a real run gets near it.
        let min_use = eligible
            .iter()
            .map(|b| self.used[b.index()])
            .min()
            .unwrap_or(100)
            .min(100);
        let pool: Vec<Boss> = eligible
            .into_iter()
            .filter(|b| self.used[b.index()] <= min_use)
            .collect();
        assert!(!pool.is_empty(), "no boss can be drawn at ante {ante}");
        let mut g = rng.generator("boss");
        let boss = pool[pick_index(pool.len(), &mut g)];
        self.used[boss.index()] += 1;
        boss
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boss_order_is_key_byte_order_within_each_group() {
        let regular: Vec<&str> = Boss::ALL
            .iter()
            .filter(|b| !b.is_showdown())
            .map(|b| b.key())
            .collect();
        let showdown: Vec<&str> = Boss::ALL
            .iter()
            .filter(|b| b.is_showdown())
            .map(|b| b.key())
            .collect();
        let mut r = regular.clone();
        r.sort();
        let mut s = showdown.clone();
        s.sort();
        assert_eq!(regular, r);
        assert_eq!(showdown, s);
        assert_eq!(regular.len(), 23);
        assert_eq!(showdown.len(), 5);
    }

    #[test]
    fn base_amounts_match_the_notes() {
        let expect: [(i32, [f64; 3]); 9] = [
            (9, [110000.0, 230000.0, 460000.0]),
            (10, [560000.0, 1100000.0, 2200000.0]),
            (11, [7200000.0, 14000000.0, 29000000.0]),
            (12, [3.0e8, 6.0e8, 1.2e9]),
            (13, [4.7e10, 9.4e10, 1.8e11]),
            (14, [2.9e13, 5.8e13, 1.1e14]),
            (15, [7.7e16, 1.5e17, 3.0e17]),
            (16, [8.6e20, 1.7e21, 3.4e21]),
            (8, [50000.0, 100000.0, 200000.0]),
        ];
        for (ante, vals) in expect {
            for s in 1..=3u8 {
                let got = base_amount(ante, s);
                let want = vals[(s - 1) as usize];
                assert!(
                    (got - want).abs() <= want * 1e-12,
                    "ante {ante} scaling {s}: {got} vs {want}"
                );
            }
        }
        assert_eq!(base_amount(0, 1), 100.0);
        assert_eq!(base_amount(-3, 3), 100.0);
        assert_eq!(base_amount(1, 2), 300.0);
        assert!(base_amount(39, 1).is_nan() || base_amount(39, 1).is_infinite());
    }

    #[test]
    fn red_deck_white_stake_targets() {
        let t = |ante, kind| blind_target(ante, 1, kind, 1.0);
        assert_eq!(t(1, BlindKind::Small), 300.0);
        assert_eq!(t(1, BlindKind::Big), 450.0);
        assert_eq!(t(1, BlindKind::Boss(Boss::Hook)), 600.0);
        assert_eq!(t(7, BlindKind::Big), 52500.0);
        assert_eq!(t(2, BlindKind::Boss(Boss::Wall)), 3200.0);
        assert_eq!(t(2, BlindKind::Boss(Boss::Needle)), 800.0);
        assert_eq!(t(8, BlindKind::Boss(Boss::VioletVessel)), 300000.0);
    }

    #[test]
    fn eligibility_by_ante() {
        let p = BossPool::default();
        let names = |v: Vec<Boss>| v.iter().map(|b| b.key()).collect::<Vec<_>>();
        assert_eq!(
            names(p.eligible(1, 8)),
            [
                "bl_club",
                "bl_goad",
                "bl_head",
                "bl_hook",
                "bl_manacle",
                "bl_pillar",
                "bl_psychic",
                "bl_window"
            ]
        );
        assert_eq!(p.eligible(2, 8).len(), 18);
        assert_eq!(p.eligible(6, 8).len(), 23);
        assert!(p.eligible(8, 8).iter().all(|b| b.is_showdown()));
        assert!(p.eligible(9, 8).iter().all(|b| !b.is_showdown()));
        assert!(p.eligible(16, 8).iter().all(|b| b.is_showdown()));
        // Ante 0 or below behaves like ante 1.
        assert_eq!(p.eligible(0, 8), p.eligible(1, 8));
        assert_eq!(p.eligible(-8, 8), p.eligible(1, 8));
    }

    #[test]
    fn least_used_rule_rotates_bosses() {
        let mut rng = Streams::new("TESTSEED");
        let mut pool = BossPool::default();
        let mut seen = std::collections::HashSet::new();
        for _ in 0..8 {
            assert!(
                seen.insert(pool.draw(&mut rng, 1, 8)),
                "a boss repeated before all 8 were used"
            );
        }
        assert_eq!(seen.len(), 8);
        // Ante 2: the 10 new bosses (count 0) come before any repeat.
        for _ in 0..10 {
            let b = pool.draw(&mut rng, 2, 8);
            assert_eq!(
                b.min_ante(),
                2,
                "{b:?} drawn before the unused ante-2 bosses"
            );
        }
    }
}
