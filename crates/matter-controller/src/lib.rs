//! A Matter controller on [rs-matter](https://github.com/project-chip/rs-matter).
//!
//! - **Onboarding**: QR codes and manual pairing codes ([`Onboarding`]).
//! - **Commissioning over IP**: the device is found by DNS-SD among
//!   commissionable nodes, or reached at an address given, then taken through
//!   PASE, the fail-safe, CSR, AddNOC, CASE and `CommissioningComplete` by
//!   rs-matter's commissioner.
//! - **A fabric with a pluggable CA** ([`CertificateAuthority`]); [`FileCa`]
//!   for a standalone controller.
//! - **Multi-admin**: [`Controller::open_commissioning_window`] lets a second
//!   ecosystem (Apple Home, Google Home, Home Assistant) commission the node too.
//! - **The interaction model**: read, write and invoke on any cluster, in JSON
//!   converted by [`matter_model`] from the CSA's published data model, and a
//!   subscription to every node that is re-established when it lapses, with
//!   attribute changes and events delivered as [`Event`]s.
//! - **A node store** ([`NodeStore`]) written when the network changes.
//!
//! rs-matter's futures are not `Send`, so the controller runs on a thread of
//! its own; [`Controller`] is the `Send + Clone` handle to it.

#![allow(async_fn_in_trait)]

pub mod ca;
mod controller;
mod fsutil;
mod identity;
mod mdns;
pub mod onboarding;
mod reports;
pub mod store;

use std::net::SocketAddr;
use std::path::PathBuf;

use serde::Serialize;
use serde_json::Value;

pub use ca::{CertificateAuthority, FileCa};
pub use controller::Controller;
pub use onboarding::{Discriminator, Onboarding};
pub use reports::decode_report_data;
pub use store::{BasicInfo, DeviceType, Endpoint, Node, NodeStore};

// The embassy-time driver and timer queue rs-matter's timers run on.
use embassy_time as _;
use embassy_time_queue_utils as _;

/// What can go wrong.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// rs-matter reported an error.
    #[error("matter: {0}")]
    Matter(String),
    /// A node answered with a status other than success.
    #[error("{0}")]
    Status(String),
    /// The onboarding payload is not valid.
    #[error("{0}")]
    Onboarding(String),
    /// The request does not make sense (unknown node, cluster, field, ...).
    #[error("{0}")]
    Invalid(String),
    /// JSON did not fit the data model.
    #[error("{0}")]
    Convert(#[from] matter_model::ConvertError),
    /// No device answered in time.
    #[error("{0}")]
    Timeout(String),
    /// The store or the CA file could not be read or written.
    #[error("store: {0}")]
    Store(String),
    /// I/O.
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    /// Something ran out.
    #[error("out of {0}")]
    Exhausted(&'static str),
    /// Not supported (yet).
    #[error("not supported: {0}")]
    Unsupported(String),
    /// The controller has stopped.
    #[error("the controller has stopped")]
    Stopped,
}

impl From<rs_matter::error::Error> for Error {
    fn from(e: rs_matter::error::Error) -> Self {
        Error::Matter(format!("{e:?}"))
    }
}

impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Error::Store(e.to_string())
    }
}

/// How the controller runs.
#[derive(Debug, Clone)]
pub struct Config {
    /// Where the CA, the controller's identity and the node store live.
    pub state_dir: PathBuf,
    /// UDP port for Matter traffic.
    pub port: u16,
    /// Network interface for mDNS (DNS-SD); the first with IPv6 if `None`.
    pub interface: Option<String>,
    /// The fabric id of a new fabric; random if `None`. Ignored once the
    /// fabric exists.
    pub fabric_id: Option<u64>,
    /// Vendor id the controller administers nodes as (`0xFFF1` is the CSA's
    /// test vendor id, for uncertified controllers).
    pub vendor_id: u16,
    /// Commission devices whose attestation has not been verified.
    ///
    /// rs-matter does not verify device attestation yet (see
    /// `docs/design.md`), so commissioning refuses every device unless this is
    /// set.
    pub allow_unverified_attestation: bool,
    /// The longest a subscription may stay silent before it is renewed,
    /// seconds (the device may agree to less).
    pub max_report_interval: u16,
}

impl Config {
    /// Defaults for a controller keeping its state in `state_dir`.
    pub fn new(state_dir: impl Into<PathBuf>) -> Self {
        Self {
            state_dir: state_dir.into(),
            port: 5541,
            interface: None,
            fabric_id: None,
            vendor_id: 0xFFF1,
            allow_unverified_attestation: true,
            max_report_interval: 60,
        }
    }
}

/// What to commission.
#[derive(Debug, Clone)]
pub struct CommissionRequest {
    /// The QR code payload or manual pairing code.
    pub code: String,
    /// The device's address, to skip discovery.
    pub address: Option<SocketAddr>,
    /// The name to give the node; a default if `None`.
    pub name: Option<String>,
    /// How long to look for the device, seconds.
    pub discovery_timeout: u16,
}

/// A path to one attribute or a wildcard over several.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AttributePath {
    /// Endpoint, or every endpoint.
    pub endpoint: Option<u16>,
    /// Cluster, or every cluster.
    pub cluster: Option<u32>,
    /// Attribute, or every attribute.
    pub attribute: Option<u32>,
}

/// An attribute's value, or why it has none.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AttributeValue {
    /// The node.
    pub node: u64,
    /// Endpoint.
    pub endpoint: u16,
    /// Cluster id.
    pub cluster: u32,
    /// Attribute id.
    pub attribute: u32,
    /// The value, decoded with the data model.
    pub value: Value,
    /// The status, when the node had no value to give (e.g.
    /// `UnsupportedAttribute`); `value` is null then.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
}

/// An event a node emitted.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct EventValue {
    /// The node.
    pub node: u64,
    /// Endpoint.
    pub endpoint: u16,
    /// Cluster id.
    pub cluster: u32,
    /// Event id.
    pub event: u32,
    /// The node's event number, increasing.
    pub number: u64,
    /// The event's fields, decoded with the data model.
    pub data: Value,
}

/// A commissioning step, as it happens.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CommissionStep {
    /// Looking for the device.
    Discovery,
    /// Establishing PASE with the passcode.
    Pase,
    /// Arming the fail-safe.
    ArmFailSafe,
    /// Checking the device's attestation.
    DeviceAttestation,
    /// Asking for its CSR.
    CsrRequest,
    /// Installing our root certificate.
    AddTrustedRootCertificate,
    /// Installing its operational certificate.
    AddNoc,
    /// Finding it on the operational network.
    FindOperational,
    /// Completing commissioning over CASE.
    CommissioningComplete,
    /// Rolling back after a failure.
    Cleanup,
    /// Reading what it is.
    Interview,
}

/// What the controller tells its users, as it happens.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    /// An attribute changed (or was reported in a subscription's priming).
    Attribute(AttributeValue),
    /// A node emitted an event.
    Event(EventValue),
    /// A commissioning advanced.
    Commissioning {
        /// The node id it will have.
        node: u64,
        /// The step it started.
        step: CommissionStep,
    },
    /// A node was added or changed in the store.
    NodeUpdated(Node),
    /// A node was removed.
    NodeRemoved {
        /// The node.
        node: u64,
    },
    /// A node's subscription was established or lapsed.
    Availability {
        /// The node.
        node: u64,
        /// Whether it is reachable.
        online: bool,
    },
}

/// A node advertising itself as commissionable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Commissionable {
    /// Its DNS-SD instance id.
    pub instance: String,
    /// Where it answers.
    pub addresses: Vec<SocketAddr>,
}

/// A commissioning window opened on a node, for another administrator.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CommissioningWindow {
    /// The node.
    pub node: u64,
    /// The 11-digit manual pairing code, with separators.
    pub manual_code: String,
    /// The QR code payload.
    pub qr_code: String,
    /// The discriminator the node advertises while the window is open.
    pub discriminator: u16,
    /// How long the window stays open, seconds.
    pub timeout: u16,
}

/// The controller's identity on its fabric.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ControllerInfo {
    /// Fabric id.
    pub fabric_id: u64,
    /// Compressed fabric id, as nodes advertise it in DNS-SD.
    pub compressed_fabric_id: u64,
    /// The controller's own node id.
    pub node_id: u64,
    /// Vendor id it administers as.
    pub vendor_id: u16,
}
