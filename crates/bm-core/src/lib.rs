//! Balatro Meta engine: the game's rules, state and scoring.
//!
//! Written from the rules notes in `rules/`, which are checked against the
//! real game. Modules land here phase by phase.

pub mod card;
pub mod hand_levels;
pub mod poker;

pub use bm_rng;
