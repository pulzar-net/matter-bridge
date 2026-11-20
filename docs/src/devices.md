# Devices

Matter devices describe themselves: the Descriptor cluster lists each
endpoint's device types and clusters, and the bridge's data model knows
every standard cluster. So there is no list of supported devices to keep;
any device that can be commissioned works through what it serves.

> **Tested so far:** connectedhomeip's `chip-all-clusters-app`, on every
> commit in CI. Commercial devices have not been tested yet. Reports are
> very welcome: open a
> [device report](https://github.com/pulzar-net/matter-bridge/issues/new?template=device.yml).

## What the bridge can reach

| Device | How | Tested |
|---|---|---|
| Ethernet devices | commission over IP | reference device in CI |
| Wi-Fi devices already on your Wi-Fi (added by another ecosystem) | commission over IP with a code from that ecosystem | expected |
| Thread devices already on a Thread network (added by another ecosystem) | commission over IP through the network's border router | expected |
| New Wi-Fi devices | commission over Bluetooth LE, with Wi-Fi credentials | simulated controllers only |
| New Thread devices | commission over Bluetooth LE, with the Thread dataset | not yet with a real Thread device |
| Sleepy (ICD) devices | not supported | planned |

Thread devices need a Thread border router on the network (an Apple TV or
HomePod, a Google Nest hub, a Home Assistant SkyConnect or ZBT-1 with
OpenThread Border Router, or another), and the bridge's host must be on the
same IPv6 network as the border router.

## Data model

The bridge's data model is generated at build time from the cluster
definitions the CSA publishes (the IDL rs-matter ships), so cluster,
attribute, command and event names are the specification's. Vendor clusters
work too, by number, with values decoded from their self-describing TLV.

## Limits

- **More than 64 live nodes** needs a change in rs-matter's session limit;
  see [Design](design.md#upstream-work-rs-matter).
- **Device attestation** is not verified yet; see the
  [security model](security.md).
