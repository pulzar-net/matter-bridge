//! The MQTT API: every payload the bridge publishes or reads, as a type.
//!
//! The bridge serializes these and nothing else, so they are the contract.
//! With the `unstable-asyncapi-docs` feature each one also derives a JSON
//! Schema and an AsyncAPI message, and `tests/asyncapi.rs` assembles them
//! into the AsyncAPI description the docs site renders as the MQTT
//! reference. That feature is docs tooling, not API: it may change in any
//! release, and the bridge's own builds never turn it on.
//!
//! Topics are under the base topic (`mqtt.base_topic`, default `matter`).
//! `<device>` is the name a person gave the device (`node_<node id>`
//! otherwise). Clusters, attributes, commands and events are named in
//! `snake_case` from the Matter data model, or by number in hex where the
//! data model does not know them. Keys and network credentials never appear
//! on MQTT.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value as Json;

#[cfg(feature = "unstable-asyncapi-docs")]
use asyncapi_rust::ToAsyncApiMessage;
#[cfg(feature = "unstable-asyncapi-docs")]
use schemars::JsonSchema;

// ---------------------------------------------------------------------------
// Bridge state and info

/// The bridge's availability, retained on `bridge/state`. The broker
/// publishes `offline` as the bridge's last will when the connection drops.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(
    feature = "unstable-asyncapi-docs",
    derive(JsonSchema, ToAsyncApiMessage)
)]
#[cfg_attr(
    feature = "unstable-asyncapi-docs",
    asyncapi(summary = "The bridge is online or offline")
)]
pub struct BridgeState {
    /// `online` while the bridge runs, `offline` once it stops or its
    /// connection drops.
    pub state: Availability,
}

/// Online or offline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "unstable-asyncapi-docs", derive(JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum Availability {
    /// Running, or reachable.
    Online,
    /// Stopped, or not reachable.
    Offline,
}

impl Availability {
    /// `online` or `offline`, as plain text.
    pub fn as_str(self) -> &'static str {
        match self {
            Availability::Online => "online",
            Availability::Offline => "offline",
        }
    }
}

impl BridgeState {
    /// The availability as the payload the bridge publishes.
    pub fn payload(state: Availability) -> Vec<u8> {
        serde_json::to_vec(&BridgeState { state }).unwrap_or_default()
    }
}

/// The bridge and its fabric, retained on `bridge/info`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(
    feature = "unstable-asyncapi-docs",
    derive(JsonSchema, ToAsyncApiMessage)
)]
#[cfg_attr(
    feature = "unstable-asyncapi-docs",
    asyncapi(summary = "The bridge's version and fabric")
)]
pub struct BridgeInfo {
    /// The bridge's version.
    pub version: String,
    /// The fabric's id, as 16 upper-case hex digits.
    pub fabric_id: String,
    /// The compressed fabric id, as 16 upper-case hex digits: what the
    /// fabric's nodes advertise over DNS-SD.
    pub compressed_fabric_id: String,
    /// The bridge's own node id on the fabric, as 16 upper-case hex digits.
    pub node_id: String,
    /// The vendor id the bridge administers nodes as.
    pub vendor_id: u16,
}

// ---------------------------------------------------------------------------
// Devices

/// Every commissioned device, retained on `bridge/devices`. Republished
/// when one is commissioned, interviewed, renamed or removed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(
    feature = "unstable-asyncapi-docs",
    derive(JsonSchema, ToAsyncApiMessage)
)]
#[cfg_attr(
    feature = "unstable-asyncapi-docs",
    asyncapi(summary = "Every device and its interview")
)]
#[serde(transparent)]
pub struct BridgeDevices(pub Vec<Device>);

/// A commissioned device and what its interview found.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "unstable-asyncapi-docs", derive(JsonSchema))]
pub struct Device {
    /// Its operational node id on the bridge's fabric.
    pub node_id: u64,
    /// The name it is published under: given at commissioning or with
    /// `rename`, `node_<node id>` otherwise.
    pub name: String,
    /// What its Basic Information cluster says.
    pub basic: BasicInfo,
    /// Its endpoints, by endpoint number, from the Descriptor cluster.
    pub endpoints: BTreeMap<u16, Endpoint>,
    /// Whether the interview has completed.
    pub interviewed: bool,
    /// When it was commissioned, in seconds since the Unix epoch.
    pub commissioned_at: u64,
}

/// What a device's Basic Information cluster says. Each is `null` until the
/// interview reads it, or when the device does not have it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "unstable-asyncapi-docs", derive(JsonSchema))]
pub struct BasicInfo {
    /// The vendor id.
    pub vendor_id: Option<u16>,
    /// The vendor's name.
    pub vendor_name: Option<String>,
    /// The product id.
    pub product_id: Option<u16>,
    /// The product's name.
    pub product_name: Option<String>,
    /// The label a person set on the device itself.
    pub node_label: Option<String>,
    /// The hardware version, as text.
    pub hardware_version: Option<String>,
    /// The software version, as text.
    pub software_version: Option<String>,
    /// The serial number.
    pub serial_number: Option<String>,
    /// An id stable across factory resets.
    pub unique_id: Option<String>,
}

/// An endpoint of a device.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "unstable-asyncapi-docs", derive(JsonSchema))]
pub struct Endpoint {
    /// What the endpoint is.
    pub device_types: Vec<DeviceType>,
    /// The clusters it serves, by id: their attributes are published.
    pub server_clusters: Vec<u32>,
    /// The clusters it is a client of, by id: it sends their commands, as a
    /// switch does.
    pub client_clusters: Vec<u32>,
    /// The endpoints it is composed of.
    pub parts: Vec<u16>,
}

/// A Matter device type an endpoint is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "unstable-asyncapi-docs", derive(JsonSchema))]
pub struct DeviceType {
    /// The device type id: `256` (`0x0100`) for an On/Off Light, for
    /// example.
    pub id: u32,
    /// Its revision.
    pub revision: u16,
}

impl From<matter_controller::Node> for Device {
    fn from(n: matter_controller::Node) -> Self {
        let b = n.basic;
        Device {
            node_id: n.node_id,
            name: n.name,
            basic: BasicInfo {
                vendor_id: b.vendor_id,
                vendor_name: b.vendor_name,
                product_id: b.product_id,
                product_name: b.product_name,
                node_label: b.node_label,
                hardware_version: b.hardware_version,
                software_version: b.software_version,
                serial_number: b.serial_number,
                unique_id: b.unique_id,
            },
            endpoints: n
                .endpoints
                .into_iter()
                .map(|(id, e)| {
                    (
                        id,
                        Endpoint {
                            device_types: e
                                .device_types
                                .into_iter()
                                .map(|d| DeviceType {
                                    id: d.id,
                                    revision: d.revision,
                                })
                                .collect(),
                            server_clusters: e.server_clusters,
                            client_clusters: e.client_clusters,
                            parts: e.parts,
                        },
                    )
                })
                .collect(),
            interviewed: n.interviewed,
            commissioned_at: n.commissioned_at,
        }
    }
}

/// A device's availability, retained on `<device>/availability`: `online`
/// while the bridge's subscription to it holds, `offline` once it lapses.
/// Plain text, not JSON.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(
    feature = "unstable-asyncapi-docs",
    derive(JsonSchema, ToAsyncApiMessage)
)]
#[cfg_attr(
    feature = "unstable-asyncapi-docs",
    asyncapi(
        summary = "The device is reachable or not",
        content_type = "text/plain"
    )
)]
#[serde(transparent)]
pub struct DeviceAvailability(pub Availability);

impl DeviceAvailability {
    /// The payload the bridge publishes: the bare word.
    pub fn payload(self) -> Vec<u8> {
        self.0.as_str().as_bytes().to_vec()
    }
}

/// An attribute's value, retained on
/// `<device>/<endpoint>/<cluster>/<attribute>`, as the device reports it:
/// `measured_value` 2150 is 21.50 °C. Integers, enumerations and bitmaps are
/// numbers, octet strings hex strings, structures objects keyed by field
/// name, lists arrays.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(
    feature = "unstable-asyncapi-docs",
    derive(JsonSchema, ToAsyncApiMessage)
)]
#[cfg_attr(
    feature = "unstable-asyncapi-docs",
    asyncapi(summary = "An attribute's value")
)]
#[serde(transparent)]
pub struct AttributeValue(pub Json);

/// An event a device reported, on
/// `<device>/<endpoint>/<cluster>/event/<event>`. Not retained.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(
    feature = "unstable-asyncapi-docs",
    derive(JsonSchema, ToAsyncApiMessage)
)]
#[cfg_attr(
    feature = "unstable-asyncapi-docs",
    asyncapi(summary = "An event the device reported")
)]
pub struct DeviceEvent {
    /// The device's event number. When the bridge's subscription is
    /// renewed, it asks for events after the last number seen.
    pub number: u64,
    /// The event's fields, by the data model's names.
    pub data: Json,
}

/// A value for an on/off or level control endpoint, on
/// `<device>/<endpoint>/on_off/set` (`ON`, `OFF`, `TOGGLE`, or `true` and
/// `false`) or `<device>/<endpoint>/level_control/set` (a level, 0 to 254,
/// switching the light on). Plain text or JSON.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(
    feature = "unstable-asyncapi-docs",
    derive(JsonSchema, ToAsyncApiMessage)
)]
#[cfg_attr(
    feature = "unstable-asyncapi-docs",
    asyncapi(summary = "Switch or dim an endpoint")
)]
#[serde(transparent)]
pub struct ClusterSet(pub Json);

/// A message's JSON, or its text as a JSON string when it is not JSON.
fn json_or_text(payload: &[u8]) -> Json {
    serde_json::from_slice(payload)
        .unwrap_or_else(|_| Json::String(String::from_utf8_lossy(payload).trim().to_string()))
}

impl ClusterSet {
    /// The value a message carries.
    pub fn from_payload(payload: &[u8]) -> Self {
        ClusterSet(json_or_text(payload))
    }
}

/// A new value for an attribute, on
/// `<device>/<endpoint>/<cluster>/<attribute>/set`, as JSON in the same
/// form the attribute is published in. An enumeration may be given by name.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(
    feature = "unstable-asyncapi-docs",
    derive(JsonSchema, ToAsyncApiMessage)
)]
#[cfg_attr(
    feature = "unstable-asyncapi-docs",
    asyncapi(summary = "Write an attribute")
)]
#[serde(transparent)]
pub struct AttributeWrite(pub Json);

impl AttributeWrite {
    /// The value a message carries; text that is not JSON is a string.
    pub fn from_payload(payload: &[u8]) -> Self {
        AttributeWrite(json_or_text(payload))
    }
}

/// A command's fields, on `<device>/<endpoint>/<cluster>/command/<command>`:
/// an object keyed by the data model's field names, or an empty message for
/// a command without fields. A required field that may be null can be left
/// out.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(
    feature = "unstable-asyncapi-docs",
    derive(JsonSchema, ToAsyncApiMessage)
)]
#[cfg_attr(
    feature = "unstable-asyncapi-docs",
    asyncapi(summary = "Invoke a command")
)]
#[serde(transparent)]
pub struct CommandFields(pub Json);

impl CommandFields {
    /// The fields a message carries; `null` for an empty message.
    pub fn from_payload(payload: &[u8]) -> Self {
        if payload.is_empty() {
            CommandFields(Json::Null)
        } else {
            CommandFields(json_or_text(payload))
        }
    }
}

/// The outcome of a command sent on a device's `command/<command>` topic,
/// on `.../command/<command>/result`. Not retained.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(
    feature = "unstable-asyncapi-docs",
    derive(JsonSchema, ToAsyncApiMessage)
)]
#[cfg_attr(
    feature = "unstable-asyncapi-docs",
    asyncapi(summary = "The command's response, or its error")
)]
pub struct CommandResult {
    /// `ok` or `error`.
    pub status: Status,
    /// The response command's fields, or `null` for a plain success, when
    /// `status` is `ok`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Json>,
    /// What went wrong, when `status` is `error`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl CommandResult {
    /// The result of a command.
    pub fn new(result: Result<Json, String>) -> Self {
        match result {
            Ok(data) => CommandResult {
                status: Status::Ok,
                data: Some(data),
                error: None,
            },
            Err(error) => CommandResult {
                status: Status::Error,
                data: None,
                error: Some(error),
            },
        }
    }
}

// ---------------------------------------------------------------------------
// Events

/// Something that happened to a device, on `bridge/event`. Not retained.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(
    feature = "unstable-asyncapi-docs",
    derive(JsonSchema, ToAsyncApiMessage)
)]
#[serde(tag = "type")]
pub enum BridgeEvent {
    /// A step of a commissioning, one per step.
    #[serde(rename = "commissioning")]
    #[cfg_attr(
        feature = "unstable-asyncapi-docs",
        asyncapi(summary = "A commissioning moved on a step")
    )]
    Commissioning {
        /// The node id the device is being given.
        node: u64,
        /// The step it reached.
        step: CommissionStep,
    },
    /// A device was commissioned, interviewed or renamed.
    #[serde(rename = "node_updated")]
    #[cfg_attr(
        feature = "unstable-asyncapi-docs",
        asyncapi(summary = "A device was commissioned, interviewed or renamed")
    )]
    NodeUpdated {
        /// The device as it is now.
        node: Box<Device>,
    },
    /// A device was removed.
    #[serde(rename = "node_removed")]
    #[cfg_attr(
        feature = "unstable-asyncapi-docs",
        asyncapi(summary = "A device was removed")
    )]
    NodeRemoved {
        /// Its node id.
        node: u64,
    },
    /// The bridge's subscription to a device started or lapsed.
    #[serde(rename = "availability")]
    #[cfg_attr(
        feature = "unstable-asyncapi-docs",
        asyncapi(summary = "A device became reachable or unreachable")
    )]
    Availability {
        /// Its node id.
        node: u64,
        /// Whether the subscription holds.
        online: bool,
    },
    /// A sleeping device woke and checked in: its subscription is renewed
    /// and requests that waited for it go out.
    #[serde(rename = "check_in")]
    #[cfg_attr(
        feature = "unstable-asyncapi-docs",
        asyncapi(summary = "A sleeping device checked in")
    )]
    CheckIn {
        /// Its node id.
        node: u64,
    },
    /// A step of a device's software update from the bridge.
    #[serde(rename = "ota")]
    #[cfg_attr(
        feature = "unstable-asyncapi-docs",
        asyncapi(summary = "A software update moved on a step")
    )]
    Ota {
        /// Its node id.
        node: u64,
        /// The step.
        step: OtaStep,
        /// The version offered, being applied, or now running.
        version: u32,
    },
}

/// A step of a commissioning.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "unstable-asyncapi-docs", derive(JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum CommissionStep {
    /// Looking for the device, over DNS-SD or Bluetooth LE.
    Discovery,
    /// Opening a session with the pairing code (PASE).
    Pase,
    /// Arming the device's fail-safe.
    ArmFailSafe,
    /// Checking the device's attestation.
    DeviceAttestation,
    /// Asking the device for its operational key.
    CsrRequest,
    /// Giving the device the fabric's root certificate.
    AddTrustedRootCertificate,
    /// Giving the device its operational certificate.
    AddNoc,
    /// Finding the device on its operational network.
    FindOperational,
    /// Giving a device reached over Bluetooth its Thread or Wi-Fi network.
    NetworkSetup,
    /// Waiting for that device to join the network.
    NetworkConnect,
    /// Completing the commissioning over the operational session (CASE).
    CommissioningComplete,
    /// Closing the commissioning's sessions.
    Cleanup,
    /// Reading what the device is.
    Interview,
}

impl From<matter_controller::CommissionStep> for CommissionStep {
    fn from(s: matter_controller::CommissionStep) -> Self {
        use matter_controller::CommissionStep as S;
        match s {
            S::Discovery => Self::Discovery,
            S::Pase => Self::Pase,
            S::ArmFailSafe => Self::ArmFailSafe,
            S::DeviceAttestation => Self::DeviceAttestation,
            S::CsrRequest => Self::CsrRequest,
            S::AddTrustedRootCertificate => Self::AddTrustedRootCertificate,
            S::AddNoc => Self::AddNoc,
            S::FindOperational => Self::FindOperational,
            S::NetworkSetup => Self::NetworkSetup,
            S::NetworkConnect => Self::NetworkConnect,
            S::CommissioningComplete => Self::CommissioningComplete,
            S::Cleanup => Self::Cleanup,
            S::Interview => Self::Interview,
        }
    }
}

/// A step of a software update, as the bridge sees it. The device's own OTA
/// Software Update Requestor events tell the download's progress.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "unstable-asyncapi-docs", derive(JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum OtaStep {
    /// The device asked for an update and was offered `version`.
    Offered,
    /// The device downloaded it and was told to apply it.
    Applying,
    /// The device reports it now runs `version`.
    Applied,
}

impl From<matter_controller::OtaStep> for OtaStep {
    fn from(s: matter_controller::OtaStep) -> Self {
        use matter_controller::OtaStep as S;
        match s {
            S::Offered => Self::Offered,
            S::Applying => Self::Applying,
            S::Applied => Self::Applied,
        }
    }
}

// ---------------------------------------------------------------------------
// Requests and responses

/// `ok` or `error`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "unstable-asyncapi-docs", derive(JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum Status {
    /// Done; `data` has the result.
    Ok,
    /// Not done; `error` says why.
    Error,
}

/// A cluster, attribute or command: its `snake_case` name from the data
/// model (any case, with or without underscores), or its id as a number.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "unstable-asyncapi-docs", derive(JsonSchema))]
#[serde(untagged)]
pub enum Id {
    /// The id.
    Number(u32),
    /// The name, or the id as `0x` hex or decimal text.
    Name(String),
}

impl Id {
    /// The id, resolving a name with `parse`.
    pub fn resolve(&self, parse: impl Fn(&str) -> Option<u32>) -> Option<u32> {
        match self {
            Id::Number(n) => Some(*n),
            Id::Name(s) => parse(s),
        }
    }
}

macro_rules! response {
    ($(#[$doc:meta])* $name:ident, $data:ty, $summary:literal) => {
        $(#[$doc])*
        ///
        /// A response is published once per request, to
        /// `bridge/response/<request>`, and is not retained.
        #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
        #[cfg_attr(feature = "unstable-asyncapi-docs", derive(JsonSchema, ToAsyncApiMessage))]
        #[cfg_attr(feature = "unstable-asyncapi-docs", asyncapi(summary = $summary))]
        pub struct $name {
            /// `ok` or `error`.
            pub status: Status,
            /// The result, when `status` is `ok`.
            #[serde(default, skip_serializing_if = "Option::is_none")]
            pub data: Option<$data>,
            /// What went wrong, when `status` is `error`.
            #[serde(default, skip_serializing_if = "Option::is_none")]
            pub error: Option<String>,
            /// The request's `transaction`, echoed when it had one.
            #[serde(default, skip_serializing_if = "Option::is_none")]
            pub transaction: Option<Json>,
        }

        impl $name {
            /// The response to a request with `transaction`.
            pub fn new(result: Result<$data, String>, transaction: Option<Json>) -> Self {
                match result {
                    Ok(data) => $name { status: Status::Ok, data: Some(data), error: None, transaction },
                    Err(error) => $name { status: Status::Error, data: None, error: Some(error), transaction },
                }
            }
        }
    };
}

macro_rules! request {
    ($(#[$doc:meta])* $name:ident, $summary:literal { $($(#[$fdoc:meta])* $field:ident : $ty:ty),* $(,)? }) => {
        $(#[$doc])*
        #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
        #[cfg_attr(feature = "unstable-asyncapi-docs", derive(JsonSchema, ToAsyncApiMessage))]
        #[cfg_attr(feature = "unstable-asyncapi-docs", asyncapi(summary = $summary))]
        pub struct $name {
            $($(#[$fdoc])* pub $field: $ty,)*
            /// Any value; the response echoes it, so a client can match the
            /// two.
            #[serde(default, skip_serializing_if = "Option::is_none")]
            pub transaction: Option<Json>,
        }
    };
}

/// The answer to a request the bridge does not know, on
/// `bridge/response/<request>`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "unstable-asyncapi-docs", derive(JsonSchema))]
pub struct UnknownRequestResponse {
    /// Always `error`.
    pub status: Status,
    /// Which request is unknown.
    pub error: String,
    /// The request's `transaction`, echoed when it had one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transaction: Option<Json>,
}

impl UnknownRequestResponse {
    /// The answer, with `transaction` echoed.
    pub fn new(error: String, transaction: Option<Json>) -> Self {
        UnknownRequestResponse {
            status: Status::Error,
            error,
            transaction,
        }
    }
}

request!(
    /// Commission a device, on `bridge/request/commission`: found by its
    /// pairing code over DNS-SD, or over Bluetooth LE when the bridge has a
    /// Bluetooth controller and the device is not on the network yet.
    CommissionRequest, "Commission a device" {
        /// The QR code's payload (`MT:...`) or the manual pairing code,
        /// with or without dashes.
        code: String,
        /// The device's name; `node_<node id>` if left out. It must not be
        /// `bridge`, or contain `/`, `#` or `+`.
        #[serde(default)]
        name: Option<String>,
        /// The device's address as `ip:port`, to skip discovery.
        #[serde(default)]
        address: Option<String>,
        /// Seconds to look for the device; `matter.discovery_timeout` if
        /// left out.
        #[serde(default)]
        timeout: Option<u16>,
        /// For a Wi-Fi device reached over Bluetooth: the network to give
        /// it. Never stored or logged.
        #[serde(default)]
        wifi: Option<WiFiCredentials>,
        /// For a Thread device reached over Bluetooth: the network's active
        /// operational dataset, as hex. Never stored or logged.
        #[serde(default)]
        thread_dataset: Option<String>,
        /// Where to read the Thread dataset instead: an OpenThread border
        /// router's REST API, e.g.
        /// `http://127.0.0.1:8081/node/dataset/active`. Without either,
        /// `matter.thread_dataset_url` is used.
        #[serde(default)]
        thread_dataset_url: Option<String>,
    }
);

/// A Wi-Fi network to give a device.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "unstable-asyncapi-docs", derive(JsonSchema))]
pub struct WiFiCredentials {
    /// The network's name.
    pub ssid: String,
    /// Its passphrase; empty or left out for an open network.
    #[serde(default)]
    pub password: String,
}

// The passphrase is a secret: never in a log.
impl std::fmt::Debug for WiFiCredentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WiFiCredentials")
            .field("ssid", &self.ssid)
            .finish_non_exhaustive()
    }
}

response!(
    /// The response to `commission`: the device, interviewed.
    CommissionResponse, Device, "The device is commissioned"
);

request!(
    /// Open a commissioning window on a device, on
    /// `bridge/request/open_commissioning_window`, so another
    /// administrator (Apple Home, Google Home, Home Assistant) can add it
    /// too.
    OpenCommissioningWindowRequest, "Let another ecosystem add a device" {
        /// The device, by name or node id.
        device: String,
        /// Seconds the window stays open, 180 to 900; 300 if left out.
        #[serde(default)]
        timeout: Option<u16>,
    }
);

/// An open commissioning window.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "unstable-asyncapi-docs", derive(JsonSchema))]
pub struct CommissioningWindow {
    /// The device's node id.
    pub node: u64,
    /// The pairing code to enter in the other app, `XXXX-XXX-XXXX`.
    pub manual_code: String,
    /// The same as a QR code payload (`MT:...`).
    pub qr_code: String,
    /// The discriminator the device advertises while the window is open.
    pub discriminator: u16,
    /// Seconds the window stays open.
    pub timeout: u16,
}

impl From<matter_controller::CommissioningWindow> for CommissioningWindow {
    fn from(w: matter_controller::CommissioningWindow) -> Self {
        CommissioningWindow {
            node: w.node,
            manual_code: w.manual_code,
            qr_code: w.qr_code,
            discriminator: w.discriminator,
            timeout: w.timeout,
        }
    }
}

response!(
    /// The response to `open_commissioning_window`.
    OpenCommissioningWindowResponse, CommissioningWindow, "The window is open, with its pairing code"
);

request!(
    /// Read attributes, on `bridge/request/read`. Any of `endpoint`,
    /// `cluster` and `attribute` left out means all of them.
    ReadRequest, "Read attributes" {
        /// The device, by name or node id.
        device: String,
        /// The endpoint.
        #[serde(default)]
        endpoint: Option<u16>,
        /// The cluster, by name or id.
        #[serde(default)]
        cluster: Option<Id>,
        /// The attribute, by name (with `cluster`) or id.
        #[serde(default)]
        attribute: Option<Id>,
    }
);

/// An attribute read.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "unstable-asyncapi-docs", derive(JsonSchema))]
pub struct ReadValue {
    /// The endpoint.
    pub endpoint: u16,
    /// The cluster's name, or its id in hex.
    pub cluster: String,
    /// The attribute's name, or its id in hex.
    pub attribute: String,
    /// Its value, `null` when `status` says why there is none.
    pub value: Json,
    /// The status the device gave instead of a value, such as
    /// `UnsupportedAttribute`; `null` for a value.
    pub status: Option<String>,
}

response!(
    /// The response to `read`.
    ReadResponse, Vec<ReadValue>, "The attributes' values"
);

request!(
    /// Write an attribute, on `bridge/request/write`.
    WriteRequest, "Write an attribute" {
        /// The device, by name or node id.
        device: String,
        /// The endpoint.
        endpoint: u16,
        /// The cluster, by name or id.
        cluster: Id,
        /// The attribute, by name or id.
        attribute: Id,
        /// The new value, in the form the attribute is published in.
        value: Json,
        /// Milliseconds for a timed write, which some clusters require
        /// (door locks, for example).
        #[serde(default)]
        timed: Option<u16>,
    }
);

response!(
    /// The response to `write`; `data` is `null`.
    WriteResponse, (), "The attribute is written"
);

request!(
    /// Invoke a command, on `bridge/request/invoke`.
    InvokeRequest, "Invoke a command" {
        /// The device, by name or node id.
        device: String,
        /// The endpoint.
        endpoint: u16,
        /// The cluster, by name or id.
        cluster: Id,
        /// The command, by name or id.
        command: Id,
        /// The command's fields, by the data model's names.
        #[serde(default)]
        fields: Option<Json>,
        /// Milliseconds for a timed invoke, which some commands require.
        #[serde(default)]
        timed: Option<u16>,
    }
);

response!(
    /// The response to `invoke`: the response command's fields, or `null`
    /// for a plain success.
    InvokeResponse, Json, "The command's response"
);

request!(
    /// Put an endpoint of a device in a group, on
    /// `bridge/request/add_to_group`: the device gets the fabric's group
    /// key and the group, with access for the group to operate the
    /// endpoint.
    AddToGroupRequest, "Put an endpoint in a group" {
        /// The device, by name or node id.
        device: String,
        /// The endpoint.
        endpoint: u16,
        /// The group, 1 to 65535.
        group: u16,
        /// The group's name on the device.
        #[serde(default)]
        name: Option<String>,
    }
);

response!(
    /// The response to `add_to_group`; `data` is `null`.
    AddToGroupResponse, (), "The endpoint is in the group"
);

request!(
    /// Take an endpoint of a device out of a group, on
    /// `bridge/request/remove_from_group`.
    RemoveFromGroupRequest, "Take an endpoint out of a group" {
        /// The device, by name or node id.
        device: String,
        /// The endpoint.
        endpoint: u16,
        /// The group, 1 to 65535.
        group: u16,
    }
);

response!(
    /// The response to `remove_from_group`; `data` is `null`.
    RemoveFromGroupResponse, (), "The endpoint left the group"
);

request!(
    /// Send one multicast command to every member of a group, on
    /// `bridge/request/group_invoke`. Group commands are not acknowledged;
    /// each member's new state comes back through its subscription.
    GroupInvokeRequest, "Send a command to a group" {
        /// The group, 1 to 65535.
        group: u16,
        /// The cluster, by name or id.
        cluster: Id,
        /// The command, by name or id.
        command: Id,
        /// The command's fields, by the data model's names.
        #[serde(default)]
        fields: Option<Json>,
    }
);

response!(
    /// The response to `group_invoke`, once the message is sent; `data` is
    /// `null`.
    GroupInvokeResponse, (), "The group command is sent"
);

request!(
    /// Read what a device is again, on `bridge/request/interview`.
    InterviewRequest, "Interview a device again" {
        /// The device, by name or node id.
        device: String,
    }
);

response!(
    /// The response to `interview`: the device as it is now.
    InterviewResponse, Device, "The device, interviewed"
);

request!(
    /// Rename a device, on `bridge/request/rename`. Its topics move to the
    /// new name; values come back under it with the next report.
    RenameRequest, "Rename a device" {
        /// The device, by name or node id.
        device: String,
        /// The new name. It must not be `bridge`, or contain `/`, `#` or
        /// `+`.
        to: String,
    }
);

response!(
    /// The response to `rename`: the device as it is now.
    RenameResponse, Device, "The device is renamed"
);

request!(
    /// Remove a device, on `bridge/request/remove`: the bridge takes its
    /// fabric off the device, then forgets it.
    RemoveRequest, "Remove a device" {
        /// The device, by name or node id.
        device: String,
        /// Forget the device even if it cannot be reached.
        #[serde(default)]
        force: bool,
    }
);

/// A device removed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "unstable-asyncapi-docs", derive(JsonSchema))]
pub struct Removed {
    /// Its node id.
    pub node: u64,
}

response!(
    /// The response to `remove`.
    RemoveResponse, Removed, "The device is removed"
);

request!(
    /// Look for devices that can be commissioned, on
    /// `bridge/request/discover`.
    DiscoverRequest, "Look for commissionable devices" {
        /// Seconds to look; 5 if left out.
        #[serde(default)]
        timeout: Option<u16>,
    }
);

/// A device that can be commissioned, seen over DNS-SD.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "unstable-asyncapi-docs", derive(JsonSchema))]
pub struct Commissionable {
    /// Its DNS-SD instance name.
    pub instance: String,
    /// Its addresses, as `ip:port`.
    pub addresses: Vec<String>,
}

impl From<matter_controller::Commissionable> for Commissionable {
    fn from(c: matter_controller::Commissionable) -> Self {
        Commissionable {
            instance: c.instance,
            addresses: c.addresses.iter().map(ToString::to_string).collect(),
        }
    }
}

response!(
    /// The response to `discover`.
    DiscoverResponse, Vec<Commissionable>, "The commissionable devices seen"
);

request!(
    /// Publish `bridge/devices` again, on `bridge/request/devices`, and
    /// answer with the same list.
    DevicesRequest, "List the devices" {}
);

response!(
    /// The response to `devices`.
    DevicesResponse, Vec<Device>, "Every device"
);

request!(
    /// List the OTA images the bridge offers, on
    /// `bridge/request/ota_images`.
    OtaImagesRequest, "List the software update images" {}
);

/// A Matter OTA image file in `matter.ota_dir`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "unstable-asyncapi-docs", derive(JsonSchema))]
pub struct OtaImage {
    /// The file's name.
    pub file: String,
    /// The vendor it is for; 0 for any.
    pub vendor_id: u16,
    /// The product it is for; 0 for any.
    pub product_id: u16,
    /// The software version it installs.
    pub version: u32,
    /// The version as text.
    pub version_str: String,
    /// The oldest version it applies on top of.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_applicable_version: Option<u32>,
    /// The newest version it applies on top of.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_applicable_version: Option<u32>,
    /// The file's size in bytes.
    pub size: u64,
}

impl From<matter_controller::OtaImage> for OtaImage {
    fn from(i: matter_controller::OtaImage) -> Self {
        OtaImage {
            file: i.file,
            vendor_id: i.vendor_id,
            product_id: i.product_id,
            version: i.version,
            version_str: i.version_str,
            min_applicable_version: i.min_applicable_version,
            max_applicable_version: i.max_applicable_version,
            size: i.size,
        }
    }
}

response!(
    /// The response to `ota_images`.
    OtaImagesResponse, Vec<OtaImage>, "The images offered"
);

request!(
    /// Tell a device to ask the bridge for a software update now, on
    /// `bridge/request/ota_update`. It downloads the image over BDX and
    /// applies it; `ota` bridge events follow.
    OtaUpdateRequest, "Update a device's software" {
        /// The device, by name or node id.
        device: String,
    }
);

response!(
    /// The response to `ota_update`: the image the device will be offered,
    /// or `null` (and no announcement) when none is newer.
    OtaUpdateResponse, Option<OtaImage>, "The image the device will be offered"
);

// ---------------------------------------------------------------------------
// Home Assistant

/// Home Assistant's birth and last will, on `homeassistant/status`
/// (`homeassistant.status_topic`). When it says `online`, the bridge
/// publishes discovery again. Plain text, not JSON.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(
    feature = "unstable-asyncapi-docs",
    derive(JsonSchema, ToAsyncApiMessage)
)]
#[cfg_attr(
    feature = "unstable-asyncapi-docs",
    asyncapi(
        summary = "Home Assistant started or stopped",
        content_type = "text/plain"
    )
)]
#[serde(transparent)]
pub struct HomeAssistantStatus(pub Availability);

impl HomeAssistantStatus {
    /// The status in a payload, if it is one.
    pub fn parse(payload: &[u8]) -> Option<Availability> {
        match payload {
            b"online" => Some(Availability::Online),
            b"offline" => Some(Availability::Offline),
            _ => None,
        }
    }
}

/// A Home Assistant MQTT discovery config, retained on
/// `<discovery_prefix>/<component>/<device key>/<endpoint>_<cluster>/config`:
/// one entity per light, switch, sensor or battery. An empty retained
/// message removes the entity. Keys are Home Assistant's; see its MQTT
/// integration's documentation.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(
    feature = "unstable-asyncapi-docs",
    derive(JsonSchema, ToAsyncApiMessage)
)]
#[cfg_attr(
    feature = "unstable-asyncapi-docs",
    asyncapi(summary = "A Home Assistant entity for one of a device's endpoints")
)]
pub struct DiscoveryConfig {
    /// The entity's name.
    pub name: String,
    /// Unique across Home Assistant:
    /// `matter_<compressed fabric id>_<node id>_<endpoint>_<cluster>`.
    pub unique_id: String,
    /// The entity id Home Assistant derives: `<device>_<endpoint>_<cluster>`.
    pub object_id: String,
    /// The device the entity belongs to.
    pub device: DiscoveryDevice,
    /// The bridge's and the device's availability topics.
    pub availability: Vec<DiscoveryAvailability>,
    /// `all`: the entity is available while both are.
    pub availability_mode: String,
    /// The attribute topic the state is read from.
    pub state_topic: String,
    /// Picks the state out of the attribute's value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value_template: Option<String>,
    /// Lights: turns the on/off attribute into `ON` or `OFF`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state_value_template: Option<String>,
    /// Lights and switches: the endpoint's `on_off/set` topic.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command_topic: Option<String>,
    /// Switches: the state that means on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state_on: Option<String>,
    /// Switches: the state that means off.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state_off: Option<String>,
    /// Dimmable lights: the endpoint's `level_control/set` topic.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub brightness_command_topic: Option<String>,
    /// Dimmable lights: the current level's topic.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub brightness_state_topic: Option<String>,
    /// Dimmable lights: the highest level, 254.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub brightness_scale: Option<u16>,
    /// Dimmable lights: reads the level, 0 while it is null.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub brightness_value_template: Option<String>,
    /// Home Assistant's device class: `temperature`, `door`, `battery`...
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_class: Option<String>,
    /// The reading's unit, after the template's scaling.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit_of_measurement: Option<String>,
    /// `measurement` for sensors.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state_class: Option<String>,
    /// `diagnostic` for the battery.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entity_category: Option<String>,
}

/// The device an entity belongs to, in Home Assistant's device registry.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "unstable-asyncapi-docs", derive(JsonSchema))]
pub struct DiscoveryDevice {
    /// `matter_<compressed fabric id>_<node id>`.
    pub identifiers: Vec<String>,
    /// The device's name.
    pub name: String,
    /// The vendor's name, from Basic Information.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manufacturer: Option<String>,
    /// The product's name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// The software version.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sw_version: Option<String>,
    /// The hardware version.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hw_version: Option<String>,
    /// The serial number.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub serial_number: Option<String>,
    /// The bridge: `matter_bridge_<compressed fabric id>`.
    pub via_device: String,
}

/// An availability topic Home Assistant watches.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "unstable-asyncapi-docs", derive(JsonSchema))]
pub struct DiscoveryAvailability {
    /// The topic.
    pub topic: String,
    /// Picks `online` or `offline` out of the payload, for `bridge/state`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value_template: Option<String>,
}
