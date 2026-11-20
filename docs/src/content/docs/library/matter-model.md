---
title: "matter-model"
description: "The Matter data model, as static tables, and conversion between JSON and Matter TLV."
---

The Matter data model, as static tables, and conversion between JSON and
Matter TLV. No I/O. [API docs](https://pulzar-net.github.io/matter-bridge/api/matter_model/index.html).

- The tables are generated at build time from the cluster definitions in
  the IDL that rs-matter ships (the CSA's `controller-clusters.matter`):
  every cluster, attribute, command, event, structure, enumeration and
  bitmap, with names and types.
- TLV is self-describing, so any value decodes to JSON even without the
  tables; with them, structures get their field names, and the encoder
  knows signedness, string kinds and field tags.
- Names map both ways: `on_off` is cluster 6, `current_level` is
  attribute 0 of Level Control.
- Vendor clusters the IDL does not have are handled by number, untyped.

The JSON encoder is fuzzed against every shape in the data model, with the
round trip (JSON to TLV to JSON) as the oracle.
