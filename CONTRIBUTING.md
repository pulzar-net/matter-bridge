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
  [design page](https://oss.pulzr.net/matter-bridge/design.html) has
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

If you change an MQTT payload type, regenerate the MQTT reference the same
way:

```sh
UPDATE_DOCS=1 cargo test -p matter-bridge --features unstable-asyncapi-docs --test asyncapi
```

## The MQTT reference

The [MQTT reference](https://oss.pulzr.net/matter-bridge/mqtt-reference.html)
is an AsyncAPI 3.0 description generated from the bridge's own payload
types, so it cannot drift from what the bridge publishes. The other Pulzar
bridges follow the same pattern:

- **One module holds every payload** (`crates/matter-bridge/src/api.rs`):
  each topic's message, each event, each request and its response. The
  bridge serializes these types and never builds JSON by hand, so they are
  the contract. Controller types (a node, a commissioning window) are
  mirrored there with `From` conversions, so the controller library carries
  no docs tooling.
- **The derives sit behind a non-default feature**, `unstable-asyncapi-docs`.
  It turns on two optional dependencies, `schemars` and `asyncapi-rust`.
  Default builds, release binaries and cross builds never enable it, and CI
  fails if a default build pulls in either crate. The feature is docs
  tooling, not API, and may change in any release.

  ```rust
  #[derive(Debug, Clone, Serialize, Deserialize)]
  #[cfg_attr(feature = "unstable-asyncapi-docs", derive(JsonSchema, ToAsyncApiMessage))]
  #[cfg_attr(feature = "unstable-asyncapi-docs", asyncapi(summary = "An event the device reported"))]
  pub struct DeviceEvent { /* ... */ }
  ```

  Every `#[asyncapi(...)]` or `#[schemars(...)]` attribute goes inside
  `cfg_attr` too.
- **Doc comments are the descriptions.** Each field's `///` comment becomes
  its description in the reference, so write them for the person reading it:
  units, ranges, when a value is `null`.
- **Name tagged-enum variants one by one** with `#[serde(rename = "...")]`,
  not `rename_all`. asyncapi-rust 0.5 splits a tagged enum into one message
  per variant only for explicit renames (mlilback/asyncapi-rust#25). A
  payload that is a bare string goes in a `#[serde(transparent)]` newtype,
  so it becomes one message rather than one per variant.
- **The generator is a test** with `required-features`
  (`crates/matter-bridge/tests/asyncapi.rs`). It holds the
  `#[derive(AsyncApi)]` struct with servers, channels, operations and MQTT
  bindings (QoS, retain, last will), and writes `docs/public/asyncapi.json`
  when `UPDATE_DOCS` is set. Otherwise it fails if the committed file
  differs. Every channel and operation has a description, and every
  operation a summary.
- **CI checks both directions.** The docs workflow runs that test, then
  `@asyncapi/cli validate --fail-severity=warn`, with the CLI's analytics
  turned off first. When the committed file is stale, the workflow uploads
  the regenerated one as the `asyncapi.json` artifact.
- **The docs page** is one `.mdx` file using `AsyncApiReference` from
  `@pulzr/docs-theme`, with `tableOfContents: false`. The hand-written MQTT
  page keeps the explanations and links to the reference, with no
  hand-maintained field tables.
- **The dependency is upstream asyncapi-rust**, as
  `asyncapi-rust = { version = "0.5", optional = true }`, and that line never
  changes. When upstream lacks something the reference needs, the fix goes
  to the `pulzar-net/asyncapi-rust` fork, which is never published, and the
  workspace pins the fork with `[patch.crates-io]` at a fixed revision for
  `asyncapi-rust`, `asyncapi-rust-codegen` and `asyncapi-rust-models`. The
  same fix is offered upstream as a pull request, and the patch goes once a
  release has it. `cargo publish` ignores patches and the published crate
  only uses the feature's derives, so publishing is unaffected.

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
- Every MQTT payload is a type in `crates/matter-bridge/src/api.rs`; never
  publish hand-built JSON.
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
- Say in the pull request what a user or a library user would notice: it
  goes into the
  [release notes](https://github.com/pulzar-net/matter-bridge/releases).
  There is no changelog file; a release's notes are its annotated tag's
  message (`git tag -a vX.Y.Z --cleanup=whitespace -F notes.md`), which the
  release workflow publishes.
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
