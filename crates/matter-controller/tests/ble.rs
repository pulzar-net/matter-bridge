//! Commissioning over Bluetooth LE, end to end, on Google's Bumble: two
//! virtual controllers on one simulated link (`tests/bumble/link.py`), one for
//! the controller under test and one for a device.
//!
//! The device is rs-matter's Wi-Fi on/off light, as in its `onoff_light_bt`
//! example, with its GATT peripheral on bluetooth-host and a Wi-Fi control
//! that stays on the network the host is on. The controller finds it by its
//! Matter advertisement, runs PASE over BTP, gives it a Wi-Fi network, and
//! completes commissioning over CASE on IP.
//!
//! Ignored by default; needs `BUMBLE_PYTHON`, a Python with `bumble`
//! (`pip install -r tests/bumble/requirements.txt`), and a network interface
//! for mDNS:
//! `BUMBLE_PYTHON=python cargo test -p matter-controller --test ble -- --ignored`.

#![cfg(feature = "ble")]

use std::process::Stdio;
use std::time::Duration;

use bluetooth_host::adv::Uuid;
use bluetooth_host::gatt::server::{Access, CharacteristicDef, ServiceDef};
use bluetooth_host::{AdvertisingParams, Host, HostConfig, HostEvent, TransportSpec};
use matter_controller::{AttributePath, CommissionRequest, Config, Controller, Networks, WiFi};
use rs_matter::crypto::{default_crypto, Crypto as _};
use rs_matter::dm::clusters::app::on_off::{self, test::TestOnOffDeviceLogic, OnOffHooks as _};
use rs_matter::dm::clusters::desc::{self, ClusterHandler as _};
use rs_matter::dm::clusters::net_comm::NetworkType;
use rs_matter::dm::devices::test::{
    DAC_PRIVKEY, TEST_BASIC_COMM, TEST_DEV_ATT, TEST_DEV_COMM, TEST_DEV_DET, TEST_PID, TEST_VID,
};
use rs_matter::dm::devices::DEV_TYPE_ON_OFF_LIGHT;
use rs_matter::dm::endpoints::ROOT_ENDPOINT_ID;
use rs_matter::dm::networks::wireless::{NoopWirelessNetCtl, WifiNetworks};
use rs_matter::dm::networks::SysNetifs;
use rs_matter::dm::{Async, DataModel, Dataver, EmptyHandler, Endpoint, Node};
use rs_matter::error::{Error as MatterError, ErrorCode};
use rs_matter::pairing::qr::{no_optional_data, CommFlowType, QrPayload};
use rs_matter::pairing::DiscoveryCapabilities;
use rs_matter::persist::DirKvBlobStore;
use rs_matter::stack::ble::GattPeripheral;
use rs_matter::stack::net::std::StdNetStack;
use rs_matter::stack::wireless::PreexistingWireless;
use rs_matter::stack::MatterStack;
use rs_matter::transport::network::btp::{AdvData, Btp};
use rs_matter::transport::network::mdns::builtin::BuiltinMdns;
use rs_matter::transport::network::BtAddr;
use rs_matter::utils::init::InitMaybeUninit as _;
use rs_matter::{clusters, devices, root_endpoint, Matter};
use tokio::io::{AsyncBufReadExt, BufReader};

const C1: [u8; 16] = 0x18EE2EF5263D4559959F4F9C429F9D11u128.to_be_bytes();
const C2: [u8; 16] = 0x18EE2EF5263D4559959F4F9C429F9D12u128.to_be_bytes();

/// rs-matter's GATT peripheral, on bluetooth-host: the Matter service, the
/// advertisement, C1 writes into BTP and BTP out as C2 indications.
struct HostGatt(Host);

fn io(e: impl std::fmt::Display) -> MatterError {
    eprintln!("device bluetooth: {e}");
    ErrorCode::StdIoError.into()
}

impl GattPeripheral for HostGatt {
    async fn run(
        &mut self,
        btp: &Btp,
        _service_name: &str,
        adv: &AdvData,
    ) -> Result<(), MatterError> {
        let host = &self.0;
        let server = host.gatt_server();
        let handles = server
            .add(&ServiceDef {
                uuid: Uuid::Uuid16(0xFFF6),
                primary: true,
                characteristics: vec![
                    CharacteristicDef {
                        uuid: Uuid::Uuid128(C1),
                        properties: vec!["write".into()],
                        read: Access::Never,
                        write: Access::Open,
                        value: Vec::new(),
                        max_len: 247,
                        description: None,
                        descriptors: Vec::new(),
                    },
                    CharacteristicDef {
                        uuid: Uuid::Uuid128(C2),
                        properties: vec!["read".into(), "indicate".into()],
                        read: Access::Open,
                        write: Access::Never,
                        value: Vec::new(),
                        max_len: 247,
                        description: None,
                        descriptors: Vec::new(),
                    },
                ],
            })
            .await
            .map_err(io)?;
        let handle = |uuid: [u8; 16]| {
            handles
                .characteristics
                .iter()
                .find(|(u, _)| *u == Uuid::Uuid128(uuid))
                .map(|(_, h)| *h)
                .ok_or_else(|| io("no handle"))
        };
        let (c1, c2) = (handle(C1)?, handle(C2)?);

        let data: Vec<u8> = adv.iter().collect();
        host.advertise(
            &AdvertisingParams {
                connectable: true,
                legacy: Some(true),
                ..Default::default()
            },
            &data,
            &[],
        )
        .await
        .map_err(io)?;

        let mut events = host.events();
        let mut peer: Option<(u16, BtAddr)> = None;
        // As the C++ SDK's peripherals do: BTP's handshake response goes out
        // once the central has subscribed to C2, which it does after writing
        // the request.
        let mut subscribed = false;
        let mut out = [0u8; 512];
        loop {
            let mtu = peer
                .and_then(|(h, _)| host.connection(h))
                .map(|c| c.att_mtu());
            let mut flush = false;
            tokio::select! {
                event = events.recv() => match event {
                    Ok(HostEvent::GattWrite { handle, attribute, value }) if attribute == c1 => {
                        let addr = host
                            .connection(handle)
                            .map(|c| BtAddr(c.peer().bytes))
                            .ok_or_else(|| io("write from no connection"))?;
                        peer = Some((handle, addr));
                        let mtu = host.connection(handle).map(|c| c.att_mtu());
                        btp.process_incoming(mtu, addr, &value)?;
                    }
                    Ok(HostEvent::GattSubscription { attribute, indicate, .. }) if attribute == c2 => {
                        subscribed = indicate;
                        flush = subscribed;
                    }
                    Ok(_) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                    Err(e) => return Err(io(e)),
                },
                () = btp.wait_outgoing(), if subscribed => flush = true,
            }
            if flush {
                loop {
                    let len = btp.process_outgoing(mtu, &mut out)?;
                    if len == 0 {
                        break;
                    }
                    server.notify(c2, &out[..len]).await.map_err(io)?;
                }
            }
        }
    }
}

const NODE: Node<'static> = Node {
    endpoints: &[
        root_endpoint!(wifi),
        Endpoint::new(
            1,
            devices!(DEV_TYPE_ON_OFF_LIGHT),
            clusters!(desc::DescHandler::CLUSTER, TestOnOffDeviceLogic::CLUSTER),
        ),
    ],
};

fn data_model<'a>(
    matter: &'a Matter<'a>,
    mut rand: impl rand::Rng + Copy,
    on_off: &'a on_off::OnOffHandler<'a, TestOnOffDeviceLogic, on_off::NoLevelControl>,
) -> impl DataModel + 'a {
    (
        NODE,
        EmptyHandler
            .chain(
                |e, _| e == ROOT_ENDPOINT_ID,
                Async(MatterStack::<0, WifiNetworks>::root_handler(
                    matter,
                    &(),
                    rand,
                )),
            )
            .chain(
                |e, c| e == 1 && c == desc::DescHandler::CLUSTER.id,
                Async(desc::DescHandler::new(Dataver::new_rand(&mut rand)).adapt()),
            )
            .chain(
                |e, c| e == 1 && c == TestOnOffDeviceLogic::CLUSTER.id,
                Async(on_off::HandlerAdaptor(on_off)),
            ),
    )
}

/// Run the device on a thread of its own until the process ends.
fn start_device(bt_port: u16, kv_dir: std::path::PathBuf) {
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let local = tokio::task::LocalSet::new();
        local.block_on(&rt, async move {
            // Bumble's simulated link routes data by random address only.
            let host = Host::open(
                &TransportSpec::Tcp(format!("127.0.0.1:{bt_port}")),
                HostConfig {
                    use_public_address: false,
                    ..HostConfig::default()
                },
            )
            .await
            .expect("device controller");

            const BUMP: usize = 32 * 1024;
            let stack: &'static MatterStack<BUMP, WifiNetworks> =
                Box::leak(Box::<MatterStack<BUMP, WifiNetworks>>::new_uninit()).init_with(
                    MatterStack::init(&TEST_DEV_DET, TEST_DEV_COMM, &TEST_DEV_ATT),
                );
            let mut store = DirKvBlobStore::new(kv_dir);
            let crypto = default_crypto(rand::rng(), DAC_PRIVKEY);
            let mut rand = crypto.rand().unwrap();
            stack.startup(&crypto, &mut store).await.unwrap();
            let kv = stack.kv(store);
            let on_off_handler = on_off::OnOffHandler::new_standalone(
                Dataver::new_rand(&mut rand),
                1,
                rs_matter::persist::VENDOR_KEYS_START + 0x10,
                TestOnOffDeviceLogic::new(true),
            );
            let result = stack
                .run_wifi_no_coex(
                    PreexistingWireless::new(
                        StdNetStack::new(),
                        SysNetifs,
                        NoopWirelessNetCtl::new(NetworkType::Wifi),
                        BuiltinMdns::new(),
                        HostGatt(host),
                    ),
                    &crypto,
                    data_model(stack.matter(), rand, &on_off_handler),
                    kv,
                    (),
                )
                .await;
            eprintln!("device stopped: {result:?}");
        });
    });
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs Bumble (BUMBLE_PYTHON) and a network interface"]
async fn commissions_a_device_over_bluetooth() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,matter_controller=debug,bluetooth_host=debug".into()),
        )
        .with_test_writer()
        .try_init();
    let python = std::env::var("BUMBLE_PYTHON").expect("BUMBLE_PYTHON");
    let (controller_port, device_port) = (free_port(), free_port());
    let script = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/bumble/link.py");
    let mut link = tokio::process::Command::new(python)
        .args([
            script,
            &controller_port.to_string(),
            &device_port.to_string(),
        ])
        .stdout(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .expect("bumble");
    let mut lines = BufReader::new(link.stdout.take().unwrap()).lines();
    let ready = tokio::time::timeout(Duration::from_secs(30), lines.next_line())
        .await
        .expect("bumble starts")
        .unwrap();
    assert_eq!(ready.as_deref(), Some("ready"));

    let device_dir = tempfile::tempdir().unwrap();
    start_device(device_port, device_dir.path().to_path_buf());

    let dir = tempfile::tempdir().unwrap();
    let mut config = Config::new(dir.path());
    config.port = 5543;
    config.bluetooth = Some(format!("tcp:127.0.0.1:{controller_port}"));
    config.bluetooth_random_address = true;
    config.interface = std::env::var("MATTER_E2E_INTERFACE").ok();
    let controller = tokio::task::spawn_blocking(move || Controller::start(config))
        .await
        .unwrap()
        .unwrap();

    // A QR code that says Bluetooth only, so the controller looks there.
    let qr = QrPayload::new(
        DiscoveryCapabilities::BLE,
        CommFlowType::Standard,
        TEST_BASIC_COMM.clone(),
        TEST_VID,
        TEST_PID,
        "",
        no_optional_data,
    );
    let mut buf = [0u8; 128];
    let (code, _) = qr.as_str(&mut buf).unwrap();

    let mut events = controller.events();
    let node = tokio::time::timeout(
        Duration::from_secs(120),
        controller.commission(CommissionRequest {
            code: code.to_string(),
            address: None,
            name: Some("ble-light".into()),
            discovery_timeout: 60,
            networks: Networks {
                thread_dataset: None,
                wifi: Some(WiFi {
                    ssid: b"test-network".to_vec(),
                    credentials: b"test-passphrase".to_vec(),
                }),
            },
        }),
    )
    .await
    .expect("commissioning finishes")
    .expect("commissioned over Bluetooth");
    assert!(node.interviewed, "{node:?}");

    // It went through Bluetooth and network setup.
    let mut steps = Vec::new();
    while let Ok(e) = events.try_recv() {
        if let matter_controller::Event::Commissioning { step, .. } = e {
            steps.push(step);
        }
    }
    use matter_controller::CommissionStep as S;
    for want in [
        S::Pase,
        S::NetworkSetup,
        S::NetworkConnect,
        S::CommissioningComplete,
    ] {
        assert!(steps.contains(&want), "{want:?} missing from {steps:?}");
    }

    // And it answers over IP.
    let vendor = controller
        .read(
            node.node_id,
            AttributePath {
                endpoint: Some(0),
                cluster: Some(0x0028),
                attribute: Some(2),
            },
        )
        .await
        .unwrap();
    assert_eq!(vendor[0].value, serde_json::json!(TEST_VID));

    let _ = link.kill().await;
}
