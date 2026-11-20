---
title: "Getting started"
description: "This page runs matter-bridge end to end with connectedhomeip's reference device, chip-all-clusters-app, the device the CSA's certification tests use and the one CI commissions on every commit."
---

This page runs matter-bridge end to end with connectedhomeip's reference
device, `chip-all-clusters-app`, the device the CSA's certification tests
use and the one CI commissions on every commit. It serves nearly every
cluster, so everything the bridge does can be tried on it.

You need:

- a 64-bit ARM Linux machine (a Raspberry Pi 4 or 5 works) with Docker: the
  CSA's `chip-cert-bins` image is published for arm64 only;
- Rust 1.92 or newer ([rustup.rs](https://rustup.rs)), or a release binary;
- an MQTT broker and its command-line clients, for example Mosquitto
  (`apt install mosquitto mosquitto-clients`);
- optionally, Home Assistant with its MQTT integration on the same broker.

On another architecture, build `chip-all-clusters-app` from
[connectedhomeip](https://github.com/project-chip/connectedhomeip), or use
any Matter device on your network with its pairing code.

## 1. Start the reference device

```sh
docker run -d --name dut --network host \
    connectedhomeip/chip-cert-bins:6ff03a282176cb02156bfafb9988498da3c35ffb \
    /root/chip-all-clusters-app --KVS /tmp/dut.kvs
```

It advertises itself as commissionable on the host's network, with the
CSA's test pairing code: manual code `3497-011-2332`, passcode
`20202021`.

## 2. Configure and run the bridge

```sh
git clone https://github.com/pulzar-net/matter-bridge
cd matter-bridge
```

Write `quickstart.toml`:

```toml
[mqtt]
host = "localhost"

[data]
dir = "./data"
```

and run it:

```sh
cargo run -p matter-bridge -- --config quickstart.toml
```

On first start it creates a fabric with its own CA in `./data`.

## 3. Watch MQTT

```sh
mosquitto_sub -h localhost -t 'matter/#' -v
```

## 4. Commission the device

```sh
mosquitto_pub -h localhost -t matter/bridge/request/commission \
    -m '{"code": "3497-011-2332", "name": "lamp", "transaction": "1"}'
```

You will see a `commissioning` event on `matter/bridge/event` for each step,
then the response with the node, then every attribute of the device on its
own retained topic, for example:

```text
matter/lamp/availability online
matter/lamp/1/on_off/on_off false
```

The bridge logs a warning that the device's attestation was not verified:
see [Security model](/matter-bridge/security.html).

## 5. Control it

```sh
mosquitto_pub -h localhost -t matter/lamp/1/on_off/set -m ON
mosquitto_pub -h localhost -t matter/lamp/1/level_control/set -m 128
mosquitto_pub -h localhost -t matter/lamp/1/on_off/command/toggle -m ''
```

Each change comes back through the subscription on
`matter/lamp/1/on_off/on_off`.

## 6. Share it with another ecosystem

```sh
mosquitto_pub -h localhost -t matter/bridge/request/open_commissioning_window -m '{"device": "lamp"}'
```

The response holds a new manual code and QR code, valid for 300 seconds,
that Apple Home, Google Home or Home Assistant can use to add the same
device to their own fabric.

## Next steps

- [Install](/matter-bridge/installation.html) a release binary as a service.
- Read [Commissioning](/matter-bridge/commissioning.html) for real devices and the
  [MQTT API](/matter-bridge/mqtt.html) for every request.
