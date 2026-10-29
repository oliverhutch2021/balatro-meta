//! Balatro's seeded random numbers, reproduced bit for bit.
//!
//! * [`luajit`]: LuaJIT 2.1's `math.randomseed` / `math.random` (TW223).
//! * [`streams`]: the seed hash and the per-event named streams the game
//!   draws from (`pseudohash`, `pseudoseed`, `pseudorandom`), plus its shuffle
//!   and list picks.
//! * [`seeds`]: seed strings, and a separate fair sampler for fresh seeds.
//!
//! Everything here is checked against LuaJIT running `reference/rng_reference.lua`
//! (see `tests/luajit_fixtures.rs`).

pub mod luajit;
pub mod seeds;
pub mod streams;

pub use luajit::LuaRandom;
pub use seeds::{normalize_seed, SeedSampler, SEED_ALPHABET};
pub use streams::{advance_stream, lua_mod1, pick_index, pseudohash, round13, shuffle, Streams};
