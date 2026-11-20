# matter-bridge (crate)

The bridge as a library: the same code as the binary. [API docs](https://pulzar-net.github.io/matter-bridge/api/matter_bridge/index.html).

- `run(config, shutdown)` runs the bridge until `shutdown` completes.
- `Config` is the configuration file; `Config::load` reads one, and
  `Config::controller` gives the controller's configuration from it.
- The topic scheme and Home Assistant discovery are in its modules.

The [MQTT API](../mqtt.md) and [configuration reference](../config-reference.md)
document the behavior.
