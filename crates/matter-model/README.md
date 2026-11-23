# matter-model

The Matter data model as static lookup tables, and conversion between JSON
and Matter TLV guided by them.

The tables are generated at build time from the Matter IDL that rs-matter
ships (the CSA's `controller-clusters.matter`, from which the C++ SDK and
rs-matter generate their own cluster code), so nothing here is typed by
hand and a newer rs-matter brings newer clusters.

TLV is self-describing, so any value decodes to JSON even without a schema;
the schema adds field names to structures and gives the encoder the
signedness, string kind and field tags it cannot guess from JSON.

```rust
let on_off = matter_model::cluster_by_name("on_off").unwrap();
assert_eq!(on_off.id, 6);
assert_eq!(on_off.attribute_by_name("OnOff").unwrap().id, 0);
```

Part of [matter-bridge](https://github.com/pulzar-net/matter-bridge), maintained by
[Pulzr](https://pulzr.net). See the [user guide](https://oss.pulzr.net/matter-bridge/), the
[library guide](https://oss.pulzr.net/matter-bridge/library/index.html) and the
[API documentation](https://oss.pulzr.net/matter-bridge/api/matter_model/index.html).

License: MIT.
