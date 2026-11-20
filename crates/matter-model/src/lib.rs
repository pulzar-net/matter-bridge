#![doc = include_str!("../README.md")]

mod json;
mod schema;

pub use json::{encode_to_vec, from_json, to_json, ConvertError, MAX_DEPTH};
pub use schema::*;

include!(concat!(env!("OUT_DIR"), "/model.rs"));

/// The cluster with `id`, if the IDL has it.
pub fn cluster(id: u32) -> Option<&'static Cluster> {
    CLUSTERS
        .binary_search_by_key(&id, |c| c.id)
        .ok()
        .map(|i| &CLUSTERS[i])
}

/// The cluster named `name`, compared without case or underscores, so
/// `OnOff`, `onOff` and `on_off` all find the On/Off cluster.
pub fn cluster_by_name(name: &str) -> Option<&'static Cluster> {
    CLUSTERS.iter().find(|c| names_match(c.name, name))
}

/// Whether two IDL-style names are the same name: ignoring case and `_`.
pub fn names_match(a: &str, b: &str) -> bool {
    let mut a = a.bytes().filter(|c| *c != b'_');
    let mut b = b.bytes().filter(|c| *c != b'_');
    loop {
        match (a.next(), b.next()) {
            (None, None) => return true,
            (Some(x), Some(y)) if x.eq_ignore_ascii_case(&y) => {}
            _ => return false,
        }
    }
}

/// The `snake_case` form of an IDL name (`OnOff` and `onOff` give `on_off`),
/// as used in MQTT topics.
pub fn snake_case(name: &str) -> String {
    let mut out = String::with_capacity(name.len() + 4);
    let bytes = name.as_bytes();
    for (i, &c) in bytes.iter().enumerate() {
        if c.is_ascii_uppercase() {
            let prev_lower =
                i > 0 && (bytes[i - 1].is_ascii_lowercase() || bytes[i - 1].is_ascii_digit());
            let next_lower = bytes.get(i + 1).is_some_and(|n| n.is_ascii_lowercase());
            let prev_upper = i > 0 && bytes[i - 1].is_ascii_uppercase();
            if i > 0 && (prev_lower || (prev_upper && next_lower)) && !out.ends_with('_') {
                out.push('_');
            }
            out.push(c.to_ascii_lowercase() as char);
        } else {
            out.push(c as char);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tables_are_sorted_and_complete() {
        assert!(CLUSTERS.len() > 80, "{} clusters", CLUSTERS.len());
        assert!(CLUSTERS.windows(2).all(|w| w[0].id < w[1].id));
        for c in CLUSTERS {
            assert!(
                c.attributes.windows(2).all(|w| w[0].id < w[1].id),
                "{}",
                c.name
            );
            assert!(
                c.commands.windows(2).all(|w| w[0].id < w[1].id),
                "{}",
                c.name
            );
        }
    }

    #[test]
    fn well_known_clusters() {
        let on_off = cluster(0x0006).unwrap();
        assert_eq!(on_off.name, "OnOff");
        assert_eq!(on_off.command_by_name("toggle").unwrap().id, 2);
        let level = cluster_by_name("LevelControl").unwrap();
        let move_to = level.command_by_name("MoveToLevel").unwrap();
        let req = move_to.request.unwrap();
        assert_eq!(req.field_by_name("level").unwrap().tag, 0);

        let descriptor = cluster(0x001D).unwrap();
        let dtl = descriptor.attribute_by_name("DeviceTypeList").unwrap();
        assert!(dtl.list);
        assert!(matches!(dtl.ty, Type::Struct(s) if s.name == "DeviceTypeStruct"));

        let basic = cluster(0x0028).unwrap();
        assert_eq!(basic.attribute(1).unwrap().name, "vendorName");
        assert!(matches!(basic.attribute(1).unwrap().ty, Type::String));

        let adm = cluster_by_name("AdministratorCommissioning").unwrap();
        assert!(
            adm.command_by_name("OpenCommissioningWindow")
                .unwrap()
                .timed
        );

        // Global attributes are in every cluster.
        assert_eq!(on_off.attribute(0xFFFD).unwrap().name, "clusterRevision");
    }

    #[test]
    fn names() {
        assert!(names_match("OnOff", "on_off"));
        assert!(names_match("onOff", "ONOFF"));
        assert!(!names_match("OnOff", "OnOf"));
        assert_eq!(snake_case("OnOff"), "on_off");
        assert_eq!(snake_case("onOff"), "on_off");
        assert_eq!(
            snake_case("MoveToLevelWithOnOff"),
            "move_to_level_with_on_off"
        );
        assert_eq!(snake_case("currentX"), "current_x");
        assert_eq!(snake_case("ICDManagement"), "icd_management");
        assert_eq!(snake_case("measuredValue"), "measured_value");
    }
}
