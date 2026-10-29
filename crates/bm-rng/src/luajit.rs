//! Bit-exact port of LuaJIT 2.1's `math.randomseed` / `math.random`.
//!
//! LuaJIT uses a combined Tausworthe generator ("TW223", period 2^223) with
//! four 64-bit words of state. `math.randomseed(d)` derives the four words from
//! the double `d` and then discards ten outputs. Every output is a double in
//! [1, 2) built from 52 random mantissa bits; `math.random()` subtracts 1.
//!
//! The algorithm is LuaJIT's (MIT licence, Copyright (C) 2005-2023 Mike Pall);
//! this is an independent Rust port of it, checked value for value against
//! LuaJIT itself (see `reference/rng_reference.lua` and the fixture tests).

use std::f64::consts::{E, PI};

/// State of LuaJIT's `math.random` after a `math.randomseed` call.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct LuaRandom {
    gen: [u64; 4],
}

/// One Tausworthe component: update `z` in place and return its new value.
#[inline(always)]
fn tw223_gen(z: &mut u64, k: u32, q: u32, s: u32) -> u64 {
    let x = *z;
    let y = (((x << q) ^ x) >> (k - s)) ^ ((x & (u64::MAX << (64 - k))) << s);
    *z = y;
    y
}

impl LuaRandom {
    /// Equivalent of `math.randomseed(d)`.
    pub fn new(d: f64) -> Self {
        let mut gen = [0u64; 4];
        // 64 - k for each component, packed as four 8-bit constants.
        let mut r: u32 = 0x1109_0601;
        let mut d = d;
        for g in gen.iter_mut() {
            let m: u64 = 1u64 << (r & 255);
            r >>= 8;
            d = d * PI + E;
            let mut u = d.to_bits();
            // Ensure the k most significant bits of each word are not all zero.
            if u < m {
                u = u.wrapping_add(m);
            }
            *g = u;
        }
        let mut rs = LuaRandom { gen };
        for _ in 0..10 {
            rs.step();
        }
        rs
    }

    /// One generator step: the bit pattern of a double in [1, 2).
    #[inline(always)]
    pub fn step(&mut self) -> u64 {
        let mut r = 0u64;
        r ^= tw223_gen(&mut self.gen[0], 63, 31, 18);
        r ^= tw223_gen(&mut self.gen[1], 58, 19, 28);
        r ^= tw223_gen(&mut self.gen[2], 55, 24, 7);
        r ^= tw223_gen(&mut self.gen[3], 47, 21, 8);
        (r & 0x000f_ffff_ffff_ffff) | 0x3ff0_0000_0000_0000
    }

    /// `math.random()`: a double in [0, 1).
    #[inline]
    pub fn random(&mut self) -> f64 {
        f64::from_bits(self.step()) - 1.0
    }

    /// `math.random(m)`: an integer-valued double in [1, m].
    #[inline]
    pub fn random_upto(&mut self, m: f64) -> f64 {
        (self.random() * m).floor() + 1.0
    }

    /// `math.random(lo, hi)`: an integer-valued double in [lo, hi].
    #[inline]
    pub fn random_range(&mut self, lo: f64, hi: f64) -> f64 {
        (self.random() * (hi - lo + 1.0)).floor() + lo
    }

    /// `math.random(n)` returned as a zero-based index into a list of `n` items.
    #[inline]
    pub fn index(&mut self, n: usize) -> usize {
        debug_assert!(n > 0, "math.random called with an empty interval");
        self.random_upto(n as f64) as usize - 1
    }

    /// The raw state words, for tests and debugging.
    pub fn state(&self) -> [u64; 4] {
        self.gen
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outputs_are_in_unit_interval() {
        let mut rs = LuaRandom::new(0.123_456_789);
        for _ in 0..10_000 {
            let x = rs.random();
            assert!((0.0..1.0).contains(&x));
        }
    }

    #[test]
    fn same_seed_same_stream() {
        let mut a = LuaRandom::new(0.5);
        let mut b = LuaRandom::new(0.5);
        for _ in 0..100 {
            assert_eq!(a.random().to_bits(), b.random().to_bits());
        }
    }

    #[test]
    fn integer_draws_stay_in_range() {
        let mut rs = LuaRandom::new(42.0);
        for _ in 0..10_000 {
            let k = rs.random_upto(52.0);
            assert!((1.0..=52.0).contains(&k) && k.fract() == 0.0);
            let j = rs.random_range(3.0, 9.0);
            assert!((3.0..=9.0).contains(&j) && j.fract() == 0.0);
        }
    }
}
