//! Balatro Meta engine: the game's rules, state and scoring.
//!
//! Written from the rules notes in `rules/`, which are checked against the
//! real game. Modules land here phase by phase.

pub mod blinds;
pub mod card;
pub mod hand_levels;
pub mod joker_data;
pub mod jokers;
pub mod poker;
pub mod run;
pub mod scoring;
pub mod state;
pub mod tags;

pub use bm_rng;
pub use run::Refused;
pub use state::{Phase, Run, RunConfig};
