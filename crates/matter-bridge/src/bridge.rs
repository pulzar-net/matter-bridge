//! The bridge: controller events out to MQTT, MQTT requests and commands in
//! to the controller. Every payload is a type in [`crate::api`].

use std::collections::{BTreeSet, HashMap};
use std::future::Future;
use std::net::SocketAddr;
use std::sync::{Arc, RwLock};

use matter_host::{AttributePath, Controller, Event, Networks, Node, WiFi};
use serde::de::DeserializeOwned;
use serde_json::Value as Json;

use crate::api::{self, Availability, BridgeState, Id};
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

    fn devices(&self) -> Vec<api::Device> {
        self.by_id
            .values()
            .cloned()
            .map(api::Device::from)
            .collect()
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
        .publish(
            mqtt::state_topic(base),
            BridgeState::payload(Availability::Offline),
            true,
        )
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
                    if api::HomeAssistantStatus::parse(&payload) == Some(Availability::Online) {
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
            .publish(
                mqtt::state_topic(&base),
                BridgeState::payload(Availability::Online),
                true,
            )
            .await;
        let info = self.controller.info().clone();
        self.mqtt
            .publish_json(
                format!("{base}/bridge/info"),
                &api::BridgeInfo {
                    version: env!("CARGO_PKG_VERSION").into(),
                    fabric_id: format!("{:016X}", info.fabric_id),
                    compressed_fabric_id: format!("{:016X}", info.compressed_fabric_id),
                    node_id: format!("{:016X}", info.node_id),
                    vendor_id: info.vendor_id,
                },
                true,
            )
            .await;
        self.publish_devices().await;
        self.publish_discovery_all().await;
    }

    async fn publish_devices(&self) {
        let devices = api::BridgeDevices(self.names.read().unwrap().devices());
        self.mqtt
            .publish_json(format!("{}/bridge/devices", self.base()), &devices, true)
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
                let payload = serde_json::to_vec(&api::AttributeValue(a.value)).unwrap_or_default();
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
                let event = api::DeviceEvent {
                    number: e.number,
                    data: e.data,
                };
                self.mqtt.publish_json(topic, &event, false).await;
            }
            Event::Availability { node, online } => {
                let name = self.names.read().unwrap().name(node).map(str::to_string);
                if let Some(name) = name {
                    let state = if online {
                        Availability::Online
                    } else {
                        Availability::Offline
                    };
                    self.publish_retained(
                        node,
                        format!("{base}/{name}/availability"),
                        api::DeviceAvailability(state).payload(),
                    )
                    .await;
                }
                self.publish_bridge_event(api::BridgeEvent::Availability { node, online })
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
                self.publish_bridge_event(api::BridgeEvent::NodeUpdated {
                    node: Box::new(node.into()),
                })
                .await;
            }
            Event::NodeRemoved { node } => {
                self.clear_retained(node).await;
                self.names.write().unwrap().by_id.remove(&node);
                self.publish_devices().await;
                self.publish_bridge_event(api::BridgeEvent::NodeRemoved { node })
                    .await;
            }
            Event::Commissioning { node, step } => {
                self.publish_bridge_event(api::BridgeEvent::Commissioning {
                    node,
                    step: step.into(),
                })
                .await;
            }
            Event::CheckIn { node } => {
                self.publish_bridge_event(api::BridgeEvent::CheckIn { node })
                    .await;
            }
            Event::Ota {
                node,
                step,
                version,
            } => {
                self.publish_bridge_event(api::BridgeEvent::Ota {
                    node,
                    step: step.into(),
                    version,
                })
                .await;
            }
        }
    }

    async fn publish_bridge_event(&self, event: api::BridgeEvent) {
        self.mqtt
            .publish_json(format!("{}/bridge/event", self.base()), &event, false)
            .await;
    }

    fn spawn_request(&self, name: String, payload: Vec<u8>) {
        let ctx = Ctx {
            controller: self.controller.clone(),
            names: self.names.clone(),
            matter: self.config.matter.clone(),
        };
        let mqtt = self.mqtt.clone();
        let base = self.base().to_string();
        tokio::spawn(async move {
            let body: Json = if payload.is_empty() {
                Json::Object(Default::default())
            } else {
                serde_json::from_slice(&payload).unwrap_or(Json::Null)
            };
            let response = request(&ctx, &name, body).await;
            mqtt.publish(format!("{base}/bridge/response/{name}"), response, false)
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
                mqtt.publish_json(
                    format!("{base}/{topic}/result"),
                    &api::CommandResult::new(result),
                    false,
                )
                .await;
            }
        });
    }
}

/// What a request needs.
struct Ctx {
    controller: Controller,
    names: Arc<RwLock<Names>>,
    matter: MatterConfig,
}

impl Ctx {
    /// The node a request names, by name or node id.
    fn node(&self, device: &str) -> Result<u64, String> {
        self.names
            .read()
            .unwrap()
            .find(device)
            .ok_or_else(|| format!("no device `{device}`"))
    }
}

fn err(e: matter_host::Error) -> String {
    e.to_string()
}

/// Answer request `name` with its body: the response, serialized.
async fn request(ctx: &Ctx, name: &str, body: Json) -> Vec<u8> {
    let transaction = body.get("transaction").cloned();
    // Parse the body as `$req`, run `$handler` on it, and answer with
    // `$resp`; a body that does not parse is answered with its error.
    macro_rules! answer {
        ($resp:ident, $handler:ident) => {{
            let result = match parse(body) {
                Ok(req) => $handler(ctx, req).await,
                Err(e) => Err(e),
            };
            if let Err(error) = &result {
                tracing::warn!(request = %name, %error, "request failed");
            }
            serde_json::to_vec(&api::$resp::new(result, transaction))
        }};
    }
    let payload = match name {
        "commission" => answer!(CommissionResponse, commission),
        "remove" => answer!(RemoveResponse, remove),
        "rename" => answer!(RenameResponse, rename),
        "interview" => answer!(InterviewResponse, interview),
        "read" => answer!(ReadResponse, read),
        "write" => answer!(WriteResponse, write),
        "invoke" => answer!(InvokeResponse, invoke),
        "add_to_group" => answer!(AddToGroupResponse, add_to_group),
        "remove_from_group" => answer!(RemoveFromGroupResponse, remove_from_group),
        "group_invoke" => answer!(GroupInvokeResponse, group_invoke),
        "open_commissioning_window" => {
            answer!(OpenCommissioningWindowResponse, open_commissioning_window)
        }
        "discover" => answer!(DiscoverResponse, discover),
        "ota_images" => answer!(OtaImagesResponse, ota_images),
        "ota_update" => answer!(OtaUpdateResponse, ota_update),
        "devices" => answer!(DevicesResponse, devices),
        other => {
            let error = format!("unknown request `{other}`");
            tracing::warn!(request = %name, %error, "request failed");
            serde_json::to_vec(&api::UnknownRequestResponse::new(error, transaction))
        }
    };
    payload.unwrap_or_default()
}

fn parse<T: DeserializeOwned>(body: Json) -> Result<T, String> {
    serde_json::from_value(body).map_err(|e| format!("invalid request: {e}"))
}

fn cluster_of(id: &Id) -> Result<u32, String> {
    id.resolve(topics::parse_cluster)
        .ok_or_else(|| format!("no cluster {id:?}"))
}

fn group_of(group: u16) -> Result<u16, String> {
    match group {
        0 => Err("`group` must be 1 to 65535".into()),
        g => Ok(g),
    }
}

/// The networks a device commissioned over Bluetooth may join: a Thread
/// dataset (`thread_dataset` as hex, or fetched from `thread_dataset_url` or
/// the configured border router at commissioning time) and Wi-Fi. Neither
/// is kept or logged.
async fn networks_of(
    req: &api::CommissionRequest,
    matter: &MatterConfig,
) -> Result<Networks, String> {
    let thread_dataset = match &req.thread_dataset {
        Some(hex) => Some(hex.clone()),
        None => match req
            .thread_dataset_url
            .clone()
            .or_else(|| matter.thread_dataset_url.clone())
        {
            Some(url) => Some(fetch_dataset(url).await?),
            None => None,
        },
    };
    let thread_dataset = thread_dataset
        .map(|hex| matter_host::network::parse_thread_dataset_hex(&hex))
        .transpose()
        .map_err(|e| e.to_string())?;
    let wifi = req.wifi.as_ref().map(|w| WiFi {
        ssid: w.ssid.as_bytes().to_vec(),
        credentials: w.password.as_bytes().to_vec(),
    });
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

async fn commission(ctx: &Ctx, req: api::CommissionRequest) -> Result<api::Device, String> {
    let address = req
        .address
        .as_deref()
        .map(|a| {
            a.parse::<SocketAddr>()
                .map_err(|_| format!("`address` {a} is not ip:port"))
        })
        .transpose()?;
    if let Some(n) = &req.name {
        check_name(n)?;
        if ctx.names.read().unwrap().find(n).is_some() {
            return Err(format!("the name `{n}` is taken"));
        }
    }
    let networks = networks_of(&req, &ctx.matter).await?;
    let node = ctx
        .controller
        .commission(matter_host::CommissionRequest {
            code: req.code,
            address,
            name: req.name,
            discovery_timeout: req.timeout.unwrap_or(ctx.matter.discovery_timeout),
            networks,
        })
        .await
        .map_err(err)?;
    Ok(node.into())
}

async fn remove(ctx: &Ctx, req: api::RemoveRequest) -> Result<api::Removed, String> {
    let node = ctx.node(&req.device)?;
    ctx.controller.remove(node, req.force).await.map_err(err)?;
    Ok(api::Removed { node })
}

async fn rename(ctx: &Ctx, req: api::RenameRequest) -> Result<api::Device, String> {
    let node = ctx.node(&req.device)?;
    check_name(&req.to)?;
    let node = ctx.controller.rename(node, req.to).await.map_err(err)?;
    Ok(node.into())
}

async fn interview(ctx: &Ctx, req: api::InterviewRequest) -> Result<api::Device, String> {
    let node = ctx.node(&req.device)?;
    Ok(ctx.controller.interview(node).await.map_err(err)?.into())
}

async fn read(ctx: &Ctx, req: api::ReadRequest) -> Result<Vec<api::ReadValue>, String> {
    let node = ctx.node(&req.device)?;
    let cluster = req.cluster.as_ref().map(cluster_of).transpose()?;
    let attribute = match (&req.attribute, cluster) {
        (Some(a), Some(c)) => Some(
            a.resolve(|s| topics::parse_attribute(c, s))
                .ok_or("unknown `attribute`")?,
        ),
        (Some(a), None) => Some(
            a.resolve(|_| None)
                .ok_or("`attribute` by name needs `cluster`")?,
        ),
        (None, _) => None,
    };
    let values = ctx
        .controller
        .read(
            node,
            AttributePath {
                endpoint: req.endpoint,
                cluster,
                attribute,
            },
        )
        .await
        .map_err(err)?;
    Ok(values
        .into_iter()
        .map(|a| api::ReadValue {
            endpoint: a.endpoint,
            cluster: topics::cluster(a.cluster),
            attribute: topics::attribute(a.cluster, a.attribute),
            value: a.value,
            status: a.status,
        })
        .collect())
}

async fn write(ctx: &Ctx, req: api::WriteRequest) -> Result<(), String> {
    let node = ctx.node(&req.device)?;
    let cluster = cluster_of(&req.cluster)?;
    let attribute = req
        .attribute
        .resolve(|s| topics::parse_attribute(cluster, s))
        .ok_or("unknown `attribute`")?;
    ctx.controller
        .write(node, req.endpoint, cluster, attribute, req.value, req.timed)
        .await
        .map_err(err)
}

async fn invoke(ctx: &Ctx, req: api::InvokeRequest) -> Result<Json, String> {
    let node = ctx.node(&req.device)?;
    let cluster = cluster_of(&req.cluster)?;
    let command = req
        .command
        .resolve(|s| topics::parse_command(cluster, s))
        .ok_or("unknown `command`")?;
    ctx.controller
        .invoke(
            node,
            req.endpoint,
            cluster,
            command,
            req.fields.unwrap_or(Json::Null),
            req.timed,
        )
        .await
        .map_err(err)
}

async fn add_to_group(ctx: &Ctx, req: api::AddToGroupRequest) -> Result<(), String> {
    let node = ctx.node(&req.device)?;
    let group = group_of(req.group)?;
    ctx.controller
        .add_to_group(node, req.endpoint, group, req.name.unwrap_or_default())
        .await
        .map_err(err)
}

async fn remove_from_group(ctx: &Ctx, req: api::RemoveFromGroupRequest) -> Result<(), String> {
    let node = ctx.node(&req.device)?;
    let group = group_of(req.group)?;
    ctx.controller
        .remove_from_group(node, req.endpoint, group)
        .await
        .map_err(err)
}

async fn group_invoke(ctx: &Ctx, req: api::GroupInvokeRequest) -> Result<(), String> {
    let group = group_of(req.group)?;
    let cluster = cluster_of(&req.cluster)?;
    let command = req
        .command
        .resolve(|s| topics::parse_command(cluster, s))
        .ok_or("unknown `command`")?;
    ctx.controller
        .group_invoke(group, cluster, command, req.fields.unwrap_or(Json::Null))
        .await
        .map_err(err)
}

async fn open_commissioning_window(
    ctx: &Ctx,
    req: api::OpenCommissioningWindowRequest,
) -> Result<api::CommissioningWindow, String> {
    let node = ctx.node(&req.device)?;
    let window = ctx
        .controller
        .open_commissioning_window(node, req.timeout.unwrap_or(300))
        .await
        .map_err(err)?;
    Ok(window.into())
}

async fn discover(
    ctx: &Ctx,
    req: api::DiscoverRequest,
) -> Result<Vec<api::Commissionable>, String> {
    let found = ctx
        .controller
        .discover(req.timeout.unwrap_or(5))
        .await
        .map_err(err)?;
    Ok(found.into_iter().map(Into::into).collect())
}

async fn ota_images(ctx: &Ctx, _req: api::OtaImagesRequest) -> Result<Vec<api::OtaImage>, String> {
    let images = ctx.controller.ota_images().map_err(err)?;
    Ok(images.into_iter().map(Into::into).collect())
}

async fn ota_update(
    ctx: &Ctx,
    req: api::OtaUpdateRequest,
) -> Result<Option<api::OtaImage>, String> {
    let node = ctx.node(&req.device)?;
    let image = ctx.controller.announce_ota(node).await.map_err(err)?;
    Ok(image.map(Into::into))
}

async fn devices(ctx: &Ctx, _req: api::DevicesRequest) -> Result<Vec<api::Device>, String> {
    let nodes = ctx.controller.nodes().await.map_err(err)?;
    Ok(nodes.into_iter().map(Into::into).collect())
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
) -> Result<Json, String> {
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

    match parts.as_slice() {
        [_, _, _, "set"] => {
            let value = api::ClusterSet::from_payload(payload).0;
            match cluster {
                ON_OFF => {
                    let command = match &value {
                        Json::Bool(true) => 1,
                        Json::Bool(false) => 0,
                        Json::String(s) if s.eq_ignore_ascii_case("on") => 1,
                        Json::String(s) if s.eq_ignore_ascii_case("off") => 0,
                        Json::String(s) if s.eq_ignore_ascii_case("toggle") => 2,
                        _ => return Err("on_off/set takes ON, OFF or TOGGLE".into()),
                    };
                    controller
                        .invoke(node, endpoint, ON_OFF, command, Json::Null, None)
                        .await
                        .map_err(err)
                }
                LEVEL_CONTROL => {
                    let level = value
                        .as_u64()
                        .or_else(|| value.as_str().and_then(|s| s.parse().ok()))
                        .filter(|l| *l <= 254)
                        .ok_or("level_control/set takes a level, 0 to 254")?;
                    controller
                        .invoke(
                            node,
                            endpoint,
                            LEVEL_CONTROL,
                            0x04, // MoveToLevelWithOnOff
                            serde_json::json!({"level": level, "optionsMask": 0, "optionsOverride": 0}),
                            None,
                        )
                        .await
                        .map_err(err)
                }
                _ => Err(format!(
                    "{}/set is not supported; set an attribute or invoke a command",
                    topics::cluster(cluster)
                )),
            }
        }
        [_, _, _, attribute, "set"] => {
            let attribute = topics::parse_attribute(cluster, attribute)
                .ok_or_else(|| format!("no attribute `{attribute}`"))?;
            let value = api::AttributeWrite::from_payload(payload).0;
            controller
                .write(node, endpoint, cluster, attribute, value, None)
                .await
                .map_err(err)?;
            Ok(Json::Null)
        }
        [_, _, _, "command", command] => {
            let command = topics::parse_command(cluster, command)
                .ok_or_else(|| format!("no command `{command}`"))?;
            let fields = api::CommandFields::from_payload(payload).0;
            controller
                .invoke(node, endpoint, cluster, command, fields, None)
                .await
                .map_err(err)
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
