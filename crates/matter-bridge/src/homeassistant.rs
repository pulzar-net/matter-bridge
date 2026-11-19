//! Home Assistant MQTT discovery, from what a node says it is: entities are
//! chosen by the clusters an endpoint serves (and its device type, where the
//! cluster alone is ambiguous), so no per-device table is needed.

use matter_controller::Node;
use serde_json::{json, Value};

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

impl Discovery<'_> {
    fn node_key(&self, node: &Node) -> String {
        format!("matter_{:016x}_{:016x}", self.fabric, node.node_id)
    }

    /// Every discovery message for `node`, as (topic, config).
    pub fn configs(&self, node: &Node) -> Vec<(String, Value)> {
        let base = self.base;
        let name = &node.name;
        let key = self.node_key(node);
        let device = json!({
            "identifiers": [key],
            "name": name,
            "manufacturer": node.basic.vendor_name,
            "model": node.basic.product_name,
            "sw_version": node.basic.software_version,
            "hw_version": node.basic.hardware_version,
            "serial_number": node.basic.serial_number,
            "via_device": format!("matter_bridge_{:016x}", self.fabric),
        });
        let availability = json!([
            {"topic": format!("{base}/bridge/state"), "value_template": "{{ value_json.state }}"},
            {"topic": format!("{base}/{name}/availability")},
        ]);

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
            let mut entity = |component: &str, cluster: u32, label: &str, extra: Value| {
                let mut config = json!({
                    "name": label,
                    "unique_id": format!("{key}_{ep}_{cluster:x}"),
                    "object_id": format!("{name}_{ep}_{}", topics::cluster(cluster)),
                    "device": device,
                    "availability": availability,
                    "availability_mode": "all",
                });
                merge(&mut config, extra);
                strip_nulls(&mut config);
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
                let state = json!({
                    "command_topic": format!("{base}/{name}/{ep}/on_off/set"),
                    "state_topic": topic(ON_OFF, 0),
                });
                if LIGHTS.iter().any(|&t| is(t)) {
                    let mut light = json!({
                        "state_value_template": "{{ 'ON' if value_json else 'OFF' }}",
                    });
                    merge(&mut light, state);
                    if serves(LEVEL_CONTROL) {
                        merge(
                            &mut light,
                            json!({
                                "brightness_command_topic": format!("{base}/{name}/{ep}/level_control/set"),
                                "brightness_state_topic": topic(LEVEL_CONTROL, 0),
                                "brightness_scale": 254,
                                "brightness_value_template": "{{ value_json if value_json is not none else 0 }}",
                            }),
                        );
                    }
                    entity("light", ON_OFF, "Light", light);
                } else {
                    let mut switch = json!({
                        "value_template": "{{ 'ON' if value_json else 'OFF' }}",
                        "state_on": "ON",
                        "state_off": "OFF",
                    });
                    merge(&mut switch, state);
                    entity("switch", ON_OFF, "Switch", switch);
                }
            }

            let mut sensor =
                |cluster: u32, label: &str, class: &str, unit: &str, template: &str| {
                    entity(
                        "sensor",
                        cluster,
                        label,
                        json!({
                            "state_topic": topic(cluster, 0),
                            "value_template": template,
                            "device_class": class,
                            "unit_of_measurement": unit,
                            "state_class": "measurement",
                        }),
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
                    json!({
                        "state_topic": topic(OCCUPANCY, 0),
                        "value_template": "{{ 'ON' if (value_json | int) | bitwise_and(1) else 'OFF' }}",
                        "device_class": "occupancy",
                    }),
                );
            }
            if serves(BOOLEAN_STATE) {
                // A contact sensor's state is true when in contact (closed);
                // Home Assistant's door is on when open.
                let (class, template) = if is(CONTACT_SENSOR) {
                    (Some("door"), "{{ 'OFF' if value_json else 'ON' }}")
                } else if is(WATER_LEAK_DETECTOR) {
                    (Some("moisture"), "{{ 'ON' if value_json else 'OFF' }}")
                } else {
                    (None, "{{ 'ON' if value_json else 'OFF' }}")
                };
                entity(
                    "binary_sensor",
                    BOOLEAN_STATE,
                    "State",
                    json!({
                        "state_topic": topic(BOOLEAN_STATE, 0),
                        "value_template": template,
                        "device_class": class,
                    }),
                );
            }
            if serves(POWER_SOURCE) {
                entity(
                    "sensor",
                    POWER_SOURCE,
                    "Battery",
                    json!({
                        "state_topic": topic(POWER_SOURCE, 0x000C),
                        "value_template": "{{ (value_json / 2) | round(0) if value_json is not none else none }}",
                        "device_class": "battery",
                        "unit_of_measurement": "%",
                        "state_class": "measurement",
                        "entity_category": "diagnostic",
                    }),
                );
            }
        }
        out
    }
}

/// Home Assistant wants a key left out rather than null.
fn strip_nulls(v: &mut Value) {
    if let Value::Object(map) = v {
        map.retain(|_, v| !v.is_null());
        map.values_mut().for_each(strip_nulls);
    }
}

fn merge(into: &mut Value, from: Value) {
    if let (Value::Object(a), Value::Object(b)) = (into, from) {
        a.extend(b);
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
        assert_eq!(config["command_topic"], "matter/lamp/1/on_off/set");
        assert_eq!(config["state_topic"], "matter/lamp/1/on_off/on_off");
        assert_eq!(
            config["brightness_state_topic"],
            "matter/lamp/1/level_control/current_level"
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
            configs[1].1["state_topic"],
            "matter/lamp/1/temperature_measurement/measured_value"
        );
    }

    #[test]
    fn contact_sensor_is_a_door() {
        let configs = D.configs(&node(CONTACT_SENSOR, &[BOOLEAN_STATE]));
        assert_eq!(configs[0].1["device_class"], "door");
        assert_eq!(
            configs[0].1["state_topic"],
            "matter/lamp/1/boolean_state/state_value"
        );
    }
}
