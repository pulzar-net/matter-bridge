//! A standalone Matter to MQTT bridge: a Matter controller (on rs-matter)
//! that commissions devices, keeps a subscription to each, and publishes what
//! they report on MQTT, with Home Assistant discovery and a plain JSON topic
//! scheme. See the README for the topics.

pub mod bridge;
pub mod config;
pub mod homeassistant;
pub mod mqtt;
pub mod topics;

pub use bridge::run;
pub use config::Config;
