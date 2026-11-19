//! Names in topics and requests: clusters, attributes, commands and events by
//! their `snake_case` data model names, or by number where the data model
//! does not know them (vendor clusters).

use matter_model::{names_match, snake_case};
use serde_json::Value;

/// A number in a topic or request: decimal or `0x` hex.
fn number(s: &str) -> Option<u32> {
    match s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        Some(hex) => u32::from_str_radix(hex, 16).ok(),
        None => s.parse().ok(),
    }
}

fn hex_id(id: u32) -> String {
    format!("0x{id:04x}")
}

/// The topic segment of a cluster.
pub fn cluster(id: u32) -> String {
    matter_model::cluster(id).map_or_else(|| hex_id(id), |c| snake_case(c.name))
}

/// The topic segment of an attribute.
pub fn attribute(cluster: u32, id: u32) -> String {
    matter_model::cluster(cluster)
        .and_then(|c| c.attribute(id))
        .map_or_else(|| hex_id(id), |a| snake_case(a.name))
}

/// The topic segment of an event.
pub fn event(cluster: u32, id: u32) -> String {
    matter_model::cluster(cluster)
        .and_then(|c| c.event(id))
        .map_or_else(|| hex_id(id), |e| snake_case(e.name))
}

/// The cluster a segment or request value names.
pub fn parse_cluster(s: &str) -> Option<u32> {
    number(s).or_else(|| {
        matter_model::CLUSTERS
            .iter()
            .find(|c| names_match(c.name, s))
            .map(|c| c.id)
    })
}

/// The attribute of `cluster` a segment names.
pub fn parse_attribute(cluster: u32, s: &str) -> Option<u32> {
    number(s).or_else(|| {
        matter_model::cluster(cluster)?
            .attribute_by_name(s)
            .map(|a| a.id)
    })
}

/// The command of `cluster` a segment names.
pub fn parse_command(cluster: u32, s: &str) -> Option<u32> {
    number(s).or_else(|| {
        matter_model::cluster(cluster)?
            .command_by_name(s)
            .map(|c| c.id)
    })
}

/// A cluster, attribute or command given in a request as a name or a number.
pub fn value_id(v: &Value, parse: impl Fn(&str) -> Option<u32>) -> Option<u32> {
    match v {
        Value::Number(n) => n.as_u64().and_then(|n| u32::try_from(n).ok()),
        Value::String(s) => parse(s),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn segments_round_trip() {
        assert_eq!(cluster(6), "on_off");
        assert_eq!(parse_cluster("on_off"), Some(6));
        assert_eq!(parse_cluster("OnOff"), Some(6));
        assert_eq!(parse_cluster("0x0006"), Some(6));
        assert_eq!(attribute(6, 0), "on_off");
        assert_eq!(parse_attribute(6, "on_off"), Some(0));
        assert_eq!(parse_command(6, "toggle"), Some(2));
        assert_eq!(event(0x28, 0), "start_up");
        assert_eq!(cluster(0xFFF1_FC01), "0xfff1fc01");
        assert_eq!(parse_cluster("0xfff1fc01"), Some(0xFFF1_FC01));
        assert_eq!(attribute(0x0402, 0), "measured_value");
        assert_eq!(cluster(0x0008), "level_control");
        assert_eq!(attribute(0x0008, 0), "current_level");
    }
}
