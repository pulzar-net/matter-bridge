#![doc = include_str!("../README.md")]

pub mod api;
pub mod bridge;
pub mod config;
pub mod homeassistant;
pub mod mqtt;
pub mod topics;

pub use bridge::run;
pub use config::Config;

// The repository README's Rust examples run as doctests, so they cannot rot.
#[cfg(doctest)]
#[doc = include_str!("../../../README.md")]
struct ReadmeDoctests;

// The library guide's examples are compiled too.
#[cfg(doctest)]
#[doc = include_str!("../../../docs/src/content/docs/library/matter-controller.md")]
struct LibraryGuideDoctests;
