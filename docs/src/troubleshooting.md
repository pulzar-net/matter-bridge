# Troubleshooting and FAQ

Start with the log: `RUST_LOG=debug` shows each step of commissioning and
every report.

## Commissioning

**The device is not found.** The bridge looks for it by DNS-SD on the local
network:

- Is the device in pairing mode? A device already in use needs a pairing
  code from its current app; a new device is usually in pairing mode for a
  limited time after power-on.
- Is the device on IP at all? A new Wi-Fi or Thread device is not: it needs
  Bluetooth commissioning.
- Is the bridge on the same network and VLAN, with IPv6? Set
  `matter.interface` if the host has several interfaces. `discover` lists
  what the bridge sees.
- Is mDNS blocked? UDP 5353 must be open, and some Wi-Fi access points
  filter multicast.
- If you know the device's address, pass it as `address` to skip discovery.

**Commissioning fails at PASE.** The passcode is wrong (a mistyped manual
code), or another controller is commissioning the device at the same time.

**Commissioning fails at CASE, after the certificates.** The device could
not be reached on its operational address: usually an IPv6 or mDNS problem
between the bridge and the device (or its Thread border router).

**`the name ... is taken`.** Choose another name, or remove the old device.

## After commissioning

**A device shows `offline` on `availability`.** Its subscription lapsed: the
device is unreachable, powered off, or rebooting. The bridge renews the
subscription on its own, backing off to every 5 minutes while the device is
away.

**Values are missing.** Every attribute arrives with the first subscription
report. Check `bridge/devices` for what the device serves, and `read` an
attribute directly.

**A write or command is refused by a door lock or similar.** Some clusters
require a timed interaction: add `timed` (for example `"timed": 1000`) to
the request.

## FAQ

**Can I use the bridge and Home Assistant's Matter integration (or Apple
Home, or Google Home) together?** Yes, through multi-admin: the device joins
several fabrics, one per controller. See
[Commissioning](commissioning.md#devices-already-in-another-ecosystem).

**Why does the log warn about attestation?** The bridge cannot verify device
attestation yet, so it cannot tell a certified device from a counterfeit
one. Only commission devices you trust. See the [security model](security.md).

**Does it expose Zigbee or other devices to Apple Home or Google Home (a
Matter bridge in the other direction)?** Not yet; it is designed, see
[Design](design.md).

**Where do I ask a question?** In
[Discussions](https://github.com/pulzar-net/matter-bridge/discussions).
Bugs go in [issues](https://github.com/pulzar-net/matter-bridge/issues). For security
problems, see [SECURITY.md](https://github.com/pulzar-net/matter-bridge/blob/main/SECURITY.md).
