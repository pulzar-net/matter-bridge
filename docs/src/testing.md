# Testing

CI runs every test on every commit.

## Running the tests

```sh
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
```

## Unit tests

The data model tables, JSON to TLV and back for every kind of value,
onboarding codes, the node store, Home Assistant discovery and the
configuration.

## End to end, against the reference device

On every commit, CI starts connectedhomeip's `chip-all-clusters-app` (from
the CSA's `chip-cert-bins` image, on an arm64 runner) and runs
`crates/matter-bridge/tests/e2e.rs` against it. The test:

- commissions it over IP, finding it by DNS-SD from its manual code;
- checks the interview, the subscription's first report and an event;
- checks Home Assistant discovery;
- reads, writes, and invokes a command whose effect comes back through the
  subscription, and one with a typed response;
- opens a commissioning window, through which a second controller on its own
  fabric commissions the device too.

The same test runs against any commissionable device on your network:

```sh
MATTER_E2E_CODE=34970112332 cargo test -p matter-bridge --test e2e -- --ignored --nocapture
```

The docs workflow also runs the README's quick start against the same
device: the release binary and the current build, configured, started and
commissioning with the README's own commands.

## Bluetooth commissioning

`crates/matter-controller/tests/ble.rs` commissions over Bluetooth LE
between two virtual controllers from Bumble. It does not pass yet.

## Fuzzing

The bridge's own decoders of untrusted input have
[cargo-fuzz](https://github.com/rust-fuzz/cargo-fuzz) targets in `fuzz/`:
the JSON encoder against every shape in the data model (with the round trip
as the oracle), onboarding codes, and Thread datasets. CI fuzzes them on
every commit. Locally (needs a nightly toolchain):

```sh
cargo install cargo-fuzz
cargo +nightly fuzz list
cargo +nightly fuzz run json_to_tlv -- -max_total_time=600
```

## Budgets

- **Memory:** under 16 MB resident with 100 nodes in the store
  (`cargo test --release -p matter-controller --test budget -- --ignored`).
- **Binary size:** static `aarch64` and `mipsel` builds in CI.

## Documentation

The `docs` workflow checks the claims this site makes: the
[configuration reference](config-reference.md) is regenerated from the code
and compared, every doctest runs (the README's and the library guide's
examples among them), the README's quick start runs, the workspace builds on
the minimum supported Rust version, and every relative link resolves.
