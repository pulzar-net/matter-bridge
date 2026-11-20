# Changelog

All notable changes are listed here. The project follows
[semantic versioning](https://semver.org/).

## [Unreleased]

## [0.1.0] - 2026-11-20

### Added

- `matter-model`: the Matter data model as tables generated from the IDL
  rs-matter ships, and JSON to TLV and back.
- `matter-controller`: onboarding (QR and manual codes), DNS-SD discovery,
  commissioning over IP on rs-matter's commissioner, a pluggable CA with a
  file CA, the node store, read, write, invoke, subscriptions with renewal,
  events, and enhanced commissioning windows for multi-admin.
- `matter-bridge`: the standalone MQTT bridge with Home Assistant discovery.
- CI against connectedhomeip's all-clusters app, fuzzing of the JSON encoder
  and onboarding codes,
  static aarch64 and mipsel builds, and a memory budget.

### Known limitations

- Device attestation is not verified yet; `allow_unverified_attestation`
  must stay on.
- Not on crates.io: rs-matter is a git dependency pinned to a commit.
- Not built yet: Bluetooth and Thread commissioning, commissioning with a CA
  whose key is held elsewhere, more than 64 live nodes, OTA, groups, ICD
  devices, exposing other devices as a Matter bridge.

[Unreleased]: https://github.com/pulzar-net/matter-bridge/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/pulzar-net/matter-bridge/releases/tag/v0.1.0
