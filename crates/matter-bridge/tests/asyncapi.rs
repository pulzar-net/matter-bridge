//! The MQTT reference on the docs site is an AsyncAPI description generated
//! from the payload types in `matter_bridge::api`, the types the bridge
//! publishes, so it cannot drift from the code. This test fails when the
//! committed description differs from what the types say; regenerate it with
//!
//! ```sh
//! UPDATE_DOCS=1 cargo test -p matter-bridge --features unstable-asyncapi-docs --test asyncapi
//! ```
//!
//! Topics are written with the default base topic (`matter`) and discovery
//! prefix (`homeassistant`); the description says how to change them.

#![allow(clippy::duplicated_attributes)] // operations share channels by design

use std::path::PathBuf;

use asyncapi_rust::AsyncApi;
use matter_bridge::api::*;

const SPEC: &str = "docs/public/asyncapi.json";

/// The bridge's MQTT API.
#[derive(AsyncApi)]
#[asyncapi(
    title = "matter-bridge MQTT API",
    version = "0.0.0",
    description = "Everything matter-bridge publishes and accepts on MQTT.\n\nTopics here use the default base topic, `matter` (`mqtt.base_topic` in the configuration), and Home Assistant's default discovery prefix, `homeassistant` (`homeassistant.discovery_prefix`). `{device}` is the name a person gave a device, given at commissioning or with `rename` (`node_<node id>` otherwise). Clusters, attributes, commands and events are named in `snake_case` from the Matter data model, or by id in hex where the data model does not know them; requests take a name or a number.\n\nThe bridge publishes and subscribes at QoS 1. Payloads are JSON unless a message says otherwise. Requests go to `matter/bridge/request/<name>`; the answer comes on `matter/bridge/response/<name>`, echoing the request's `transaction` so a client can match the two. Fabric keys, Check-In keys and network credentials never appear on MQTT."
)]
#[asyncapi_server(
    name = "broker",
    host = "localhost:1883",
    protocol = "mqtt",
    description = "Your MQTT broker: `mqtt.host` and `mqtt.port` in the configuration. The bridge connects with a clean session, its `mqtt.client_id`, and `matter/bridge/state` set to offline as its last will.",
    mqtt(
        client_id = "matter-bridge",
        clean_session = true,
        keep_alive = 30,
        last_will(
            topic = "matter/bridge/state",
            qos = 1,
            message = "{\"state\":\"offline\"}",
            retain = true
        )
    )
)]
// Bridge state, info, devices and events.
#[asyncapi_channel(
    name = "bridgeState",
    address = "matter/bridge/state",
    description = "The bridge's availability, retained. `offline` is also the bridge's last will."
)]
#[asyncapi_channel(
    name = "bridgeInfo",
    address = "matter/bridge/info",
    description = "The bridge and its fabric, retained."
)]
#[asyncapi_channel(
    name = "bridgeDevices",
    address = "matter/bridge/devices",
    description = "Every commissioned device and its interview, retained."
)]
#[asyncapi_channel(
    name = "bridgeEvent",
    address = "matter/bridge/event",
    description = "Commissioning steps, device updates and removals, availability changes, Check-Ins and software update steps, as they happen. `type` names the event."
)]
// Devices.
#[asyncapi_channel(
    name = "deviceAvailability",
    address = "matter/{device}/availability",
    description = "Whether the bridge's subscription to a device holds, retained. Plain text.",
    parameter(
        name = "device",
        description = "The device's name, given at commissioning or with `rename` (`node_<node id>` otherwise).",
        examples = ["lamp", "node_2"]
    )
)]
#[asyncapi_channel(
    name = "attributeValue",
    address = "matter/{device}/{endpoint}/{cluster}/{attribute}",
    description = "An attribute's value as the device reports it, retained, updated by the bridge's subscription. An empty retained message clears it when the device is removed or renamed.",
    parameter(
        name = "device",
        description = "The device's name, given at commissioning or with `rename` (`node_<node id>` otherwise).",
        examples = ["lamp", "node_2"]
    ),
    parameter(
        name = "endpoint",
        description = "The endpoint number.",
        examples = ["1"]
    ),
    parameter(
        name = "cluster",
        description = "The cluster's `snake_case` name from the Matter data model, or its id in hex for a cluster the data model does not know.",
        examples = ["on_off", "level_control", "0xfff1fc01"]
    ),
    parameter(
        name = "attribute",
        description = "The attribute's `snake_case` name, or its id in hex.",
        examples = ["on_off", "current_level"]
    )
)]
#[asyncapi_channel(
    name = "attributeSet",
    address = "matter/{device}/{endpoint}/{cluster}/{attribute}/set",
    description = "Write an attribute.",
    parameter(
        name = "device",
        description = "The device's name, given at commissioning or with `rename` (`node_<node id>` otherwise).",
        examples = ["lamp", "node_2"]
    ),
    parameter(
        name = "endpoint",
        description = "The endpoint number.",
        examples = ["1"]
    ),
    parameter(
        name = "cluster",
        description = "The cluster's `snake_case` name from the Matter data model, or its id in hex for a cluster the data model does not know.",
        examples = ["on_off", "level_control", "0xfff1fc01"]
    ),
    parameter(
        name = "attribute",
        description = "The attribute's `snake_case` name, or its id in hex.",
        examples = ["on_off", "current_level"]
    )
)]
#[asyncapi_channel(
    name = "clusterSet",
    address = "matter/{device}/{endpoint}/{cluster}/set",
    description = "Switch or dim a light or switch: `on_off/set` takes `ON`, `OFF` or `TOGGLE`, `level_control/set` a level from 0 to 254. Other clusters have no `set`.",
    parameter(
        name = "device",
        description = "The device's name, given at commissioning or with `rename` (`node_<node id>` otherwise).",
        examples = ["lamp", "node_2"]
    ),
    parameter(
        name = "endpoint",
        description = "The endpoint number.",
        examples = ["1"]
    ),
    parameter(
        name = "cluster",
        description = "The cluster's `snake_case` name from the Matter data model, or its id in hex for a cluster the data model does not know.",
        examples = ["on_off", "level_control", "0xfff1fc01"]
    )
)]
#[asyncapi_channel(
    name = "deviceEvent",
    address = "matter/{device}/{endpoint}/{cluster}/event/{event}",
    description = "An event the device reported.",
    parameter(
        name = "device",
        description = "The device's name, given at commissioning or with `rename` (`node_<node id>` otherwise).",
        examples = ["lamp", "node_2"]
    ),
    parameter(
        name = "endpoint",
        description = "The endpoint number.",
        examples = ["1"]
    ),
    parameter(
        name = "cluster",
        description = "The cluster's `snake_case` name from the Matter data model, or its id in hex for a cluster the data model does not know.",
        examples = ["on_off", "level_control", "0xfff1fc01"]
    ),
    parameter(
        name = "event",
        description = "The event's `snake_case` name, or its id in hex.",
        examples = ["state_change", "start_up"]
    )
)]
#[asyncapi_channel(
    name = "command",
    address = "matter/{device}/{endpoint}/{cluster}/command/{command}",
    description = "Invoke a command on a device's endpoint.",
    parameter(
        name = "device",
        description = "The device's name, given at commissioning or with `rename` (`node_<node id>` otherwise).",
        examples = ["lamp", "node_2"]
    ),
    parameter(
        name = "endpoint",
        description = "The endpoint number.",
        examples = ["1"]
    ),
    parameter(
        name = "cluster",
        description = "The cluster's `snake_case` name from the Matter data model, or its id in hex for a cluster the data model does not know.",
        examples = ["on_off", "level_control", "0xfff1fc01"]
    ),
    parameter(
        name = "command",
        description = "The command's `snake_case` name, or its id in hex.",
        examples = ["toggle", "move_to_level"]
    )
)]
#[asyncapi_channel(
    name = "commandResult",
    address = "matter/{device}/{endpoint}/{cluster}/command/{command}/result",
    description = "The outcome of a command sent on the `command` topic.",
    parameter(
        name = "device",
        description = "The device's name, given at commissioning or with `rename` (`node_<node id>` otherwise).",
        examples = ["lamp", "node_2"]
    ),
    parameter(
        name = "endpoint",
        description = "The endpoint number.",
        examples = ["1"]
    ),
    parameter(
        name = "cluster",
        description = "The cluster's `snake_case` name from the Matter data model, or its id in hex for a cluster the data model does not know.",
        examples = ["on_off", "level_control", "0xfff1fc01"]
    ),
    parameter(
        name = "command",
        description = "The command's `snake_case` name, or its id in hex.",
        examples = ["toggle", "move_to_level"]
    )
)]
// Requests and their responses.
#[asyncapi_channel(
    name = "commissionRequest",
    address = "matter/bridge/request/commission",
    description = "Commission a device: found by its pairing code over DNS-SD, or over Bluetooth LE when the bridge has a Bluetooth controller."
)]
#[asyncapi_channel(
    name = "commissionResponse",
    address = "matter/bridge/response/commission",
    description = "The answer to `commission`."
)]
#[asyncapi_channel(
    name = "openCommissioningWindowRequest",
    address = "matter/bridge/request/open_commissioning_window",
    description = "Open a commissioning window on a device, so another ecosystem can add it too (multi-admin)."
)]
#[asyncapi_channel(
    name = "openCommissioningWindowResponse",
    address = "matter/bridge/response/open_commissioning_window",
    description = "The answer to `open_commissioning_window`."
)]
#[asyncapi_channel(
    name = "readRequest",
    address = "matter/bridge/request/read",
    description = "Read attributes of a device."
)]
#[asyncapi_channel(
    name = "readResponse",
    address = "matter/bridge/response/read",
    description = "The answer to `read`."
)]
#[asyncapi_channel(
    name = "writeRequest",
    address = "matter/bridge/request/write",
    description = "Write an attribute of a device."
)]
#[asyncapi_channel(
    name = "writeResponse",
    address = "matter/bridge/response/write",
    description = "The answer to `write`."
)]
#[asyncapi_channel(
    name = "invokeRequest",
    address = "matter/bridge/request/invoke",
    description = "Invoke a command on a device."
)]
#[asyncapi_channel(
    name = "invokeResponse",
    address = "matter/bridge/response/invoke",
    description = "The answer to `invoke`."
)]
#[asyncapi_channel(
    name = "addToGroupRequest",
    address = "matter/bridge/request/add_to_group",
    description = "Put an endpoint of a device in a group."
)]
#[asyncapi_channel(
    name = "addToGroupResponse",
    address = "matter/bridge/response/add_to_group",
    description = "The answer to `add_to_group`."
)]
#[asyncapi_channel(
    name = "removeFromGroupRequest",
    address = "matter/bridge/request/remove_from_group",
    description = "Take an endpoint of a device out of a group."
)]
#[asyncapi_channel(
    name = "removeFromGroupResponse",
    address = "matter/bridge/response/remove_from_group",
    description = "The answer to `remove_from_group`."
)]
#[asyncapi_channel(
    name = "groupInvokeRequest",
    address = "matter/bridge/request/group_invoke",
    description = "Send one multicast command to every member of a group."
)]
#[asyncapi_channel(
    name = "groupInvokeResponse",
    address = "matter/bridge/response/group_invoke",
    description = "The answer to `group_invoke`."
)]
#[asyncapi_channel(
    name = "interviewRequest",
    address = "matter/bridge/request/interview",
    description = "Read what a device is again."
)]
#[asyncapi_channel(
    name = "interviewResponse",
    address = "matter/bridge/response/interview",
    description = "The answer to `interview`."
)]
#[asyncapi_channel(
    name = "renameRequest",
    address = "matter/bridge/request/rename",
    description = "Rename a device. Its topics move to the new name."
)]
#[asyncapi_channel(
    name = "renameResponse",
    address = "matter/bridge/response/rename",
    description = "The answer to `rename`."
)]
#[asyncapi_channel(
    name = "removeRequest",
    address = "matter/bridge/request/remove",
    description = "Remove a device from the bridge's fabric and forget it."
)]
#[asyncapi_channel(
    name = "removeResponse",
    address = "matter/bridge/response/remove",
    description = "The answer to `remove`."
)]
#[asyncapi_channel(
    name = "discoverRequest",
    address = "matter/bridge/request/discover",
    description = "Look for devices that can be commissioned."
)]
#[asyncapi_channel(
    name = "discoverResponse",
    address = "matter/bridge/response/discover",
    description = "The answer to `discover`."
)]
#[asyncapi_channel(
    name = "devicesRequest",
    address = "matter/bridge/request/devices",
    description = "Publish `matter/bridge/devices` again."
)]
#[asyncapi_channel(
    name = "devicesResponse",
    address = "matter/bridge/response/devices",
    description = "The answer to `devices`."
)]
#[asyncapi_channel(
    name = "otaImagesRequest",
    address = "matter/bridge/request/ota_images",
    description = "List the OTA images the bridge offers."
)]
#[asyncapi_channel(
    name = "otaImagesResponse",
    address = "matter/bridge/response/ota_images",
    description = "The answer to `ota_images`."
)]
#[asyncapi_channel(
    name = "otaUpdateRequest",
    address = "matter/bridge/request/ota_update",
    description = "Tell a device to ask the bridge for a software update now."
)]
#[asyncapi_channel(
    name = "otaUpdateResponse",
    address = "matter/bridge/response/ota_update",
    description = "The answer to `ota_update`."
)]
// Home Assistant.
#[asyncapi_channel(
    name = "homeAssistantDiscovery",
    address = "homeassistant/{component}/{device_key}/{entity}/config",
    description = "Home Assistant MQTT discovery: one retained config per entity, sent when a device is commissioned, interviewed or renamed, and when Home Assistant comes online. An empty retained message removes an entity. Only with `homeassistant.enabled`.",
    parameter(
        name = "component",
        description = "The entity type.",
        enum_values = ["light", "switch", "sensor", "binary_sensor"]
    ),
    parameter(
        name = "device_key",
        description = "`matter_<compressed fabric id>_<node id>`, both as 16 lower-case hex digits.",
        examples = ["matter_00000000000000ab_0000000000000002"]
    ),
    parameter(
        name = "entity",
        description = "`<endpoint>_<cluster id in hex>`.",
        examples = ["1_6", "1_402"]
    )
)]
#[asyncapi_channel(
    name = "homeAssistantStatus",
    address = "homeassistant/status",
    description = "Home Assistant's birth and last will (`homeassistant.status_topic`). When it says `online`, the bridge publishes discovery again. Only with `homeassistant.enabled`."
)]
// What the bridge publishes.
#[asyncapi_operation(name = "publishBridgeState", summary = "The bridge is online or offline", action = "send", channel = "bridgeState", description = "Published `online` on connecting and `offline` on stopping.", messages = [BridgeState], mqtt(qos = 1, retain = true))]
#[asyncapi_operation(name = "publishBridgeInfo", summary = "The bridge's version and fabric", action = "send", channel = "bridgeInfo", description = "Published on connecting.", messages = [BridgeInfo], mqtt(qos = 1, retain = true))]
#[asyncapi_operation(name = "publishBridgeDevices", summary = "Every device and its interview", action = "send", channel = "bridgeDevices", description = "Published on connecting and whenever a device is commissioned, interviewed, renamed or removed.", messages = [BridgeDevices], mqtt(qos = 1, retain = true))]
#[asyncapi_operation(name = "publishBridgeEvent", summary = "Something happened to a device", action = "send", channel = "bridgeEvent", description = "Published as each event happens.", messages = [BridgeEvent], mqtt(qos = 1, retain = false))]
#[asyncapi_operation(name = "publishDeviceAvailability", summary = "The device is reachable or not", action = "send", channel = "deviceAvailability", description = "Published when the bridge's subscription to the device starts or lapses.", messages = [DeviceAvailability], mqtt(qos = 1, retain = true))]
#[asyncapi_operation(name = "publishAttributeValue", summary = "An attribute's value", action = "send", channel = "attributeValue", description = "Published for each attribute in each subscription report, starting with the priming report.", messages = [AttributeValue], mqtt(qos = 1, retain = true))]
#[asyncapi_operation(name = "publishDeviceEvent", summary = "An event the device reported", action = "send", channel = "deviceEvent", description = "Published for each event in a subscription report.", messages = [DeviceEvent], mqtt(qos = 1, retain = false))]
#[asyncapi_operation(name = "publishCommandResult", summary = "The command's response, or its error", action = "send", channel = "commandResult", description = "Published once per command.", messages = [CommandResult], mqtt(qos = 1, retain = false))]
#[asyncapi_operation(name = "publishDiscovery", summary = "A Home Assistant entity", action = "send", channel = "homeAssistantDiscovery", description = "Published per entity.", messages = [DiscoveryConfig], mqtt(qos = 1, retain = true))]
// What the bridge receives.
#[asyncapi_operation(name = "receiveAttributeSet", summary = "Write an attribute", action = "receive", channel = "attributeSet", description = "JSON in the form the attribute is published in; text that is not JSON is a string.", messages = [AttributeWrite], mqtt(qos = 1))]
#[asyncapi_operation(name = "receiveClusterSet", summary = "Switch or dim an endpoint", action = "receive", channel = "clusterSet", description = "Plain text or JSON.", messages = [ClusterSet], mqtt(qos = 1))]
#[asyncapi_operation(name = "receiveCommand", summary = "Invoke a command", action = "receive", channel = "command", description = "The result comes on the `result` topic.", messages = [CommandFields], mqtt(qos = 1))]
#[asyncapi_operation(name = "receiveHomeAssistantStatus", summary = "Home Assistant started or stopped", action = "receive", channel = "homeAssistantStatus", description = "Read only with `homeassistant.enabled`.", messages = [HomeAssistantStatus], mqtt(qos = 1))]
#[asyncapi_operation(name = "receiveCommission", summary = "Commission a device", action = "receive", channel = "commissionRequest", description = "Read on `matter/bridge/request/commission`; any other field is ignored. Answered once on `matter/bridge/response/commission`.", messages = [CommissionRequest], mqtt(qos = 1))]
#[asyncapi_operation(name = "sendCommission", summary = "The device is commissioned", action = "send", channel = "commissionResponse", description = "Published once per `commission` request, when it is done or has failed, echoing its `transaction`.", messages = [CommissionResponse], mqtt(qos = 1, retain = false))]
#[asyncapi_operation(name = "receiveOpenCommissioningWindow", summary = "Let another ecosystem add a device", action = "receive", channel = "openCommissioningWindowRequest", description = "Read on `matter/bridge/request/open_commissioning_window`; any other field is ignored. Answered once on `matter/bridge/response/open_commissioning_window`.", messages = [OpenCommissioningWindowRequest], mqtt(qos = 1))]
#[asyncapi_operation(name = "sendOpenCommissioningWindow", summary = "The window is open, with its pairing code", action = "send", channel = "openCommissioningWindowResponse", description = "Published once per `open_commissioning_window` request, when it is done or has failed, echoing its `transaction`.", messages = [OpenCommissioningWindowResponse], mqtt(qos = 1, retain = false))]
#[asyncapi_operation(name = "receiveRead", summary = "Read attributes", action = "receive", channel = "readRequest", description = "Read on `matter/bridge/request/read`; any other field is ignored. Answered once on `matter/bridge/response/read`.", messages = [ReadRequest], mqtt(qos = 1))]
#[asyncapi_operation(name = "sendRead", summary = "The attributes' values", action = "send", channel = "readResponse", description = "Published once per `read` request, when it is done or has failed, echoing its `transaction`.", messages = [ReadResponse], mqtt(qos = 1, retain = false))]
#[asyncapi_operation(name = "receiveWrite", summary = "Write an attribute", action = "receive", channel = "writeRequest", description = "Read on `matter/bridge/request/write`; any other field is ignored. Answered once on `matter/bridge/response/write`.", messages = [WriteRequest], mqtt(qos = 1))]
#[asyncapi_operation(name = "sendWrite", summary = "The attribute is written", action = "send", channel = "writeResponse", description = "Published once per `write` request, when it is done or has failed, echoing its `transaction`.", messages = [WriteResponse], mqtt(qos = 1, retain = false))]
#[asyncapi_operation(name = "receiveInvoke", summary = "Invoke a command", action = "receive", channel = "invokeRequest", description = "Read on `matter/bridge/request/invoke`; any other field is ignored. Answered once on `matter/bridge/response/invoke`.", messages = [InvokeRequest], mqtt(qos = 1))]
#[asyncapi_operation(name = "sendInvoke", summary = "The command's response", action = "send", channel = "invokeResponse", description = "Published once per `invoke` request, when it is done or has failed, echoing its `transaction`.", messages = [InvokeResponse], mqtt(qos = 1, retain = false))]
#[asyncapi_operation(name = "receiveAddToGroup", summary = "Put an endpoint in a group", action = "receive", channel = "addToGroupRequest", description = "Read on `matter/bridge/request/add_to_group`; any other field is ignored. Answered once on `matter/bridge/response/add_to_group`.", messages = [AddToGroupRequest], mqtt(qos = 1))]
#[asyncapi_operation(name = "sendAddToGroup", summary = "The endpoint is in the group", action = "send", channel = "addToGroupResponse", description = "Published once per `add_to_group` request, when it is done or has failed, echoing its `transaction`.", messages = [AddToGroupResponse], mqtt(qos = 1, retain = false))]
#[asyncapi_operation(name = "receiveRemoveFromGroup", summary = "Take an endpoint out of a group", action = "receive", channel = "removeFromGroupRequest", description = "Read on `matter/bridge/request/remove_from_group`; any other field is ignored. Answered once on `matter/bridge/response/remove_from_group`.", messages = [RemoveFromGroupRequest], mqtt(qos = 1))]
#[asyncapi_operation(name = "sendRemoveFromGroup", summary = "The endpoint left the group", action = "send", channel = "removeFromGroupResponse", description = "Published once per `remove_from_group` request, when it is done or has failed, echoing its `transaction`.", messages = [RemoveFromGroupResponse], mqtt(qos = 1, retain = false))]
#[asyncapi_operation(name = "receiveGroupInvoke", summary = "Send a command to a group", action = "receive", channel = "groupInvokeRequest", description = "Read on `matter/bridge/request/group_invoke`; any other field is ignored. Answered once on `matter/bridge/response/group_invoke`.", messages = [GroupInvokeRequest], mqtt(qos = 1))]
#[asyncapi_operation(name = "sendGroupInvoke", summary = "The group command is sent", action = "send", channel = "groupInvokeResponse", description = "Published once per `group_invoke` request, when it is done or has failed, echoing its `transaction`.", messages = [GroupInvokeResponse], mqtt(qos = 1, retain = false))]
#[asyncapi_operation(name = "receiveInterview", summary = "Interview a device again", action = "receive", channel = "interviewRequest", description = "Read on `matter/bridge/request/interview`; any other field is ignored. Answered once on `matter/bridge/response/interview`.", messages = [InterviewRequest], mqtt(qos = 1))]
#[asyncapi_operation(name = "sendInterview", summary = "The device, interviewed", action = "send", channel = "interviewResponse", description = "Published once per `interview` request, when it is done or has failed, echoing its `transaction`.", messages = [InterviewResponse], mqtt(qos = 1, retain = false))]
#[asyncapi_operation(name = "receiveRename", summary = "Rename a device", action = "receive", channel = "renameRequest", description = "Read on `matter/bridge/request/rename`; any other field is ignored. Answered once on `matter/bridge/response/rename`.", messages = [RenameRequest], mqtt(qos = 1))]
#[asyncapi_operation(name = "sendRename", summary = "The device is renamed", action = "send", channel = "renameResponse", description = "Published once per `rename` request, when it is done or has failed, echoing its `transaction`.", messages = [RenameResponse], mqtt(qos = 1, retain = false))]
#[asyncapi_operation(name = "receiveRemove", summary = "Remove a device", action = "receive", channel = "removeRequest", description = "Read on `matter/bridge/request/remove`; any other field is ignored. Answered once on `matter/bridge/response/remove`.", messages = [RemoveRequest], mqtt(qos = 1))]
#[asyncapi_operation(name = "sendRemove", summary = "The device is removed", action = "send", channel = "removeResponse", description = "Published once per `remove` request, when it is done or has failed, echoing its `transaction`.", messages = [RemoveResponse], mqtt(qos = 1, retain = false))]
#[asyncapi_operation(name = "receiveDiscover", summary = "Look for commissionable devices", action = "receive", channel = "discoverRequest", description = "Read on `matter/bridge/request/discover`; any other field is ignored. Answered once on `matter/bridge/response/discover`.", messages = [DiscoverRequest], mqtt(qos = 1))]
#[asyncapi_operation(name = "sendDiscover", summary = "The commissionable devices seen", action = "send", channel = "discoverResponse", description = "Published once per `discover` request, when it is done or has failed, echoing its `transaction`.", messages = [DiscoverResponse], mqtt(qos = 1, retain = false))]
#[asyncapi_operation(name = "receiveDevices", summary = "List the devices", action = "receive", channel = "devicesRequest", description = "Read on `matter/bridge/request/devices`; any other field is ignored. Answered once on `matter/bridge/response/devices`.", messages = [DevicesRequest], mqtt(qos = 1))]
#[asyncapi_operation(name = "sendDevices", summary = "Every device", action = "send", channel = "devicesResponse", description = "Published once per `devices` request, when it is done or has failed, echoing its `transaction`.", messages = [DevicesResponse], mqtt(qos = 1, retain = false))]
#[asyncapi_operation(name = "receiveOtaImages", summary = "List the software update images", action = "receive", channel = "otaImagesRequest", description = "Read on `matter/bridge/request/ota_images`; any other field is ignored. Answered once on `matter/bridge/response/ota_images`.", messages = [OtaImagesRequest], mqtt(qos = 1))]
#[asyncapi_operation(name = "sendOtaImages", summary = "The images offered", action = "send", channel = "otaImagesResponse", description = "Published once per `ota_images` request, when it is done or has failed, echoing its `transaction`.", messages = [OtaImagesResponse], mqtt(qos = 1, retain = false))]
#[asyncapi_operation(name = "receiveOtaUpdate", summary = "Update a device's software", action = "receive", channel = "otaUpdateRequest", description = "Read on `matter/bridge/request/ota_update`; any other field is ignored. Answered once on `matter/bridge/response/ota_update`.", messages = [OtaUpdateRequest], mqtt(qos = 1))]
#[asyncapi_operation(name = "sendOtaUpdate", summary = "The image the device will be offered", action = "send", channel = "otaUpdateResponse", description = "Published once per `ota_update` request, when it is done or has failed, echoing its `transaction`.", messages = [OtaUpdateResponse], mqtt(qos = 1, retain = false))]
struct MqttApi;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn render() -> String {
    let mut spec = MqttApi::asyncapi_spec();
    // The description is versioned with the bridge.
    spec.info.version = env!("CARGO_PKG_VERSION").into();
    serde_json::to_string_pretty(&spec).unwrap() + "\n"
}

#[test]
fn asyncapi_description_is_current() {
    let json = render();
    let path = repo_root().join(SPEC);
    if std::env::var_os("UPDATE_DOCS").is_some() {
        std::fs::write(&path, &json).unwrap();
        return;
    }
    let committed = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(
        committed == json,
        "{SPEC} is stale: regenerate it with \
         UPDATE_DOCS=1 cargo test -p matter-bridge --features unstable-asyncapi-docs --test asyncapi"
    );
}
