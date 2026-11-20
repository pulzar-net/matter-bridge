---
title: "Home Assistant"
description: "matter-bridge works with Home Assistant through its MQTT integration and MQTT discovery."
---

matter-bridge works with Home Assistant through its
[MQTT integration](https://www.home-assistant.io/integrations/mqtt/) and
MQTT discovery. It is an alternative to Home Assistant's own Matter
integration, not a companion: devices the bridge commissions are on the
bridge's fabric. A device can be on both, through
[multi-admin](/matter-bridge/commissioning.html#devices-already-in-another-ecosystem).

## Setup

1. Run an MQTT broker, such as the
   [Mosquitto add-on](https://github.com/home-assistant/addons/tree/master/mosquitto).
2. Add the MQTT integration in Home Assistant and point it at the broker.
3. Point matter-bridge at the same broker and [commission](/matter-bridge/commissioning.html)
   devices.

## What is created

Each Matter node becomes one Home Assistant device. Entities come from what
each endpoint serves and what its device type says it is:

| Endpoint | Entity |
|---|---|
| On/Off on a light device type | `light`, with brightness when it serves Level Control |
| On/Off on another device type (plugs, switches) | `switch` |
| Temperature, Relative Humidity, Pressure, Illuminance Measurement | `sensor` with the matching device class |
| Occupancy Sensing | `binary_sensor` (occupancy) |
| Boolean State on a contact sensor | `binary_sensor` (door) |
| Boolean State on a water leak detector | `binary_sensor` (moisture) |
| Boolean State otherwise | `binary_sensor` |
| Power Source battery level | `sensor` (battery) |

Everything else (other clusters, commands, events) is on the plain
[MQTT topics](/matter-bridge/mqtt.html).

An entity is available while both the bridge (`matter/bridge/state`) and
the device's subscription (`matter/<device>/availability`) are online.

Discovery topics have the form
`homeassistant/<component>/matter_<compressed fabric id>_<node id>/<endpoint>_<cluster>/config`,
for example `homeassistant/light/matter_00000000000000ab_0000000000000002/1_6/config`.

## Topics

| Setting | Default | What |
|---|---|---|
| `homeassistant.discovery_prefix` | `homeassistant` | where discovery messages go |
| `homeassistant.status_topic` | `homeassistant/status` | Home Assistant's birth message; discovery is sent again when it says `online` |
| `homeassistant.enabled` | `true` | set `false` to publish no discovery |
