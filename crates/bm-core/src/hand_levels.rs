//! Poker-hand levels and per-hand counters (`rules/hands.md` section 6).

use crate::poker::HandType;

/// Base chips, base mult, chips per level, mult per level.
pub fn base_values(h: HandType) -> (f64, f64, f64, f64) {
    match h {
        HandType::FlushFive => (160.0, 16.0, 50.0, 3.0),
        HandType::FlushHouse => (140.0, 14.0, 40.0, 4.0),
        HandType::FiveOfAKind => (120.0, 12.0, 35.0, 3.0),
        HandType::StraightFlush => (100.0, 8.0, 40.0, 4.0),
        HandType::FourOfAKind => (60.0, 7.0, 30.0, 3.0),
        HandType::FullHouse => (40.0, 4.0, 25.0, 2.0),
        HandType::Flush => (35.0, 4.0, 15.0, 2.0),
        HandType::Straight => (30.0, 4.0, 30.0, 3.0),
        HandType::ThreeOfAKind => (30.0, 3.0, 20.0, 2.0),
        HandType::TwoPair => (20.0, 2.0, 20.0, 1.0),
        HandType::Pair => (10.0, 2.0, 15.0, 1.0),
        HandType::HighCard => (5.0, 1.0, 10.0, 1.0),
    }
}

/// Hidden at the start of a run: Five of a Kind, Flush House, Flush Five.
pub fn starts_visible(h: HandType) -> bool {
    !matches!(
        h,
        HandType::FiveOfAKind | HandType::FlushHouse | HandType::FlushFive
    )
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HandInfo {
    pub level: i32,
    pub chips: f64,
    pub mult: f64,
    pub played: u32,
    pub played_this_round: u32,
    pub visible: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct HandLevels {
    pub hands: [HandInfo; 12],
}

impl Default for HandLevels {
    fn default() -> Self {
        let hands = std::array::from_fn(|i| {
            let h = HandType::from_index(i);
            let (c, m, _, _) = base_values(h);
            HandInfo {
                level: 1,
                chips: c,
                mult: m,
                played: 0,
                played_this_round: 0,
                visible: starts_visible(h),
            }
        });
        HandLevels { hands }
    }
}

impl HandLevels {
    pub fn get(&self, h: HandType) -> &HandInfo {
        &self.hands[h.index()]
    }

    pub fn get_mut(&mut self, h: HandType) -> &mut HandInfo {
        &mut self.hands[h.index()]
    }

    /// Change a hand's level by `amount` and recompute its chips and mult.
    pub fn level_up(&mut self, h: HandType, amount: i32) {
        let (c, m, lc, lm) = base_values(h);
        let info = self.get_mut(h);
        info.level = (info.level + amount).max(0);
        let steps = (info.level - 1) as f64;
        info.mult = (m + lm * steps).max(1.0);
        info.chips = (c + lc * steps).max(0.0);
    }

    /// Start of a round: every hand's played-this-round count goes to 0.
    pub fn new_round(&mut self) {
        for h in self.hands.iter_mut() {
            h.played_this_round = 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn level_formula() {
        let mut t = HandLevels::default();
        t.level_up(HandType::Pair, 2);
        let p = t.get(HandType::Pair);
        assert_eq!((p.level, p.chips, p.mult), (3, 40.0, 4.0));
        assert!(!t.get(HandType::FlushFive).visible);
        assert!(t.get(HandType::HighCard).visible);
    }
}
