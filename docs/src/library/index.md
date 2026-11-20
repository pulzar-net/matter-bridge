# The library crates

matter-bridge is built as library crates with a thin binary on top.

| Crate | Use it to |
|---|---|
| [matter-controller](matter-controller.md) | run a Matter controller from Rust: commission, read, write, invoke, subscribe |
| [matter-model](matter-model.md) | name clusters, attributes, commands and events, and convert between JSON and Matter TLV |
| [matter-bridge](matter-bridge.md) | run or embed the bridge itself |

The [API documentation](https://pulzar-net.github.io/matter-bridge/api/matter_controller/index.html) covers every public
item.

## Depending on the crates

The crates are not published on crates.io yet, because rs-matter is a git
dependency pinned to a commit. Depend on a release tag:

```toml
[dependencies]
matter-controller = { git = "https://github.com/pulzar-net/matter-bridge", tag = "v0.1.0" }
```

While the project is at 0.x, minor versions may break the API; the
[changelog](../changelog.md) says what changed.

## Conventions

- **No `unsafe`** in this repository; the workspace lints forbid it.
- **JSON at the edge.** Values cross the API as `serde_json::Value`,
  converted to and from TLV by matter-model with the data model's types.
- **Runtime-agnostic handle.** rs-matter's futures are not `Send`, so the
  controller runs on a thread of its own; `Controller` is a `Send + Clone`
  handle usable from any tokio runtime.
