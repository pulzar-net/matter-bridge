# Architecture

## Who does what

Matter is an application layer over IPv6. A controller does not drive a
radio: it talks to devices over UDP on the local network (Thread devices
through a border router), and only uses Bluetooth LE to commission new
devices. matter-bridge is a controller, built on
[rs-matter](https://github.com/project-chip/rs-matter):

- **rs-matter** provides the protocol: the transport with its reliability
  layer, PASE and CASE sessions, the interaction model client and server,
  mDNS, the commissioner, and BTP for Bluetooth.
- **matter-controller** is the controller on top: onboarding, discovery,
  the fabric and its CA, commissioning flows, the node store,
  subscriptions, and JSON in and out.
- **matter-bridge** maps it to MQTT and Home Assistant.

## Layers

```text
 MQTT clients (Home Assistant, Node-RED, scripts)
        │
 matter-bridge ───── topics, requests, Home Assistant discovery
        │  Controller handle (Send + Clone, async)   ▲ Event (channel)
        ▼                                            │
 matter-controller ─ a thread of its own, running rs-matter:
        │            onboarding, discovery, commissioning, CA and identity,
        │            node store, read/write/invoke, subscriptions
        ├── matter-model   data model tables, JSON ⇄ TLV
        ▼
 rs-matter ───────── UDP/IPv6 transport, PASE, CASE, interaction model, mDNS, BTP
        │
 Matter devices (Ethernet, Wi-Fi, Thread through a border router)
```

The [design page](design.md) goes deeper: why the controller runs on its own
thread, how subscriptions are renewed, the CA trait, and what is planned.

## The life of a report

1. A light turns on. It sends a ReportData message on the subscription the
   bridge holds, over a CASE session.
2. rs-matter decrypts it and hands the report to the controller's report
   handler.
3. The controller decodes each attribute with matter-model into JSON
   (`true` for On/Off's `on_off`) and emits an `Event::Attribute`.
4. The bridge publishes it, retained, on `matter/<device>/1/on_off/on_off`.

Commands go the other way: `matter/<device>/1/on_off/command/toggle` becomes
an invoke, encoded by matter-model from the command's definition, sent over
the node's session; the response comes back on `.../result`, and the effect
through the subscription.

## Design rules

- **No `unsafe` code** in this repository, forbidden by the workspace lints.
- **Check input against the data model.** Values from MQTT are encoded only
  through the data model's types, and values from devices are converted to
  JSON through it; anything that does not fit is reported as an error.
- **Bounded resources.** CI checks that the controller stays under 16 MB
  resident with 100 nodes.
- **Keys stay put.** The fabric's CA and the controller's key never leave
  the data directory; Wi-Fi passphrases and Thread datasets are used during
  commissioning and never stored.
- **Small and static.** Release binaries are statically linked against musl
  and built for size.
