//! Shroud's enemy brain. Pure Rust, no engine dependency, so the same code drives
//! enemies in the Bevy game and in the fast offline trainer.

pub mod arena;
pub mod difficulty;
pub mod evolve;
pub mod genome;
pub mod insights;
pub mod ghost;
pub mod limits;
pub mod math;
pub mod mind;
pub mod nav;
pub mod perception;
pub mod neat;
pub mod player_model;
pub mod sim;
pub mod squad;
pub mod trainer;
pub mod utility;
