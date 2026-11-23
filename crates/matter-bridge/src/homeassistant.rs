//! Home Assistant MQTT discovery, from what a node says it is: entities are
//! chosen by the clusters an endpoint serves (and its device type, where the
//! cluster alone is ambiguous), so no per-device table is needed.

use matter_controller::Node;

use crate::api::{DiscoveryAvailability, DiscoveryConfig, DiscoveryDevice};
use crate::topics;

const ON_OFF: u32 = 0x0006;
const LEVEL_CONTROL: u32 = 0x0008;
const POWER_SOURCE: u32 = 0x002F;
const BOOLEAN_STATE: u32 = 0x0045;
const ILLUMINANCE: u32 = 0x0400;
const TEMPERATURE: u32 = 0x0402;
const PRESSURE: u32 = 0x0403;
const HUMIDITY: u32 = 0x0405;
const OCCUPANCY: u32 = 0x0406;

/// Light device types: On/Off, Dimmable, Color Temperature and Extended
/// Color Light.
const LIGHTS: [u32; 4] = [0x0100, 0x0101, 0x010C, 0x010D];
const CONTACT_SENSOR: u32 = 0x0015;
const WATER_LEAK_DETECTOR: u32 = 0x0043;

/// Where discovery goes and what the bridge's topics are.
pub struct Discovery<'a> {
    /// Home Assistant's discovery prefix.
    pub prefix: &'a str,
    /// The bridge's base topic.
    pub base: &'a str,
    /// The compressed fabric id, which keeps ids unique across fabrics.
    pub fabric: u64,
}

fn some(s: &str) -> Option<String> {
    Some(s.to_string())
}

impl Discovery<'_> {
    fn node_key(&self, node: &Node) -> String {
        format!("matter_{:016x}_{:016x}", self.fabric, node.node_id)
    }

    /// Every discovery message for `node`, as (topic, config).
    pub fn configs(&self, node: &Node) -> Vec<(String, DiscoveryConfig)> {
        let base = self.base;
        let name = &node.name;
        let key = self.node_key(node);
        let device = DiscoveryDevice {
            identifiers: vec![key.clone()],
            name: name.clone(),
            manufacturer: node.basic.vendor_name.clone(),
            model: node.basic.product_name.clone(),
            sw_version: node.basic.software_version.clone(),
            hw_version: node.basic.hardware_version.clone(),
            serial_number: node.basic.serial_number.clone(),
            via_device: format!("matter_bridge_{:016x}", self.fabric),
        };
        let availability = vec![
            DiscoveryAvailability {
                topic: format!("{base}/bridge/state"),
                value_template: some("{{ value_json.state }}"),
            },
            DiscoveryAvailability {
                topic: format!("{base}/{name}/availability"),
                value_template: None,
            },
        ];

        let mut out = Vec::new();
        for (&ep, endpoint) in &node.endpoints {
            let serves = |c: u32| endpoint.server_clusters.contains(&c);
            let is = |t: u32| endpoint.device_types.iter().any(|d| d.id == t);
            let topic = |cluster: u32, attribute: u32| {
                format!(
                    "{base}/{name}/{ep}/{}/{}",
                    topics::cluster(cluster),
                    topics::attribute(cluster, attribute)
                )
            };
            // An entity of `component` for `cluster`, its state on
            // `state_topic`; `extra` fills in the rest.
            let mut entity = |component: &str,
                              cluster: u32,
                              label: &str,
                              state_topic: String,
                              extra: DiscoveryConfig| {
                let config = DiscoveryConfig {
                    name: label.to_string(),
                    unique_id: format!("{key}_{ep}_{cluster:x}"),
                    object_id: format!("{name}_{ep}_{}", topics::cluster(cluster)),
                    device: device.clone(),
                    availability: availability.clone(),
                    availability_mode: "all".into(),
                    state_topic,
                    ..extra
                };
                out.push((
                    format!("{}/{component}/{key}/{ep}_{cluster:x}/config", self.prefix),
                    config,
                ));
            };

            // The root endpoint carries only the node's own clusters; a
            // power source there is the node's battery.
            if ep == 0 && !serves(POWER_SOURCE) {
                continue;
            }

            if serves(ON_OFF) {
                let command_topic = Some(format!("{base}/{name}/{ep}/on_off/set"));
                if LIGHTS.iter().any(|&t| is(t)) {
                    let mut light = DiscoveryConfig {
                        command_topic,
                        state_value_template: some("{{ 'ON' if value_json else 'OFF' }}"),
                        ..Default::default()
                    };
                    if serves(LEVEL_CONTROL) {
                        light.brightness_command_topic =
                            Some(format!("{base}/{name}/{ep}/level_control/set"));
                        light.brightness_state_topic = Some(topic(LEVEL_CONTROL, 0));
                        light.brightness_scale = Some(254);
                        light.brightness_value_template =
                            some("{{ value_json if value_json is not none else 0 }}");
                    }
                    entity("light", ON_OFF, "Light", topic(ON_OFF, 0), light);
                } else {
                    let switch = DiscoveryConfig {
                        command_topic,
                        value_template: some("{{ 'ON' if value_json else 'OFF' }}"),
                        state_on: some("ON"),
                        state_off: some("OFF"),
                        ..Default::default()
                    };
                    entity("switch", ON_OFF, "Switch", topic(ON_OFF, 0), switch);
                }
            }

            let mut sensor =
                |cluster: u32, label: &str, class: &str, unit: &str, template: &str| {
                    entity(
                        "sensor",
                        cluster,
                        label,
                        topic(cluster, 0),
                        DiscoveryConfig {
                            value_template: some(template),
                            device_class: some(class),
                            unit_of_measurement: some(unit),
                            state_class: some("measurement"),
                            ..Default::default()
                        },
                    );
                };
            if serves(TEMPERATURE) {
                sensor(
                    TEMPERATURE,
                    "Temperature",
                    "temperature",
                    "°C",
                    "{{ (value_json / 100) | round(2) if value_json is not none else none }}",
                );
            }
            if serves(HUMIDITY) {
                sensor(
                    HUMIDITY,
                    "Humidity",
                    "humidity",
                    "%",
                    "{{ (value_json / 100) | round(2) if value_json is not none else none }}",
                );
            }
            if serves(PRESSURE) {
                sensor(
                    PRESSURE,
                    "Pressure",
                    "pressure",
                    "kPa",
                    "{{ (value_json / 10) | round(1) if value_json is not none else none }}",
                );
            }
            if serves(ILLUMINANCE) {
                sensor(
                    ILLUMINANCE,
                    "Illuminance",
                    "illuminance",
                    "lx",
                    "{{ (10 ** ((value_json - 1) / 10000)) | round(1) if value_json else 0 }}",
                );
            }

            if serves(OCCUPANCY) {
                entity(
                    "binary_sensor",
                    OCCUPANCY,
                    "Occupancy",
                    topic(OCCUPANCY, 0),
                    DiscoveryConfig {
                        value_template: some(
                            "{{ 'ON' if (value_json | int) | bitwise_and(1) else 'OFF' }}",
                        ),
                        device_class: some("occupancy"),
                        ..Default::default()
                    },
                );
            }
            if serves(BOOLEAN_STATE) {
                // A contact sensor's state is true when in contact (closed);
                // Home Assistant's door is on when open.
                let (class, template) = if is(CONTACT_SENSOR) {
                    (some("door"), "{{ 'OFF' if value_json else 'ON' }}")
                } else if is(WATER_LEAK_DETECTOR) {
                    (some("moisture"), "{{ 'ON' if value_json else 'OFF' }}")
                } else {
                    (None, "{{ 'ON' if value_json else 'OFF' }}")
                };
                entity(
                    "binary_sensor",
                    BOOLEAN_STATE,
                    "State",
                    topic(BOOLEAN_STATE, 0),
                    DiscoveryConfig {
                        value_template: some(template),
                        device_class: class,
                        ..Default::default()
                    },
                );
            }
            if serves(POWER_SOURCE) {
                entity(
                    "sensor",
                    POWER_SOURCE,
                    "Battery",
                    topic(POWER_SOURCE, 0x000C),
                    DiscoveryConfig {
                        value_template: some(
                            "{{ (value_json / 2) | round(0) if value_json is not none else none }}",
                        ),
                        device_class: some("battery"),
                        unit_of_measurement: some("%"),
                        state_class: some("measurement"),
                        entity_category: some("diagnostic"),
                        ..Default::default()
                    },
                );
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use matter_controller::{BasicInfo, DeviceType, Endpoint};

    fn node(device_type: u32, clusters: &[u32]) -> Node {
        let mut endpoints = std::collections::BTreeMap::new();
        endpoints.insert(
            0,
            Endpoint {
                device_types: vec![DeviceType {
                    id: 0x0016,
                    revision: 1,
                }],
                server_clusters: vec![0x001D, 0x0028],
                ..Default::default()
            },
        );
        endpoints.insert(
            1,
            Endpoint {
                device_types: vec![DeviceType {
                    id: device_type,
                    revision: 1,
                }],
                server_clusters: clusters.to_vec(),
                ..Default::default()
            },
        );
        Node {
            node_id: 2,
            name: "lamp".into(),
            basic: BasicInfo::default(),
            endpoints,
            interviewed: true,
            commissioned_at: 0,
        }
    }

    const D: Discovery<'static> = Discovery {
        prefix: "homeassistant",
        base: "matter",
        fabric: 0xAB,
    };

    #[test]
    fn dimmable_light() {
        let configs = D.configs(&node(0x0101, &[ON_OFF, LEVEL_CONTROL]));
        assert_eq!(configs.len(), 1);
        let (topic, config) = &configs[0];
        assert_eq!(
            topic,
            "homeassistant/light/matter_00000000000000ab_0000000000000002/1_6/config"
        );
        assert_eq!(
            config.command_topic.as_deref(),
            Some("matter/lamp/1/on_off/set")
        );
        assert_eq!(config.state_topic, "matter/lamp/1/on_off/on_off");
        assert_eq!(
            config.brightness_state_topic.as_deref(),
            Some("matter/lamp/1/level_control/current_level")
        );
    }

    #[test]
    fn plug_and_sensors() {
        let configs = D.configs(&node(0x010A, &[ON_OFF, TEMPERATURE, HUMIDITY]));
        let components: Vec<_> = configs
            .iter()
            .map(|(t, _)| t.split('/').nth(1).unwrap().to_string())
            .collect();
        assert_eq!(components, ["switch", "sensor", "sensor"]);
        assert_eq!(
            configs[1].1.state_topic,
            "matter/lamp/1/temperature_measurement/measured_value"
        );
    }

    #[test]
    fn contact_sensor_is_a_door() {
        let configs = D.configs(&node(CONTACT_SENSOR, &[BOOLEAN_STATE]));
        assert_eq!(configs[0].1.device_class.as_deref(), Some("door"));
        assert_eq!(
            configs[0].1.state_topic,
            "matter/lamp/1/boolean_state/state_value"
        );
    }
}
