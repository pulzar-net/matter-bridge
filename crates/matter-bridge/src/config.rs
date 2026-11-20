//! The bridge's configuration file (TOML). Every section is optional.

use std::path::{Path, PathBuf};

use serde::Deserialize;

/// The whole configuration.
#[derive(Debug, Clone, Default, Deserialize)]
#[cfg_attr(feature = "schema", derive(serde::Serialize, schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// The MQTT broker.
    #[serde(default)]
    pub mqtt: MqttConfig,
    /// Home Assistant discovery.
    #[serde(default)]
    pub homeassistant: HomeAssistantConfig,
    /// The Matter controller.
    #[serde(default)]
    pub matter: MatterConfig,
    /// Where state lives.
    #[serde(default)]
    pub data: DataConfig,
}

impl Config {
    /// Read the configuration at `path`.
    pub fn load(path: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        toml::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))
    }
}

/// The MQTT broker.
#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "schema", derive(serde::Serialize, schemars::JsonSchema))]
#[serde(deny_unknown_fields, default)]
pub struct MqttConfig {
    /// Broker host.
    pub host: String,
    /// Broker port.
    pub port: u16,
    /// User name, if the broker wants one.
    pub username: Option<String>,
    /// A file holding the password (so it is not in the configuration).
    pub password_file: Option<PathBuf>,
    /// Client id.
    pub client_id: String,
    /// Prefix of every topic the bridge uses.
    pub base_topic: String,
    /// Keep-alive, seconds.
    pub keepalive: u64,
}

impl Default for MqttConfig {
    fn default() -> Self {
        MqttConfig {
            host: "localhost".into(),
            port: 1883,
            username: None,
            password_file: None,
            client_id: "matter-bridge".into(),
            base_topic: "matter".into(),
            keepalive: 30,
        }
    }
}

/// Home Assistant discovery.
#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "schema", derive(serde::Serialize, schemars::JsonSchema))]
#[serde(deny_unknown_fields, default)]
pub struct HomeAssistantConfig {
    /// Publish discovery messages.
    pub enabled: bool,
    /// Home Assistant's discovery prefix.
    pub discovery_prefix: String,
    /// Home Assistant's birth topic; discovery is re-sent when it says "online".
    pub status_topic: String,
}

impl Default for HomeAssistantConfig {
    fn default() -> Self {
        HomeAssistantConfig {
            enabled: true,
            discovery_prefix: "homeassistant".into(),
            status_topic: "homeassistant/status".into(),
        }
    }
}

/// The Matter controller.
#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "schema", derive(serde::Serialize, schemars::JsonSchema))]
#[serde(deny_unknown_fields, default)]
pub struct MatterConfig {
    /// UDP port for Matter.
    pub port: u16,
    /// Network interface for mDNS; the first with IPv6 if unset.
    pub interface: Option<String>,
    /// Fabric id of a new fabric (random if unset). Ignored once the fabric
    /// exists.
    pub fabric_id: Option<u64>,
    /// Vendor id the controller administers nodes as.
    pub vendor_id: u16,
    /// Commission devices whose attestation is not verified. Required for
    /// now: verification is not implemented yet (see the README).
    pub allow_unverified_attestation: bool,
    /// Longest silence before a subscription is renewed, seconds.
    pub max_report_interval: u16,
    /// How long to look for a device to commission, seconds.
    pub discovery_timeout: u16,
    /// The Bluetooth controller for commissioning over BLE: `hci0` (the
    /// Linux HCI user channel; BlueZ must not hold it) or `tcp:host:port`.
    pub bluetooth: Option<String>,
    /// Where to read the Thread network's dataset for a Thread device: an
    /// OpenThread border router's REST API, e.g.
    /// `http://127.0.0.1:8081/node/dataset/active`.
    pub thread_dataset_url: Option<String>,
}

impl Default for MatterConfig {
    fn default() -> Self {
        let c = matter_controller::Config::new("");
        MatterConfig {
            port: c.port,
            interface: None,
            fabric_id: None,
            vendor_id: c.vendor_id,
            allow_unverified_attestation: c.allow_unverified_attestation,
            max_report_interval: c.max_report_interval,
            discovery_timeout: 30,
            bluetooth: None,
            thread_dataset_url: None,
        }
    }
}

/// Where state lives.
#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "schema", derive(serde::Serialize, schemars::JsonSchema))]
#[serde(deny_unknown_fields, default)]
pub struct DataConfig {
    /// The directory for the fabric, the controller's identity and the node
    /// store.
    pub dir: PathBuf,
}

impl Default for DataConfig {
    fn default() -> Self {
        DataConfig {
            dir: "/var/lib/matter-bridge".into(),
        }
    }
}

impl Config {
    /// The controller's configuration.
    pub fn controller(&self) -> matter_controller::Config {
        let mut c = matter_controller::Config::new(&self.data.dir);
        c.port = self.matter.port;
        c.interface = self.matter.interface.clone();
        c.fabric_id = self.matter.fabric_id;
        c.vendor_id = self.matter.vendor_id;
        c.allow_unverified_attestation = self.matter.allow_unverified_attestation;
        c.max_report_interval = self.matter.max_report_interval;
        c.bluetooth = self.matter.bluetooth.clone();
        c
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn example_parses() {
        let text = include_str!("../../../config.example.toml");
        let config: Config = toml::from_str(text).unwrap();
        assert_eq!(config.mqtt.base_topic, "matter");
        assert!(config.homeassistant.enabled);
    }

    #[test]
    fn empty_is_all_defaults() {
        let config: Config = toml::from_str("").unwrap();
        assert_eq!(config.mqtt.port, 1883);
        assert_eq!(config.matter.port, 5541);
    }

    #[test]
    fn unknown_keys_are_refused() {
        assert!(toml::from_str::<Config>("[mqtt]\nhots = \"x\"").is_err());
    }
}
