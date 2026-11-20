# matter-bridge

[![CI](https://github.com/pulzar-net/matter-bridge/actions/workflows/ci.yml/badge.svg)](https://github.com/pulzar-net/matter-bridge/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/pulzar-net/matter-bridge)](https://github.com/pulzar-net/matter-bridge/releases/latest)
[![Docs](https://img.shields.io/badge/docs-pulzar--net.github.io-blue)](https://pulzar-net.github.io/matter-bridge/)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue)](LICENSE)
[![MSRV](https://img.shields.io/badge/MSRV-1.92-blue)](Cargo.toml)

A Matter controller and a Matter to MQTT bridge, written in Rust on
[rs-matter](https://github.com/project-chip/rs-matter), the Connectivity
Standards Alliance's Rust implementation of Matter. It commissions Matter
devices onto a fabric of its own, keeps a subscription to each, and
publishes every attribute change and event to MQTT with Home Assistant
discovery; commands and writes go back the same way. It is one static binary
with no runtime that stays under 16 MB of memory with 100 nodes, so it fits
on routers and small gateways as well as on a Raspberry Pi or a server. The
controller is also a library crate for anyone building Matter software in
Rust.

It exists because the established Matter controllers need a Node.js or
Python runtime or a phone ecosystem, and there was no Matter controller in
Rust that could run headless on small hardware.

**[Documentation](https://pulzar-net.github.io/matter-bridge/)** |
**[Quick start](#quick-start)** |
**[Devices](#supported-devices)** |
**[MQTT API](https://pulzar-net.github.io/matter-bridge/mqtt.html)** |
**[Library](#use-it-as-a-library)** |
**[Changelog](CHANGELOG.md)**

## Status

matter-bridge is **0.x and young**. In CI it commissions and controls
connectedhomeip's reference device (`chip-all-clusters-app`) on every
commit, but **it has not yet been tested with commercial devices**, and
**it does not verify device attestation yet** (see [Security](#security)).
Expect rough edges and breaking changes between 0.x releases.

| Feature | State |
|---|---|
| Onboarding codes: QR (`MT:...`) and 11- or 21-digit manual codes | done in 0.1.0 |
| Commissioning over IP, with DNS-SD discovery | done in 0.1.0 |
| A fabric of its own, with a pluggable CA (a file CA included) | done in 0.1.0 |
| Read, write, invoke and events on any cluster, in JSON | done in 0.1.0 |
| A subscription to every device, renewed when it lapses | done in 0.1.0 |
| Multi-admin: open a commissioning window for another ecosystem | done in 0.1.0 |
| Home Assistant MQTT discovery for lights, switches, sensors | done in 0.1.0 |
| Commissioning over Bluetooth LE | on `main`, next release |
| Giving Wi-Fi and Thread credentials to a device being commissioned | on `main`, next release |
| Device attestation verification | in progress (blocks production use) |
| More than 64 live nodes; a CA whose key is held elsewhere | in progress |
| OTA, groups, sleepy (ICD) devices | planned |
| Exposing other devices as a Matter bridge | planned |

## Supported devices

Matter devices describe themselves, so there is no per-device support
list: any Matter device that can be commissioned over the network the
bridge is on works through the clusters it serves.

| Device | How to add it | State |
|---|---|---|
| connectedhomeip's `chip-all-clusters-app` (reference device) | commission over IP | tested in CI on every commit |
| Matter devices on Ethernet, or already on your Wi-Fi or Thread network | commission over IP with their pairing code | expected |
| Devices already in Apple Home, Google Home, Home Assistant or another ecosystem | ask that app for a pairing code ("share" or "turn on pairing mode"), then commission over IP | expected |
| New Wi-Fi or Thread devices that join over Bluetooth | commission over Bluetooth LE | next release |

Home Assistant discovery covers lights (On/Off, with brightness from Level
Control), switches and plugs, temperature, humidity, pressure and
illuminance sensors, occupancy, contact and leak sensors, and battery
level. Everything else is on the plain MQTT topics. The
[devices page](https://pulzar-net.github.io/matter-bridge/devices.html)
has the details.

## Quick start

From nothing to a device in Home Assistant, on 64-bit Linux (x86_64 or
aarch64) on the same network as the device. You need an MQTT broker (for
example Mosquitto) that Home Assistant's MQTT integration already uses, and
IPv6 on the network interface (it is on by default on Linux).

**1. Download the release and check its checksum.**

<!-- quickstart:download -->
```sh
VERSION=0.1.0
ARCH=$(uname -m)   # x86_64 or aarch64
BASE=https://github.com/pulzar-net/matter-bridge/releases/download/v$VERSION
curl -fsSLO "$BASE/matter-bridge-$ARCH-linux-musl"
curl -fsSLO "$BASE/SHA256SUMS"
sha256sum --check --ignore-missing SHA256SUMS
sudo install -m 0755 "matter-bridge-$ARCH-linux-musl" /usr/local/bin/matter-bridge
matter-bridge --version
```

**2. Write a minimal configuration** to `/etc/matter-bridge/config.toml`.
Every key has a default; this one only names the broker.

<!-- quickstart:config -->
```toml
[mqtt]
host = "localhost"   # your broker
```

**3. Run it.** On first start it creates a new fabric, with its own CA, in
`/var/lib/matter-bridge`.

<!-- quickstart:run -->
```sh
sudo matter-bridge --config /etc/matter-bridge/config.toml
```

**4. Commission a device.** Put it in pairing mode and use its pairing
code: the QR code (`MT:...`) or the 11-digit code printed under it, or a code
another ecosystem's app gave you. This example uses the code of the CSA's
test device; use yours.

<!-- quickstart:pair -->
```sh
mosquitto_pub -h localhost -t matter/bridge/request/commission -m '{"code": "3497-011-2332", "name": "lamp"}'
```

The bridge finds the device on the network, commissions it, and subscribes
to it. Within a minute it appears in Home Assistant under
**Settings > Devices & services > MQTT**. To watch everything:

```sh
mosquitto_sub -h localhost -t 'matter/#' -v
```

The [getting started guide](https://pulzar-net.github.io/matter-bridge/getting-started.html)
runs the same steps against the reference device in a container. To run the
bridge as a service, see [installation](#installation).

## Installation

- **Release binaries.** Each [release](https://github.com/pulzar-net/matter-bridge/releases)
  has static Linux binaries for `x86_64`, `aarch64` (Raspberry Pi 3 and newer
  with a 64-bit OS) and `mipsel` (MT7621 and MT7628 routers), with a
  `SHA256SUMS` file. They have no runtime dependencies.
- **From source**, with Rust 1.92 or newer:
  `cargo install --locked --git https://github.com/pulzar-net/matter-bridge matter-bridge`
- **systemd**: a hardened unit is in [`dist/systemd/`](dist/systemd/matter-bridge.service).
- **OpenWrt**: a procd init script is in [`dist/openwrt/`](dist/openwrt/matter-bridge.init).

The [installation guide](https://pulzar-net.github.io/matter-bridge/installation.html)
covers each one step by step.

## Configuration

The bridge reads one TOML file, `/etc/matter-bridge/config.toml` by default
(`--config FILE` to change it); without the file, every default applies.

| Table | What it sets |
|---|---|
| `[mqtt]` | broker, credentials (password from a file), client id, base topic |
| `[homeassistant]` | discovery on or off, discovery prefix, Home Assistant's status topic |
| `[matter]` | UDP port, mDNS interface, fabric id, vendor id, subscription renewal, discovery timeout; on `main`, the Bluetooth controller and a Thread dataset source |
| `[data]` | the data directory: the fabric's CA, the controller's identity, the node store |

Unknown keys are an error, so a typo stops the bridge at startup. Every key,
its type and default are in the
[configuration reference](https://pulzar-net.github.io/matter-bridge/config-reference.html),
which is generated from the code. [`config.example.toml`](config.example.toml)
shows them all. Log verbosity comes from `RUST_LOG` (default `info`).

## MQTT API

Everything lives under a base topic, `matter` by default. Clusters,
attributes, commands and events are named in `snake_case` from the Matter
data model (`on_off`, `current_level`, `move_to_level`); a vendor cluster or
attribute is its number in hex (`0xfff1fc01`).

| Topic | Direction | What |
|---|---|---|
| `matter/bridge/state` | out, retained | `{"state":"online"}` or `{"state":"offline"}` (also the last will) |
| `matter/bridge/info` | out, retained | version, fabric id, compressed fabric id, the controller's node id |
| `matter/bridge/devices` | out, retained | every node: vendor, product, endpoints, device types, clusters |
| `matter/bridge/event` | out | commissioning steps, node updates and removals, availability |
| `matter/bridge/request/<name>` | in | `commission`, `open_commissioning_window`, `read`, `write`, `invoke`, `interview`, `rename`, `remove`, `discover`, `devices` |
| `matter/bridge/response/<name>` | out | `{"status":"ok","data":...}` or `{"status":"error","error":"..."}`, with the request's `transaction` |
| `matter/<device>/availability` | out, retained | `online` while its subscription holds, else `offline` |
| `matter/<device>/<endpoint>/<cluster>/<attribute>` | out, retained | the attribute's value, JSON |
| `matter/<device>/<endpoint>/<cluster>/<attribute>/set` | in | write the attribute |
| `matter/<device>/<endpoint>/<cluster>/command/<command>` | in | invoke a command; the response comes on `.../result` |
| `matter/<device>/<endpoint>/<cluster>/event/<event>` | out | `{"number": N, "data": {...}}` |
| `matter/<device>/<endpoint>/on_off/set`, `.../level_control/set` | in | `ON`, `OFF`, `TOGGLE`, or a level |

`<device>` is the name given at commissioning or with `rename`. Keys never
appear on MQTT. The [MQTT reference](https://pulzar-net.github.io/matter-bridge/mqtt.html)
has every request and payload with examples.

## Use it as a library

| Crate | What it is |
|---|---|
| [`matter-controller`](crates/matter-controller) | the controller: onboarding, discovery, commissioning, the CA trait and a file CA, the controller's identity, the node store, read, write, invoke, subscriptions with renewal, events, commissioning windows |
| [`matter-model`](crates/matter-model) | the Matter data model as tables generated from the CSA's published definitions, and JSON to TLV and back, guided by them |
| [`matter-bridge`](crates/matter-bridge) | the MQTT bridge, as a binary and a library |

The crates are not on crates.io yet (rs-matter is a git dependency pinned to
a commit); depend on them from git:

```toml
[dependencies]
matter-controller = { git = "https://github.com/pulzar-net/matter-bridge", tag = "v0.1.0" }
```

Onboarding codes are the first thing a controller reads. This example parses
the manual pairing code of the CSA's test device:

```rust
use matter_controller::Onboarding;

let code = Onboarding::parse("3497-011-2332")?;
assert_eq!(code.passcode, 20202021);
// A manual code does not say how to find the device, so every way is tried.
assert!(code.capabilities.is_none());
# Ok::<(), matter_controller::Error>(())
```

The [library guide](https://pulzar-net.github.io/matter-bridge/library/index.html)
shows how to start a controller, commission, read and subscribe, and the
[API docs](https://pulzar-net.github.io/matter-bridge/api/matter_controller/index.html)
cover every item.

## Architecture

```text
 Home Assistant, Node-RED, scripts
                │  MQTT (JSON, Home Assistant discovery)
┌───────────────┴───────────────────────────────────────┐
│ matter-bridge      topics, requests, discovery        │
├───────────────────────────────────────────────────────┤
│ matter-controller  onboarding, DNS-SD discovery,      │
│                    commissioning, fabric and CA,      │
│                    node store, interaction model,     │
│                    subscriptions                      │
├──────────────────────────┬────────────────────────────┤
│ matter-model             │ rs-matter                  │
│ data model, JSON ⇄ TLV   │ PASE, CASE, sessions, IM,  │
│                          │ the commissioner, BTP      │
└──────────────────────────┴─────────────┬──────────────┘
                                         │ UDP over IPv6, mDNS (Bluetooth LE to commission)
                         Matter devices on Ethernet, Wi-Fi or Thread
```

The [architecture page](https://pulzar-net.github.io/matter-bridge/architecture.html)
explains the layers, the fabric and its CA, and the design rules (no
`unsafe`, input checked against the data model, bounded memory).

## Compared with other projects

| | matter-bridge | [Matter Server](https://github.com/home-assistant-libs/python-matter-server) (Home Assistant) | [matter.js](https://github.com/project-chip/matter.js) |
|---|---|---|---|
| Language and runtime | Rust, one static binary | Python, on connectedhomeip's SDK | TypeScript, Node.js |
| Interface | MQTT and Home Assistant MQTT discovery | Home Assistant's Matter integration (WebSocket) | a library, and a controller and device toolkit |
| Commissioning | over IP; Bluetooth next release | Bluetooth (through the Home Assistant app) and IP | Bluetooth and IP |
| Small hardware | runs on 128 MB MIPS routers | needs a host for the SDK | needs a Node.js capable host |
| Maturity | 0.x | mature, very widely used | mature, widely used |

**Pick Home Assistant's Matter integration** if you use Home Assistant and
want Matter with its native UI, today. **Pick matter.js** for a full Matter
toolkit in TypeScript, including building devices. **Pick matter-bridge** if
you want Matter devices on MQTT for any consumer, a controller that runs
headless on small hardware, or a Matter controller to embed in Rust, and you
can accept 0.x maturity and, for now, unverified attestation.

## Roadmap

- **Next release:** commissioning over Bluetooth LE, and handing Wi-Fi and
  Thread credentials to devices being commissioned (done on `main`).
- **In progress:** device attestation verification, more than 64 live
  nodes, a CA whose key lives elsewhere (an HSM, a platform service).
- **Later:** OTA, groups, sleepy (ICD) devices, exposing other devices as a
  Matter bridge.

The [design page](https://pulzar-net.github.io/matter-bridge/design.html)
has the details. Questions go in [Discussions](https://github.com/pulzar-net/matter-bridge/discussions); bugs and requests in [issues](https://github.com/pulzar-net/matter-bridge/issues).

## Contributing

Contributions are welcome, from device reports to code. See
[CONTRIBUTING.md](CONTRIBUTING.md) for how to build, test, fuzz and send a
change, and [the code of conduct](CODE_OF_CONDUCT.md).

## Security

- The data directory holds the fabric's CA keys and the controller's
  operational key, readable by its owner only. Anyone with it can control
  every device on the fabric.
- **Device attestation is not verified yet.** A Matter commissioner should
  check that a device's attestation certificate chains to a Product
  Attestation Authority the CSA lists, and its certification declaration.
  That is not implemented yet, so `allow_unverified_attestation` must stay
  on, and the bridge logs a warning for every device it commissions: **only
  commission devices you trust.**
- The bridge administers devices as vendor id `0xFFF1`, the CSA's test vendor
  id for uncertified controllers.

Please report vulnerabilities privately through
[GitHub private vulnerability reporting](https://github.com/pulzar-net/matter-bridge/security/advisories/new),
never in a public issue. See [SECURITY.md](SECURITY.md) and the
[security model](https://pulzar-net.github.io/matter-bridge/security.html).

## License

MIT. See [LICENSE](LICENSE). rs-matter is Apache-2.0.

Matter is a trademark of the Connectivity Standards Alliance. This project
is not affiliated with or certified by the CSA.

## Maintained by Pulzar

matter-bridge is maintained by [Pulzar](https://pulzr.net), which uses it in
its own gateways. It is developed in the open, for anyone to use.
