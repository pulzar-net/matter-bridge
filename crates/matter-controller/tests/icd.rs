//! An intermittently connected device: the controller commissions a LIT ICD
//! (CI runs connectedhomeip's `lit-icd-app` in a container with a short idle
//! interval), registers for its Check-Ins, then restarts. The new controller
//! does not reach for the sleeping device: it waits for its Check-In, and a
//! read made meanwhile is queued until then.
//!
//! Ignored by default: it needs the device. Run with
//! `cargo test -p matter-controller --test icd -- --ignored`, with
//!
//! - `MATTER_ICD_CODE`: the device's pairing code (default: the CSA test
//!   device's, `34970112332`);
//! - `MATTER_ICD_INTERFACE`: the interface to discover it on (optional).

use std::time::Duration;

use matter_controller::{AttributePath, CommissionRequest, Config, Controller, Event, Networks};

fn config(dir: &std::path::Path) -> Config {
    let mut config = Config::new(dir);
    config.interface = std::env::var("MATTER_ICD_INTERFACE").ok();
    config
}

#[tokio::test]
#[ignore = "needs a Matter LIT ICD"]
async fn sleeping_device_checks_in() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,matter_controller=debug".into()),
        )
        .with_test_writer()
        .try_init();
    let code = std::env::var("MATTER_ICD_CODE").unwrap_or_else(|_| "34970112332".into());
    let dir = tempfile::tempdir().unwrap();

    let controller = Controller::start(config(dir.path())).expect("controller");
    let node = controller
        .commission(CommissionRequest {
            code,
            address: None,
            name: Some("icd".into()),
            discovery_timeout: 60,
            networks: Networks::default(),
        })
        .await
        .expect("commission")
        .node_id;
    let registrations: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.path().join("icd.json")).expect("icd.json"))
            .unwrap();
    assert!(
        registrations.get(node.to_string()).is_some(),
        "not registered: {registrations}"
    );
    log::info!("node {node:016X} registered; restarting the controller");
    drop(controller);
    tokio::time::sleep(Duration::from_secs(2)).await;

    let controller = Controller::start(config(dir.path())).expect("controller");
    let mut events = controller.events();
    // Queued until the device checks in.
    let read = controller.read(
        node,
        AttributePath {
            endpoint: Some(0),
            cluster: Some(0x0028),
            attribute: Some(0x0001), // VendorName
        },
    );
    let values = tokio::time::timeout(Duration::from_secs(180), read)
        .await
        .expect("no Check-In within 180 s")
        .expect("read");
    log::info!("read after the Check-In: {values:?}");
    assert_eq!(values.len(), 1);

    let mut checked_in = false;
    let mut online = false;
    while let Ok(event) = events.try_recv() {
        checked_in |= event == Event::CheckIn { node };
        online |= event == (Event::Availability { node, online: true });
    }
    assert!(checked_in, "the read went out before a Check-In");

    // The subscription is renewed on the Check-In.
    if !online {
        let renewed = tokio::time::timeout(Duration::from_secs(60), async {
            loop {
                match events.recv().await {
                    Some(Event::Availability {
                        node: n,
                        online: true,
                    }) if n == node => break,
                    Some(_) => {}
                    None => panic!("events ended"),
                }
            }
        })
        .await;
        assert!(renewed.is_ok(), "no subscription after the Check-In");
    }
}
