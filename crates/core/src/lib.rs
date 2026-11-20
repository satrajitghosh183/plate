//! Domain rules for the gym app.
//!
//! Everything here is pure: no clocks, no storage, no network. The backend and the
//! phone app both use these types, so a rule is written once and tested once.

pub mod api;
pub mod catalogue;
pub mod classify;
pub mod plates;
pub mod progress;
pub mod session;
pub mod split;
pub mod tags;
pub mod units;

pub use tags::{Equipment, LiftTags, Mechanic, Muscle, Pattern, TagSource};
pub use units::{Unit, Weight};
