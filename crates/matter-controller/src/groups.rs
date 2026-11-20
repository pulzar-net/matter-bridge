//! The fabric's group key and the groups this controller sends to, kept in
//! `groups.json` in the state directory (owner-only: it holds a key).
//!
//! A group command is one multicast message every member decrypts with the
//! group's key, so the controller and every member share a key set: the
//! controller makes one epoch key on first use and gives it to each node it
//! puts in a group.

use std::collections::BTreeSet;
use std::num::NonZeroU8;
use std::path::Path;

use rs_matter::crypto::CanonAeadKey;
use rs_matter::group_keys::{GroupEpochKeyEntry, GroupKeySet};
use rs_matter::Matter;
use serde::{Deserialize, Serialize};

use crate::fsutil::{read_json, write_private};
use crate::Error;

/// The id of the controller's group key set (key set 0 is the IPK).
const KEY_SET_ID: u16 = 0x4D42;

#[derive(Serialize, Deserialize)]
pub(crate) struct GroupKey {
    pub key_set_id: u16,
    #[serde(with = "hex::serde")]
    pub epoch_key: [u8; 16],
    /// Microseconds since the Matter epoch; any non-zero time already past
    /// makes the key current.
    pub epoch_start_time: u64,
    pub groups: BTreeSet<u16>,
}

impl GroupKey {
    pub fn load_or_create(path: &Path) -> Result<Self, Error> {
        if let Some(key) = read_json::<GroupKey>(path)? {
            return Ok(key);
        }
        let key = GroupKey {
            key_set_id: KEY_SET_ID,
            epoch_key: rand::random(),
            epoch_start_time: 1,
            groups: BTreeSet::new(),
        };
        write_private(path, &serde_json::to_vec_pretty(&key)?)?;
        Ok(key)
    }

    pub fn add(&mut self, group: u16, path: &Path) -> Result<(), Error> {
        if self.groups.insert(group) {
            write_private(path, &serde_json::to_vec_pretty(self)?)?;
        }
        Ok(())
    }

    /// Give the controller's own fabric the key set and its groups, so it
    /// can encrypt group messages.
    pub fn install(&self, matter: &Matter<'_>, fab_idx: NonZeroU8) -> Result<(), Error> {
        matter.with_state(|state| {
            let fabric = state.fabrics.fabric_mut(fab_idx)?;
            let groups = fabric.groups_mut();
            if groups.key_set_get(self.key_set_id).is_none() {
                let mut epoch_key = CanonAeadKey::new();
                epoch_key.access_mut().copy_from_slice(&self.epoch_key);
                let mut set = GroupKeySet {
                    group_key_set_id: self.key_set_id,
                    group_key_security_policy: 0,
                    epoch_keys: Default::default(),
                };
                let _ = set.epoch_keys.push(GroupEpochKeyEntry {
                    epoch_key,
                    epoch_start_time: self.epoch_start_time,
                });
                groups.key_set_add(set)?;
            }
            for &group in &self.groups {
                groups.key_map_set_group(group, self.key_set_id)?;
            }
            Ok::<_, rs_matter::error::Error>(())
        })?;
        Ok(())
    }
}
