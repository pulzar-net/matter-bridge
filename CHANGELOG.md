# Changelog

All notable changes are listed here. The project follows
[semantic versioning](https://semver.org/).

## [Unreleased]

### Added

- OTA provider: the controller offers the Matter OTA image files in
  `Config::ota_dir` (`matter.ota_dir`) to the nodes it commissioned, the
  newest that applies to each, and serves them over BDX, on rs-matter's OTA
  Provider cluster. `Controller::announce_ota` (MQTT `ota_update`) tells a
  node to ask now; `ota_images` lists the images; `Event::Ota` (bridge event
  `ota`) follows each update. CI updates connectedhomeip's
  `chip-ota-requestor-app` with it.

## [0.1.0] - 2026-11-20

### Added

- `matter-model`: the Matter data model as tables generated from the IDL
  rs-matter ships, and JSON to TLV and back.
- `matter-controller`: onboarding (QR and manual codes), DNS-SD discovery,
  commissioning over IP on rs-matter's commissioner, a pluggable CA with a
  file CA, the node store, read, write, invoke, subscriptions with renewal,
  events, enhanced commissioning windows for multi-admin, and
  `Controller::abort_commissioning`.
- Commissioning over Bluetooth LE (feature `ble`, on by default), with
  bluetooth-bridge's `bluetooth-host` as the GATT central under rs-matter's
  BTP: `bluetooth = "hci0"` or `"tcp:host:port"`.
- Network commissioning for devices reached over Bluetooth: the controller
  reads the device's Network Commissioning features and gives it the Thread
  dataset (`thread_dataset`, `thread_dataset_url` or the configured border
  router) or the Wi-Fi network (`wifi`) the commission request carries, inside
  the fail-safe, before CASE. Datasets and passphrases are never stored or
  logged.
- Groups: `Controller::add_to_group` gives a node the fabric's group key and
  puts an endpoint in a group (Groupcast on Matter 1.6 nodes, Groups and Group
  Key Management before), with access for the group to operate it;
  `group_invoke` sends one multicast command to every member.
- `IntermediateCa`: a controller under a root CA elsewhere, its intermediate
  key made and kept here, signing its own and its devices' certificates from a
  node id range the root CA assigns; `Config.ca` picks it or the file CA.
- `x509::to_matter_tlv`: X.509 DER certificates in the Matter profile to
  Matter TLV, each checked by rebuilding the signed DER.
- `matter-bridge`: the standalone MQTT bridge with Home Assistant discovery,
  including `add_to_group`, `remove_from_group` and `group_invoke` requests.
- CI against connectedhomeip's all-clusters app (commissioning over IP, reads,
  subscriptions, writes, invokes, a group command, MQTT, a second admin), a
  Bluetooth commissioning test on a virtual controller pair (Bumble) with a
  Wi-Fi device, fuzzing of the JSON encoder, onboarding codes, Thread datasets
  and X.509 conversion, static aarch64 and mipsel builds with a size budget,
  and a memory budget.

### Known limitations

- Device attestation is not verified yet; `allow_unverified_attestation`
  must stay on.
- Not on crates.io: rs-matter is a git dependency pinned to a commit.
- Thread commissioning is tested only up to the request it sends; CI has no
  Thread device.
- Not built yet: more than 64 live nodes, OTA, ICD devices, exposing other
  devices as a Matter bridge.

[Unreleased]: https://github.com/pulzar-net/matter-bridge/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/pulzar-net/matter-bridge/releases/tag/v0.1.0
