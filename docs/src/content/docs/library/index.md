---
title: "The library crates"
description: "matter-bridge is built as library crates with a thin binary on top."
---

matter-bridge is built as library crates with a thin binary on top.

| Crate | Use it to |
|---|---|
| [matter-host](/matter-bridge/library/matter-host.html) | run a Matter controller from Rust: commission, read, write, invoke, subscribe |
| [matter-model](/matter-bridge/library/matter-model.html) | name clusters, attributes, commands and events, and convert between JSON and Matter TLV |
| [matter-bridge](/matter-bridge/library/matter-bridge.html) | run or embed the bridge itself |

The [API documentation](https://oss.pulzr.net/matter-bridge/api/matter_host/index.html) covers every public
item.

## Depending on the crates

The crates are not published on crates.io yet, because rs-matter is a git
dependency pinned to a commit. Depend on a release tag:

```toml
[dependencies]
matter-host = { git = "https://github.com/pulzar-net/matter-bridge", tag = "v0.1.0" }
```

While the project is at 0.x, minor versions may break the API; the
[release notes](https://github.com/pulzar-net/matter-bridge/releases) say what changed.

## Conventions

- **No `unsafe`** in this repository; the workspace lints forbid it.
- **JSON at the edge.** Values cross the API as `serde_json::Value`,
  converted to and from TLV by matter-model with the data model's types.
- **Runtime-agnostic handle.** rs-matter's futures are not `Send`, so the
  controller runs on a thread of its own; `Controller` is a `Send + Clone`
  handle usable from any tokio runtime.
