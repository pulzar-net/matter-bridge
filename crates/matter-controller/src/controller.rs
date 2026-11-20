//! The controller service: one thread running rs-matter's node (transport,
//! mDNS, the IM responder that takes subscription reports) and the jobs the
//! [`Controller`] handle sends it.

use std::cell::RefCell;
use std::collections::HashMap;
use std::net::{SocketAddr, UdpSocket};
use std::num::NonZeroU8;
use std::rc::Rc;
use std::time::{Duration, Instant, SystemTime};

use matter_model::{encode_to_vec, Shape};
use rs_matter::acl::{AclEntry, AuthMode, Target};
use rs_matter::attest::DeviceAttestation;
use rs_matter::basic_info::BasicInfoConfig;
use rs_matter::bdx::{Bdx, PROTO_ID_BDX};
use rs_matter::cert::gen::VALID_FOREVER;
use rs_matter::cert::{MAX_CERT_TLV_AND_ASN1_LEN, MAX_CERT_TLV_LEN};
use rs_matter::crypto::{
    default_crypto, CanonAeadKeyRef, CanonPkcPublicKeyRef, CanonPkcSecretKeyRef, Crypto,
};
use rs_matter::dm::clusters::desc::{self, ClusterHandler as _};
use rs_matter::dm::clusters::ota_prov::{OtaBdxHandler, OtaProviderHandler};
use rs_matter::dm::devices::DEV_TYPE_OTA_PROVIDER;
use rs_matter::dm::{
    Async, Cluster, Dataver, EmptyHandler, Endpoint as DmEndpoint, Node as DmNode, Privilege,
};
use rs_matter::im::PROTO_ID_INTERACTION_MODEL;
use rs_matter::im::{
    AttrDataTag, AttrPath, CmdDataTag, CmdResp, EventFilter, EventPath, GenericPath, IMStatusCode,
    ImClient, InteractionModel, InteractionModelState, ReportContext, ReportDataHandler,
    ReportDataResp, SubscribeOutcome,
};
use rs_matter::onboard::noc::NocGenerator;
use rs_matter::onboard::{CommissionHooks, CommissionOptions, Commissioner};
use rs_matter::pairing::qr::{no_optional_data, CommFlowType, QrPayload};
use rs_matter::pairing::{BasicCommData, DevCommData, DiscoveryCapabilities};
use rs_matter::persist::DummyKvBlobStore;
use rs_matter::respond::ChainedExchangeHandler;
use rs_matter::respond::Responder;
use rs_matter::sc::pase::{Spake2pVerifierPassword, Spake2pVerifierPasswordRef};
use rs_matter::sc::ExchangeInitiator as _;
use rs_matter::sc::SecureChannel;
use rs_matter::tlv::{TLVElement, TLVTag, ToTLV as _};
use rs_matter::transport::exchange::{Exchange, MatterBuffers};
use rs_matter::transport::network::mdns::{MdnsBrowseMatch, MdnsBrowseQuery};
use rs_matter::transport::network::{Address, NoNetwork};
use rs_matter::utils::init::InitMaybeUninit as _;
use rs_matter::Matter;
use serde_json::{json, Value};
use tokio::sync::{mpsc, oneshot};

use crate::ca::{AnyCa, CertificateAuthority, FileCa, IntermediateCa};
use crate::identity::Identity;
use crate::onboarding::Onboarding;
use crate::ota::{OtaDir, OtaImage};

#[cfg(feature = "ble")]
use crate::ble::Link;
/// Without Bluetooth there is never a link to close.
#[cfg(not(feature = "ble"))]
enum Link {}
#[cfg(not(feature = "ble"))]
impl Link {
    async fn close(self) {}
}
use crate::reports::{items, Collected};
use crate::store::{BasicInfo, DeviceType, Endpoint, Node, NodeStore};
use crate::{
    AttributePath, AttributeValue, CommissionRequest, CommissionStep, Commissionable,
    CommissioningWindow, Config, ControllerInfo, Error, Event,
};

const ROOT_ENDPOINT: u16 = 0;
const DESCRIPTOR: u32 = 0x001D;
const BASIC_INFORMATION: u32 = 0x0028;
const OPERATIONAL_CREDENTIALS: u32 = 0x003E;
const ADMINISTRATOR_COMMISSIONING: u32 = 0x003C;
const GROUPS: u32 = 0x0004;
const GROUP_KEY_MANAGEMENT: u32 = 0x003F;
const GROUPCAST: u32 = 0x0065;
const ACCESS_CONTROL: u32 = 0x001F;
const NETWORK_COMMISSIONING: u32 = 0x0031;
const OTA_REQUESTOR: u32 = 0x002A;
/// OTA Software Update Requestor's AnnounceOTAProvider.
const ANNOUNCE_OTA_PROVIDER: u32 = 0x0000;

/// The OTA Provider cluster as rs-matter's handler serves it.
const OTA_PROVIDER: Cluster<'static> = <OtaProviderHandler<&OtaDir> as rs_matter::dm::clusters::ota_prov::ClusterAsyncHandler>::CLUSTER;

/// The controller node's data model: an OTA provider on the root endpoint.
const CONTROLLER_NODE: DmNode<'static> = DmNode {
    endpoints: &[DmEndpoint::new(
        ROOT_ENDPOINT,
        &[DEV_TYPE_OTA_PROVIDER],
        &[desc::DescHandler::CLUSTER, OTA_PROVIDER],
    )],
};
const ADD_WIFI_NETWORK: u32 = 0x02;
const ADD_THREAD_NETWORK: u32 = 0x03;
const CONNECT_NETWORK: u32 = 0x06;
/// The timed-interaction timeout for commands and writes that need one.
const TIMED_MS: u16 = 10_000;
/// The largest request payload we encode.
const MAX_PAYLOAD: usize = 1024;

/// The handle to a running controller. Cheap to clone; the controller stops
/// when the last handle is dropped.
#[derive(Clone)]
pub struct Controller {
    jobs: mpsc::UnboundedSender<Job>,
    events: EventBus,
    info: ControllerInfo,
    ota_dir: std::path::PathBuf,
}

type Reply<T> = oneshot::Sender<Result<T, Error>>;

/// Hands every [`Event`] to every receiver [`Controller::events`] gave out,
/// without loss; receivers that were dropped are forgotten.
#[derive(Clone, Default)]
pub(crate) struct EventBus(std::sync::Arc<std::sync::Mutex<Vec<mpsc::UnboundedSender<Event>>>>);

impl EventBus {
    fn subscribe(&self) -> mpsc::UnboundedReceiver<Event> {
        let (tx, rx) = mpsc::unbounded_channel();
        self.0.lock().unwrap().push(tx);
        rx
    }

    pub(crate) fn send(&self, event: Event) {
        let mut subscribers = self.0.lock().unwrap();
        subscribers.retain(|s| !s.is_closed());
        if let Some((last, rest)) = subscribers.split_last() {
            for s in rest {
                let _ = s.send(event.clone());
            }
            let _ = last.send(event);
        }
    }
}

enum Job {
    Commission(CommissionRequest, Reply<Node>),
    Read(u64, AttributePath, Reply<Vec<AttributeValue>>),
    Write(u64, u16, u32, u32, Value, Option<u16>, Reply<()>),
    Invoke(u64, u16, u32, u32, Value, Option<u16>, Reply<Value>),
    OpenWindow(u64, u16, Reply<CommissioningWindow>),
    Remove(u64, bool, Reply<()>),
    Rename(u64, String, Reply<Node>),
    Interview(u64, Reply<Node>),
    Discover(u16, Reply<Vec<Commissionable>>),
    Nodes(Reply<Vec<Node>>),
    Abort(Reply<bool>),
    AddToGroup(u64, u16, u16, String, Reply<()>),
    RemoveFromGroup(u64, u16, u16, Reply<()>),
    GroupInvoke(u16, u32, u32, Value, Reply<()>),
    AnnounceOta(u64, Reply<Option<OtaImage>>),
}

impl Controller {
    /// Start a controller with `config`: open (or create) its fabric, CA,
    /// identity and node store, and start subscribing to every node.
    pub fn start(config: Config) -> Result<Self, Error> {
        let (jobs_tx, jobs_rx) = mpsc::unbounded_channel();
        let events = EventBus::default();
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();

        let thread_events = events.clone();
        let ota_dir = config.ota_dir.clone();
        std::thread::Builder::new()
            .name("matter".into())
            .spawn(move || {
                let rt = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(rt) => rt,
                    Err(e) => {
                        let _ = ready_tx.send(Err(Error::Io(e)));
                        return;
                    }
                };
                let local = tokio::task::LocalSet::new();
                local.block_on(&rt, async move {
                    if let Err(e) = run(config, jobs_rx, thread_events, &ready_tx).await {
                        log::error!("controller stopped: {e}");
                        let _ = ready_tx.send(Err(e));
                    }
                });
            })?;

        let info = ready_rx.recv().map_err(|_| Error::Stopped)??;
        Ok(Self {
            jobs: jobs_tx,
            events,
            info,
            ota_dir,
        })
    }

    /// The controller's identity on its fabric.
    pub fn info(&self) -> &ControllerInfo {
        &self.info
    }

    /// Receive the controller's [`Event`]s from now on.
    ///
    /// Delivery is lossless: a subscription's priming report can carry
    /// thousands of attributes at once, and each must reach its consumer.
    pub fn events(&self) -> mpsc::UnboundedReceiver<Event> {
        self.events.subscribe()
    }

    async fn call<T>(&self, job: impl FnOnce(Reply<T>) -> Job) -> Result<T, Error> {
        let (tx, rx) = oneshot::channel();
        self.jobs.send(job(tx)).map_err(|_| Error::Stopped)?;
        rx.await.map_err(|_| Error::Stopped)?
    }

    /// Commission a device: find it, add it to the fabric, interview it and
    /// subscribe to it.
    pub async fn commission(&self, req: CommissionRequest) -> Result<Node, Error> {
        self.call(|r| Job::Commission(req, r)).await
    }

    /// Read attributes; wildcards read several.
    pub async fn read(&self, node: u64, path: AttributePath) -> Result<Vec<AttributeValue>, Error> {
        self.call(|r| Job::Read(node, path, r)).await
    }

    /// Write one attribute. A write the data model says must be timed is.
    pub async fn write(
        &self,
        node: u64,
        endpoint: u16,
        cluster: u32,
        attribute: u32,
        value: Value,
        timed_ms: Option<u16>,
    ) -> Result<(), Error> {
        self.call(|r| Job::Write(node, endpoint, cluster, attribute, value, timed_ms, r))
            .await
    }

    /// Invoke a command with `fields` (an object, or null for none) and
    /// return the response command's fields, or null for a plain success.
    pub async fn invoke(
        &self,
        node: u64,
        endpoint: u16,
        cluster: u32,
        command: u32,
        fields: Value,
        timed_ms: Option<u16>,
    ) -> Result<Value, Error> {
        self.call(|r| Job::Invoke(node, endpoint, cluster, command, fields, timed_ms, r))
            .await
    }

    /// Open an enhanced commissioning window on `node` for `timeout` seconds
    /// (180 to 900), so another administrator can commission it too.
    pub async fn open_commissioning_window(
        &self,
        node: u64,
        timeout: u16,
    ) -> Result<CommissioningWindow, Error> {
        self.call(|r| Job::OpenWindow(node, timeout, r)).await
    }

    /// Remove a node: take our fabric off it, then forget it. With `force`,
    /// forget it even if it cannot be reached.
    pub async fn remove(&self, node: u64, force: bool) -> Result<(), Error> {
        self.call(|r| Job::Remove(node, force, r)).await
    }

    /// Rename a node.
    pub async fn rename(&self, node: u64, name: String) -> Result<Node, Error> {
        self.call(|r| Job::Rename(node, name, r)).await
    }

    /// Read what a node is again (Basic Information, Descriptor).
    pub async fn interview(&self, node: u64) -> Result<Node, Error> {
        self.call(|r| Job::Interview(node, r)).await
    }

    /// Look for commissionable nodes for `seconds`.
    pub async fn discover(&self, seconds: u16) -> Result<Vec<Commissionable>, Error> {
        self.call(|r| Job::Discover(seconds, r)).await
    }

    /// Put an endpoint of `node` in `group` (named `name` on the node): the
    /// node gets the fabric's group key and the group, so a group command
    /// reaches it.
    pub async fn add_to_group(
        &self,
        node: u64,
        endpoint: u16,
        group: u16,
        name: String,
    ) -> Result<(), Error> {
        self.call(|r| Job::AddToGroup(node, endpoint, group, name, r))
            .await
    }

    /// Take an endpoint of `node` out of `group`.
    pub async fn remove_from_group(
        &self,
        node: u64,
        endpoint: u16,
        group: u16,
    ) -> Result<(), Error> {
        self.call(|r| Job::RemoveFromGroup(node, endpoint, group, r))
            .await
    }

    /// Send a command to every member of `group` at once, as one multicast
    /// message (no response comes back from a group).
    pub async fn group_invoke(
        &self,
        group: u16,
        cluster: u32,
        command: u32,
        fields: Value,
    ) -> Result<(), Error> {
        self.call(|r| Job::GroupInvoke(group, cluster, command, fields, r))
            .await
    }

    /// The OTA image files the controller offers (see [`Config::ota_dir`]).
    pub fn ota_images(&self) -> Result<Vec<OtaImage>, Error> {
        crate::ota::list(&self.ota_dir)
    }

    /// Tell `node` to ask this controller for a software update, if an image
    /// in the OTA directory is newer for it; the image it will be offered,
    /// or `None` (and no announcement) when there is none. The update's
    /// steps arrive as [`Event::Ota`] and the node's own OTA Requestor
    /// events.
    pub async fn announce_ota(&self, node: u64) -> Result<Option<OtaImage>, Error> {
        self.call(|r| Job::AnnounceOta(node, r)).await
    }

    /// Stop the commissioning in progress, rolling the device back;
    /// whether one was running.
    pub async fn abort_commissioning(&self) -> Result<bool, Error> {
        self.call(Job::Abort).await
    }

    /// Every node in the store.
    pub async fn nodes(&self) -> Result<Vec<Node>, Error> {
        self.call(Job::Nodes).await
    }
}

/// The controller node's own Basic Information. Nobody commissions it; the
/// values only fill the cluster it serves.
static CONTROLLER_INFO: BasicInfoConfig<'static> = BasicInfoConfig {
    vid: 0xFFF1,
    pid: 0x8000,
    hw_ver: 1,
    hw_ver_str: "1",
    sw_ver: 1,
    sw_ver_str: env!("CARGO_PKG_VERSION"),
    serial_no: "",
    vendor_name: "matter-bridge",
    product_name: "matter-bridge controller",
    device_name: "matter-bridge",
    ..BasicInfoConfig::new()
};

/// A controller has no device attestation of its own.
struct NoAttestation;

static ZERO_PUBLIC_KEY: [u8; 65] = [0; 65];
static ZERO_SECRET_KEY: [u8; 32] = [0; 32];

impl DeviceAttestation for NoAttestation {
    fn cert_declaration(&self) -> &[u8] {
        &[]
    }
    fn pai(&self) -> &[u8] {
        &[]
    }
    fn dac(&self) -> &[u8] {
        &[]
    }
    fn dac_pub_key(&self) -> CanonPkcPublicKeyRef<'_> {
        CanonPkcPublicKeyRef::new(&ZERO_PUBLIC_KEY)
    }
    fn dac_priv_key(&self) -> CanonPkcSecretKeyRef<'_> {
        CanonPkcSecretKeyRef::new(&ZERO_SECRET_KEY)
    }
}

static NO_ATTESTATION: NoAttestation = NoAttestation;

/// The state every job shares, on the controller thread.
struct Ctx<C: 'static> {
    matter: &'static Matter<'static>,
    crypto: C,
    fab_idx: NonZeroU8,
    ca: AnyCa,
    store: RefCell<NodeStore>,
    events: EventBus,
    config: Config,
    subs: Rc<Subscriptions>,
    tasks: RefCell<HashMap<u64, tokio::task::JoinHandle<()>>>,
    commissioning: tokio::sync::Mutex<()>,
    abort: tokio::sync::Notify,
    kv: &'static rs_matter::persist::MatterKvBlobStoreAccess<'static, DummyKvBlobStore>,
    groups: RefCell<crate::groups::GroupKey>,
    #[cfg(feature = "ble")]
    ble: Option<crate::ble::Ble>,
}

/// The subscriptions we hold, by node: which subscription id is current and
/// when it last reported. The report handler checks reports against it.
#[derive(Default)]
struct Subscriptions {
    by_node: RefCell<HashMap<u64, SubState>>,
    /// Reports split over several messages, put together until the last.
    partial: RefCell<HashMap<u64, Collected>>,
}

struct SubState {
    id: Option<u32>,
    last_report: Instant,
    last_event: Option<u64>,
}

async fn run(
    config: Config,
    mut jobs: mpsc::UnboundedReceiver<Job>,
    events: EventBus,
    ready: &std::sync::mpsc::Sender<Result<ControllerInfo, Error>>,
) -> Result<(), Error> {
    std::fs::create_dir_all(&config.state_dir)?;

    // The crypto backend's "singleton" key is a device's attestation key; a
    // controller never uses it.
    let singleton: &'static [u8; 32] = Box::leak(Box::new(rand::random()));
    let crypto: &'static _ = Box::leak(Box::new(default_crypto(
        rand::rng(),
        CanonPkcSecretKeyRef::new(singleton),
    )));

    let ca = match config.ca {
        crate::CaChoice::File => AnyCa::File(FileCa::open_or_create(
            &config.state_dir.join("ca.json"),
            config.fabric_id,
            crypto,
        )?),
        crate::CaChoice::Intermediate => AnyCa::Intermediate(IntermediateCa::open(
            &config.state_dir.join("intermediate.json"),
        )?),
    };
    let identity =
        Identity::load_or_create(&config.state_dir.join("controller.json"), &ca, crypto).await?;
    let store = NodeStore::open(&config.state_dir.join("nodes.json"))?;

    // The controller node's commissioning passcode is never used (it opens no
    // window), but must be set; a random one keeps it unguessable.
    let passcode: u32 = 1 + rand::random::<u32>() % 99_999_997;
    let dev_comm = DevCommData::Passcode(BasicCommData {
        password: Spake2pVerifierPassword::new_from_ref(Spake2pVerifierPasswordRef::new(
            &passcode.to_le_bytes(),
        )),
        discriminator: rand::random::<u16>() & 0x0FFF,
    });

    // Matter is initialized in place: it is too large for the stack.
    let matter: &'static Matter<'static> = Box::leak(Box::<Matter>::new_uninit()).init_with(
        Matter::init(&CONTROLLER_INFO, dev_comm, &NO_ATTESTATION, config.port),
    );

    let fab_idx = matter.with_state(|state| {
        state
            .fabrics
            .add(
                crypto,
                CanonPkcSecretKeyRef::new(&identity.key),
                ca.root_cert(),
                &identity.noc,
                ca.intermediate_cert().unwrap_or(&[]),
                Some(CanonAeadKeyRef::new(ca.ipk())),
                config.vendor_id,
                identity.node_id,
            )
            .map(|f| f.fab_idx())
    })?;

    let compressed_fabric_id = matter.with_state(|state| {
        state
            .fabrics
            .fabric(fab_idx)
            .map(|f| f.compressed_fabric_id())
    })?;

    let info = ControllerInfo {
        fabric_id: ca.fabric_id(),
        compressed_fabric_id,
        node_id: identity.node_id,
        vendor_id: config.vendor_id,
    };

    // Matter over UDP, IPv6 and IPv4.
    let socket = async_io::Async::<UdpSocket>::bind(SocketAddr::from((
        std::net::Ipv6Addr::UNSPECIFIED,
        config.port,
    )))?;

    // BTP, for commissioning over Bluetooth: rs-matter's transport sends
    // `Address::Btp` packets through it, everything else over UDP.
    #[cfg(feature = "ble")]
    let btp: &'static rs_matter::transport::network::btp::Btp =
        Box::leak(Box::<rs_matter::transport::network::btp::Btp>::new_uninit())
            .init_with(rs_matter::transport::network::btp::Btp::init());
    #[cfg(feature = "ble")]
    let ble = match &config.bluetooth {
        Some(spec) => {
            Some(crate::ble::Ble::open(spec, config.bluetooth_random_address, btp).await?)
        }
        None => None,
    };
    #[cfg(not(feature = "ble"))]
    if config.bluetooth.is_some() {
        return Err(Error::Unsupported(
            "Bluetooth: this build has no `ble` feature".into(),
        ));
    }

    let subs = Rc::new(Subscriptions::default());

    // The controller's own data model serves the OTA provider; its
    // interaction model also takes the reports of the subscriptions we hold.
    std::fs::create_dir_all(&config.ota_dir)?;
    let ota: &'static OtaDir = Box::leak(Box::new(OtaDir::new(
        config.ota_dir.clone(),
        events.clone(),
    )));
    // Every node of the fabric may ask for an update.
    matter.with_state(|state| {
        let mut entry = AclEntry::new(None, Privilege::OPERATE, AuthMode::Case);
        entry.add_target(Target::new(
            Some(ROOT_ENDPOINT),
            Some(OTA_PROVIDER.id),
            None,
        ))?;
        state
            .fabrics
            .fabric_mut(fab_idx)?
            .acl_add(entry)
            .map(|_| ())
    })?;
    let buffers: &'static MatterBuffers = Box::leak(Box::new(MatterBuffers::new()));
    let im_state: &'static InteractionModelState<1, 1024> =
        Box::leak(Box::new(InteractionModelState::new()));
    let kv: &'static _ = Box::leak(Box::new(matter.kv(DummyKvBlobStore)));

    // The fabric's group key, and the groups this controller sends to.
    let groups = crate::groups::GroupKey::load_or_create(&config.state_dir.join("groups.json"))?;
    groups.install(matter, fab_idx)?;
    let sink = ReportSink {
        subs: subs.clone(),
        events: events.clone(),
    };
    let im: &'static _ = Box::leak(Box::new(InteractionModel::new_with_reports(
        matter,
        crypto,
        buffers,
        (
            CONTROLLER_NODE,
            EmptyHandler
                .chain(
                    |e, c| e == ROOT_ENDPOINT && c == desc::DescHandler::CLUSTER.id,
                    Async(desc::DescHandler::new(Dataver::new(rand::random())).adapt()),
                )
                .chain(
                    |e, c| e == ROOT_ENDPOINT && c == OTA_PROVIDER.id,
                    OtaProviderHandler::new(Dataver::new(rand::random()), matter, ota).adapt(),
                ),
        ),
        kv,
        sink,
        im_state,
    )));
    // The interaction model and secure channel, and BDX for the images.
    let bdx_buffers: &'static MatterBuffers<2> =
        Box::leak(Box::<MatterBuffers<2>>::new_uninit()).init_with(MatterBuffers::init());
    let responder = Responder::new(
        "Responder",
        ChainedExchangeHandler::new(
            PROTO_ID_INTERACTION_MODEL,
            im,
            SecureChannel::new(matter, crypto, im),
        )
        .chain(PROTO_ID_BDX, Bdx::new(OtaBdxHandler::new(bdx_buffers, ota))),
        matter,
        0,
    );

    let ctx = Rc::new(Ctx {
        matter,
        crypto,
        fab_idx,
        ca,
        store: RefCell::new(store),
        events,
        config: config.clone(),
        subs,
        tasks: RefCell::new(HashMap::new()),
        commissioning: tokio::sync::Mutex::new(()),
        abort: tokio::sync::Notify::new(),
        kv,
        groups: RefCell::new(groups),
        #[cfg(feature = "ble")]
        ble,
    });

    let _ = ready.send(Ok(info));

    let node_ids: Vec<u64> = ctx.store.borrow().nodes().map(|n| n.node_id).collect();
    for node_id in node_ids {
        ctx.start_subscription(node_id);
    }

    let serve = async {
        while let Some(job) = jobs.recv().await {
            let ctx = ctx.clone();
            tokio::task::spawn_local(async move { ctx.handle(job).await });
        }
    };

    let mdns = crate::mdns::run(matter, crypto, config.interface.as_deref());

    tokio::select! {
        r = run_transport(
            matter,
            crypto,
            &socket,
            #[cfg(feature = "ble")]
            btp,
        ) => r,
        r = mdns => r,
        r = responder.run::<4>() => r.map_err(Error::from),
        () = serve => Ok(()),
    }
}

/// Run Matter's transport: UDP, and BTP for commissioning over Bluetooth.
async fn run_transport<C: Crypto>(
    matter: &Matter<'_>,
    crypto: C,
    socket: &async_io::Async<UdpSocket>,
    #[cfg(feature = "ble")] btp: &rs_matter::transport::network::btp::Btp,
) -> Result<(), Error> {
    #[cfg(feature = "ble")]
    {
        use rs_matter::transport::network::ChainedNetwork;
        let mut send = ChainedNetwork::new(|a: &Address| a.is_btp(), btp, socket);
        let mut recv = ChainedNetwork::new(|a: &Address| a.is_btp(), btp, socket);
        matter
            .run(crypto, &mut send, &mut recv, NoNetwork)
            .await
            .map_err(Error::from)
    }
    #[cfg(not(feature = "ble"))]
    {
        matter
            .run(crypto, socket, socket, NoNetwork)
            .await
            .map_err(Error::from)
    }
}

/// Takes the reports of our subscriptions, as the controller node's IM
/// receives them.
struct ReportSink {
    subs: Rc<Subscriptions>,
    events: EventBus,
}

impl ReportDataHandler for ReportSink {
    async fn handle_report(
        &self,
        ctx: impl ReportContext,
        report: &ReportDataResp<'_>,
    ) -> Result<(), IMStatusCode> {
        let sub = ctx.subscription();
        let node = sub.peer_node_id;

        {
            let mut by_node = self.subs.by_node.borrow_mut();
            match by_node.get_mut(&node) {
                Some(state) if state.id.is_none() || state.id == sub.subscription_id => {
                    state.last_report = Instant::now();
                }
                // Not a subscription we hold (an old one, or another
                // controller's): tell the node to drop it.
                _ => return Err(IMStatusCode::InvalidSubscription),
            }
        }

        let mut partial = self.subs.partial.borrow_mut();
        let collected = partial.entry(node).or_default();
        collected.add_chunk(node, report);
        if report.more_chunks.unwrap_or(false) {
            return Ok(());
        }
        let collected = partial.remove(&node).unwrap_or_default();
        drop(partial);

        if let Some(last) = collected.events.iter().map(|e| e.number).max() {
            if let Some(state) = self.subs.by_node.borrow_mut().get_mut(&node) {
                state.last_event = Some(last);
            }
        }
        publish(&self.events, collected);
        Ok(())
    }
}

fn publish(events: &EventBus, collected: Collected) {
    for a in collected.attributes {
        events.send(Event::Attribute(a));
    }
    for e in collected.events {
        events.send(Event::Event(e));
    }
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Reports commissioning steps as events.
struct Hooks<'a> {
    node: u64,
    events: &'a EventBus,
    abort: &'a tokio::sync::Notify,
}

impl Hooks<'_> {
    fn emit(&self, step: CommissionStep) {
        self.events.send(Event::Commissioning {
            node: self.node,
            step,
        });
    }
}

impl CommissionHooks for Hooks<'_> {
    fn step(&self, step: rs_matter::onboard::CommissionStep) {
        use rs_matter::onboard::CommissionStep as S;
        let step = match step {
            S::Pase => CommissionStep::Pase,
            S::ArmFailSafe => CommissionStep::ArmFailSafe,
            S::DeviceAttestation => CommissionStep::DeviceAttestation,
            S::CsrRequest => CommissionStep::CsrRequest,
            S::AddTrustedRootCertificate => CommissionStep::AddTrustedRootCertificate,
            S::AddNoc => CommissionStep::AddNoc,
            S::FindOperational => CommissionStep::FindOperational,
            S::CommissioningComplete => CommissionStep::CommissioningComplete,
            S::Cleanup => CommissionStep::Cleanup,
        };
        self.events.send(Event::Commissioning {
            node: self.node,
            step,
        });
    }

    async fn cancelled(&self) {
        self.abort.notified().await
    }
}

impl<C: Crypto + Copy + 'static> Ctx<C> {
    async fn handle(self: Rc<Self>, job: Job) {
        match job {
            Job::Commission(req, r) => {
                let _ = r.send(self.commission(req).await);
            }
            Job::Read(node, path, r) => {
                let _ = r.send(self.read(node, path).await);
            }
            Job::Write(node, ep, cl, at, value, timed, r) => {
                let _ = r.send(self.write(node, ep, cl, at, &value, timed).await);
            }
            Job::Invoke(node, ep, cl, cmd, fields, timed, r) => {
                let _ = r.send(self.invoke(node, ep, cl, cmd, &fields, timed).await);
            }
            Job::OpenWindow(node, timeout, r) => {
                let _ = r.send(self.open_window(node, timeout).await);
            }
            Job::Remove(node, force, r) => {
                let _ = r.send(self.remove(node, force).await);
            }
            Job::Rename(node, name, r) => {
                let _ = r.send(self.rename(node, name));
            }
            Job::Interview(node, r) => {
                let _ = r.send(self.interview(node).await);
            }
            Job::Discover(seconds, r) => {
                let _ = r.send(self.discover(seconds).await);
            }
            Job::Nodes(r) => {
                let _ = r.send(Ok(self.store.borrow().nodes().cloned().collect()));
            }
            Job::AddToGroup(node, ep, group, name, r) => {
                let _ = r.send(self.add_to_group(node, ep, group, &name).await);
            }
            Job::RemoveFromGroup(node, ep, group, r) => {
                let _ = r.send(self.remove_from_group(node, ep, group).await);
            }
            Job::GroupInvoke(group, cl, cmd, fields, r) => {
                let _ = r.send(self.group_invoke(group, cl, cmd, &fields).await);
            }
            Job::AnnounceOta(node, r) => {
                let _ = r.send(self.announce_ota(node).await);
            }
            Job::Abort(r) => {
                // A commissioning holds the lock; with none running there is
                // nothing to stop.
                let running = self.commissioning.try_lock().is_err();
                if running {
                    self.abort.notify_waiters();
                }
                let _ = r.send(Ok(running));
            }
        }
    }

    fn known(&self, node: u64) -> Result<Node, Error> {
        self.store
            .borrow()
            .get(node)
            .cloned()
            .ok_or_else(|| Error::Invalid(format!("no node {node}")))
    }

    async fn exchange(&self, node: u64) -> Result<Exchange<'static>, Error> {
        Ok(Exchange::initiate(self.matter, self.crypto, self.fab_idx, node).await?)
    }

    async fn commission(self: &Rc<Self>, req: CommissionRequest) -> Result<Node, Error> {
        let _one_at_a_time = self.commissioning.lock().await;
        let onboarding = Onboarding::parse(&req.code)?;
        if onboarding.custom_flow {
            log::warn!("the device needs a custom commissioning flow; it may have to be put in pairing mode first");
        }

        let node_id = self.ca.allocate_node_id().await?;
        let hooks = Hooks {
            node: node_id,
            events: &self.events,
            abort: &self.abort,
        };
        self.events.send(Event::Commissioning {
            node: node_id,
            step: CommissionStep::Discovery,
        });

        let (address, link) = match req.address {
            Some(addr) => (Address::Udp(addr), None),
            None => self.locate(&onboarding, req.discovery_timeout).await?,
        };
        log::info!("commissioning {address:?} as node {node_id:016X}");

        let signing_key = self.ca.signing_key().ok_or_else(|| {
            Error::Unsupported(
                "commissioning with a CA whose key is elsewhere needs rs-matter's commissioner to take an async NOC issuer".into(),
            )
        })?;
        let mut noc_buf = vec![0u8; MAX_CERT_TLV_AND_ASN1_LEN];
        let mut noc_gen = NocGenerator::create(
            CanonPkcSecretKeyRef::new(signing_key),
            self.ca.root_cert(),
            self.ca.intermediate_cert().unwrap_or(&[]),
            &mut noc_buf,
        )?;
        let mut buf = vec![0u8; MAX_CERT_TLV_LEN];
        let mut commissioner = Commissioner::new(
            self.matter,
            self.crypto,
            self.fab_idx,
            &mut noc_gen,
            &mut buf,
        );

        if self.config.allow_unverified_attestation {
            log::warn!("device attestation is not verified (not implemented in rs-matter yet)");
        }
        let opts = CommissionOptions {
            allow_test_attestation: self.config.allow_unverified_attestation,
            // Joining a Thread or Wi-Fi network takes a while inside the
            // fail-safe.
            fail_safe_secs: if address.is_btp() { 180 } else { 60 },
        };

        let commissioned = async {
            let phase1 = commissioner
                .commission(
                    address,
                    onboarding.passcode,
                    &opts,
                    node_id,
                    VALID_FOREVER,
                    &hooks,
                )
                .await?;
            // Over Bluetooth the device is not on an operational network
            // yet: it gets one before CASE, which runs over IP.
            if address.is_btp() {
                let pase = phase1.pase_session_id;
                if let Err(e) = self.network_commission(pase, &req.networks, &hooks).await {
                    let _ = commissioner.disarm_fail_safe(pase).await;
                    commissioner.end_pase(pase);
                    return Err(e);
                }
            }
            commissioner.complete_via_case(&phase1, &hooks).await?;
            Ok::<_, Error>(())
        }
        .await;
        if let Some(link) = link {
            link.close().await;
        }
        commissioned?;

        let name = {
            let store = self.store.borrow();
            match req.name {
                Some(name) => name,
                None => store.unique_name(&Node::default_name(node_id)),
            }
        };
        let node = Node {
            node_id,
            name,
            basic: BasicInfo::default(),
            endpoints: Default::default(),
            interviewed: false,
            commissioned_at: now_secs(),
        };
        self.store.borrow_mut().put(node.clone())?;
        self.events.send(Event::NodeUpdated(node.clone()));

        self.events.send(Event::Commissioning {
            node: node_id,
            step: CommissionStep::Interview,
        });
        let node = match self.interview(node_id).await {
            Ok(node) => node,
            Err(e) => {
                log::warn!("node {node_id:016X}: interview failed, will retry: {e}");
                node
            }
        };
        self.start_subscription(node_id);
        Ok(node)
    }

    /// Give a device being commissioned over Bluetooth its operational
    /// network, over the PASE session and inside the fail-safe: the kind its
    /// Network Commissioning cluster supports (Thread or Wi-Fi), added, then
    /// connected. An Ethernet device needs nothing.
    #[cfg_attr(not(feature = "ble"), allow(dead_code))]
    async fn network_commission(
        &self,
        pase_session: u32,
        networks: &crate::Networks,
        hooks: &Hooks<'_>,
    ) -> Result<(), Error> {
        const WIFI: u64 = 1 << 0;
        const THREAD: u64 = 1 << 1;
        const ETHERNET: u64 = 1 << 2;

        let pase_exchange =
            || Exchange::initiate_for_session(self.matter.transport(), self.crypto, pase_session);

        hooks.emit(CommissionStep::NetworkSetup);
        let features = self
            .read_on(
                pase_exchange()?,
                AttributePath {
                    endpoint: Some(ROOT_ENDPOINT),
                    cluster: Some(NETWORK_COMMISSIONING),
                    attribute: Some(0xFFFC), // FeatureMap
                },
            )
            .await?
            .first()
            .and_then(|a| a.value.as_u64())
            .ok_or_else(|| {
                Error::Invalid("the device has no Network Commissioning cluster".into())
            })?;

        let (add, add_fields, network_id) = if features & THREAD != 0 {
            let dataset = networks.thread_dataset.as_ref().ok_or_else(|| {
                Error::Invalid("the device joins a Thread network: give a Thread dataset".into())
            })?;
            let ext_pan_id = crate::network::thread_extended_pan_id(dataset)?;
            log::info!(
                "giving the device Thread network {}",
                hex::encode(ext_pan_id)
            );
            (
                ADD_THREAD_NETWORK,
                json!({ "0": hex::encode(dataset), "1": 1 }),
                ext_pan_id.to_vec(),
            )
        } else if features & WIFI != 0 {
            let wifi = networks.wifi.as_ref().ok_or_else(|| {
                Error::Invalid(
                    "the device joins a Wi-Fi network: give its SSID and passphrase".into(),
                )
            })?;
            log::info!(
                "giving the device Wi-Fi network {}",
                String::from_utf8_lossy(&wifi.ssid)
            );
            (
                ADD_WIFI_NETWORK,
                json!({ "0": hex::encode(&wifi.ssid), "1": hex::encode(&wifi.credentials), "2": 1 }),
                wifi.ssid.clone(),
            )
        } else if features & ETHERNET != 0 {
            return Ok(());
        } else {
            return Err(Error::Invalid(format!(
                "the device's network interface is not one this controller knows (features {features:#x})"
            )));
        };

        let response = self
            .invoke_on(
                pase_exchange()?,
                ROOT_ENDPOINT,
                NETWORK_COMMISSIONING,
                add,
                &add_fields,
                None,
            )
            .await?;
        networking_status("adding the network", &response)?;

        hooks.emit(CommissionStep::NetworkConnect);
        let response = self
            .invoke_on(
                pase_exchange()?,
                ROOT_ENDPOINT,
                NETWORK_COMMISSIONING,
                CONNECT_NETWORK,
                &json!({ "0": hex::encode(network_id), "1": 2 }),
                None,
            )
            .await?;
        networking_status("connecting to the network", &response)
    }

    /// Find a device to commission: on the IP network by DNS-SD, or over
    /// Bluetooth LE when the controller has Bluetooth, as its onboarding
    /// payload says it can be found (a manual code does not say: IP first,
    /// then Bluetooth).
    async fn locate(
        &self,
        onboarding: &Onboarding,
        timeout: u16,
    ) -> Result<(Address, Option<Link>), Error> {
        let caps = onboarding.capabilities;
        let try_ip = caps.is_none_or(|c| c.on_network);
        #[cfg(feature = "ble")]
        if let Some(ble) = &self.ble {
            let try_ble = caps.is_none_or(|c| c.ble);
            if try_ble {
                let total = Duration::from_secs(timeout.max(2).into());
                if try_ip {
                    let ip_secs = (timeout / 2).clamp(1, 10);
                    if let Ok(address) = self.find(onboarding, ip_secs).await {
                        return Ok((address, None));
                    }
                }
                let peer = ble.find(&onboarding.filter(), total).await?;
                let (addr, link) = ble.connect(peer).await?;
                log::info!("reached the device over Bluetooth ({addr})");
                return Ok((Address::Btp(addr), Some(link)));
            }
        }
        if !try_ip {
            return Err(Error::Unsupported(
                "the device can only be found over Bluetooth, which this controller has not got"
                    .into(),
            ));
        }
        Ok((self.find(onboarding, timeout).await?, None))
    }

    /// Find the commissionable node an onboarding payload describes.
    async fn find(&self, onboarding: &Onboarding, timeout: u16) -> Result<Address, Error> {
        let transport = self.matter.transport();
        let query = MdnsBrowseQuery::Commissionable(onboarding.filter());
        let deadline = Instant::now() + Duration::from_secs(timeout.max(1).into());

        while Instant::now() < deadline {
            let mut ids = Vec::new();
            transport
                .browse(&query, 1000, |m| {
                    if let MdnsBrowseMatch::Commissionable { id } = m {
                        ids.push(id);
                    }
                })
                .await?;
            for id in ids {
                match transport.resolve_commissionable_addrs(id).await {
                    Ok(addrs) if !addrs.is_empty() => return Ok(addrs[0]),
                    Ok(_) => {}
                    Err(e) => log::debug!("resolving {id:016X}: {e:?}"),
                }
            }
        }
        Err(Error::Timeout(format!(
            "no commissionable device with discriminator {:?} answered in {timeout} s",
            onboarding.discriminator
        )))
    }

    async fn discover(&self, seconds: u16) -> Result<Vec<Commissionable>, Error> {
        let transport = self.matter.transport();
        let query = MdnsBrowseQuery::Commissionable(Default::default());
        let mut ids = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(seconds.max(1).into());
        while Instant::now() < deadline {
            transport
                .browse(&query, 1000, |m| {
                    if let MdnsBrowseMatch::Commissionable { id } = m {
                        if !ids.contains(&id) {
                            ids.push(id);
                        }
                    }
                })
                .await?;
        }
        let mut out = Vec::new();
        for id in ids {
            let addresses = match transport.resolve_commissionable_addrs(id).await {
                Ok(addrs) => addrs
                    .iter()
                    .filter_map(|a| match a {
                        Address::Udp(sa) => Some(*sa),
                        _ => None,
                    })
                    .collect(),
                Err(_) => Vec::new(),
            };
            out.push(Commissionable {
                instance: format!("{id:016X}"),
                addresses,
            });
        }
        Ok(out)
    }

    async fn read(&self, node: u64, path: AttributePath) -> Result<Vec<AttributeValue>, Error> {
        self.known(node)?;
        let exchange = self.exchange(node).await?;
        let mut values = self.read_on(exchange, path).await?;
        for v in &mut values {
            v.node = node;
        }
        Ok(values)
    }

    /// Read on an exchange already set up (over CASE, or PASE while
    /// commissioning). Values carry node 0; the caller sets it.
    async fn read_on(
        &self,
        exchange: Exchange<'static>,
        path: AttributePath,
    ) -> Result<Vec<AttributeValue>, Error> {
        let paths = [AttrPath::from_gp(&GenericPath::new(
            path.endpoint,
            path.cluster,
            path.attribute,
        ))];
        let mut chunk = exchange
            .read_with(|b| b.attr_requests_from(&paths)?.fabric_filtered(true)?.end())
            .await?;
        let mut collected = Collected::default();
        loop {
            collected.add_chunk(0, &chunk.response()?);
            match chunk.complete().await? {
                Some(next) => chunk = next,
                None => break,
            }
        }
        Ok(collected.attributes)
    }

    async fn write(
        &self,
        node: u64,
        endpoint: u16,
        cluster: u32,
        attribute: u32,
        value: &Value,
        timed: Option<u16>,
    ) -> Result<(), Error> {
        self.known(node)?;
        let attr = matter_model::cluster(cluster).and_then(|c| c.attribute(attribute));
        if let Some(attr) = attr {
            if !attr.writable {
                return Err(Error::Invalid(format!("{} is read-only", attr.name)));
            }
        }
        let shape = attr.map_or(Shape::ANY, |a| a.shape());
        let data = encode_to_vec(shape, value, MAX_PAYLOAD)?;
        let timed = timed.or(attr.filter(|a| a.timed_write).map(|_| TIMED_MS));

        let exchange = self.exchange(node).await?;
        let handle = exchange
            .write_with(timed, |b| {
                b.timed_request(timed.is_some())?
                    .write_requests()?
                    .push()?
                    .path(endpoint, cluster, attribute)?
                    .data(|w| {
                        TLVElement::new(&data).to_tlv(&TLVTag::Context(AttrDataTag::Data as u8), w)
                    })?
                    .end()?
                    .end()?
                    .end()
            })
            .await?;
        for status in items(&handle.response()?.write_responses)? {
            let status = status?;
            if status.status.status != IMStatusCode::Success {
                return Err(Error::Status(format!(
                    "write {endpoint}/{cluster:#06x}/{attribute:#06x}: {:?}",
                    status.status.status
                )));
            }
        }
        Ok(())
    }

    async fn invoke(
        &self,
        node: u64,
        endpoint: u16,
        cluster: u32,
        command: u32,
        fields: &Value,
        timed: Option<u16>,
    ) -> Result<Value, Error> {
        self.known(node)?;
        let exchange = self.exchange(node).await?;
        self.invoke_on(exchange, endpoint, cluster, command, fields, timed)
            .await
    }

    /// Invoke a command on an exchange already set up (over CASE to a node,
    /// or over the PASE session of a commissioning).
    async fn invoke_on(
        &self,
        exchange: Exchange<'static>,
        endpoint: u16,
        cluster: u32,
        command: u32,
        fields: &Value,
        timed: Option<u16>,
    ) -> Result<Value, Error> {
        let model = matter_model::cluster(cluster);
        let cmd = model.and_then(|c| c.command(command));
        let shape = match cmd {
            Some(c) => c.request.map_or(Shape::ANY, |s| s.shape()),
            None => Shape::ANY,
        };
        let fields = if fields.is_null() {
            json!({})
        } else {
            fields.clone()
        };
        let data = encode_to_vec(shape, &fields, MAX_PAYLOAD)?;
        let timed = timed.or(cmd.filter(|c| c.timed).map(|_| TIMED_MS));

        let mut chunk = exchange
            // A request after a Timed Request says so itself, or the node
            // refuses it (TIMED_REQUEST_MISMATCH); rs-matter's client
            // leaves the flag to the caller.
            .invoke_with(timed, |b| {
                b.timed_request(timed.is_some())?
                    .invoke_requests()?
                    .push()?
                    .path(endpoint, cluster, command)?
                    .data(|w| {
                        TLVElement::new(&data).to_tlv(&TLVTag::Context(CmdDataTag::Data as u8), w)
                    })?
                    .end()?
                    .end()?
                    .end()
            })
            .await?;

        let mut result = Value::Null;
        loop {
            if let Some(resp) = chunk.response()? {
                if let Some(responses) = &resp.invoke_responses {
                    for r in items(responses)? {
                        match r? {
                            CmdResp::Cmd(data) => {
                                let shape = data
                                    .path
                                    .cmd
                                    .and_then(|id| model.and_then(|c| c.response(id)))
                                    .map_or(Shape::ANY, |s| s.shape());
                                result = matter_model::to_json(&data.data, shape)?;
                            }
                            CmdResp::Status(s) if s.status.status != IMStatusCode::Success => {
                                return Err(Error::Status(format!(
                                    "invoke {endpoint}/{cluster:#06x}/{command:#06x}: {:?}{}",
                                    s.status.status,
                                    s.status
                                        .cluster_status
                                        .map(|c| format!(" (cluster status {c})"))
                                        .unwrap_or_default()
                                )));
                            }
                            CmdResp::Status(_) => {}
                        }
                    }
                }
            }
            match chunk.complete().await? {
                Some(next) => chunk = next,
                None => break,
            }
        }
        Ok(result)
    }

    async fn interview(&self, node_id: u64) -> Result<Node, Error> {
        let mut node = self.known(node_id)?;

        let basic = self
            .read(
                node_id,
                AttributePath {
                    endpoint: Some(ROOT_ENDPOINT),
                    cluster: Some(BASIC_INFORMATION),
                    attribute: None,
                },
            )
            .await?;
        let get = |id: u32| basic.iter().find(|a| a.attribute == id).map(|a| &a.value);
        let string = |id: u32| get(id).and_then(Value::as_str).map(str::to_string);
        let number = |id: u32| {
            get(id)
                .and_then(Value::as_u64)
                .and_then(|v| u16::try_from(v).ok())
        };
        node.basic = BasicInfo {
            vendor_name: string(1),
            vendor_id: number(2),
            product_name: string(3),
            product_id: number(4),
            node_label: string(5).filter(|s| !s.is_empty()),
            hardware_version: string(8),
            software_version: string(0x0A),
            serial_number: string(0x0F),
            unique_id: string(0x12),
        };

        let descriptor = self
            .read(
                node_id,
                AttributePath {
                    endpoint: None,
                    cluster: Some(DESCRIPTOR),
                    attribute: None,
                },
            )
            .await?;
        node.endpoints.clear();
        for a in descriptor.iter().filter(|a| a.status.is_none()) {
            let ep = node
                .endpoints
                .entry(a.endpoint)
                .or_insert_with(Endpoint::default);
            let items = a.value.as_array().map(Vec::as_slice).unwrap_or_default();
            match a.attribute {
                0 => {
                    ep.device_types = items
                        .iter()
                        .filter_map(|d| {
                            Some(DeviceType {
                                id: d.get("deviceType")?.as_u64()?.try_into().ok()?,
                                revision: d.get("revision")?.as_u64()?.try_into().ok()?,
                            })
                        })
                        .collect()
                }
                1 => ep.server_clusters = numbers(items),
                2 => ep.client_clusters = numbers(items),
                3 => ep.parts = numbers(items),
                _ => {}
            }
        }
        node.interviewed = true;

        self.store.borrow_mut().put(node.clone())?;
        self.events.send(Event::NodeUpdated(node.clone()));
        Ok(node)
    }

    async fn open_window(&self, node_id: u64, timeout: u16) -> Result<CommissioningWindow, Error> {
        let node = self.known(node_id)?;
        if !(180..=900).contains(&timeout) {
            return Err(Error::Invalid(
                "a commissioning window lasts 180 to 900 seconds".into(),
            ));
        }

        let passcode = random_passcode();
        let discriminator = rand::random::<u16>() & 0x0FFF;
        let salt: [u8; 32] = rand::random();
        const ITERATIONS: u32 = 1000;

        let password = Spake2pVerifierPassword::new_from_ref(Spake2pVerifierPasswordRef::new(
            &passcode.to_le_bytes(),
        ));
        let mut verifier = rs_matter::pairing::Spake2pVerifierStr::new();
        rs_matter::sc::pase::spake2p::Spake2P::compute_verifier(
            self.crypto,
            password.reference(),
            ITERATIONS,
            &salt,
            &mut verifier,
        )?;

        self.invoke(
            node_id,
            ROOT_ENDPOINT,
            ADMINISTRATOR_COMMISSIONING,
            0x00, // OpenCommissioningWindow
            &json!({
                "0": timeout,
                "1": hex::encode(verifier.access()),
                "2": discriminator,
                "3": ITERATIONS,
                "4": hex::encode(salt),
            }),
            Some(TIMED_MS),
        )
        .await?;

        let comm = BasicCommData {
            password,
            discriminator,
        };
        let manual_code = comm.compute_pretty_pairing_code().to_string();
        let qr = QrPayload::new(
            DiscoveryCapabilities::IP,
            CommFlowType::Standard,
            comm,
            node.basic.vendor_id.unwrap_or(0),
            node.basic.product_id.unwrap_or(0),
            "",
            no_optional_data,
        );
        let mut qr_buf = [0u8; 128];
        let (qr_code, _) = qr.as_str(&mut qr_buf)?;

        Ok(CommissioningWindow {
            node: node_id,
            manual_code,
            qr_code: qr_code.to_string(),
            discriminator,
            timeout,
        })
    }

    async fn remove(&self, node_id: u64, force: bool) -> Result<(), Error> {
        self.known(node_id)?;
        let left = async {
            let index = self
                .read(
                    node_id,
                    AttributePath {
                        endpoint: Some(ROOT_ENDPOINT),
                        cluster: Some(OPERATIONAL_CREDENTIALS),
                        attribute: Some(5), // CurrentFabricIndex
                    },
                )
                .await?
                .first()
                .and_then(|a| a.value.as_u64())
                .ok_or_else(|| Error::Invalid("the node did not say its fabric index".into()))?;
            self.invoke(
                node_id,
                ROOT_ENDPOINT,
                OPERATIONAL_CREDENTIALS,
                0x0A, // RemoveFabric
                &json!({ "0": index }),
                None,
            )
            .await
            .map(|_| ())
        }
        .await;

        match left {
            Ok(()) => {}
            // The node drops our session as it leaves the fabric, so the
            // response may never come.
            Err(Error::Matter(e)) if force || e.contains("Timeout") => {
                log::warn!("node {node_id:016X}: RemoveFabric: {e}");
            }
            Err(e) if force => log::warn!("node {node_id:016X}: RemoveFabric: {e}"),
            Err(e) => return Err(e),
        }

        if let Some(task) = self.tasks.borrow_mut().remove(&node_id) {
            task.abort();
        }
        self.subs.by_node.borrow_mut().remove(&node_id);
        self.matter.with_state(|state| {
            state.sessions.remove_case_for_peer(self.fab_idx, node_id);
        });
        self.store.borrow_mut().remove(node_id)?;
        self.events.send(Event::NodeRemoved { node: node_id });
        Ok(())
    }

    /// Whether `node` manages its groups with the Groupcast cluster (Matter
    /// 1.6), rather than the Groups cluster and the group key map.
    fn uses_groupcast(&self, node: u64) -> Result<bool, Error> {
        Ok(self
            .known(node)?
            .endpoints
            .get(&ROOT_ENDPOINT)
            .is_some_and(|ep| ep.server_clusters.contains(&GROUPCAST)))
    }

    async fn add_to_group(
        &self,
        node: u64,
        endpoint: u16,
        group: u16,
        name: &str,
    ) -> Result<(), Error> {
        if group == 0 {
            return Err(Error::Invalid("group 0 is not a group".into()));
        }
        let (key_set, epoch_key, start) = {
            let g = self.groups.borrow();
            (g.key_set_id, g.epoch_key, g.epoch_start_time)
        };

        if self.uses_groupcast(node)? {
            // One command: the group, the endpoint and the key set with its
            // key; the node listens on the group's own multicast address.
            self.invoke(
                node,
                ROOT_ENDPOINT,
                GROUPCAST,
                0x00, // JoinGroup
                &json!({
                    "groupID": group,
                    "endpoints": [endpoint],
                    "keySetID": key_set,
                    "key": hex::encode(epoch_key),
                    // The node grants the group Operate on these endpoints,
                    // or it would refuse the group's commands.
                    "useAuxiliaryACL": true,
                    "mcastAddrPolicy": 1, // per group
                }),
                None,
            )
            .await?;
        } else {
            self.join_group_legacy(node, endpoint, group, name, key_set, epoch_key, start)
                .await?;
        }

        self.groups
            .borrow_mut()
            .add(group, &self.config.state_dir.join("groups.json"))?;
        self.matter.with_state(|state| {
            let fabric = state.fabrics.fabric_mut(self.fab_idx)?;
            fabric.groups_mut().key_map_set_group(group, key_set)
        })?;
        Ok(())
    }

    /// A node from before Groupcast: the key set, the group key map, and
    /// the Groups cluster.
    #[allow(clippy::too_many_arguments)]
    async fn join_group_legacy(
        &self,
        node: u64,
        endpoint: u16,
        group: u16,
        name: &str,
        key_set: u16,
        epoch_key: [u8; 16],
        start: u64,
    ) -> Result<(), Error> {
        // The fabric's group key set...
        self.invoke(
            node,
            ROOT_ENDPOINT,
            GROUP_KEY_MANAGEMENT,
            0x00, // KeySetWrite
            &json!({ "groupKeySet": {
                "groupKeySetID": key_set,
                "groupKeySecurityPolicy": 0, // trust first
                "epochKey0": hex::encode(epoch_key),
                "epochStartTime0": start,
                "groupKeyMulticastPolicy": 0, // per group id
            } }),
            None,
        )
        .await?;

        // ...mapped to the group, beside the node's other mappings on our
        // fabric (the write replaces them all)...
        let current = self
            .read(
                node,
                AttributePath {
                    endpoint: Some(ROOT_ENDPOINT),
                    cluster: Some(GROUP_KEY_MANAGEMENT),
                    attribute: Some(0), // GroupKeyMap
                },
            )
            .await?;
        let mut map: Vec<Value> = current
            .first()
            .and_then(|a| a.value.as_array().cloned())
            .unwrap_or_default()
            .into_iter()
            .filter(|m| m.get("groupId").and_then(Value::as_u64) != Some(u64::from(group)))
            .map(|m| json!({ "1": m["groupId"], "2": m["groupKeySetID"], "254": 0 }))
            .collect();
        map.push(json!({ "1": group, "2": key_set, "254": 0 }));
        self.write(
            node,
            ROOT_ENDPOINT,
            GROUP_KEY_MANAGEMENT,
            0,
            &Value::Array(map),
            None,
        )
        .await?;

        // ...an access control entry letting the group operate the
        // endpoint, beside the node's other entries on our fabric (the write
        // replaces them all, our own administrator entry included)...
        let acl = self
            .read(
                node,
                AttributePath {
                    endpoint: Some(ROOT_ENDPOINT),
                    cluster: Some(ACCESS_CONTROL),
                    attribute: Some(0), // ACL
                },
            )
            .await?;
        // Entries the node generates itself (auxiliary ones) are not
        // written back, and only the fields a client writes are kept.
        let mut entries: Vec<Value> = acl
            .first()
            .and_then(|a| a.value.as_array().cloned())
            .unwrap_or_default()
            .into_iter()
            .filter(|e| e.get("auxiliaryType").is_none_or(Value::is_null))
            .map(|e| {
                json!({
                    "privilege": e["privilege"],
                    "authMode": e["authMode"],
                    "subjects": e["subjects"],
                    "targets": e["targets"],
                    "fabricIndex": 0,
                })
            })
            .collect();
        let entry = json!({
            "privilege": 3, // Operate
            "authMode": 3,  // Group
            "subjects": [group],
            "targets": [{ "cluster": null, "endpoint": endpoint, "deviceType": null }],
            "fabricIndex": 0,
        });
        let has = |e: &Value| {
            e["authMode"] == entry["authMode"]
                && e["subjects"] == entry["subjects"]
                && e["targets"] == entry["targets"]
        };
        if !entries.iter().any(has) {
            entries.push(entry);
            self.write(
                node,
                ROOT_ENDPOINT,
                ACCESS_CONTROL,
                0,
                &Value::Array(entries),
                None,
            )
            .await?;
        }

        // ...and the endpoint in the group.
        let response = self
            .invoke(
                node,
                endpoint,
                GROUPS,
                0x00,
                &json!({ "0": group, "1": name }),
                None,
            )
            .await?;
        match response.get("status").and_then(Value::as_u64) {
            Some(0) | None => Ok(()),
            Some(status) => Err(Error::Status(format!("AddGroup: status {status:#04x}"))),
        }
    }

    async fn remove_from_group(&self, node: u64, endpoint: u16, group: u16) -> Result<(), Error> {
        if self.uses_groupcast(node)? {
            self.invoke(
                node,
                ROOT_ENDPOINT,
                GROUPCAST,
                0x01, // LeaveGroup
                &json!({ "groupID": group, "endpoints": [endpoint] }),
                None,
            )
            .await?;
            return Ok(());
        }
        let response = self
            .invoke(node, endpoint, GROUPS, 0x03, &json!({ "0": group }), None)
            .await?;
        match response.get("status").and_then(Value::as_u64) {
            Some(0) | None => Ok(()),
            Some(status) => Err(Error::Status(format!("RemoveGroup: status {status:#04x}"))),
        }
    }

    /// Read one attribute of a node's root endpoint as a number.
    async fn read_number(&self, node: u64, cluster: u32, attribute: u32) -> Result<u64, Error> {
        let values = self
            .read(
                node,
                AttributePath {
                    endpoint: Some(ROOT_ENDPOINT),
                    cluster: Some(cluster),
                    attribute: Some(attribute),
                },
            )
            .await?;
        values
            .first()
            .and_then(|v| v.value.as_u64())
            .ok_or_else(|| Error::Status(format!("{cluster:#06x}/{attribute:#06x}: no value")))
    }

    async fn announce_ota(&self, node: u64) -> Result<Option<OtaImage>, Error> {
        let known = self.known(node)?;
        let requestor = known
            .endpoints
            .iter()
            .find(|(_, e)| e.server_clusters.contains(&OTA_REQUESTOR))
            .map(|(id, _)| *id)
            .ok_or_else(|| Error::Unsupported(format!("node {node} has no OTA Requestor")))?;
        let vendor_id = self.read_number(node, BASIC_INFORMATION, 0x0002).await? as u16;
        let product_id = self.read_number(node, BASIC_INFORMATION, 0x0004).await? as u16;
        let version = self.read_number(node, BASIC_INFORMATION, 0x0009).await? as u32;

        let images = crate::ota::list(&self.config.ota_dir)?;
        let Some(image) = crate::ota::best(&images, vendor_id, product_id, version).cloned() else {
            return Ok(None);
        };
        let info = self.matter.with_state(|state| {
            state
                .fabrics
                .fabric(self.fab_idx)
                .map(|f| (f.node_id(), f.vendor_id()))
        })?;
        self.invoke(
            node,
            requestor,
            OTA_REQUESTOR,
            ANNOUNCE_OTA_PROVIDER,
            &json!({
                "providerNodeID": info.0,
                "vendorID": info.1,
                // UrgentUpdateAvailable: a person asked for it, so the node
                // asks at once rather than after its usual delay.
                "announcementReason": 2,
                "endpoint": ROOT_ENDPOINT,
            }),
            None,
        )
        .await?;
        Ok(Some(image))
    }

    async fn group_invoke(
        &self,
        group: u16,
        cluster: u32,
        command: u32,
        fields: &Value,
    ) -> Result<(), Error> {
        use rs_matter::im::CmdPath;

        if !self.groups.borrow().groups.contains(&group) {
            return Err(Error::Invalid(format!(
                "no device has been put in group {group}"
            )));
        }
        let shape = matter_model::cluster(cluster)
            .and_then(|c| c.command(command))
            .and_then(|c| c.request)
            .map_or(Shape::ANY, |s| s.shape());
        let fields = if fields.is_null() {
            json!({})
        } else {
            fields.clone()
        };
        let data = encode_to_vec(shape, &fields, MAX_PAYLOAD)?;
        let exchange =
            Exchange::initiate_group(self.matter, self.crypto, self.kv, self.fab_idx, group)?;
        exchange
            .group_invoke_with(|b| {
                b.push()?
                    .path_from(&CmdPath::new(None, Some(cluster), Some(command)))?
                    .data(|w| {
                        TLVElement::new(&data).to_tlv(&TLVTag::Context(CmdDataTag::Data as u8), w)
                    })?
                    .end()
            })
            .await?;
        Ok(())
    }

    fn rename(&self, node_id: u64, name: String) -> Result<Node, Error> {
        if name.is_empty() || name.contains(['/', '#', '+']) {
            return Err(Error::Invalid(
                "a name must not be empty or contain /, # or +".into(),
            ));
        }
        let mut node = self.known(node_id)?;
        node.name = name;
        self.store.borrow_mut().put(node.clone())?;
        self.events.send(Event::NodeUpdated(node.clone()));
        Ok(node)
    }

    /// Keep a subscription to every attribute and event of `node_id`,
    /// re-established whenever it lapses.
    fn start_subscription(self: &Rc<Self>, node_id: u64) {
        let ctx = self.clone();
        let task = tokio::task::spawn_local(async move { ctx.subscription(node_id).await });
        if let Some(old) = self.tasks.borrow_mut().insert(node_id, task) {
            old.abort();
        }
    }

    async fn subscription(self: Rc<Self>, node: u64) {
        let mut backoff = Duration::from_secs(1);
        let mut online = false;
        loop {
            match self.subscribe(node).await {
                Ok(max_interval) => {
                    backoff = Duration::from_secs(1);
                    if !online {
                        online = true;
                        self.events.send(Event::Availability { node, online });
                    }
                    if !self.node_interviewed(node) {
                        if let Err(e) = self.interview(node).await {
                            log::warn!("node {node:016X}: interview: {e}");
                        }
                    }
                    // A node reports at least every max interval; allow for
                    // retransmissions before calling the subscription lost.
                    let grace =
                        Duration::from_secs(u64::from(max_interval)) + Duration::from_secs(30);
                    loop {
                        tokio::time::sleep(Duration::from_secs(5)).await;
                        let last = self.subs.by_node.borrow().get(&node).map(|s| s.last_report);
                        match last {
                            Some(last) if last.elapsed() <= grace => {}
                            _ => break,
                        }
                    }
                    log::info!("node {node:016X}: subscription lapsed, renewing");
                }
                Err(e) => {
                    log::info!("node {node:016X}: subscribe: {e}; retrying in {backoff:?}");
                    if online {
                        online = false;
                        self.events.send(Event::Availability { node, online });
                    }
                    tokio::time::sleep(backoff).await;
                    backoff = (backoff * 2).min(Duration::from_secs(300));
                }
            }
        }
    }

    fn node_interviewed(&self, node: u64) -> bool {
        self.store.borrow().get(node).is_some_and(|n| n.interviewed)
    }

    /// Subscribe to everything `node` has, deliver the priming reports, and
    /// return the maximum interval the node agreed to.
    async fn subscribe(&self, node: u64) -> Result<u16, Error> {
        let last_event = {
            let mut by_node = self.subs.by_node.borrow_mut();
            let state = by_node.entry(node).or_insert(SubState {
                id: None,
                last_report: Instant::now(),
                last_event: None,
            });
            // Until the node names the new subscription, take any report.
            state.id = None;
            state.last_event
        };

        let attrs = [AttrPath::from_gp(&GenericPath::new(None, None, None))];
        let events = [EventPath::from_gp(&GenericPath::new(None, None, None))];
        let filters = last_event.map(|n| {
            [EventFilter {
                node: None,
                event_min: Some(n + 1),
            }]
        });
        let max_ceil = self.config.max_report_interval;

        let exchange = self.exchange(node).await?;
        let mut chunk = exchange
            .subscribe_with(|b| {
                let b = b
                    .keep_subs(true)?
                    .min_int_floor(0)?
                    .max_int_ceil(max_ceil)?
                    .attr_requests_from(&attrs)?
                    .event_requests_from(&events)?;
                match &filters {
                    Some(f) => b.event_filters_from(f)?.fabric_filtered(true)?.end(),
                    None => b.fabric_filtered(true)?.end(),
                }
            })
            .await?;

        let mut collected = Collected::default();
        let established = loop {
            collected.add_chunk(node, &chunk.response()?);
            match chunk.complete().await? {
                SubscribeOutcome::NextChunk(next) => chunk = next,
                SubscribeOutcome::Established(est) => break est,
            }
        };

        if let Some(state) = self.subs.by_node.borrow_mut().get_mut(&node) {
            state.id = Some(established.subscription_id);
            state.last_report = Instant::now();
            if let Some(last) = collected.events.iter().map(|e| e.number).max() {
                state.last_event = Some(last);
            }
        }
        publish(&self.events, collected);
        Ok(established.max_int)
    }
}

/// Check a `NetworkConfigResponse` or `ConnectNetworkResponse`: its
/// `networkingStatus` (field 0) is 0 on success.
fn networking_status(doing: &str, response: &Value) -> Result<(), Error> {
    let status = response
        .get("networkingStatus")
        .or_else(|| response.get("0"))
        .and_then(Value::as_u64);
    match status {
        Some(0) => Ok(()),
        Some(code) => {
            let name = matter_model::cluster(NETWORK_COMMISSIONING)
                .and_then(|c| c.response(0x05))
                .and_then(|s| s.field(0))
                .and_then(|f| match f.ty {
                    matter_model::Type::Enum(e) => e.name_of(code),
                    _ => None,
                })
                .unwrap_or("unknown");
            let debug = response
                .get("debugText")
                .and_then(Value::as_str)
                .unwrap_or_default();
            Err(Error::Status(format!("{doing}: {name} ({code}) {debug}")))
        }
        None => Err(Error::Status(format!(
            "{doing}: no networking status in {response}"
        ))),
    }
}

fn numbers<T: TryFrom<u64>>(items: &[Value]) -> Vec<T> {
    items
        .iter()
        .filter_map(|v| v.as_u64().and_then(|v| T::try_from(v).ok()))
        .collect()
}

/// A setup passcode the spec allows: 1 to 99999998, none of the trivial
/// ones.
fn random_passcode() -> u32 {
    const INVALID: [u32; 12] = [
        0, 11111111, 22222222, 33333333, 44444444, 55555555, 66666666, 77777777, 88888888,
        99999999, 12345678, 87654321,
    ];
    loop {
        let p = 1 + rand::random::<u32>() % 99_999_998;
        if !INVALID.contains(&p) {
            return p;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The group requests fit the data model as it is now.
    #[test]
    fn group_requests_encode() {
        let gkm = matter_model::cluster(GROUP_KEY_MANAGEMENT).unwrap();
        let req = gkm.command(0x00).unwrap().request.unwrap();
        encode_to_vec(
            req.shape(),
            &json!({ "groupKeySet": {
                "groupKeySetID": 0x4D42,
                "groupKeySecurityPolicy": 0,
                "epochKey0": "00112233445566778899aabbccddeeff",
                "epochStartTime0": 1,
                "groupKeyMulticastPolicy": 0,
            } }),
            MAX_PAYLOAD,
        )
        .unwrap();
        let map = gkm.attribute(0).unwrap();
        encode_to_vec(
            map.shape(),
            &json!([{ "1": 257, "2": 0x4D42, "254": 0 }]),
            MAX_PAYLOAD,
        )
        .unwrap();
        let add = matter_model::cluster(GROUPS)
            .unwrap()
            .command(0x00)
            .unwrap()
            .request
            .unwrap();
        encode_to_vec(add.shape(), &json!({ "0": 257, "1": "e2e" }), MAX_PAYLOAD).unwrap();
    }

    #[test]
    fn passcodes_are_valid() {
        for _ in 0..1000 {
            let p = random_passcode();
            assert!((1..=99_999_998).contains(&p));
        }
    }
}
