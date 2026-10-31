//! Balatro's seeding recipe, on top of LuaJIT's generator.
//!
//! A run's seed string is hashed once. Every kind of random event draws from
//! its own named stream (`"shuffle"`, `"rarity1sho"`, `"lucky_mult"`, ...).
//! A stream starts from a hash of `key .. seed` and advances by a fixed linear
//! step, rounded to 13 decimal places. Each draw averages the stream's value
//! with the seed's hash and uses the result to reseed LuaJIT's generator.
//!
//! See `rules/rng.md` for the recipe in words and its public sources.

use crate::luajit::LuaRandom;
use std::collections::HashMap;
use std::f64::consts::PI;
use std::sync::Arc;

/// Lua's `x % 1` on doubles: `x - floor(x)`.
#[inline(always)]
pub fn lua_mod1(x: f64) -> f64 {
    x - x.floor()
}

/// `tonumber(string.format("%.13f", x))`: round to 13 decimal places the way
/// the game does, by printing and parsing.
///
/// The game runs LuaJIT 2.0.5, whose `string.format` hands numbers to the C
/// runtime's `sprintf`; on Windows that is the Universal CRT, which prints the
/// exact binary value correctly rounded, exact halves to even (Windows 10 2004
/// and later; checked on the user's PC by `make rng-game`). `tonumber` then
/// returns the nearest double to the printed decimal. (LuaJIT 2.1, used for the
/// cloud fixture, prints exact halves away from zero instead, so the reference
/// script corrects ties itself.)
///
/// For |x| < 512 this computes the same thing exactly with integers:
/// N = x * 10^13 rounded half to even, then N / 10^13 (both exact doubles, so
/// the division is the correctly rounded nearest double). Larger values, never
/// seen in the game's streams, fall back to Rust's formatter, which also rounds
/// exactly with halves to even. Non-finite values pass through.
#[inline]
pub fn round13(x: f64) -> f64 {
    if !x.is_finite() {
        return x;
    }
    let ax = x.abs();
    if ax >= 512.0 {
        let s = format!("{x:.13}");
        return s.parse::<f64>().expect("a formatted double parses");
    }
    if ax == 0.0 {
        return x;
    }
    // ax = m * 2^e exactly, with e < 0 for every value below 512.
    let bits = ax.to_bits();
    let biased = ((bits >> 52) & 0x7ff) as i32;
    let frac = bits & ((1u64 << 52) - 1);
    let (m, e) = if biased == 0 {
        (frac, -1074)
    } else {
        (frac | (1u64 << 52), biased - 1075)
    };
    let sh = (-e) as u32;
    let n: u128 = if sh > 100 {
        // m * 10^13 < 2^97, so the value is below half a unit: rounds to 0.
        0
    } else {
        let p = (m as u128) * 10_000_000_000_000u128;
        let q = p >> sh;
        let r = p - (q << sh);
        let twice = r << 1;
        let unit = 1u128 << sh;
        if twice > unit || (twice == unit && q & 1 == 1) {
            q + 1
        } else {
            q
        }
    };
    let v = (n as f64) / 1e13;
    if x < 0.0 {
        -v
    } else {
        v
    }
}

/// The game's string hash: a double in [0, 1) (or NaN for a few degenerate
/// strings, where the running value collapses to zero).
pub fn pseudohash(s: &str) -> f64 {
    let bytes = s.as_bytes();
    let mut num = 1.0f64;
    for i in (1..=bytes.len()).rev() {
        let b = bytes[i - 1] as f64;
        num = lua_mod1((1.1239285023 / num) * b * PI + PI * (i as f64));
    }
    num
}

/// One step of a named stream: the new stream value.
#[inline]
pub fn advance_stream(v: f64) -> f64 {
    round13(lua_mod1(2.134453429141 + v * 1.72431234)).abs()
}

/// All of a run's named random streams.
///
/// Copies of a run are made by the million (every hand a search scores is played on one), so copying the
/// streams allocates nothing per stream: the names are shared, and `clone_from` reuses the table.
#[derive(Debug)]
pub struct Streams {
    seed: String,
    hashed_seed: f64,
    state: HashMap<Arc<str>, f64>,
}

impl Clone for Streams {
    fn clone(&self) -> Self {
        Streams {
            seed: self.seed.clone(),
            hashed_seed: self.hashed_seed,
            state: self.state.clone(),
        }
    }

    fn clone_from(&mut self, source: &Self) {
        self.seed.clone_from(&source.seed);
        self.hashed_seed = source.hashed_seed;
        self.state.clone_from(&source.state);
    }
}

impl Streams {
    /// Streams for a run with this seed string (already normalised).
    pub fn new(seed: &str) -> Self {
        Streams {
            seed: seed.to_string(),
            hashed_seed: pseudohash(seed),
            state: HashMap::new(),
        }
    }

    /// The run's seed string.
    pub fn seed(&self) -> &str {
        &self.seed
    }

    /// `pseudohash(seed)`, shared by every draw.
    pub fn hashed_seed(&self) -> f64 {
        self.hashed_seed
    }

    /// The current value of a stream, if it has been used.
    pub fn stream_value(&self, key: &str) -> Option<f64> {
        self.state.get(key).copied()
    }

    /// Every stream used so far and its current value, sorted by key.
    pub fn snapshot(&self) -> Vec<(String, f64)> {
        let mut v: Vec<_> = self
            .state
            .iter()
            .map(|(k, x)| (k.to_string(), *x))
            .collect();
        v.sort_by(|a, b| a.0.cmp(&b.0));
        v
    }

    /// `pseudoseed(key)`: advance the named stream and return the seed value
    /// for this draw.
    pub fn pseudoseed(&mut self, key: &str) -> f64 {
        let v = match self.state.get_mut(key) {
            Some(v) => v,
            None => {
                let init = pseudohash(&format!("{key}{}", self.seed));
                self.state.entry(Arc::from(key)).or_insert(init)
            }
        };
        *v = advance_stream(*v);
        (*v + self.hashed_seed) / 2.0
    }

    /// The seed value the next draw from `key` would use, without advancing it.
    pub fn peek_pseudoseed(&self, key: &str) -> f64 {
        let v = match self.state.get(key) {
            Some(v) => *v,
            None => pseudohash(&format!("{key}{}", self.seed)),
        };
        (advance_stream(v) + self.hashed_seed) / 2.0
    }

    /// `pseudorandom(key)`: a double in [0, 1).
    pub fn random(&mut self, key: &str) -> f64 {
        LuaRandom::new(self.pseudoseed(key)).random()
    }

    /// `pseudorandom(key, lo, hi)`: an integer in [lo, hi].
    pub fn random_range(&mut self, key: &str, lo: i64, hi: i64) -> i64 {
        LuaRandom::new(self.pseudoseed(key)).random_range(lo as f64, hi as f64) as i64
    }

    /// A generator seeded from the named stream, for events that take several
    /// numbers from one seeding (shuffles, picking from a list).
    pub fn generator(&mut self, key: &str) -> LuaRandom {
        LuaRandom::new(self.pseudoseed(key))
    }
}

/// The game's shuffle body: a Fisher-Yates pass from the end, using
/// `math.random(i)` for i = n down to 2. Callers first put `items` into the
/// game's canonical order for that list.
pub fn shuffle<T>(items: &mut [T], rng: &mut LuaRandom) {
    let n = items.len();
    for i in (2..=n).rev() {
        let j = rng.random_upto(i as f64) as usize;
        items.swap(i - 1, j - 1);
    }
}

/// `math.random(#list)` over a list already in the game's canonical order:
/// the zero-based index picked.
#[inline]
pub fn pick_index(n: usize, rng: &mut LuaRandom) -> usize {
    rng.index(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn streams_are_independent_of_call_order() {
        // Drawing from one stream never moves another.
        let mut a = Streams::new("ABCDEFGH");
        let mut b = Streams::new("ABCDEFGH");
        let a1 = a.pseudoseed("shuffle");
        let _ = b.pseudoseed("rarity1sho");
        let _ = b.pseudoseed("rarity1sho");
        let b1 = b.pseudoseed("shuffle");
        assert_eq!(a1.to_bits(), b1.to_bits());
    }

    #[test]
    fn peek_matches_the_next_draw() {
        let mut s = Streams::new("7LB2WVPK");
        for _ in 0..5 {
            let p = s.peek_pseudoseed("lucky_mult");
            assert_eq!(p.to_bits(), s.pseudoseed("lucky_mult").to_bits());
        }
    }

    #[test]
    fn shuffle_is_a_permutation() {
        let mut v: Vec<u32> = (1..=52).collect();
        let mut rng = LuaRandom::new(0.25);
        shuffle(&mut v, &mut rng);
        let mut sorted = v.clone();
        sorted.sort();
        assert_eq!(sorted, (1..=52).collect::<Vec<_>>());
        assert_ne!(v, (1..=52).collect::<Vec<_>>());
    }
}
