# matter-controller

A Matter controller on [rs-matter](https://github.com/project-chip/rs-matter).

- **Onboarding**: QR codes and manual pairing codes (`Onboarding`).
- **Commissioning over IP**: the device is found by DNS-SD among
  commissionable nodes, or reached at an address given, then taken through
  PASE, the fail-safe, CSR, AddNOC, CASE and `CommissioningComplete` by
  rs-matter's commissioner.
- **A fabric with a pluggable CA** (`CertificateAuthority`); `FileCa`
  for a standalone controller.
- **Multi-admin**: `Controller::open_commissioning_window` lets a second
  ecosystem (Apple Home, Google Home, Home Assistant) commission the node too.
- **The interaction model**: read, write and invoke on any cluster, in JSON
  converted by `matter_model` from the CSA's published data model, and a
  subscription to every node that is re-established when it lapses, with
  attribute changes and events delivered as `Event`s.
- **A node store** (`NodeStore`) written when the network changes.

rs-matter's futures are not `Send`, so the controller runs on a thread of
its own; `Controller` is the `Send + Clone` handle to it.

Part of [matter-bridge](https://github.com/pulzar-net/matter-bridge), maintained by
[Pulzar](https://pulzr.net). See the [user guide](https://pulzar-net.github.io/matter-bridge/), the
[library guide](https://pulzar-net.github.io/matter-bridge/library/index.html) and the
[API documentation](https://pulzar-net.github.io/matter-bridge/api/matter_controller/index.html).

License: MIT.
