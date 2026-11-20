//! The bridge: controller events out to MQTT, MQTT requests and commands in
//! to the controller.

use std::collections::{BTreeSet, HashMap};
use std::future::Future;
use std::net::SocketAddr;
use std::sync::{Arc, RwLock};

use matter_controller::{
    AttributePath, CommissionRequest, Controller, Event, Networks, Node, WiFi,
};
use serde_json::{json, Value};

use crate::config::{Config, MatterConfig};
use crate::homeassistant::Discovery;
use crate::mqtt::{self, Incoming, Mqtt};
use crate::topics;

const ON_OFF: u32 = 0x0006;
const LEVEL_CONTROL: u32 = 0x0008;

/// Node names, shared by the event loop and request tasks.
#[derive(Default)]
struct Names {
    by_id: HashMap<u64, Node>,
}

impl Names {
    fn name(&self, node: u64) -> Option<&str> {
        self.by_id.get(&node).map(|n| n.name.as_str())
    }

    /// The node a request or topic names: by name, or by node id.
    fn find(&self, name: &str) -> Option<u64> {
        self.by_id
            .values()
            .find(|n| n.name == name)
            .map(|n| n.node_id)
            .or_else(|| {
                let id = match name.strip_prefix("0x") {
                    Some(hex) => u64::from_str_radix(hex, 16).ok(),
                    None => name.parse().ok(),
                }?;
                self.by_id.contains_key(&id).then_some(id)
            })
    }
}

struct Bridge {
    controller: Controller,
    mqtt: Mqtt,
    config: Config,
    names: Arc<RwLock<Names>>,
    /// Retained topics published per node, cleared when it goes or is renamed.
    retained: HashMap<u64, BTreeSet<String>>,
}

/// Run the bridge until `shutdown` completes.
pub async fn run(config: Config, shutdown: impl Future<Output = ()>) -> Result<(), String> {
    let controller_config = config.controller();
    let controller = tokio::task::spawn_blocking(move || Controller::start(controller_config))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| format!("controller: {e}"))?;
    let info = controller.info().clone();
    tracing::info!(
        fabric = format!("{:016X}", info.fabric_id),
        node = format!("{:016X}", info.node_id),
        "controller started"
    );

    let (mqtt, mut incoming) = mqtt::connect(&config.mqtt)?;
    let mut events = controller.events();

    let names = Arc::new(RwLock::new(Names::default()));
    for node in controller.nodes().await.map_err(|e| e.to_string())? {
        names.write().unwrap().by_id.insert(node.node_id, node);
    }

    let mut bridge = Bridge {
        controller,
        mqtt,
        config,
        names,
        retained: HashMap::new(),
    };

    tokio::pin!(shutdown);
    loop {
        tokio::select! {
            msg = incoming.recv() => match msg {
                Some(msg) => bridge.on_mqtt(msg).await,
                None => break,
            },
            event = events.recv() => match event {
                Some(event) => bridge.on_event(event).await,
                None => return Err("the controller stopped".into()),
            },
            () = &mut shutdown => break,
        }
    }

    let base = &bridge.config.mqtt.base_topic;
    bridge
        .mqtt
        .publish(mqtt::state_topic(base), r#"{"state":"offline"}"#, true)
        .await;
    bridge.mqtt.disconnect().await;
    Ok(())
}

impl Bridge {
    fn base(&self) -> &str {
        &self.config.mqtt.base_topic
    }

    async fn on_mqtt(&mut self, msg: Incoming) {
        match msg {
            Incoming::Connected => self.on_connected().await,
            Incoming::Message { topic, payload } => {
                if topic == self.config.homeassistant.status_topic {
                    if payload == b"online" {
                        self.publish_discovery_all().await;
                    }
                    return;
                }
                let Some(rest) = topic
                    .strip_prefix(self.base())
                    .and_then(|t| t.strip_prefix('/'))
                else {
                    return;
                };
                let parts: Vec<&str> = rest.split('/').collect();
                if let ["bridge", "request", name] = parts.as_slice() {
                    self.spawn_request(name.to_string(), payload);
                } else {
                    self.spawn_device_command(
                        parts.iter().map(|s| s.to_string()).collect(),
                        payload,
                    );
                }
            }
        }
    }

    async fn on_connected(&mut self) {
        let base = self.base().to_string();
        for filter in [
            format!("{base}/bridge/request/+"),
            format!("{base}/+/+/+/set"),
            format!("{base}/+/+/+/+/set"),
            format!("{base}/+/+/+/command/+"),
        ] {
            self.mqtt.subscribe(&filter).await;
        }
        if self.config.homeassistant.enabled {
            let status = self.config.homeassistant.status_topic.clone();
            self.mqtt.subscribe(&status).await;
        }

        self.mqtt
            .publish(mqtt::state_topic(&base), r#"{"state":"online"}"#, true)
            .await;
        let info = self.controller.info().clone();
        self.mqtt
            .publish_json(
                format!("{base}/bridge/info"),
                &json!({
                    "version": env!("CARGO_PKG_VERSION"),
                    "fabric_id": format!("{:016X}", info.fabric_id),
                    "compressed_fabric_id": format!("{:016X}", info.compressed_fabric_id),
                    "node_id": format!("{:016X}", info.node_id),
                    "vendor_id": info.vendor_id,
                }),
                true,
            )
            .await;
        self.publish_devices().await;
        self.publish_discovery_all().await;
    }

    async fn publish_devices(&self) {
        let nodes: Vec<Node> = self.names.read().unwrap().by_id.values().cloned().collect();
        self.mqtt
            .publish_json(
                format!("{}/bridge/devices", self.base()),
                &serde_json::to_value(nodes).unwrap_or_default(),
                true,
            )
            .await;
    }

    fn discovery(&self) -> Discovery<'_> {
        Discovery {
            prefix: &self.config.homeassistant.discovery_prefix,
            base: &self.config.mqtt.base_topic,
            fabric: self.controller.info().compressed_fabric_id,
        }
    }

    async fn publish_discovery_all(&mut self) {
        let nodes: Vec<Node> = self.names.read().unwrap().by_id.values().cloned().collect();
        for node in nodes {
            self.publish_discovery(&node).await;
        }
    }

    async fn publish_discovery(&mut self, node: &Node) {
        if !self.config.homeassistant.enabled {
            return;
        }
        for (topic, config) in self.discovery().configs(node) {
            self.mqtt.publish_json(topic.clone(), &config, true).await;
            self.retained.entry(node.node_id).or_default().insert(topic);
        }
    }

    /// Clear every retained topic of `node`.
    async fn clear_retained(&mut self, node: u64) {
        for topic in self.retained.remove(&node).unwrap_or_default() {
            self.mqtt.publish(topic, Vec::new(), true).await;
        }
    }

    async fn publish_retained(&mut self, node: u64, topic: String, payload: Vec<u8>) {
        self.mqtt.publish(topic.clone(), payload, true).await;
        self.retained.entry(node).or_default().insert(topic);
    }

    async fn on_event(&mut self, event: Event) {
        let base = self.base().to_string();
        match event {
            Event::Attribute(a) => {
                if a.status.is_some() {
                    return;
                }
                let Some(name) = self.names.read().unwrap().name(a.node).map(str::to_string) else {
                    return;
                };
                let topic = format!(
                    "{base}/{name}/{}/{}/{}",
                    a.endpoint,
                    topics::cluster(a.cluster),
                    topics::attribute(a.cluster, a.attribute)
                );
                let payload = serde_json::to_vec(&a.value).unwrap_or_default();
                self.publish_retained(a.node, topic, payload).await;
            }
            Event::Event(e) => {
                let Some(name) = self.names.read().unwrap().name(e.node).map(str::to_string) else {
                    return;
                };
                let topic = format!(
                    "{base}/{name}/{}/{}/event/{}",
                    e.endpoint,
                    topics::cluster(e.cluster),
                    topics::event(e.cluster, e.event)
                );
                self.mqtt
                    .publish_json(topic, &json!({"number": e.number, "data": e.data}), false)
                    .await;
            }
            Event::Availability { node, online } => {
                let name = self.names.read().unwrap().name(node).map(str::to_string);
                if let Some(name) = name {
                    let state = if online { "online" } else { "offline" };
                    self.publish_retained(
                        node,
                        format!("{base}/{name}/availability"),
                        state.into(),
                    )
                    .await;
                }
                self.publish_bridge_event(
                    &json!({"type": "availability", "node": node, "online": online}),
                )
                .await;
            }
            Event::NodeUpdated(node) => {
                let renamed = self
                    .names
                    .read()
                    .unwrap()
                    .by_id
                    .get(&node.node_id)
                    .is_some_and(|old| old.name != node.name);
                if renamed {
                    // Topics carry the name: drop the old ones, the values come
                    // back under the new name with the next report.
                    self.clear_retained(node.node_id).await;
                }
                self.names
                    .write()
                    .unwrap()
                    .by_id
                    .insert(node.node_id, node.clone());
                self.publish_discovery(&node).await;
                self.publish_devices().await;
                self.publish_bridge_event(&json!({"type": "node_updated", "node": node}))
                    .await;
            }
            Event::NodeRemoved { node } => {
                self.clear_retained(node).await;
                self.names.write().unwrap().by_id.remove(&node);
                self.publish_devices().await;
                self.publish_bridge_event(&json!({"type": "node_removed", "node": node}))
                    .await;
            }
            Event::Commissioning { node, step } => {
                self.publish_bridge_event(
                    &json!({"type": "commissioning", "node": node, "step": step}),
                )
                .await;
            }
            Event::CheckIn { node } => {
                self.publish_bridge_event(&json!({"type": "check_in", "node": node}))
                    .await;
            }
            Event::Ota {
                node,
                step,
                version,
            } => {
                self.publish_bridge_event(
                    &json!({"type": "ota", "node": node, "step": step, "version": version}),
                )
                .await;
            }
        }
    }

    async fn publish_bridge_event(&self, event: &Value) {
        self.mqtt
            .publish_json(format!("{}/bridge/event", self.base()), event, false)
            .await;
    }

    fn spawn_request(&self, name: String, payload: Vec<u8>) {
        let controller = self.controller.clone();
        let mqtt = self.mqtt.clone();
        let names = self.names.clone();
        let base = self.base().to_string();
        let matter = self.config.matter.clone();
        tokio::spawn(async move {
            let body: Value = if payload.is_empty() {
                json!({})
            } else {
                serde_json::from_slice(&payload).unwrap_or(Value::Null)
            };
            let transaction = body.get("transaction").cloned();
            let result = request(&controller, &names, &name, &body, &matter).await;
            let mut response = match result {
                Ok(data) => json!({"status": "ok", "data": data}),
                Err(error) => {
                    tracing::warn!(request = %name, %error, "request failed");
                    json!({"status": "error", "error": error})
                }
            };
            if let Some(t) = transaction {
                response["transaction"] = t;
            }
            mqtt.publish_json(format!("{base}/bridge/response/{name}"), &response, false)
                .await;
        });
    }

    /// `<device>/<endpoint>/<cluster>/set`, `<device>/<endpoint>/<cluster>/<attribute>/set`
    /// and `<device>/<endpoint>/<cluster>/command/<command>`.
    fn spawn_device_command(&self, parts: Vec<String>, payload: Vec<u8>) {
        let controller = self.controller.clone();
        let mqtt = self.mqtt.clone();
        let names = self.names.clone();
        let base = self.base().to_string();
        tokio::spawn(async move {
            let topic = parts.join("/");
            let is_command = parts.len() == 5 && parts[3] == "command";
            let result = device_command(&controller, &names, &parts, &payload).await;
            if let Err(error) = &result {
                tracing::warn!(%topic, %error, "command failed");
            }
            if is_command {
                let response = match result {
                    Ok(data) => json!({"status": "ok", "data": data}),
                    Err(error) => json!({"status": "error", "error": error}),
                };
                mqtt.publish_json(format!("{base}/{topic}/result"), &response, false)
                    .await;
            }
        });
    }
}

/// The networks a device commissioned over Bluetooth may join: a Thread
/// dataset (`thread_dataset` as hex, or fetched from `thread_dataset_url` or
/// the configured border router at commissioning time) and Wi-Fi
/// (`wifi: {ssid, password}`). Neither is kept or logged.
async fn networks_of(body: &Value, matter: &MatterConfig) -> Result<Networks, String> {
    let thread_dataset = match body.get("thread_dataset").and_then(Value::as_str) {
        Some(hex) => Some(hex.to_string()),
        None => {
            let url = body
                .get("thread_dataset_url")
                .and_then(Value::as_str)
                .map(str::to_string)
                .or_else(|| matter.thread_dataset_url.clone());
            match url {
                Some(url) => Some(fetch_dataset(url).await?),
                None => None,
            }
        }
    };
    let thread_dataset = thread_dataset
        .map(|hex| matter_controller::network::parse_thread_dataset_hex(&hex))
        .transpose()
        .map_err(|e| e.to_string())?;
    let wifi = match body.get("wifi") {
        Some(w) => Some(WiFi {
            ssid: w
                .get("ssid")
                .and_then(Value::as_str)
                .ok_or("`wifi.ssid` is required")?
                .as_bytes()
                .to_vec(),
            credentials: w
                .get("password")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .as_bytes()
                .to_vec(),
        }),
        None => None,
    };
    Ok(Networks {
        thread_dataset,
        wifi,
    })
}

/// A Thread border router's active dataset, from its REST API (OpenThread's
/// `GET /node/dataset/active` with `Accept: text/plain` gives it as hex).
async fn fetch_dataset(url: String) -> Result<String, String> {
    tokio::task::spawn_blocking(move || {
        ureq::get(&url)
            .header("Accept", "text/plain")
            .call()
            .and_then(|mut r| r.body_mut().read_to_string())
            .map_err(|e| format!("the Thread dataset from {url}: {e}"))
    })
    .await
    .map_err(|e| e.to_string())?
}

fn node_of(names: &RwLock<Names>, body: &Value) -> Result<u64, String> {
    let name = body
        .get("device")
        .and_then(Value::as_str)
        .ok_or("`device` is required")?;
    names
        .read()
        .unwrap()
        .find(name)
        .ok_or_else(|| format!("no device `{name}`"))
}

fn group_of(body: &Value) -> Result<u16, String> {
    body.get("group")
        .and_then(Value::as_u64)
        .and_then(|g| u16::try_from(g).ok())
        .filter(|g| *g != 0)
        .ok_or_else(|| "`group` is required: 1 to 65535".to_string())
}

fn path_of(body: &Value) -> Result<(u16, u32), String> {
    let endpoint = body
        .get("endpoint")
        .and_then(Value::as_u64)
        .and_then(|e| u16::try_from(e).ok())
        .ok_or("`endpoint` is required")?;
    let cluster = body
        .get("cluster")
        .and_then(|v| topics::value_id(v, topics::parse_cluster))
        .ok_or("`cluster` is required: a name or a number")?;
    Ok((endpoint, cluster))
}

fn timed(body: &Value) -> Option<u16> {
    body.get("timed")
        .and_then(Value::as_u64)
        .and_then(|t| u16::try_from(t).ok())
}

fn named(values: Vec<matter_controller::AttributeValue>) -> Value {
    values
        .into_iter()
        .map(|a| {
            json!({
                "endpoint": a.endpoint,
                "cluster": topics::cluster(a.cluster),
                "attribute": topics::attribute(a.cluster, a.attribute),
                "value": a.value,
                "status": a.status,
            })
        })
        .collect()
}

async fn request(
    controller: &Controller,
    names: &RwLock<Names>,
    name: &str,
    body: &Value,
    matter: &MatterConfig,
) -> Result<Value, String> {
    let e = |e: matter_controller::Error| e.to_string();
    match name {
        "commission" => {
            let code = body
                .get("code")
                .and_then(Value::as_str)
                .ok_or("`code` is required: the QR code payload or the manual pairing code")?;
            let address = match body.get("address").and_then(Value::as_str) {
                Some(a) => Some(
                    a.parse::<SocketAddr>()
                        .map_err(|_| format!("`address` {a} is not ip:port"))?,
                ),
                None => None,
            };
            let req_name = body.get("name").and_then(Value::as_str).map(str::to_string);
            if let Some(n) = &req_name {
                check_name(n)?;
                if names.read().unwrap().find(n).is_some() {
                    return Err(format!("the name `{n}` is taken"));
                }
            }
            let timeout = body
                .get("timeout")
                .and_then(Value::as_u64)
                .and_then(|t| u16::try_from(t).ok())
                .unwrap_or(matter.discovery_timeout);
            let networks = networks_of(body, matter).await?;
            let node = controller
                .commission(CommissionRequest {
                    code: code.to_string(),
                    address,
                    name: req_name,
                    discovery_timeout: timeout,
                    networks,
                })
                .await
                .map_err(e)?;
            Ok(serde_json::to_value(node).unwrap_or_default())
        }
        "remove" => {
            let node = node_of(names, body)?;
            let force = body.get("force").and_then(Value::as_bool).unwrap_or(false);
            controller.remove(node, force).await.map_err(e)?;
            Ok(json!({"node": node}))
        }
        "rename" => {
            let node = node_of(names, body)?;
            let to = body
                .get("to")
                .and_then(Value::as_str)
                .ok_or("`to` is required")?;
            check_name(to)?;
            let node = controller.rename(node, to.to_string()).await.map_err(e)?;
            Ok(serde_json::to_value(node).unwrap_or_default())
        }
        "interview" => {
            let node = node_of(names, body)?;
            let node = controller.interview(node).await.map_err(e)?;
            Ok(serde_json::to_value(node).unwrap_or_default())
        }
        "read" => {
            let node = node_of(names, body)?;
            let endpoint = body
                .get("endpoint")
                .and_then(Value::as_u64)
                .and_then(|v| u16::try_from(v).ok());
            let cluster = body
                .get("cluster")
                .map(|v| topics::value_id(v, topics::parse_cluster).ok_or("unknown `cluster`"))
                .transpose()?;
            let attribute = match (body.get("attribute"), cluster) {
                (Some(v), Some(c)) => Some(
                    topics::value_id(v, |s| topics::parse_attribute(c, s))
                        .ok_or("unknown `attribute`")?,
                ),
                (Some(v), None) => Some(
                    topics::value_id(v, |_| None).ok_or("`attribute` by name needs `cluster`")?,
                ),
                (None, _) => None,
            };
            let values = controller
                .read(
                    node,
                    AttributePath {
                        endpoint,
                        cluster,
                        attribute,
                    },
                )
                .await
                .map_err(e)?;
            Ok(named(values))
        }
        "write" => {
            let node = node_of(names, body)?;
            let (endpoint, cluster) = path_of(body)?;
            let attribute = body
                .get("attribute")
                .and_then(|v| topics::value_id(v, |s| topics::parse_attribute(cluster, s)))
                .ok_or("`attribute` is required")?;
            let value = body.get("value").cloned().ok_or("`value` is required")?;
            controller
                .write(node, endpoint, cluster, attribute, value, timed(body))
                .await
                .map_err(e)?;
            Ok(Value::Null)
        }
        "invoke" => {
            let node = node_of(names, body)?;
            let (endpoint, cluster) = path_of(body)?;
            let command = body
                .get("command")
                .and_then(|v| topics::value_id(v, |s| topics::parse_command(cluster, s)))
                .ok_or("`command` is required")?;
            let fields = body.get("fields").cloned().unwrap_or(Value::Null);
            controller
                .invoke(node, endpoint, cluster, command, fields, timed(body))
                .await
                .map_err(e)
        }
        "add_to_group" | "remove_from_group" => {
            let node = node_of(names, body)?;
            let endpoint = body
                .get("endpoint")
                .and_then(Value::as_u64)
                .and_then(|e| u16::try_from(e).ok())
                .ok_or("`endpoint` is required")?;
            let group = group_of(body)?;
            if name == "add_to_group" {
                let label = body
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                controller
                    .add_to_group(node, endpoint, group, label)
                    .await
                    .map_err(e)?;
            } else {
                controller
                    .remove_from_group(node, endpoint, group)
                    .await
                    .map_err(e)?;
            }
            Ok(Value::Null)
        }
        "group_invoke" => {
            let group = group_of(body)?;
            let cluster = body
                .get("cluster")
                .and_then(|v| topics::value_id(v, topics::parse_cluster))
                .ok_or("`cluster` is required: a name or a number")?;
            let command = body
                .get("command")
                .and_then(|v| topics::value_id(v, |s| topics::parse_command(cluster, s)))
                .ok_or("`command` is required")?;
            let fields = body.get("fields").cloned().unwrap_or(Value::Null);
            controller
                .group_invoke(group, cluster, command, fields)
                .await
                .map_err(e)?;
            Ok(Value::Null)
        }
        "open_commissioning_window" => {
            let node = node_of(names, body)?;
            let timeout = body
                .get("timeout")
                .and_then(Value::as_u64)
                .and_then(|t| u16::try_from(t).ok())
                .unwrap_or(300);
            let window = controller
                .open_commissioning_window(node, timeout)
                .await
                .map_err(e)?;
            Ok(serde_json::to_value(window).unwrap_or_default())
        }
        "discover" => {
            let seconds = body
                .get("timeout")
                .and_then(Value::as_u64)
                .and_then(|t| u16::try_from(t).ok())
                .unwrap_or(5);
            let found = controller.discover(seconds).await.map_err(e)?;
            Ok(serde_json::to_value(found).unwrap_or_default())
        }
        "ota_images" => {
            let images = controller.ota_images().map_err(e)?;
            Ok(serde_json::to_value(images).unwrap_or_default())
        }
        "ota_update" => {
            let node = node_of(names, body)?;
            let image = controller.announce_ota(node).await.map_err(e)?;
            Ok(serde_json::to_value(image).unwrap_or_default())
        }
        "devices" => {
            let nodes = controller.nodes().await.map_err(e)?;
            Ok(serde_json::to_value(nodes).unwrap_or_default())
        }
        other => Err(format!("unknown request `{other}`")),
    }
}

/// A node name must work as one topic segment and not shadow the bridge's
/// own topics.
fn check_name(name: &str) -> Result<(), String> {
    if name.is_empty() || name == "bridge" || name.contains(['/', '#', '+']) {
        return Err(format!(
            "`{name}` cannot be a name: it must not be empty, `bridge`, or contain /, # or +"
        ));
    }
    Ok(())
}

async fn device_command(
    controller: &Controller,
    names: &RwLock<Names>,
    parts: &[String],
    payload: &[u8],
) -> Result<Value, String> {
    let e = |e: matter_controller::Error| e.to_string();
    let parts: Vec<&str> = parts.iter().map(String::as_str).collect();
    let (device, endpoint, cluster) = match parts.as_slice() {
        [d, ep, cl, ..] => (*d, *ep, *cl),
        _ => return Err("not a device topic".into()),
    };
    let node = names
        .read()
        .unwrap()
        .find(device)
        .ok_or_else(|| format!("no device `{device}`"))?;
    let endpoint: u16 = endpoint
        .parse()
        .map_err(|_| format!("`{endpoint}` is not an endpoint number"))?;
    let cluster =
        topics::parse_cluster(cluster).ok_or_else(|| format!("no cluster `{cluster}`"))?;
    let text = String::from_utf8_lossy(payload);
    let json: Value =
        serde_json::from_slice(payload).unwrap_or_else(|_| Value::String(text.trim().to_string()));

    match parts.as_slice() {
        [_, _, _, "set"] => match cluster {
            ON_OFF => {
                let command = match &json {
                    Value::Bool(true) => 1,
                    Value::Bool(false) => 0,
                    Value::String(s) if s.eq_ignore_ascii_case("on") => 1,
                    Value::String(s) if s.eq_ignore_ascii_case("off") => 0,
                    Value::String(s) if s.eq_ignore_ascii_case("toggle") => 2,
                    _ => return Err("on_off/set takes ON, OFF or TOGGLE".into()),
                };
                controller
                    .invoke(node, endpoint, ON_OFF, command, Value::Null, None)
                    .await
                    .map_err(e)
            }
            LEVEL_CONTROL => {
                let level = json
                    .as_u64()
                    .or_else(|| json.as_str().and_then(|s| s.parse().ok()))
                    .filter(|l| *l <= 254)
                    .ok_or("level_control/set takes a level, 0 to 254")?;
                controller
                    .invoke(
                        node,
                        endpoint,
                        LEVEL_CONTROL,
                        0x04, // MoveToLevelWithOnOff
                        json!({"level": level, "optionsMask": 0, "optionsOverride": 0}),
                        None,
                    )
                    .await
                    .map_err(e)
            }
            _ => Err(format!(
                "{}/set is not supported; set an attribute or invoke a command",
                topics::cluster(cluster)
            )),
        },
        [_, _, _, attribute, "set"] => {
            let attribute = topics::parse_attribute(cluster, attribute)
                .ok_or_else(|| format!("no attribute `{attribute}`"))?;
            controller
                .write(node, endpoint, cluster, attribute, json, None)
                .await
                .map_err(e)?;
            Ok(Value::Null)
        }
        [_, _, _, "command", command] => {
            let command = topics::parse_command(cluster, command)
                .ok_or_else(|| format!("no command `{command}`"))?;
            let fields = if payload.is_empty() {
                Value::Null
            } else {
                json
            };
            controller
                .invoke(node, endpoint, cluster, command, fields, None)
                .await
                .map_err(e)
        }
        _ => Err("not a device topic".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert!(check_name("lamp").is_ok());
        assert!(check_name("bridge").is_err());
        assert!(check_name("a/b").is_err());
        assert!(check_name("").is_err());
    }
}
