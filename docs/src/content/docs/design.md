---
title: "Design"
description: "How matter-bridge is put together, what it needs from rs-matter that rs-matter does not have yet, and how the next pieces fit."
---

How matter-bridge is put together, what it needs from rs-matter that rs-matter
does not have yet, and how the next pieces fit.

## Layers

```text
 MQTT  ◄──────────────►  matter-bridge (binary)
                          topics, requests, Home Assistant discovery
                               │  Controller handle (Send + Clone, async)
                               ▼
                          matter-host  ── one thread, its own runtime
                          ├ onboarding        rs-matter pairing parsers
                          ├ commissioning     rs-matter Commissioner, mDNS browse/resolve
                          ├ CA + identity     CertificateAuthority trait, FileCa
                          ├ node store        nodes.json, written on change
                          ├ IM client         read / write / invoke, JSON via matter-model
                          ├ subscriptions     one per node, renewed when silent
                          └ report sink       ReportDataHandler on the controller's own IM
                               │
                          rs-matter: transport (UDP, MRP), PASE, CASE, builtin mDNS,
                          interaction model client and server
                               │
                          matter-model: tables from the CSA IDL, TLV ⇄ JSON
```

- **One thread.** rs-matter's futures are not `Send`. The controller runs a
  current-thread tokio runtime with a `LocalSet` on a thread of its own; the
  `Controller` handle sends it jobs over a channel and receives `Event`s on a
  broadcast channel. The bridge (or any other user, such as a platform adapter)
  can then live on any runtime.
- **The controller is a Matter node.** It holds one fabric (its own), runs
  rs-matter's transport and built-in mDNS (advertising itself as an
  operational node, browsing and resolving others), and an interaction model
  that serves no clusters: it is there to take the `ReportData` of the
  subscriptions it holds, through `InteractionModel::new_with_reports`.
- **JSON at the edge.** Every request and report is converted by
  `matter-model`, whose tables are generated at build time from the IDL
  rs-matter ships (`rs_matter_codegen::parse_idl`). TLV is self-describing, so
  any value decodes; the tables add field names and tell the encoder signedness,
  string kind and field tags. A vendor cluster the IDL lacks is handled by
  number, untyped.
- **Store.** `nodes.json`: identity and interview (Basic Information,
  Descriptor) per node, replaced atomically when a node is commissioned,
  interviewed, renamed or removed. Attribute values are not stored; they
  return with the first subscription report.
- **Subscriptions.** One wildcard subscription (every attribute and event)
  per node, `keep_subscriptions`, max interval ceiling 60 s by default. A task
  per node watches the last report; past the agreed max interval plus 30 s it
  renews, with exponential backoff to 5 minutes while the node is away. Events
  resume from the last event number seen. Reports for a subscription we do not
  hold are refused with `InvalidSubscription`, so stale ones are torn down.

## The fabric and its CA

`CertificateAuthority` gives the fabric id, the root (RCAC) and intermediate
(ICAC) certificates, the IPK, node id allocation and NOC issuance. The
controller's own NOC is issued through it at first start and kept in
`controller.json`.

- `FileCa` (standalone): one owner-only file with a random fabric id, RCAC and
  ICAC keys and certificates, IPK and the next node id.
- A platform CA (for example one fabric per organization) holds the root
  off the gateway, issues each gateway controller's NOC and every device NOC,
  and allocates node ids so gateways never collide. Every gateway of the
  organization then reaches every device of it. To make that work the
  device's ACL should name a CASE Authenticated Tag (CAT) shared by the
  organization's controllers rather than one controller's node id; the
  commissioner passes `case_admin_subject`, which rs-matter takes from the
  fabric's node id today (upstream item 4).

## Upstream work (rs-matter)

What matter-bridge needs that rs-matter lacks. Each goes to rs-matter as its
own change; until it merges, matter-bridge either works around it (named in a
comment that links the upstream change) or depends on a branch with it.

1. **Device attestation verification** (security; blocks production use).
   `Commissioner` has a `DeviceAttestation` step that either skips
   (`allow_test_attestation`) or fails. Needed: `AttestationRequest` and
   `CertificateChainRequest`, the DAC → PAI → PAA chain (path validation,
   VID/PID rules), the attestation signature over the elements and the
   challenge, the certification declaration (CMS, signed by the CSA's keys),
   and a PAA trust store. rs-matter already has a DCL client
   (`rs-matter-dm/src/clusters/ota_prov/dcl.rs`) that a store fed from the
   DCL can build on; revocation later.
2. **An async NOC issuer for the commissioner.** `Commissioner::new` takes a
   `NocGenerator` (a key in the process). A CA elsewhere needs the commissioner
   to hand out the CSR and await a NOC: a `NocIssuer` trait with an async
   `issue`, `NocGenerator` its local implementation. Until then
   matter-host commissions only with a CA that exposes `signing_key`.
3. **Controller scale.** Sessions are sized at compile time and stop at 64
   (`max-sessions-64`); a controller with 100+ live subscriptions needs a CASE
   session per node. Options: larger features (128, 256), or an
   allocation-backed table behind `alloc`. The budget test measures 100 nodes
   in the store; 100 live nodes waits on this.
4. **A CAT as the admin subject.** Let the caller of `Commissioner` choose
   `case_admin_subject` (a CAT for a multi-controller fabric).
5. **Network commissioning helpers.** After PASE, before AddNOC completes,
   a Wi-Fi or Thread device needs `AddOrUpdateWiFiNetwork` /
   `AddOrUpdateThreadNetwork` and `ConnectNetwork`. `Commissioner` leaves the
   PASE session open for this; matter-host can drive it with its
   generic invoke, but the ordering inside the fail-safe belongs in the
   commissioner.

Anything else found while building on rs-matter joins this list.

## Commissioning over Bluetooth and Thread

- **BLE.** rs-matter has BTP in the central role (`Btp::set_initiator(true)`)
  and parses Matter advertisements (`AdvData`). The GATT central comes from
  bluetooth-bridge's `bluetooth-host` crate: scan and filter adverts by service
  data `0xFFF6`, `connect` (no pairing), `exchange_mtu(247)`, discover the
  `0xFFF6` service, `write` to C1 (completes on the Write Response), subscribe
  to C2 and take its indications from the connection's events (listening
  before subscribing, so the BTP handshake response is not missed), `read` C3.
  matter-host feeds those indications to `Btp::process_incoming` and
  writes what `Btp::process_outgoing` yields; commissioning then runs PASE
  over BTP, network commissioning, and CASE over IP.
- **Thread.** A Thread device is commissioned over BLE and given the Thread
  network's active operational dataset with `AddOrUpdateThreadNetwork` and
  `ConnectNetwork`, over PASE, inside the fail-safe. The dataset comes from
  `thread_dataset` (hex TLVs, in the configuration or the commission request)
  or `thread_dataset_url`, an OpenThread border router's REST API
  (`GET /node/dataset/active` with `Accept: text/plain`, as Home Assistant
  and python-matter-server read it). It is read at commissioning time, never
  written to disk or logged; only its extended PAN id (TLV 2) is logged.
  Operational traffic then reaches the device through the border router, and
  DNS-SD through its SRP server and advertising proxy. A Thread device's
  `ThreadNetworkDiagnostics.ExtAddress` arrives with its other attributes,
  which lets a platform place the node in the Thread mesh.

## Groups, software updates, sleepy devices

- **Groups.** The controller makes one group key set for its fabric and
  gives it to each node it puts in a group: with Groupcast's `JoinGroup` on
  Matter 1.6 nodes (with the auxiliary ACL entry that lets the group operate
  the endpoint), or `KeySetWrite`, the group key map, an ACL entry and
  `AddGroup` on older ones. A group command is one multicast message
  (`Exchange::initiate_group`).
- **OTA provider.** The controller's own node serves rs-matter's OTA Provider
  cluster on its root endpoint, with an ACL entry letting the fabric's nodes
  query it, and chains rs-matter's BDX handler into its responder. Images are
  the Matter OTA files in `ota_dir`, read by header: a node is offered the
  newest for its vendor and product that applies on top of its version, and
  the file name is the BDX designator. `AnnounceOTAProvider` (as an urgent
  update, since a person asked) makes a node ask at once. Images from the
  DCL's `OtaUrl` could fill the directory later.
- **ICD (sleepy devices).** A node whose ICD Management cluster supports
  Check-In is registered after commissioning (`RegisterClient`, a random
  key, the controller as check-in node and monitored subject). rs-matter's
  secure channel hands Check-Ins to the controller, which finds the node by
  the key that opens the message, refuses counters it has seen, renews the
  subscription (with a maximum interval at least the node's idle interval)
  and releases requests that waited for the node. After a restart the
  controller waits for each sleeping node's Check-In rather than reaching
  for it.

## Later, designed for now

- **The reverse: exposing devices as a Matter bridge.** rs-matter's device
  side does bridges well (the `bridge` example: an aggregator endpoint and
  bridged endpoints with Bridged Device Basic Information). A separate binary
  in this repository maps a generic device model (on/off, level, sensors) onto
  bridged endpoints and commissions into Apple Home, Google Home or Home
  Assistant like any bridge; a Zigbee, Z-Wave or BLE device then appears in
  those ecosystems. The mapping from each stack to the generic model lives with
  its user, not here.

## Testing

- Unit tests: data model tables, TLV ⇄ JSON both ways, onboarding codes, the
  store, Home Assistant discovery, configuration.
- End to end, every commit: connectedhomeip's `chip-all-clusters-app` in a
  container on a GitHub-hosted arm64 runner. The test commissions it over IP
  (DNS-SD discovery from the manual code), checks the interview, the
  subscription's priming report and an event, Home Assistant discovery, a
  read, a command whose effect comes back through the subscription, a write,
  an invoke with a typed response, and a commissioning window through which a
  second controller on another fabric commissions the device too.
- Fuzzing: JSON → TLV against every shape of the data model (with the round
  trip as the oracle), and onboarding codes. No `unsafe` anywhere in this
  repository (`unsafe_code = "forbid"`).
- Budget: under 16 MB resident with 100 nodes (in the store, with their
  subscription tasks); binary size reported for aarch64 and mipsel.

## Certification

Running this code needs nothing from the CSA. Selling a product that carries
the Matter logo, or that a commercial ecosystem treats as certified, does:

- **CSA membership.** Certification is open to members at Adopter level or
  above; membership comes with a Vendor ID. Fees are on csa-iot.org and change
  yearly.
- **Per-product certification.** Testing at an Authorized Test Laboratory
  (quoted per product and scope), then the CSA's certification fee: 2,000 USD
  per product, 1,500 USD for a derivative of one's own (2026 participant
  schedule).
- **Device attestation for a device product.** A DAC per unit, chained to a
  PAI under a PAA the CSA trusts (usually a commercial PKI service), and a
  certification declaration from the CSA. A controller product needs no DAC
  for its controller role, but a gateway that also exposes devices as a Matter
  bridge is a device and needs all of it.
- **Thread.** A Thread border router product is certified separately by the
  Thread Group (its own membership).
- **This project.** A certified product must pass the test cases for what it
  claims (the CSA's test harness runs them). For a controller that means the
  commissioner cases, device attestation verification first among them, which
  is why upstream item 1 comes first.
