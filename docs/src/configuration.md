# Configuration

matter-bridge reads one TOML file, `/etc/matter-bridge/config.toml` unless
`--config FILE` names another. Every key has a default, and without the file
every default applies. A typical file only names the broker:

```toml
[mqtt]
host = "broker.lan"
```

Unknown keys and wrong types are errors, so a typo stops the bridge at
startup. The [configuration reference](config-reference.md) lists every
key; it is generated from the code, so it is always current.

A [JSON Schema](config.schema.json) generated from the same code lets an
editor with TOML schema support (Taplo, or VS Code's Even Better TOML)
complete and check the file. Point it at the schema with a first line of:

```toml
#:schema https://pulzar-net.github.io/matter-bridge/config.schema.json
```

## The tables

| Table | What it sets |
|---|---|
| `[mqtt]` | broker host and port, user name, password file, client id, base topic, keep-alive |
| `[homeassistant]` | discovery on or off, the discovery prefix, Home Assistant's status topic |
| `[matter]` | the UDP port, the mDNS interface, the fabric id of a new fabric, the vendor id, attestation policy, subscription renewal, the discovery timeout; not in 0.1.0: the Bluetooth controller and a Thread dataset source |
| `[data]` | the data directory |

## A complete example

[`config.example.toml`](https://github.com/pulzar-net/matter-bridge/blob/main/config.example.toml)
in the repository sets every key to its default, with comments. CI checks
that it parses.

```toml
{{#include ../../config.example.toml}}
```

## The data directory

`[data] dir` (default `/var/lib/matter-bridge`) holds:

| What | Why it matters |
|---|---|
| the fabric's CA: root and intermediate keys and certificates, the IPK, the next node id | anyone with it can add nodes to the fabric and control every device on it |
| the controller's identity: its node id, operational key and certificate | |
| `nodes.json`: each node's identity and interview | rebuilt by interviewing again if lost |

The files are written atomically and are readable by their owner only.
**Back up the whole directory, and protect the backups like a password.**
Without it, the bridge cannot reach its devices: they would have to be
factory reset and commissioned again.

## Logging

The bridge logs to standard error at `info` unless `RUST_LOG` says
otherwise (`debug`, or per target such as `info,matter_controller=debug`).
Wi-Fi passphrases and Thread datasets given at commissioning are never
logged or stored.
