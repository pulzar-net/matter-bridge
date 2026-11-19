//! The memory budget: a controller with 100 nodes in its store, each with a
//! subscription task trying to reach it, stays under 16 MB resident.
//!
//! Linux only (it reads `/proc/self/status`), release builds, and needs a
//! network interface for mDNS. Run with
//! `cargo test --release -p matter-controller --test budget -- --ignored --nocapture`.

use std::time::Duration;

use matter_controller::{BasicInfo, Config, Controller, DeviceType, Endpoint, Node, NodeStore};

const NODES: u64 = 100;
const BUDGET_KB: u64 = 16 * 1024;

fn rss_kb() -> u64 {
    let status = std::fs::read_to_string("/proc/self/status").unwrap();
    status
        .lines()
        .find_map(|l| l.strip_prefix("VmRSS:"))
        .and_then(|v| v.trim().trim_end_matches("kB").trim().parse().ok())
        .unwrap()
}

#[test]
#[ignore = "Linux, release build, needs a network interface"]
fn hundred_nodes_under_budget() {
    let dir = tempfile::tempdir().unwrap();
    {
        let mut store = NodeStore::open(&dir.path().join("nodes.json")).unwrap();
        for id in 1000..1000 + NODES {
            let mut endpoints = std::collections::BTreeMap::new();
            for ep in 0..3u16 {
                endpoints.insert(
                    ep,
                    Endpoint {
                        device_types: vec![DeviceType {
                            id: 0x0100,
                            revision: 3,
                        }],
                        server_clusters: vec![0x0003, 0x0004, 0x0006, 0x0008, 0x001D, 0x0028],
                        client_clusters: vec![],
                        parts: vec![],
                    },
                );
            }
            store
                .put(Node {
                    node_id: id,
                    name: format!("light_{id}"),
                    basic: BasicInfo {
                        vendor_name: Some("Vendor".into()),
                        product_name: Some("A light with a long product name".into()),
                        ..Default::default()
                    },
                    endpoints,
                    interviewed: true,
                    commissioned_at: 0,
                })
                .unwrap();
        }
    }

    let before = rss_kb();
    let controller = Controller::start(Config::new(dir.path())).unwrap();
    // Let every subscription task try its node at least once.
    std::thread::sleep(Duration::from_secs(20));
    let after = rss_kb();
    println!(
        "resident: {before} kB before the controller, {after} kB with {NODES} nodes (budget {BUDGET_KB} kB)"
    );
    assert!(
        after <= BUDGET_KB,
        "{after} kB is over the {BUDGET_KB} kB budget"
    );
    drop(controller);
}
