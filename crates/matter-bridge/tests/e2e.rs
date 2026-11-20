//! The whole bridge against a real Matter device: an MQTT broker,
//! `matter_bridge::run`, and a device commissionable on the local network
//! (CI runs connectedhomeip's `chip-all-clusters-app` in a container).
//!
//! The device is commissioned over IP, interviewed, subscribed to; its
//! attributes and events arrive over MQTT, a command and a write go out, an
//! invoke returns typed response fields, and a commissioning window is opened
//! through which a second administrator (a controller on a fabric of its own)
//! commissions the device too.
//!
//! Ignored by default: it needs the device. Run with
//! `cargo test -p matter-bridge --test e2e -- --ignored`, with
//!
//! - `MATTER_E2E_CODE`: the device's pairing code (default: the CSA test
//!   device's, `34970112332`);
//! - `MATTER_E2E_INTERFACE`: the interface to discover it on (optional);
//! - `MATTER_E2E_WINDOW_FILE`: where to write the second admin's code
//!   (optional).

use std::collections::HashMap;
use std::net::SocketAddr;
use std::time::Duration;

use matter_bridge::Config;
use rumqttc::{AsyncClient, Event, MqttOptions, Packet, QoS};
use serde_json::{json, Value as Json};
use tokio::sync::{mpsc, oneshot};

fn free_port() -> SocketAddr {
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    l.local_addr().unwrap()
}

fn start_broker() -> SocketAddr {
    let addr = free_port();
    let mut servers = HashMap::new();
    servers.insert(
        "v4".to_string(),
        rumqttd::ServerSettings {
            name: "v4".into(),
            listen: addr,
            tls: None,
            next_connection_delay_ms: 1,
            connections: rumqttd::ConnectionSettings {
                connection_timeout_ms: 5000,
                max_payload_size: 256 * 1024,
                max_inflight_count: 500,
                auth: None,
                external_auth: None,
                dynamic_filters: true,
            },
        },
    );
    let config = rumqttd::Config {
        id: 0,
        router: rumqttd::RouterConfig {
            max_connections: 16,
            max_outgoing_packet_count: 1000,
            max_segment_size: 64 * 1024 * 1024,
            max_segment_count: 10,
            ..Default::default()
        },
        v4: Some(servers),
        ..Default::default()
    };
    std::thread::spawn(move || {
        let mut broker = rumqttd::Broker::new(config);
        let _ = broker.start();
    });
    addr
}

struct Watcher {
    client: AsyncClient,
    rx: mpsc::UnboundedReceiver<(String, Vec<u8>)>,
    /// Every message so far, so a wait finds one that arrived earlier.
    seen: Vec<(String, Json)>,
}

impl Watcher {
    async fn connect(broker: SocketAddr) -> Watcher {
        let mut options = MqttOptions::new("e2e-watcher", broker.ip().to_string(), broker.port());
        options.set_keep_alive(Duration::from_secs(10));
        options.set_max_packet_size(256 * 1024, 256 * 1024);
        let (client, mut event_loop) = AsyncClient::new(options, 1000);
        let (tx, rx) = mpsc::unbounded_channel();
        let (ready_tx, ready_rx) = oneshot::channel();
        let mut ready = Some(ready_tx);
        tokio::spawn(async move {
            loop {
                match event_loop.poll().await {
                    Ok(Event::Incoming(Packet::SubAck(_))) => {
                        if let Some(r) = ready.take() {
                            let _ = r.send(());
                        }
                    }
                    Ok(Event::Incoming(Packet::Publish(p))) => {
                        let _ = tx.send((p.topic.clone(), p.payload.to_vec()));
                    }
                    Ok(_) => {}
                    Err(_) => tokio::time::sleep(Duration::from_millis(100)).await,
                }
            }
        });
        client.subscribe("#", QoS::AtLeastOnce).await.unwrap();
        tokio::time::timeout(Duration::from_secs(10), ready_rx)
            .await
            .expect("subscribed")
            .unwrap();
        Watcher {
            client,
            rx,
            seen: Vec::new(),
        }
    }

    /// Wait for a message on a topic `topic` accepts whose payload (JSON, or
    /// a JSON string if it is not JSON) satisfies `pred`, whether it already
    /// arrived or arrives within `secs`.
    async fn wait_for(
        &mut self,
        secs: u64,
        what: &str,
        mut topic: impl FnMut(&str) -> bool,
        mut pred: impl FnMut(&Json) -> bool,
    ) -> (String, Json) {
        if let Some((t, v)) = self.seen.iter().find(|(t, v)| topic(t) && pred(v)) {
            return (t.clone(), v.clone());
        }
        let deadline = tokio::time::Instant::now() + Duration::from_secs(secs);
        loop {
            let (t, payload) = tokio::time::timeout_at(deadline, self.rx.recv())
                .await
                .unwrap_or_else(|_| panic!("timed out waiting for {what}"))
                .expect("watcher stopped");
            let value: Json = serde_json::from_slice(&payload)
                .unwrap_or_else(|_| Json::String(String::from_utf8_lossy(&payload).into()));
            self.seen.push((t.clone(), value.clone()));
            if topic(&t) && pred(&value) {
                return (t, value);
            }
        }
    }

    async fn wait(&mut self, topic: &str, pred: impl FnMut(&Json) -> bool) -> Json {
        self.wait_for(60, topic, |t| t == topic, pred).await.1
    }

    async fn publish(&self, topic: &str, payload: &str) {
        self.client
            .publish(topic, QoS::AtLeastOnce, false, payload.as_bytes().to_vec())
            .await
            .unwrap();
    }

    async fn request(&mut self, name: &str, body: Json, secs: u64) -> Json {
        self.client
            .publish(
                format!("matter/bridge/request/{name}"),
                QoS::AtLeastOnce,
                false,
                serde_json::to_vec(&body).unwrap(),
            )
            .await
            .unwrap();
        let transaction = body["transaction"].clone();
        let topic = format!("matter/bridge/response/{name}");
        let (_, response) = self
            .wait_for(
                secs,
                &topic,
                |t| t == topic,
                |v| v["transaction"] == transaction,
            )
            .await;
        assert_eq!(response["status"], "ok", "{name}: {response}");
        response["data"].clone()
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "needs a commissionable Matter device on the network"]
async fn commissions_a_device_and_bridges_it() {
    let _ = tracing_subscriber_init();
    let code = std::env::var("MATTER_E2E_CODE").unwrap_or_else(|_| "34970112332".into());
    let interface = std::env::var("MATTER_E2E_INTERFACE").ok();

    let broker = start_broker();
    let data = tempfile::tempdir().unwrap();
    let mut config = Config::default();
    config.mqtt.host = broker.ip().to_string();
    config.mqtt.port = broker.port();
    config.data.dir = data.path().to_path_buf();
    config.matter.interface = interface;
    // The broker thread needs a moment to listen.
    tokio::time::sleep(Duration::from_millis(300)).await;
    let mut mqtt = Watcher::connect(broker).await;

    let (stop_tx, stop_rx) = oneshot::channel::<()>();
    let bridge = tokio::spawn(matter_bridge::run(config, async move {
        let _ = stop_rx.await;
    }));

    mqtt.wait("matter/bridge/state", |v| v["state"] == "online")
        .await;

    // Commission over IP: discovered by DNS-SD from the manual code.
    let node = mqtt
        .request(
            "commission",
            json!({"code": code, "name": "dut", "timeout": 60, "transaction": "c"}),
            180,
        )
        .await;
    assert_eq!(node["name"], "dut");
    assert_eq!(node["interviewed"], true, "{node}");
    let endpoints = node["endpoints"].as_object().unwrap();
    assert!(endpoints["1"]["server_clusters"]
        .as_array()
        .unwrap()
        .contains(&json!(6)));
    assert!(node["basic"]["vendor_id"].is_u64(), "{node}");

    // The subscription delivers the priming report.
    mqtt.wait("matter/dut/availability", |v| v == "online")
        .await;
    let on = mqtt
        .wait("matter/dut/1/on_off/on_off", Json::is_boolean)
        .await
        .as_bool()
        .unwrap();
    let start_up = mqtt
        .wait_for(
            60,
            "the StartUp event",
            |t| t == "matter/dut/0/basic_information/event/start_up",
            |v| v["number"].is_u64(),
        )
        .await;
    println!("event: {start_up:?}");

    // Home Assistant discovery from the Descriptor cluster.
    let (topic, discovery) = mqtt
        .wait_for(
            30,
            "discovery of endpoint 1's On/Off",
            |t| t.starts_with("homeassistant/") && t.ends_with("/1_6/config"),
            |_| true,
        )
        .await;
    assert_eq!(
        discovery["command_topic"], "matter/dut/1/on_off/set",
        "{topic}"
    );

    // Read.
    let read = mqtt
        .request(
            "read",
            json!({"device": "dut", "endpoint": 1, "cluster": "OnOff", "attribute": "on_off", "transaction": "r"}),
            30,
        )
        .await;
    assert_eq!(read[0]["value"], json!(on), "{read}");

    // A command, and the change it makes comes back in the subscription.
    mqtt.publish("matter/dut/1/on_off/command/toggle", "").await;
    let result = mqtt
        .wait("matter/dut/1/on_off/command/toggle/result", |_| true)
        .await;
    assert_eq!(result["status"], "ok", "{result}");
    mqtt.wait("matter/dut/1/on_off/on_off", |v| v == &json!(!on))
        .await;

    // The Home Assistant command topic.
    mqtt.publish("matter/dut/1/on_off/set", if on { "ON" } else { "OFF" })
        .await;
    mqtt.wait("matter/dut/1/on_off/on_off", |v| v == &json!(on))
        .await;

    // A write, reported back.
    mqtt.publish("matter/dut/0/basic_information/node_label/set", "\"e2e\"")
        .await;
    mqtt.wait("matter/dut/0/basic_information/node_label", |v| v == "e2e")
        .await;

    // A group: the light's endpoint 1 joins it, and one multicast Toggle to
    // the group changes it, as the subscription reports.
    let before = mqtt
        .wait("matter/dut/1/on_off/on_off", Json::is_boolean)
        .await;
    mqtt.request(
        "add_to_group",
        json!({"device": "dut", "endpoint": 1, "group": 257, "name": "e2e", "transaction": "g"}),
        30,
    )
    .await;
    mqtt.seen.clear();
    mqtt.request(
        "group_invoke",
        json!({"group": 257, "cluster": "on_off", "command": "toggle", "transaction": "gi"}),
        30,
    )
    .await;
    mqtt.wait("matter/dut/1/on_off/on_off", |v| {
        v.is_boolean() && v != &before
    })
    .await;

    // An invoke whose response command is decoded with field names.
    let response = mqtt
        .request(
            "invoke",
            json!({
                "device": "dut", "endpoint": 0, "cluster": "general_commissioning",
                "command": "ArmFailSafe", "fields": {"expiryLengthSeconds": 0, "breadcrumb": 0},
                "transaction": "i",
            }),
            30,
        )
        .await;
    assert_eq!(response["errorCode"], 0, "{response}");

    // Multi-admin: a window for another ecosystem.
    let window = mqtt
        .request(
            "open_commissioning_window",
            json!({"device": "dut", "timeout": 300, "transaction": "w"}),
            30,
        )
        .await;
    let manual = window["manual_code"].as_str().unwrap().replace('-', "");
    assert_eq!(manual.len(), 11, "{window}");
    assert!(window["qr_code"].as_str().unwrap().starts_with("MT:"));
    let parsed = matter_controller::Onboarding::parse(&manual).unwrap();
    assert_eq!(
        parsed.discriminator,
        matter_controller::Discriminator::Short(
            (window["discriminator"].as_u64().unwrap() >> 8) as u8
        )
    );
    if let Ok(path) = std::env::var("MATTER_E2E_WINDOW_FILE") {
        std::fs::write(path, &manual).unwrap();
    }

    // A second administrator, on a fabric of its own, commissions the device
    // through that window while the first keeps it.
    let second_dir = tempfile::tempdir().unwrap();
    let mut second_config = matter_controller::Config::new(second_dir.path());
    second_config.port = 5542;
    second_config.interface = std::env::var("MATTER_E2E_INTERFACE").ok();
    let second =
        tokio::task::spawn_blocking(move || matter_controller::Controller::start(second_config))
            .await
            .unwrap()
            .unwrap();
    assert_ne!(second.info().fabric_id, 0);
    let node2 = second
        .commission(matter_controller::CommissionRequest {
            code: manual.clone(),
            address: None,
            name: Some("dut".into()),
            discovery_timeout: 60,
            networks: Default::default(),
        })
        .await
        .expect("the second admin commissions through the window");
    let fabrics = second
        .read(
            node2.node_id,
            matter_controller::AttributePath {
                endpoint: Some(0),
                cluster: Some(0x003E), // Operational Credentials
                attribute: Some(3),    // CommissionedFabrics
            },
        )
        .await
        .unwrap();
    assert_eq!(fabrics[0].value, json!(2), "{fabrics:?}");

    // The first admin still controls it (the group toggle left it at !on).
    mqtt.seen.clear();
    mqtt.publish("matter/dut/1/on_off/command/toggle", "").await;
    mqtt.wait("matter/dut/1/on_off/on_off", |v| v == &json!(on))
        .await;
    drop(second);

    let _ = stop_tx.send(());
    bridge.await.unwrap().unwrap();
}

fn tracing_subscriber_init() -> Result<(), ()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,rs_matter=info".into()),
        )
        .with_test_writer()
        .try_init()
        .map_err(|_| ())
}
