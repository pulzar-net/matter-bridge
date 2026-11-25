//! The node store: every commissioned node, what its interview found, and
//! the name a person gave it.
//!
//! One JSON file, replaced atomically when something changes (a node
//! commissioned, interviewed, renamed or removed). Attribute values are not
//! kept: they come back with the first subscription report after a restart,
//! so a gateway's flash sees a write per change of the network, not per
//! message.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::fsutil::{read_json, write_atomic};
use crate::Error;

/// A device type an endpoint says it is (Descriptor cluster).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceType {
    /// Device type id, e.g. `0x0100` for an On/Off Light.
    pub id: u32,
    /// Its revision.
    pub revision: u16,
}

/// An endpoint of a node, from its Descriptor cluster.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Endpoint {
    /// What the endpoint is.
    pub device_types: Vec<DeviceType>,
    /// Clusters it serves.
    pub server_clusters: Vec<u32>,
    /// Clusters it is a client of (it sends their commands, e.g. a switch).
    pub client_clusters: Vec<u32>,
    /// Endpoints it is composed of.
    pub parts: Vec<u16>,
}

/// What the Basic Information cluster says about a node.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BasicInfo {
    /// Vendor id.
    pub vendor_id: Option<u16>,
    /// Vendor name.
    pub vendor_name: Option<String>,
    /// Product id.
    pub product_id: Option<u16>,
    /// Product name.
    pub product_name: Option<String>,
    /// The label a user set on the node itself.
    pub node_label: Option<String>,
    /// Hardware version string.
    pub hardware_version: Option<String>,
    /// Software version string.
    pub software_version: Option<String>,
    /// Serial number.
    pub serial_number: Option<String>,
    /// Unique id, stable across factory resets.
    pub unique_id: Option<String>,
}

/// A commissioned node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Node {
    /// Operational node id on our fabric.
    pub node_id: u64,
    /// The name a person gave it (unique in the store).
    pub name: String,
    /// Basic Information, from the interview.
    #[serde(default)]
    pub basic: BasicInfo,
    /// Endpoints by id, from the interview.
    #[serde(default)]
    pub endpoints: BTreeMap<u16, Endpoint>,
    /// Whether the interview has completed.
    #[serde(default)]
    pub interviewed: bool,
    /// When it was commissioned, seconds since the Unix epoch.
    pub commissioned_at: u64,
}

impl Node {
    /// The default name of node `node_id`.
    pub fn default_name(node_id: u64) -> String {
        format!("node_{node_id}")
    }
}

#[derive(Default, Serialize, Deserialize)]
struct StoreFile {
    version: u32,
    nodes: Vec<Node>,
}

/// The node store.
pub struct NodeStore {
    path: PathBuf,
    nodes: BTreeMap<u64, Node>,
}

impl NodeStore {
    const VERSION: u32 = 1;

    /// Open the store at `path`, empty if there is no file yet.
    pub fn open(path: &Path) -> Result<Self, Error> {
        let file = read_json::<StoreFile>(path)?.unwrap_or_default();
        if file.version > Self::VERSION {
            return Err(Error::Store(format!(
                "{}: version {} is newer than this build reads ({})",
                path.display(),
                file.version,
                Self::VERSION
            )));
        }
        Ok(Self {
            path: path.to_path_buf(),
            nodes: file.nodes.into_iter().map(|n| (n.node_id, n)).collect(),
        })
    }

    fn save(&self) -> Result<(), Error> {
        let file = StoreFile {
            version: Self::VERSION,
            nodes: self.nodes.values().cloned().collect(),
        };
        write_atomic(&self.path, &serde_json::to_vec_pretty(&file)?, false)
    }

    /// Every node, by node id.
    pub fn nodes(&self) -> impl Iterator<Item = &Node> {
        self.nodes.values()
    }

    /// The node with `node_id`.
    pub fn get(&self, node_id: u64) -> Option<&Node> {
        self.nodes.get(&node_id)
    }

    /// The node named `name`, or with node id `name` in decimal or `0x` hex.
    pub fn find(&self, name: &str) -> Option<&Node> {
        self.nodes.values().find(|n| n.name == name).or_else(|| {
            let id = match name.strip_prefix("0x") {
                Some(hex) => u64::from_str_radix(hex, 16).ok(),
                None => name.parse().ok(),
            }?;
            self.nodes.get(&id)
        })
    }

    /// Add or replace a node, and save.
    pub fn put(&mut self, node: Node) -> Result<(), Error> {
        if let Some(other) = self
            .nodes
            .values()
            .find(|n| n.name == node.name && n.node_id != node.node_id)
        {
            return Err(Error::Invalid(format!(
                "the name `{}` is taken by node {}",
                node.name, other.node_id
            )));
        }
        if self.nodes.get(&node.node_id) == Some(&node) {
            return Ok(());
        }
        self.nodes.insert(node.node_id, node);
        self.save()
    }

    /// Remove a node, and save.
    pub fn remove(&mut self, node_id: u64) -> Result<Option<Node>, Error> {
        let node = self.nodes.remove(&node_id);
        if node.is_some() {
            self.save()?;
        }
        Ok(node)
    }

    /// A name for a new node that no node has: `base`, or `base_2`, ...
    pub fn unique_name(&self, base: &str) -> String {
        if !self.nodes.values().any(|n| n.name == base) {
            return base.to_string();
        }
        (2..)
            .map(|i| format!("{base}_{i}"))
            .find(|name| !self.nodes.values().any(|n| &n.name == name))
            .unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(id: u64, name: &str) -> Node {
        Node {
            node_id: id,
            name: name.into(),
            basic: BasicInfo::default(),
            endpoints: BTreeMap::new(),
            interviewed: false,
            commissioned_at: 0,
        }
    }

    #[test]
    fn persists_and_finds() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nodes.json");
        let mut store = NodeStore::open(&path).unwrap();
        store.put(node(5, "lamp")).unwrap();
        store.put(node(6, "plug")).unwrap();
        assert!(store.put(node(7, "lamp")).is_err());
        assert_eq!(store.unique_name("lamp"), "lamp_2");

        let store = NodeStore::open(&path).unwrap();
        assert_eq!(store.find("lamp").unwrap().node_id, 5);
        assert_eq!(store.find("6").unwrap().name, "plug");
        assert_eq!(store.find("0x6").unwrap().name, "plug");
        assert!(store.find("nope").is_none());
    }

    #[test]
    fn unchanged_put_does_not_write() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nodes.json");
        let mut store = NodeStore::open(&path).unwrap();
        store.put(node(1, "a")).unwrap();
        let before = std::fs::metadata(&path).unwrap().modified().unwrap();
        std::fs::remove_file(&path).unwrap();
        store.put(node(1, "a")).unwrap();
        assert!(!path.exists(), "an unchanged node was written again");
        let _ = before;
    }

    #[test]
    fn rejects_a_newer_version() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nodes.json");
        std::fs::write(&path, br#"{"version": 99, "nodes": []}"#).unwrap();
        assert!(NodeStore::open(&path).is_err());
    }
}
