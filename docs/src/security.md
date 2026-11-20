# Security model

This page describes what matter-bridge protects, what it relies on you for,
and what it cannot do yet. To report a vulnerability, see
[SECURITY.md](https://github.com/pulzar-net/matter-bridge/blob/main/SECURITY.md):
privately, never in a public issue. A problem in rs-matter itself is
reported to rs-matter's own security process; we will help coordinate.

## What is at stake

- **The fabric's CA** (root and intermediate keys) and **the controller's
  operational key**, in the data directory. Whoever has them can add nodes
  to the fabric and control every device on it.
- **Control of devices.** Whoever can publish to `matter/#` can control
  them, commission new ones, and open commissioning windows that hand a
  device to another controller.
- **Network credentials** given to devices during Bluetooth commissioning
  (not in 0.1.0).

## Threats and defenses

| Threat | Defense |
|---|---|
| Eavesdropping or tampering on the network | Matter's own security: every operational message is encrypted and authenticated in a CASE session; commissioning starts with PASE, authenticated by the device's passcode. |
| Malformed or hostile messages from a device or the network | Messages are decoded by rs-matter, and values are converted through the data model's types before they reach MQTT. The bridge's own decoders (JSON to TLV, onboarding codes, Thread datasets) are fuzzed. No `unsafe` code in this repository. |
| Malformed requests over MQTT | Values are encoded only through the data model's types; a request that does not fit is answered with an error. File and device names are checked. |
| Keys leaking | The CA and the controller's key stay in the data directory, written atomically and readable by their owner only; they are never published or logged. Wi-Fi passphrases and Thread datasets are never stored or logged. |
| Losing the fabric | The data directory is all of it: back it up. |

## What you are responsible for

- **The MQTT broker.** Require authentication on it, use its access control
  so only the bridge and trusted clients can publish to `matter/#`, and keep
  it off untrusted networks.
- **The data directory.** Back it up and protect the backups like a
  password.
- **Which devices you commission** (see below).

## Current limits

- **Device attestation is not verified yet.** A Matter commissioner should
  check that a device's attestation certificate chains to a Product
  Attestation Authority the CSA lists, and the device's certification
  declaration, so that a counterfeit or tampered device is refused. That is
  not implemented yet; `allow_unverified_attestation` must stay on, and the
  bridge logs a warning for every device it commissions. **Only commission
  devices you trust.** This is the first item of the
  [upstream work](design.md#upstream-work-rs-matter).
- **Test vendor id.** The bridge administers devices as vendor id `0xFFF1`,
  the CSA's test vendor id for uncertified controllers.
- **No TLS to the MQTT broker yet.** Run the broker on the same host or a
  trusted network, or put a TLS tunnel in between.
- **Not certified** by the CSA, and not yet through a third-party security
  audit.
