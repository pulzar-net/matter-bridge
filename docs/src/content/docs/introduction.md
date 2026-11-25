---
title: "matter-bridge"
description: "matter-bridge is a Matter controller and a Matter to MQTT bridge, written in Rust on rs-matter."
---

matter-bridge is a Matter controller and a Matter to MQTT bridge, written in
Rust on [rs-matter](https://github.com/project-chip/rs-matter). It
commissions Matter devices onto a fabric of its own, subscribes to them, and
publishes every attribute change and event to MQTT, with Home Assistant
discovery; reads, writes and commands go the other way. It is a single static
binary with no runtime, small enough for routers and gateways, and its
controller is a library crate for anyone building Matter software in Rust.

This site is the user guide and the reference. The
[README](https://github.com/pulzar-net/matter-bridge#readme) is the
two-minute overview; the [API docs](https://oss.pulzr.net/matter-bridge/api/matter_host/index.html)
cover every type and function.

## Where to start

| You want to | Read |
|---|---|
| try it with a reference device | [Getting started](/matter-bridge/getting-started.html) |
| run it | [Installation](/matter-bridge/installation.html) |
| add devices | [Commissioning](/matter-bridge/commissioning.html) |
| connect it to Home Assistant | [Home Assistant](/matter-bridge/home-assistant.html) |
| script it or build on its topics | [MQTT API](/matter-bridge/mqtt.html) |
| change a setting | [Configuration](/matter-bridge/configuration.html) and the [reference](/matter-bridge/config-reference.html) |
| embed the controller in Rust | [Library overview](/matter-bridge/library.html) |
| understand how it works | [Architecture](/matter-bridge/architecture.html), [Design](/matter-bridge/design.html) |
| fix a problem | [Troubleshooting and FAQ](/matter-bridge/troubleshooting.html) |

## Status

matter-bridge is at 0.x. CI commissions and controls connectedhomeip's
reference device on every commit, but **it has not been tested with
commercial devices yet**, and **it does not verify device attestation yet**:
only commission devices you trust.

Matter is a trademark of the Connectivity Standards Alliance. This project
is not affiliated with or certified by the CSA.

## Terms used in these docs

- **Node**: a Matter device, identified on a fabric by a 64-bit node id. It
  has **endpoints**, each serving **clusters** (On/Off, Level Control,
  Temperature Measurement) made of **attributes**, **commands** and
  **events**.
- **Fabric**: a set of nodes that trust one root certificate authority. The
  bridge creates its own fabric, with its own CA, and adds devices to it.
  A device can belong to several fabrics at once (**multi-admin**): Apple
  Home, Google Home and the bridge can all control it.
- **Commissioning**: adding a device to a fabric, using its **pairing code**
  (the QR code `MT:...` or the 11-digit manual code).
- **Device attestation**: the check that a device is a certified product, by
  its attestation certificate and the CSA's certification declaration.
- **Thread**: a low-power IPv6 mesh network some Matter devices use, reached
  through a **border router**.
