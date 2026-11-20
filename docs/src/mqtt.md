# MQTT API

The bridge publishes and listens under one base topic, `matter` unless
`mqtt.base_topic` says otherwise. Payloads are JSON. This page uses the
default base topic throughout.

## Names in topics

- `<device>` is the device's name, given at commissioning or with `rename`
  (`node_<node id>` otherwise).
- `<endpoint>` is the endpoint number.
- Clusters, attributes, commands and events are named in `snake_case` from
  the Matter data model: `on_off`, `level_control`, `current_level`,
  `move_to_level`. A cluster or attribute the data model does not know (a
  vendor's) is its number in hex, such as `0xfff1fc01`. Requests accept a
  name or a number.

## Topics at a glance

| Topic | Direction | Retained | What |
|---|---|---|---|
| `matter/bridge/state` | out | yes | `{"state":"online"}` or `{"state":"offline"}` (also the last will) |
| `matter/bridge/info` | out | yes | the bridge and its fabric |
| `matter/bridge/devices` | out | yes | every node and its interview |
| `matter/bridge/event` | out | no | commissioning steps, node updates, removals, availability |
| `matter/bridge/request/<name>` | in | | a request |
| `matter/bridge/response/<name>` | out | no | its answer |
| `matter/<device>/availability` | out | yes | `online` while its subscription holds, else `offline` |
| `matter/<device>/<endpoint>/<cluster>/<attribute>` | out | yes | the attribute's value |
| `matter/<device>/<endpoint>/<cluster>/<attribute>/set` | in | | write it |
| `matter/<device>/<endpoint>/<cluster>/command/<command>` | in | | invoke a command |
| `matter/<device>/<endpoint>/<cluster>/command/<command>/result` | out | no | the command's response, or its error |
| `matter/<device>/<endpoint>/<cluster>/event/<event>` | out | no | an event |
| `matter/<device>/<endpoint>/on_off/set` | in | | `ON`, `OFF` or `TOGGLE` |
| `matter/<device>/<endpoint>/level_control/set` | in | | a level, 0 to 254 |

## Bridge topics

### `bridge/info`

```json
{ "version": "0.1.0", "fabric_id": "...", "compressed_fabric_id": "...", "node_id": "...", "vendor_id": 65521 }
```

`fabric_id`, `compressed_fabric_id` and the controller's own `node_id` are
16 upper-case hex digits.

### `bridge/devices`

Every node with its interview: node id, name, vendor and product (from Basic
Information), and its endpoints with their device types and the clusters
each serves (from Descriptor). The same shape is in the `commission`,
`interview` and `rename` responses.

### `bridge/event`

| `type` | Fields |
|---|---|
| `commissioning` | `node`, `step`: one per step of a commissioning |
| `node_updated` | `node`: commissioned, interviewed or renamed |
| `node_removed` | `node` |
| `availability` | `node`, `online`: its subscription started or lapsed |

## Values

Attribute values are JSON:

| Matter type | JSON |
|---|---|
| integers, enumerations, bitmaps | numbers (an enumeration can be written by name) |
| booleans | `true`, `false` |
| character strings | strings |
| octet strings | hex strings |
| structures | objects keyed by field name |
| lists | arrays |
| null | `null` |

```text
matter/lamp/1/on_off/on_off false
matter/lamp/1/level_control/current_level 128
matter/sensor/1/temperature_measurement/measured_value 2150
```

Values are as the device reports them: `measured_value` 2150 is 21.50 °C.

## Writing and invoking

Write an attribute with a JSON value:

```sh
mosquitto_pub -t matter/lamp/1/level_control/on_level/set -m 200
```

Invoke a command with its fields as a JSON object (the data model's field
names), or nothing for a command without fields:

```sh
mosquitto_pub -t matter/lamp/1/on_off/command/toggle -m ''
mosquitto_pub -t matter/lamp/1/level_control/command/move_to_level -m '{"level": 128, "transitionTime": 10}'
```

The command's response fields, or its error, come on
`.../command/<command>/result`. A required field that may be null can be
left out.

For lights and switches, `on_off/set` takes `ON`, `OFF` or `TOGGLE`, and
`level_control/set` a level from 0 to 254.

## Events

```text
matter/door/1/boolean_state/event/state_change {"number": 3, "data": {"stateValue": true}}
```

`number` is the device's event number. When a subscription is renewed,
it asks for events from the last number seen.

## Requests and responses

Publish a request to `matter/bridge/request/<name>`; the answer comes on
`matter/bridge/response/<name>`:

```json
{ "status": "ok", "data": { ... }, "transaction": "1" }
{ "status": "error", "error": "the name `lamp` is taken", "transaction": "1" }
```

Every request takes an optional `transaction`, copied into the response.

| Request | Fields | Data |
|---|---|---|
| `commission` | `code`, optional `name`, `address`, `timeout`; for Bluetooth: `wifi` (`ssid`, `password`), `thread_dataset`, `thread_dataset_url` | the node |
| `open_commissioning_window` | `device`, optional `timeout` (180 to 900 s, default 300) | `node`, `manual_code`, `qr_code`, `discriminator`, `timeout` |
| `read` | `device`, optional `endpoint`, `cluster`, `attribute` (any left out: all of them) | a list of `endpoint`, `cluster`, `attribute`, `value`, `status` |
| `write` | `device`, `endpoint`, `cluster`, `attribute`, `value`, optional `timed` (ms) | `null` |
| `invoke` | `device`, `endpoint`, `cluster`, `command`, optional `fields`, `timed` (ms) | the response's fields |
| `interview` | `device` | the node |
| `rename` | `device`, `to` | the node |
| `remove` | `device`, optional `force` | `node` |
| `discover` | optional `timeout` (seconds, default 5) | the commissionable devices seen |
| `devices` | | every node |

`timed` sends the write or command as a timed interaction, which some
clusters require (door locks, for example).

## Home Assistant topics

With `homeassistant.enabled` (the default), the bridge also publishes
discovery messages under `homeassistant/`. See [Home Assistant](home-assistant.md).
