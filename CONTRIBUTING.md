# Contributing to matter-bridge

Thanks for helping. Every kind of contribution is welcome: a report that a
device works, a bug report with a log, a fix to the docs, a new feature.

Please follow the [code of conduct](CODE_OF_CONDUCT.md). Security problems go
through [private reporting](SECURITY.md), never a public issue. A problem in
rs-matter itself goes to rs-matter's own security process.

## Before you start

- **Questions** (how do I, is this expected): ask in
  [Discussions](https://github.com/pulzar-net/matter-bridge/discussions).
- **Device reports:** use the
  [device form](https://github.com/pulzar-net/matter-bridge/issues/new?template=device.yml).
- **Bugs:** use the [bug form](https://github.com/pulzar-net/matter-bridge/issues/new?template=bug.yml),
  with the version, your configuration and a `RUST_LOG=debug` log.
- **Larger changes** (a new commissioning flow, a change to the MQTT API or
  the configuration): open an issue first. The
  [design page](https://pulzar-net.github.io/matter-bridge/design.html) has
  the plan for the planned features.
- **Changes rs-matter needs** go to rs-matter as their own pull requests;
  the design page lists the ones known.

## Build

You need Rust 1.92 or newer ([rustup.rs](https://rustup.rs)). rs-matter is a
git dependency pinned to a commit; cargo fetches it.

```sh
git clone https://github.com/pulzar-net/matter-bridge
cd matter-bridge
cargo build --workspace
```

## Test

Before you send a change, run what CI runs:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
```

The end-to-end test commissions a real or reference device and is ignored
by default. On a Linux machine with Docker (arm64, for the CSA's image) or
with any commissionable device on the network:

```sh
docker run -d --name dut --network host connectedhomeip/chip-cert-bins:<tag> \
    /root/chip-all-clusters-app --KVS /tmp/dut.kvs
cargo test -p matter-bridge --test e2e -- --ignored --nocapture
```

`MATTER_E2E_CODE` sets another device's pairing code. CI pins the image tag
in `.github/workflows/ci.yml`. On macOS, mDNS and multicast make the local
network prompt appear; the end-to-end tests are meant for Linux.

If you change a configuration type, regenerate the configuration reference
and commit the result:

```sh
UPDATE_DOCS=1 cargo test -p matter-bridge --features schema --test config_reference
```

## Fuzz

The JSON encoder, onboarding codes and Thread datasets have fuzz targets in
`fuzz/`. If you change one of those, fuzz it for a few minutes (needs a
nightly toolchain and `cargo install cargo-fuzz`):

```sh
cargo +nightly fuzz list
cargo +nightly fuzz run json_to_tlv -- -max_total_time=300
```

## Code rules

- No `unsafe` code; the workspace lints forbid it.
- Values cross the API as JSON, converted only through the data model;
  never build TLV by hand.
- No panics on input from the network or MQTT: return errors.
- Secrets (CA keys, passphrases, datasets, passcodes) are never logged or
  published.
- Every public item has a doc comment (`missing_docs` is on).
- New behavior comes with a test: a unit test, or an end-to-end check
  against the reference device.
- Work around a gap in rs-matter only with a comment that links the
  upstream change that fixes it.

## Commits and pull requests

- One logical change per commit. The subject line says what the change does,
  in plain words, under 72 characters, for example
  `Commissioning over Bluetooth, end to end on Bumble`; the body says why.
- Add a line to `CHANGELOG.md` under `[Unreleased]` for anything a user or a
  library user would notice.
- Update the README and `docs/` when you change behavior, a configuration
  key or an MQTT topic.
- Sign off every commit (see below).

CI must pass before a pull request is merged: the tests, fuzzing and the
cross builds, and the end-to-end test against the reference device. Pull requests are squash-merged, so the pull
request's title and description become the commit on `main`: write them
as you would a commit message.

## Developer Certificate of Origin

Contributions are accepted under the [Developer Certificate of Origin
1.1](https://developercertificate.org): by signing off a commit, you certify
that you wrote it, or otherwise have the right to submit it under the
project's MIT license. Sign off with `git commit -s`, which adds a line like:

```text
Signed-off-by: Your Name <you@example.com>
```

using your real name. Commits without a sign-off cannot be merged.

## License

By contributing, you agree that your contributions are licensed under the
[MIT license](LICENSE).
