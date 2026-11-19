# matter-bridge

A Matter controller in Rust, and a bridge from Matter to MQTT with Home
Assistant discovery. Built on [rs-matter](https://github.com/project-chip/rs-matter),
the Connectivity Standards Alliance's Rust implementation of Matter.
Maintained by [Pulzar](https://pulzr.net). MIT licensed.

It commissions Matter devices onto a fabric of its own, keeps a subscription
to each, and publishes every attribute change and event on MQTT; commands and
writes go back the same way. Devices describe themselves (the Descriptor
cluster), and the data model comes from the CSA's published definitions, so
there is no per-device table to maintain.

## What it does

- **Onboarding**: QR codes (`MT:...`) and 11- or 21-digit manual pairing codes.
- **Commissioning over IP**: the device is found by DNS-SD among commissionable
  nodes (or reached at an address you give), then taken through PASE, the
  fail-safe, CSR, AddNOC, CASE and CommissioningComplete.
- **A fabric of its own**, rooted in a CA behind a trait: a file CA for a
  standalone install, or your own (an HSM, a platform service) when several
  controllers share a fabric.
- **Multi-admin**: open a commissioning window on a device so Apple Home,
  Google Home or Home Assistant can add it too.
- **The interaction model**: read, write and invoke on any cluster, in JSON;
  a subscription to everything each device has, renewed when it lapses;
  events.
- **A node store** written when the network changes, not per message.

Not yet: Bluetooth commissioning (BTP over GATT, with the GATT central from
the sibling bluetooth-bridge project), handing
Wi-Fi or Thread credentials to a device being commissioned, verifying device
attestation (see [Security](#security)), OTA, groups, sleepy (ICD) devices,
and exposing other devices *as* a Matter bridge. The design for each is in
[docs/design.md](docs/design.md).

## Crates

| Crate | Does |
|---|---|
| `matter-model` | The Matter data model as static tables generated at build time from the IDL rs-matter ships (the CSA's `controller-clusters.matter`), and JSON to TLV and back, guided by them |
| `matter-controller` | The controller: onboarding, discovery, commissioning, the CA trait and a file CA, the controller's identity, the node store, read/write/invoke, subscriptions with renewal, events, commissioning windows |
| `matter-bridge` | The standalone binary: MQTT with Home Assistant discovery and a plain JSON topic scheme |

## Running the bridge

```sh
cp config.example.toml /etc/matter-bridge/config.toml   # every key is optional
matter-bridge --config /etc/matter-bridge/config.toml
```

Commission a device that is on your network and in pairing mode:

```sh
mosquitto_pub -t matter/bridge/request/commission -m '{"code": "3497-011-2332", "name": "lamp"}'
mosquitto_sub -t 'matter/#' -v
```

`matter-bridge parse CODE` says what a QR or manual code holds.

### Topics

Under the base topic (`matter` by default). Clusters, attributes, commands and
events are named in `snake_case` from the data model (`on_off`,
`current_level`, `move_to_level`); a vendor cluster or attribute the data
model does not know is its number in hex (`0xfff1fc01`).

| Topic | Retained | |
|---|---|---|
| `bridge/state` | yes | `{"state":"online"}` or `"offline"` (the last will) |
| `bridge/info` | yes | version, fabric id, compressed fabric id, the controller's node id |
| `bridge/devices` | yes | every node and its interview: vendor, product, endpoints, device types, clusters |
| `bridge/event` | no | `commissioning` (each step), `node_updated`, `node_removed`, `availability` |
| `bridge/request/<name>` | no | see below |
| `bridge/response/<name>` | no | `{"status":"ok","data":...}` or `{"status":"error","error":...}`, with the request's `transaction` echoed |
| `<device>/availability` | yes | `online` while its subscription holds, else `offline` |
| `<device>/<endpoint>/<cluster>/<attribute>` | yes | the attribute's value, JSON |
| `<device>/<endpoint>/<cluster>/<attribute>/set` | no | write the attribute: a JSON value |
| `<device>/<endpoint>/<cluster>/command/<command>` | no | invoke: the fields as a JSON object, or nothing |
| `<device>/<endpoint>/<cluster>/command/<command>/result` | no | the response's fields, or the error |
| `<device>/<endpoint>/<cluster>/event/<event>` | no | `{"number": N, "data": {...}}` |
| `<device>/<endpoint>/on_off/set` | no | `ON`, `OFF` or `TOGGLE` |
| `<device>/<endpoint>/level_control/set` | no | a level, 0 to 254 |

Requests (`bridge/request/<name>`, a JSON object; every one takes an optional
`transaction`):

| Request | Body |
|---|---|
| `commission` | `code` (QR or manual), optional `name`, `address` (`[ip]:port`, to skip discovery), `timeout` (seconds) |
| `open_commissioning_window` | `device`, optional `timeout` (180 to 900, default 300): returns a manual code and a QR code for another ecosystem |
| `read` | `device`, optional `endpoint`, `cluster`, `attribute` (left out: every one) |
| `write` | `device`, `endpoint`, `cluster`, `attribute`, `value`, optional `timed` (ms) |
| `invoke` | `device`, `endpoint`, `cluster`, `command`, optional `fields`, `timed` (ms) |
| `interview` | `device` |
| `rename` | `device`, `to` |
| `remove` | `device`, optional `force` (forget it even if it cannot be reached) |
| `discover` | optional `timeout` (seconds): commissionable nodes on the network |
| `devices` | |

Values are JSON: numbers for integers, enumerations and bitmaps (an
enumeration can be written by name), hex strings for octet strings, objects
keyed by field name for structures. A `fields` object for a command uses the
data model's field names (`{"level": 128, "transitionTime": 10}`); a required
field that may be null can be left out.

### Home Assistant

Every device gets discovery under `homeassistant/` from what its endpoints
serve: lights (On/Off, with brightness from Level Control), switches and
plugs, temperature, humidity, pressure and illuminance sensors, occupancy,
contact and leak sensors, and battery level. Everything else is on the plain
topics above.

## Security

- The data directory holds the fabric's CA keys and the controller's
  operational key, readable by its owner only. Anyone with it can control
  every device on the fabric.
- **Device attestation is not verified yet.** A Matter commissioner should
  check that a device's attestation certificate chains to a Product
  Attestation Authority the CSA lists, and its certification declaration.
  rs-matter does not implement that yet; it is the first item of the upstream
  work in [docs/design.md](docs/design.md). Until then
  `allow_unverified_attestation` must stay on, and the bridge logs a warning
  for every device it commissions: only commission devices you trust.
- The bridge administers devices as vendor id `0xFFF1`, the CSA's test vendor
  id for uncertified controllers.

## Development

```sh
cargo test --workspace
cargo +nightly fuzz run tlv_to_json   # see fuzz/ for every target
```

CI commissions and controls connectedhomeip's `chip-all-clusters-app` (from
the CSA's `chip-cert-bins` image) over IP on every commit, checks the memory
budget with 100 nodes, fuzzes every parser, and builds static binaries for
aarch64 and mipsel. The end-to-end test runs against any commissionable
device on your network:

```sh
MATTER_E2E_CODE=34970112332 cargo test -p matter-bridge --test e2e -- --ignored --nocapture
```

## License

MIT. rs-matter is Apache-2.0.
