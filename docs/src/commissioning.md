# Commissioning

Commissioning adds a device to the bridge's fabric. You need the device's
**pairing code** and a way to reach the device.

## Pairing codes

A Matter device has a QR code (a text payload starting with `MT:`) and an
11-digit manual code, usually printed under the QR code or in the box. The
bridge takes either, with or without dashes. Ask what a code holds with:

```sh
matter-bridge parse MT:...
matter-bridge parse 3497-011-2332
```

It prints the discriminator, vendor and product ids and how the device can
be found, never the passcode.

A device can be commissioned only while it is in **pairing mode**: a new
device enters it when powered on; a device already in use opens it from
its current ecosystem (see below).

## Over IP

For devices already on the network (Ethernet devices, Wi-Fi devices already
on your Wi-Fi, Thread devices already on your Thread network):

```sh
mosquitto_pub -t matter/bridge/request/commission -m '{"code": "3497-011-2332", "name": "lamp"}'
```

The bridge looks for the device among commissionable nodes on the network
by DNS-SD for up to `matter.discovery_timeout` seconds (or `timeout` in the
request), then takes it through the steps of commissioning: PASE with the
passcode, the fail-safe, the device's certificate signing request, its
operational certificate, CASE, and CommissioningComplete. It then
interviews the device, subscribes to it, and publishes everything it has.
Each step is a `commissioning` event on `matter/bridge/event`; the response
is the node.

| Field | |
|---|---|
| `code` | the QR or manual code (required) |
| `name` | the name used in topics (`matter/<name>/...`), `node_<node id>` if not given; it must not be empty or `bridge`, or contain `/`, `#` or `+`. Requests also accept the node id |
| `address` | `[ip]:port` to skip discovery, for example `"[fe80::1%eth0]:5540"` |
| `timeout` | seconds to look for the device |

`discover` lists the commissionable devices the bridge can see, which helps
when a device is not found.

## Devices already in another ecosystem

Matter devices can belong to several fabrics. To add a device that Apple
Home, Google Home, Home Assistant or a manufacturer's app already controls,
ask that app for a pairing code ("Turn on pairing mode", "Share with other
apps" or "Linked Matter apps & services"), and commission with that code
over IP as above. The device stays in the other ecosystem too.

The other way round, `open_commissioning_window` gives a device the bridge
controls to another ecosystem:

```sh
mosquitto_pub -t matter/bridge/request/open_commissioning_window -m '{"device": "lamp", "timeout": 300}'
```

The response holds `manual_code` and `qr_code`, valid for `timeout`
seconds (180 to 900, default 300). Enter either in the other app.

## Over Bluetooth LE

> Not tested yet: Bluetooth commissioning is built, but its end-to-end test
> on simulated controllers does not pass yet, and it has not been tried on
> real hardware. Commissioning over IP is the tested path. Handing a Thread device the
> network's dataset is built, but untested with a real Thread device.

A new Wi-Fi or Thread device is not on any IP network yet: it advertises
over Bluetooth LE, and commissioning gives it the network's credentials. Set
a Bluetooth controller (`matter.bluetooth = "hci0"`, which BlueZ must not
hold; see bluetooth-bridge's docs on the HCI user channel), and give the
network in the request:

```sh
# A Wi-Fi device:
mosquitto_pub -t matter/bridge/request/commission \
    -m '{"code": "MT:...", "name": "plug", "wifi": {"ssid": "home", "password": "..."}}'

# A Thread device, with the Thread network's active dataset in hex:
mosquitto_pub -t matter/bridge/request/commission \
    -m '{"code": "MT:...", "name": "sensor", "thread_dataset": "0e08..."}'
```

For Thread, `thread_dataset_url` (in the request or in `[matter]`) reads the
dataset from an OpenThread border router's REST API at commissioning time
instead. Credentials are given to the device inside the fail-safe and are
never stored or logged.

## Removing a device

```sh
mosquitto_pub -t matter/bridge/request/remove -m '{"device": "lamp"}'
```

The bridge removes its fabric from the device and forgets it. With
`"force": true` it forgets a device it cannot reach. Other fabrics on the
device are not touched.
