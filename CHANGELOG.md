# Changelog

All notable changes are listed here. The project follows
[semantic versioning](https://semver.org/).

## [Unreleased]

### Added

- `matter-model`: the Matter data model as tables generated from the IDL
  rs-matter ships, and JSON to TLV and back.
- `matter-controller`: onboarding (QR and manual codes), DNS-SD discovery,
  commissioning over IP on rs-matter's commissioner, a pluggable CA with a
  file CA, the node store, read, write, invoke, subscriptions with renewal,
  events, and enhanced commissioning windows for multi-admin.
- `matter-bridge`: the standalone MQTT bridge with Home Assistant discovery.
- CI against connectedhomeip's all-clusters app, fuzzing of every parser,
  static aarch64 and mipsel builds, and a memory budget.
