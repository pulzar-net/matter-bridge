//! The fabric's certificate authority.
//!
//! A Matter fabric is rooted in a CA: its root certificate (RCAC), usually an
//! intermediate (ICAC), and the node operational certificates (NOCs) it signs
//! for every node, the controller included. Where that CA lives is the
//! deployer's choice, so it is a trait:
//!
//! - [`FileCa`] keeps the chain and the ICAC key in a directory. It is what a
//!   standalone controller uses, and is its own root of trust.
//! - A platform CA (an HTTP service, an HSM) keeps its keys elsewhere and
//!   issues NOCs on request. Several controllers on one fabric then share the
//!   root, and the CA hands out node ids so they never collide.
//!
//! Until rs-matter's `Commissioner` can await an issuer, commissioning signs
//! NOCs in-process, so it needs [`CertificateAuthority::signing_key`]. A CA
//! that cannot give one can still issue the controller's own certificate; see
//! `docs/design.md`, "Upstream work".

use std::future::Future;
use std::path::{Path, PathBuf};

use rs_matter::cert::gen::VALID_FOREVER;
use rs_matter::cert::MAX_CERT_TLV_AND_ASN1_LEN;
use rs_matter::crypto::{CanonPkcSecretKeyRef, Crypto};
use rs_matter::onboard::cac::{IcacGenerator, RcacGenerator};
use rs_matter::onboard::noc::NocGenerator;
use serde::{Deserialize, Serialize};

use crate::fsutil::{read_json, write_private};
use crate::Error;

/// The length of a P-256 secret key in its canonical form.
pub const SECRET_KEY_LEN: usize = 32;

/// Issues the fabric's operational certificates.
pub trait CertificateAuthority {
    /// The fabric id.
    fn fabric_id(&self) -> u64;

    /// The root certificate (RCAC), Matter TLV encoding.
    fn root_cert(&self) -> &[u8];

    /// The intermediate certificate (ICAC) NOCs are signed with, Matter TLV
    /// encoding, or `None` when the root signs them directly.
    fn intermediate_cert(&self) -> Option<&[u8]>;

    /// Allocate a node id that no other node of the fabric has.
    fn allocate_node_id(&self) -> impl Future<Output = Result<u64, Error>>;

    /// Issue a NOC for the public key in `csr` (DER PKCS#10) and `node_id`,
    /// with up to three CASE Authenticated Tags.
    fn issue_noc<C: Crypto>(
        &self,
        crypto: C,
        csr: &[u8],
        node_id: u64,
        cats: &[u32],
    ) -> impl Future<Output = Result<Vec<u8>, Error>>;

    /// The key NOCs are signed with, if this process holds it.
    fn signing_key(&self) -> Option<&[u8; SECRET_KEY_LEN]>;

    /// The fabric's identity protection key (IPK) epoch key. Every
    /// controller and node of the fabric has the same one; CASE needs it.
    fn ipk(&self) -> &[u8; IPK_LEN];

    /// A CASE Authenticated Tag every controller of the fabric carries in
    /// its certificate, if the fabric uses one.
    fn admin_cat(&self) -> Option<u32> {
        None
    }
}

/// The length of an IPK epoch key.
pub const IPK_LEN: usize = 16;

#[derive(Serialize, Deserialize)]
struct FileCaState {
    fabric_id: u64,
    #[serde(with = "hex::serde")]
    root_cert: Vec<u8>,
    #[serde(with = "hex::serde")]
    root_key: Vec<u8>,
    #[serde(with = "hex::serde")]
    intermediate_cert: Vec<u8>,
    #[serde(with = "hex::serde")]
    intermediate_key: Vec<u8>,
    #[serde(with = "hex::serde")]
    ipk: Vec<u8>,
    next_node_id: u64,
}

/// A CA kept in one file of the state directory, readable by its owner only.
///
/// It holds the root key too, so it can mint a new intermediate; a deployment
/// that wants the root offline uses a CA of its own instead.
pub struct FileCa {
    path: PathBuf,
    state: std::cell::RefCell<FileCaState>,
    root_cert: Vec<u8>,
    intermediate_cert: Vec<u8>,
    intermediate_key: [u8; SECRET_KEY_LEN],
    ipk: [u8; IPK_LEN],
}

impl FileCa {
    /// The first node id handed out; the controller takes one before any
    /// device does.
    pub const FIRST_NODE_ID: u64 = 1;

    /// Open the CA in `path`, creating a new fabric `fabric_id` (random if
    /// `None`) with a new root and intermediate when there is none yet.
    pub fn open_or_create<C: Crypto>(
        path: &Path,
        fabric_id: Option<u64>,
        crypto: C,
    ) -> Result<Self, Error> {
        let state = match read_json::<FileCaState>(path)? {
            Some(state) => state,
            None => {
                let state = Self::create(fabric_id, &crypto)?;
                write_private(path, &serde_json::to_vec_pretty(&state)?)?;
                log::info!(
                    "created fabric {:016X} in {}",
                    state.fabric_id,
                    path.display()
                );
                state
            }
        };

        let intermediate_key = state
            .intermediate_key
            .as_slice()
            .try_into()
            .map_err(|_| Error::Store(format!("{}: bad intermediate key", path.display())))?;

        let ipk = state
            .ipk
            .as_slice()
            .try_into()
            .map_err(|_| Error::Store(format!("{}: bad IPK", path.display())))?;

        Ok(Self {
            ipk,
            path: path.to_path_buf(),
            root_cert: state.root_cert.clone(),
            intermediate_cert: state.intermediate_cert.clone(),
            intermediate_key,
            state: std::cell::RefCell::new(state),
        })
    }

    fn create<C: Crypto>(fabric_id: Option<u64>, crypto: C) -> Result<FileCaState, Error> {
        // Operational fabric ids are any value but 0.
        let fabric_id = match fabric_id {
            Some(id) => id,
            None => loop {
                let id = rand::random::<u64>();
                if id != 0 {
                    break id;
                }
            },
        };

        let mut rcac_buf = vec![0u8; MAX_CERT_TLV_AND_ASN1_LEN];
        let mut rcac_gen = RcacGenerator::new(&mut rcac_buf);
        let (root_key, root_cert) = rcac_gen.generate(&crypto, fabric_id, VALID_FOREVER)?;
        let root_cert = root_cert.to_vec();

        let mut icac_buf = vec![0u8; MAX_CERT_TLV_AND_ASN1_LEN];
        let mut icac_gen = IcacGenerator::new(&mut icac_buf);
        let (icac_key, icac) =
            icac_gen.generate(&crypto, root_key.reference(), &root_cert, VALID_FOREVER)?;

        Ok(FileCaState {
            fabric_id,
            root_key: root_key.access().to_vec(),
            intermediate_cert: icac.to_vec(),
            intermediate_key: icac_key.access().to_vec(),
            ipk: rand::random::<[u8; IPK_LEN]>().to_vec(),
            root_cert,
            next_node_id: Self::FIRST_NODE_ID,
        })
    }

    fn save(&self) -> Result<(), Error> {
        let state = self.state.borrow();
        write_private(&self.path, &serde_json::to_vec_pretty(&*state)?)
    }
}

impl CertificateAuthority for FileCa {
    fn fabric_id(&self) -> u64 {
        self.state.borrow().fabric_id
    }

    fn root_cert(&self) -> &[u8] {
        &self.root_cert
    }

    fn intermediate_cert(&self) -> Option<&[u8]> {
        Some(&self.intermediate_cert)
    }

    async fn allocate_node_id(&self) -> Result<u64, Error> {
        let id = {
            let mut state = self.state.borrow_mut();
            let id = state.next_node_id;
            // Operational node ids stop below the reserved ranges.
            if id >= 0xFFFF_FFEF_FFFF_FFFF {
                return Err(Error::Exhausted("node ids"));
            }
            state.next_node_id += 1;
            id
        };
        self.save()?;
        Ok(id)
    }

    async fn issue_noc<C: Crypto>(
        &self,
        crypto: C,
        csr: &[u8],
        node_id: u64,
        cats: &[u32],
    ) -> Result<Vec<u8>, Error> {
        let mut buf = vec![0u8; MAX_CERT_TLV_AND_ASN1_LEN];
        let mut noc_gen = NocGenerator::create(
            CanonPkcSecretKeyRef::new(&self.intermediate_key),
            &self.root_cert,
            &self.intermediate_cert,
            &mut buf,
        )?;
        Ok(noc_gen
            .generate(&crypto, csr, node_id, cats, VALID_FOREVER)?
            .to_vec())
    }

    fn signing_key(&self) -> Option<&[u8; SECRET_KEY_LEN]> {
        Some(&self.intermediate_key)
    }

    fn ipk(&self) -> &[u8; IPK_LEN] {
        &self.ipk
    }
}

/// A range of node ids a root CA handed this controller.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeIdRange {
    /// The next id to hand out.
    pub next: u64,
    /// The last id of the range.
    pub last: u64,
}

/// What a root CA returns for an intermediate's CSR.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Issued {
    /// The fabric id.
    pub fabric_id: u64,
    /// The root certificate, Matter TLV.
    #[serde(with = "hex::serde")]
    pub root_cert: Vec<u8>,
    /// The intermediate certificate issued for our key, Matter TLV.
    #[serde(with = "hex::serde")]
    pub intermediate_cert: Vec<u8>,
    /// The fabric's IPK epoch key.
    #[serde(with = "hex::serde")]
    pub ipk: Vec<u8>,
    /// The node ids this controller may hand out.
    pub node_ids: NodeIdRange,
    /// A CASE Authenticated Tag every controller of the fabric carries, to
    /// put in the controller's own certificate.
    #[serde(default)]
    pub admin_cat: Option<u32>,
}

#[derive(Serialize, Deserialize)]
struct IntermediateState {
    #[serde(flatten)]
    issued: Issued,
    #[serde(with = "hex::serde")]
    intermediate_key: Vec<u8>,
}

#[derive(Serialize, Deserialize)]
struct PendingKey {
    #[serde(with = "hex::serde")]
    key: Vec<u8>,
    #[serde(with = "hex::serde")]
    csr: Vec<u8>,
}

/// An intermediate CA whose certificate a root CA elsewhere issued (a
/// platform's, holding one fabric's root for several controllers), for a key
/// made here that never leaves this controller.
///
/// 1. [`IntermediateCa::csr`] makes the key (once) and returns its CSR.
/// 2. The root CA issues the intermediate certificate and a range of node ids.
/// 3. [`IntermediateCa::install`] stores them with the key; from then on the
///    controller signs its own and its devices' certificates, and every
///    controller of the fabric reaches every device of it.
pub struct IntermediateCa {
    path: PathBuf,
    state: std::cell::RefCell<IntermediateState>,
    root_cert: Vec<u8>,
    intermediate_cert: Vec<u8>,
    intermediate_key: [u8; SECRET_KEY_LEN],
    ipk: [u8; IPK_LEN],
    fabric_id: u64,
    admin_cat: Option<u32>,
}

impl IntermediateCa {
    fn pending_path(path: &Path) -> PathBuf {
        path.with_extension("key.json")
    }

    /// The CSR (DER PKCS#10) for this controller's intermediate key, made on
    /// the first call and the same on later ones until [`Self::install`].
    pub fn csr<C: Crypto>(path: &Path, crypto: C) -> Result<Vec<u8>, Error> {
        use rs_matter::crypto::{CanonPkcSecretKey, SecretKey as _, SigningSecretKey as _};

        let pending_path = Self::pending_path(path);
        if let Some(p) = read_json::<PendingKey>(&pending_path)? {
            return Ok(p.csr);
        }
        let key = crypto.generate_secret_key()?;
        let mut buf = [0u8; 256];
        let csr = key.csr(&mut buf)?.to_vec();
        let mut canon = CanonPkcSecretKey::new();
        key.write_canon(&mut canon)?;
        let pending = PendingKey {
            key: canon.access().to_vec(),
            csr: csr.clone(),
        };
        write_private(&pending_path, &serde_json::to_vec_pretty(&pending)?)?;
        Ok(csr)
    }

    /// Store what the root CA issued for the CSR from [`Self::csr`].
    pub fn install(path: &Path, issued: Issued) -> Result<(), Error> {
        let pending_path = Self::pending_path(path);
        let pending = read_json::<PendingKey>(&pending_path)?.ok_or_else(|| {
            Error::Invalid("no key is waiting for a certificate: ask for a CSR first".into())
        })?;
        if issued.ipk.len() != IPK_LEN || issued.fabric_id == 0 {
            return Err(Error::Invalid(
                "the issued fabric credentials are malformed".into(),
            ));
        }
        if issued.node_ids.next == 0 || issued.node_ids.next > issued.node_ids.last {
            return Err(Error::Invalid("the node id range is empty".into()));
        }
        let state = IntermediateState {
            issued,
            intermediate_key: pending.key,
        };
        write_private(path, &serde_json::to_vec_pretty(&state)?)?;
        std::fs::remove_file(&pending_path)?;
        Ok(())
    }

    /// Whether credentials are installed at `path`.
    pub fn installed(path: &Path) -> bool {
        path.exists()
    }

    /// Open the installed intermediate CA at `path`.
    pub fn open(path: &Path) -> Result<Self, Error> {
        let state = read_json::<IntermediateState>(path)?.ok_or_else(|| {
            Error::Invalid(format!(
                "{}: no fabric credentials installed",
                path.display()
            ))
        })?;
        let bad = |what: &str| Error::Store(format!("{}: bad {what}", path.display()));
        Ok(Self {
            path: path.to_path_buf(),
            root_cert: state.issued.root_cert.clone(),
            intermediate_cert: state.issued.intermediate_cert.clone(),
            intermediate_key: state
                .intermediate_key
                .as_slice()
                .try_into()
                .map_err(|_| bad("key"))?,
            ipk: state
                .issued
                .ipk
                .as_slice()
                .try_into()
                .map_err(|_| bad("IPK"))?,
            fabric_id: state.issued.fabric_id,
            admin_cat: state.issued.admin_cat,
            state: std::cell::RefCell::new(state),
        })
    }

    /// Node ids left in the range.
    pub fn node_ids_left(&self) -> u64 {
        let r = self.state.borrow().issued.node_ids;
        (r.last + 1).saturating_sub(r.next)
    }

    /// Take a further range from the root CA.
    pub fn extend_node_ids(&self, range: NodeIdRange) -> Result<(), Error> {
        if range.next == 0 || range.next > range.last {
            return Err(Error::Invalid("the node id range is empty".into()));
        }
        self.state.borrow_mut().issued.node_ids = range;
        let state = self.state.borrow();
        write_private(&self.path, &serde_json::to_vec_pretty(&*state)?)
    }
}

impl CertificateAuthority for IntermediateCa {
    fn fabric_id(&self) -> u64 {
        self.fabric_id
    }

    fn root_cert(&self) -> &[u8] {
        &self.root_cert
    }

    fn intermediate_cert(&self) -> Option<&[u8]> {
        Some(&self.intermediate_cert)
    }

    async fn allocate_node_id(&self) -> Result<u64, Error> {
        let id = {
            let mut state = self.state.borrow_mut();
            let range = &mut state.issued.node_ids;
            if range.next > range.last {
                return Err(Error::Exhausted(
                    "node ids: the root CA has to issue a further range",
                ));
            }
            let id = range.next;
            range.next += 1;
            id
        };
        let state = self.state.borrow();
        write_private(&self.path, &serde_json::to_vec_pretty(&*state)?)?;
        Ok(id)
    }

    async fn issue_noc<C: Crypto>(
        &self,
        crypto: C,
        csr: &[u8],
        node_id: u64,
        cats: &[u32],
    ) -> Result<Vec<u8>, Error> {
        let mut buf = vec![0u8; MAX_CERT_TLV_AND_ASN1_LEN];
        let mut noc_gen = NocGenerator::create(
            CanonPkcSecretKeyRef::new(&self.intermediate_key),
            &self.root_cert,
            &self.intermediate_cert,
            &mut buf,
        )?;
        Ok(noc_gen
            .generate(&crypto, csr, node_id, cats, VALID_FOREVER)?
            .to_vec())
    }

    fn signing_key(&self) -> Option<&[u8; SECRET_KEY_LEN]> {
        Some(&self.intermediate_key)
    }

    fn ipk(&self) -> &[u8; IPK_LEN] {
        &self.ipk
    }

    fn admin_cat(&self) -> Option<u32> {
        self.admin_cat
    }
}

/// [`IntermediateCa::csr`] for the controller whose state is in
/// `state_dir` (where [`crate::CaChoice::Intermediate`] looks), with this
/// crate's crypto.
pub fn intermediate_csr(state_dir: &Path) -> Result<Vec<u8>, Error> {
    let seed: [u8; SECRET_KEY_LEN] = rand::random();
    let crypto = rs_matter::crypto::default_crypto(rand::rng(), CanonPkcSecretKeyRef::new(&seed));
    IntermediateCa::csr(&intermediate_path(state_dir), &crypto)
}

/// Where a controller keeps its intermediate CA.
pub fn intermediate_path(state_dir: &Path) -> PathBuf {
    state_dir.join("intermediate.json")
}

/// Install (or replace) what a root CA issued for the controller in
/// `state_dir`. With no new key waiting (no CSR since the last install), only
/// a further node id range is taken.
pub fn install_intermediate(state_dir: &Path, issued: Issued) -> Result<(), Error> {
    let path = intermediate_path(state_dir);
    if IntermediateCa::pending_path(&path).exists() || !path.exists() {
        return IntermediateCa::install(&path, issued);
    }
    let ca = IntermediateCa::open(&path)?;
    if ca.fabric_id() != issued.fabric_id {
        return Err(Error::Invalid(
            "another fabric's credentials: ask for a new CSR first".into(),
        ));
    }
    ca.extend_node_ids(issued.node_ids)
}

/// The CA a controller runs with.
pub enum AnyCa {
    /// A standalone fabric.
    File(FileCa),
    /// An intermediate under a root CA elsewhere.
    Intermediate(IntermediateCa),
}

impl CertificateAuthority for AnyCa {
    fn fabric_id(&self) -> u64 {
        match self {
            AnyCa::File(c) => c.fabric_id(),
            AnyCa::Intermediate(c) => c.fabric_id(),
        }
    }
    fn root_cert(&self) -> &[u8] {
        match self {
            AnyCa::File(c) => c.root_cert(),
            AnyCa::Intermediate(c) => c.root_cert(),
        }
    }
    fn intermediate_cert(&self) -> Option<&[u8]> {
        match self {
            AnyCa::File(c) => c.intermediate_cert(),
            AnyCa::Intermediate(c) => c.intermediate_cert(),
        }
    }
    async fn allocate_node_id(&self) -> Result<u64, Error> {
        match self {
            AnyCa::File(c) => c.allocate_node_id().await,
            AnyCa::Intermediate(c) => c.allocate_node_id().await,
        }
    }
    async fn issue_noc<C: Crypto>(
        &self,
        crypto: C,
        csr: &[u8],
        node_id: u64,
        cats: &[u32],
    ) -> Result<Vec<u8>, Error> {
        match self {
            AnyCa::File(c) => c.issue_noc(crypto, csr, node_id, cats).await,
            AnyCa::Intermediate(c) => c.issue_noc(crypto, csr, node_id, cats).await,
        }
    }
    fn signing_key(&self) -> Option<&[u8; SECRET_KEY_LEN]> {
        match self {
            AnyCa::File(c) => c.signing_key(),
            AnyCa::Intermediate(c) => c.signing_key(),
        }
    }
    fn ipk(&self) -> &[u8; IPK_LEN] {
        match self {
            AnyCa::File(c) => c.ipk(),
            AnyCa::Intermediate(c) => c.ipk(),
        }
    }
    fn admin_cat(&self) -> Option<u32> {
        match self {
            AnyCa::File(c) => c.admin_cat(),
            AnyCa::Intermediate(c) => c.admin_cat(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn crypto() -> impl Crypto {
        static KEY: [u8; 32] = [7; 32];
        rs_matter::crypto::default_crypto(rand::rng(), CanonPkcSecretKeyRef::new(&KEY))
    }

    /// A root CA elsewhere: a FileCa issuing an intermediate for a CSR.
    #[test]
    fn an_intermediate_issued_for_a_csr_signs_nocs() {
        let crypto = crypto();
        let dir = tempfile::tempdir().unwrap();
        let root =
            FileCa::open_or_create(&dir.path().join("root.json"), Some(0xAB), &crypto).unwrap();

        let path = dir.path().join("intermediate.json");
        let csr = IntermediateCa::csr(&path, &crypto).unwrap();
        assert_eq!(
            IntermediateCa::csr(&path, &crypto).unwrap(),
            csr,
            "the same key until installed"
        );

        // The root signs the intermediate's key. (A platform's CA issues an
        // ICAC; for the test, a NOC-shaped certificate under the root's
        // intermediate is enough to exercise install and signing.)
        let state: FileCaState = read_json(&dir.path().join("root.json")).unwrap().unwrap();
        let issued = Issued {
            fabric_id: 0xAB,
            root_cert: state.root_cert.clone(),
            intermediate_cert: state.intermediate_cert.clone(),
            ipk: state.ipk.clone(),
            node_ids: NodeIdRange {
                next: 100,
                last: 101,
            },
            admin_cat: Some(0xFFFE_0001),
        };
        let _ = root;
        // Install with the root's own intermediate key standing in.
        write_private(
            &IntermediateCa::pending_path(&path),
            &serde_json::to_vec(&PendingKey {
                key: state.intermediate_key.clone(),
                csr,
            })
            .unwrap(),
        )
        .unwrap();
        IntermediateCa::install(&path, issued).unwrap();

        let ca = IntermediateCa::open(&path).unwrap();
        assert_eq!(ca.fabric_id(), 0xAB);
        assert_eq!(ca.admin_cat(), Some(0xFFFE_0001));
        let block_on = |f| {
            tokio::runtime::Builder::new_current_thread()
                .build()
                .unwrap()
                .block_on(f)
        };
        assert_eq!(block_on(ca.allocate_node_id()).unwrap(), 100);
        assert_eq!(block_on(ca.allocate_node_id()).unwrap(), 101);
        assert!(
            block_on(ca.allocate_node_id()).is_err(),
            "the range is used up"
        );
        ca.extend_node_ids(NodeIdRange {
            next: 200,
            last: 300,
        })
        .unwrap();
        assert_eq!(IntermediateCa::open(&path).unwrap().node_ids_left(), 101);

        // It signs a NOC for a device's CSR.
        use rs_matter::crypto::SigningSecretKey as _;
        let device_key = crypto.generate_secret_key().unwrap();
        let mut buf = [0u8; 256];
        let device_csr = device_key.csr(&mut buf).unwrap();
        let noc = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap()
            .block_on(ca.issue_noc(&crypto, device_csr, 200, &[]))
            .unwrap();
        assert!(!noc.is_empty());
    }
}
