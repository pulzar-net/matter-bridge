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
