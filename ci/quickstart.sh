#!/usr/bin/env bash
# Runs the README's quick start as written, with connectedhomeip's
# chip-all-clusters-app (the CSA's reference device) as the device: download
# the release and check its sum, then write the README's configuration, run
# the bridge with the README's command, commission with the README's command
# (the reference device's pairing code), and wait for the device's state and
# its Home Assistant discovery. It runs twice: with the release binary the
# README downloads, and with this commit's build; the device starts fresh for
# each, since a commissioned device leaves pairing mode.
#
# Needs Linux on arm64 (the device image is arm64 only), Docker, sudo, cargo,
# curl, jq, and mosquitto with its clients. CHIP_CERT_BINS names the image.
set -euo pipefail
cd "$(dirname "$0")/.."
: "${CHIP_CERT_BINS:?set CHIP_CERT_BINS to the connectedhomeip/chip-cert-bins image}"

# The body of the code block that follows `<!-- quickstart:NAME -->`.
block() {
    awk -v marker="<!-- quickstart:$1 -->" '
        $0 == marker { found = 1; next }
        found && /^```/ { if (inside) exit; inside = 1; next }
        inside { print }
    ' README.md
}

work="$(mktemp -d)"
pids=()
cleanup() {
    for pid in "${pids[@]}"; do sudo kill "${pid}" 2>/dev/null || true; done
    docker rm -f dut >/dev/null 2>&1 || true
    rm -rf "${work}"
}
trap cleanup EXIT

# Wait up to $3 seconds for a message on topic $1 that the jq filter $2
# accepts. Retained messages count: the bridge may publish before we listen.
wait_for() {
    local deadline=$((SECONDS + $3))
    while [ "${SECONDS}" -lt "${deadline}" ]; do
        if mosquitto_sub -C 1 -W 5 -t "$1" 2>/dev/null | jq -e -R "(. as \$raw | try fromjson catch \$raw) | $2" >/dev/null 2>&1; then
            return 0
        fi
    done
    echo "timed out waiting for $1 ($2)" >&2
    cat "${work}/bridge.log" >&2
    docker logs dut 2>&1 | tail -50 >&2
    return 1
}

echo "--- 1. Download, verify and install the release"
download="$(block download)"
(cd "${work}" && bash -euxo pipefail -c "${download}")

echo "--- This commit's bridge"
cargo build --locked -p matter-bridge
build="$(pwd)/target/debug/matter-bridge"

# A fresh broker for each run, so no retained message from an earlier run
# can satisfy a check. The README's commands use the default port.
sudo systemctl stop mosquitto 2>/dev/null || true

for bridge in matter-bridge "${build}"; do
    echo "=== quick start with ${bridge}"
    sudo rm -rf /var/lib/matter-bridge
    mosquitto -p 1883 >"${work}/broker.log" 2>&1 &
    pids+=("$!")
    docker rm -f dut >/dev/null 2>&1 || true
    docker run -d --name dut --network host "${CHIP_CERT_BINS}" \
        /root/chip-all-clusters-app --KVS /tmp/dut.kvs >/dev/null
    sleep 3

    echo "--- 2. The README's configuration"
    sudo mkdir -p /etc/matter-bridge
    block config | sudo tee /etc/matter-bridge/config.toml

    echo "--- 3. Run"
    run="$(block run | sed "s|^sudo matter-bridge |sudo ${bridge} |")"
    bash -c "${run}" >"${work}/bridge.log" 2>&1 &
    wait_for matter/bridge/state '.state == "online"' 30

    echo "--- 4. Commission"
    bash -euxo pipefail -c "$(block pair)"
    wait_for matter/lamp/availability '. == "online"' 120
    wait_for matter/lamp/1/on_off/on_off 'type == "boolean"' 60
    wait_for 'homeassistant/+/+/1_6/config' '.state_topic | startswith("matter/lamp/")' 60

    sudo pkill -f -- "matter-bridge --config" || true
    sleep 1
    for pid in "${pids[@]}"; do kill "${pid}" 2>/dev/null || true; done
    pids=()
    sleep 1
done
echo "quick start: ok"
