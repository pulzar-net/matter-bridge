# Configuration reference

<!-- Generated from the configuration types by crates/matter-bridge/tests/config_reference.rs. Do not edit by hand. -->

Every key the configuration file accepts. Unknown keys are an error, so a typo stops the bridge at startup instead of being ignored. This page is generated from the configuration types; a [JSON Schema](config.schema.json) generated from the same types lets editors with TOML schema support (for example Taplo or Even Better TOML) complete and check the file.

## `[mqtt]`

The MQTT broker.

| Key | Type | Default | Description |
|---|---|---|---|
| `host` | string | `"localhost"` | Broker host. |
| `port` | integer (0 to 65535) | `1883` | Broker port. |
| `username` | string | unset | User name, if the broker wants one. |
| `password_file` | string | unset | A file holding the password (so it is not in the configuration). |
| `client_id` | string | `"matter-bridge"` | Client id. |
| `base_topic` | string | `"matter"` | Prefix of every topic the bridge uses. |
| `keepalive` | integer (from 0) | `30` | Keep-alive, seconds. |

## `[homeassistant]`

Home Assistant discovery.

| Key | Type | Default | Description |
|---|---|---|---|
| `enabled` | boolean | `true` | Publish discovery messages. |
| `discovery_prefix` | string | `"homeassistant"` | Home Assistant's discovery prefix. |
| `status_topic` | string | `"homeassistant/status"` | Home Assistant's birth topic; discovery is re-sent when it says "online". |

## `[matter]`

The Matter controller.

| Key | Type | Default | Description |
|---|---|---|---|
| `port` | integer (0 to 65535) | `5541` | UDP port for Matter. |
| `interface` | string | unset | Network interface for mDNS; the first with IPv6 if unset. |
| `fabric_id` | integer (from 0) | unset | Fabric id of a new fabric (random if unset). Ignored once the fabric exists. |
| `vendor_id` | integer (0 to 65535) | `65521` | Vendor id the controller administers nodes as. |
| `allow_unverified_attestation` | boolean | `true` | Commission devices whose attestation is not verified. Required for now: verification is not implemented yet (see the README). |
| `max_report_interval` | integer (0 to 65535) | `60` | Longest silence before a subscription is renewed, seconds. |
| `discovery_timeout` | integer (0 to 65535) | `30` | How long to look for a device to commission, seconds. |
| `bluetooth` | string | unset | The Bluetooth controller for commissioning over BLE: `hci0` (the Linux HCI user channel; BlueZ must not hold it) or `tcp:host:port`. |
| `thread_dataset_url` | string | unset | Where to read the Thread network's dataset for a Thread device: an OpenThread border router's REST API, e.g. `http://127.0.0.1:8081/node/dataset/active`. |

## `[data]`

Where state lives.

| Key | Type | Default | Description |
|---|---|---|---|
| `dir` | string | `"/var/lib/matter-bridge"` | The directory for the fabric, the controller's identity and the node store. |
