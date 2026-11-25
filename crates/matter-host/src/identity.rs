//! The controller's own operational identity: its key, its NOC and its node
//! id on the fabric, issued once by the CA and kept in the state directory.

use std::path::Path;

use rs_matter::crypto::{CanonPkcSecretKey, Crypto, SecretKey as _, SigningSecretKey as _};
use serde::{Deserialize, Serialize};

use crate::ca::{CertificateAuthority, SECRET_KEY_LEN};
use crate::fsutil::{read_json, write_private};
use crate::Error;

#[derive(Serialize, Deserialize)]
pub(crate) struct Identity {
    pub fabric_id: u64,
    pub node_id: u64,
    #[serde(with = "hex::serde")]
    pub noc: Vec<u8>,
    #[serde(with = "hex::serde")]
    pub key: [u8; SECRET_KEY_LEN],
}

impl Identity {
    /// Load the identity at `path`, or have `ca` issue one.
    pub async fn load_or_create<A: CertificateAuthority, C: Crypto + Copy>(
        path: &Path,
        ca: &A,
        crypto: C,
    ) -> Result<Self, Error> {
        if let Some(id) = read_json::<Identity>(path)? {
            if id.fabric_id == ca.fabric_id() {
                return Ok(id);
            }
            log::warn!(
                "{} belongs to fabric {:016X}, not {:016X}; issuing a new identity",
                path.display(),
                id.fabric_id,
                ca.fabric_id()
            );
        }

        let node_id = ca.allocate_node_id().await?;
        let secret = crypto.generate_secret_key()?;
        let mut csr_buf = [0u8; 256];
        let csr = secret.csr(&mut csr_buf)?;
        let cats: Vec<u32> = ca.admin_cat().into_iter().collect();
        let noc = ca.issue_noc(crypto, csr, node_id, &cats).await?;

        let mut canon = CanonPkcSecretKey::new();
        secret.write_canon(&mut canon)?;

        let id = Identity {
            fabric_id: ca.fabric_id(),
            node_id,
            noc,
            key: *canon.access(),
        };
        write_private(path, &serde_json::to_vec_pretty(&id)?)?;
        log::info!(
            "controller is node {node_id:016X} on fabric {:016X}",
            id.fabric_id
        );
        Ok(id)
    }
}
