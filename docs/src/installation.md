# Installation

matter-bridge is one static Linux binary. Pick one of the ways below, then
[configure](configuration.md) it.

## Release binaries

Each [release](https://github.com/pulzar-net/matter-bridge/releases) has
statically linked binaries with no runtime dependencies, and a `SHA256SUMS`
file:

| File | For |
|---|---|
| `matter-bridge-x86_64-linux-musl` | PCs and servers |
| `matter-bridge-aarch64-linux-musl` | Raspberry Pi 3, 4, 5 and other 64-bit ARM boards with a 64-bit OS |
| `matter-bridge-mipsel-linux-musl` | MT7621 and MT7628 routers (little-endian MIPS) |

Download, verify and install (this is the README's quick start; CI runs it):

```sh
VERSION=0.1.0
ARCH=$(uname -m)   # x86_64 or aarch64
BASE=https://github.com/pulzar-net/matter-bridge/releases/download/v$VERSION
curl -fsSLO "$BASE/matter-bridge-$ARCH-linux-musl"
curl -fsSLO "$BASE/SHA256SUMS"
sha256sum --check --ignore-missing SHA256SUMS
sudo install -m 0755 "matter-bridge-$ARCH-linux-musl" /usr/local/bin/matter-bridge
matter-bridge --version
```

## From source

With Rust 1.92 or newer:

```sh
cargo install --locked --git https://github.com/pulzar-net/matter-bridge matter-bridge
```

Or from a clone, `cargo build --release -p matter-bridge`. No system
packages are required. Bluetooth commissioning (feature `ble`, on by
default, not in 0.1.0) is built in; `--no-default-features` leaves it out.

## Network requirements

Matter runs over IPv6 on the local network, and finds devices with DNS-SD
(mDNS):

- the machine needs IPv6 (link-local is enough) on the interface that
  reaches the devices; set `matter.interface` when it has several;
- UDP port `matter.port` (5541 by default) and mDNS (UDP 5353) must not be
  blocked by a firewall;
- mDNS does not cross routers or VLANs without a reflector; keep the bridge
  on the devices' network.

## As a systemd service

The repository has a hardened unit in
[`dist/systemd/matter-bridge.service`](https://github.com/pulzar-net/matter-bridge/blob/main/dist/systemd/matter-bridge.service).
It runs the bridge as an unprivileged dynamic user and keeps the fabric in
`/var/lib/matter-bridge`.

```sh
sudo mkdir -p /etc/matter-bridge
sudo cp config.example.toml /etc/matter-bridge/config.toml   # then edit it
sudo install -m 0644 dist/systemd/matter-bridge.service /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now matter-bridge
journalctl -u matter-bridge -f
```

For Bluetooth commissioning (not in 0.1.0), the unit needs
`CAP_NET_ADMIN` and `AF_BLUETOOTH`; the commented lines in it show how.

## On OpenWrt

> Needs real hardware: this procedure has not been tested on a router yet.

The `mipsel` binary runs on MT7621 and MT7628 routers with OpenWrt.

1. Copy the binary to `/usr/bin/matter-bridge` and make it executable.
2. Write `/etc/matter-bridge/config.toml`. **Set `[data] dir` to a directory
   on flash**, such as `/etc/matter-bridge/data`: `/var` is in RAM on
   OpenWrt, and the fabric's keys would be lost at reboot, orphaning every
   device.
3. Allow UDP 5541 and 5353 from the LAN zone in the firewall.
4. Install the procd script from
   [`dist/openwrt/matter-bridge.init`](https://github.com/pulzar-net/matter-bridge/blob/main/dist/openwrt/matter-bridge.init):

```sh
cp matter-bridge.init /etc/init.d/matter-bridge
chmod +x /etc/init.d/matter-bridge
/etc/init.d/matter-bridge enable
/etc/init.d/matter-bridge start
logread -f -e matter-bridge
```

## Command line

```text
matter-bridge [--config FILE]     run the bridge
matter-bridge parse CODE          say what a QR code or manual pairing code holds
matter-bridge --version
```

`FILE` defaults to `/etc/matter-bridge/config.toml`; without it every
default applies. `parse` prints the code's discriminator, vendor and product
ids and how the device can be found, as JSON (never the passcode).

The log goes to standard error. `RUST_LOG` sets its level, for example
`RUST_LOG=debug`.
