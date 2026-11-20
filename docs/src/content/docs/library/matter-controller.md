---
title: "matter-controller"
description: "A Matter controller on rs-matter."
---

A Matter controller on rs-matter. [API docs](https://pulzar-net.github.io/matter-bridge/api/matter_controller/index.html).

## What it does

- **Onboarding**: `Onboarding::parse` reads QR codes and manual codes.
- **Discovery** of commissionable devices by DNS-SD, and of operational
  nodes.
- **Commissioning** over IP and over Bluetooth LE (with
  network credentials), on rs-matter's commissioner.
- **A fabric of its own**, rooted in a `CertificateAuthority`: `FileCa`
  keeps the CA in the state directory; implement the trait to hold the root
  elsewhere.
- **The interaction model**: read, write and invoke on any cluster, with
  JSON values.
- **Subscriptions**: one per node to every attribute and event, renewed
  when it lapses; reports arrive as `Event`s.
- **Commissioning windows** for multi-admin.
- **A node store**, written when the network changes.

## Example

This starts a controller, commissions a device on the network, and prints
every attribute report. It needs a commissionable device, so it is compiled
in CI but not run there (the end-to-end test does the same against the
reference device).

```rust,no_run
use matter_controller::{CommissionRequest, Config, Controller, Event};

# async fn run() -> Result<(), Box<dyn std::error::Error>> {
let controller = Controller::start(Config::new("/var/lib/my-controller"))?;
let mut events = controller.events();

let node = controller
    .commission(CommissionRequest {
        code: "3497-011-2332".into(),
        address: None,
        name: Some("lamp".into()),
        discovery_timeout: 30,
        networks: Default::default(),
    })
    .await?;
println!("commissioned node {}", node.node_id);

while let Some(event) = events.recv().await {
    if let Event::Attribute(a) = event {
        println!("node {} endpoint {} cluster {:#x} attribute {:#x}: {}",
            a.node, a.endpoint, a.cluster, a.attribute, a.value);
    }
}
# Ok(())
# }
```

`Controller` is a cheap handle (`Send + Clone`); the controller itself runs
on its own thread. `events()` gives a receiver of `Event`s: attribute
values, events, commissioning steps, node updates and availability.
