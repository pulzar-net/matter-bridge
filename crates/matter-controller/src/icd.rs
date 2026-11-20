//! Intermittently connected devices (ICDs): nodes that sleep most of the
//! time and are reachable only when they wake.
//!
//! The controller registers with a node whose ICD Management cluster speaks
//! the Check-In protocol: it gives the node a key, and from then on the node
//! sends a Check-In message, encrypted with that key, whenever it wakes with
//! no subscription from the controller. A Check-In renews the node's
//! subscription and releases the requests queued while it slept.
//!
//! The registrations, keys included, are kept in `icd.json` in the state
//! directory (owner-only), never in the node store.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rs_matter::crypto::{CanonAeadKeyRef, Crypto};
use rs_matter::error::Error as MatterError;
use rs_matter::sc::checkin::CheckIn;
use rs_matter::sc::AsyncScHandler;
use rs_matter::transport::exchange::Exchange;
use serde::{Deserialize, Serialize};
use tokio::sync::Notify;

use crate::controller::EventBus;
use crate::fsutil::{read_json, write_private};
use crate::{Error, Event};

/// How long after a Check-In a node counts as awake: it stays active while
/// the controller talks to it.
const AWAKE_AFTER_CHECK_IN: Duration = Duration::from_secs(10);

/// How much longer than its idle interval to wait for a node's Check-In.
const CHECK_IN_GRACE: Duration = Duration::from_secs(60);

/// A registration with one node.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct Registration {
    /// The key the node encrypts its Check-Ins with.
    #[serde(with = "hex::serde")]
    pub key: [u8; 16],
    /// The node's Check-In counter when it was registered.
    pub start_counter: u32,
    /// The counter of the last Check-In taken.
    pub last_counter: Option<u32>,
    /// The longest the node sleeps, seconds.
    pub idle_mode_duration: u32,
    /// How long the node stays awake, milliseconds.
    pub active_mode_duration: u32,
}

impl Registration {
    /// Whether `counter` is newer than every Check-In taken so far: within
    /// half the counter space ahead, since it wraps.
    fn is_new(&self, counter: u32) -> bool {
        const HALF: u32 = 1 << 31;
        match self.last_counter {
            Some(last) => (1..HALF).contains(&counter.wrapping_sub(last)),
            None => counter.wrapping_sub(self.start_counter) < HALF,
        }
    }

    /// The longest to wait for the node's next Check-In.
    pub fn check_in_wait(&self) -> Duration {
        Duration::from_secs(u64::from(self.idle_mode_duration)) + CHECK_IN_GRACE
    }
}

/// The ICD registrations and the Check-Ins they receive.
pub(crate) struct Icds<C> {
    path: PathBuf,
    crypto: C,
    events: EventBus,
    registrations: Mutex<BTreeMap<u64, Registration>>,
    wake: Mutex<HashMap<u64, Arc<Notify>>>,
    checked_in: Mutex<HashMap<u64, Instant>>,
}

impl<C: Crypto> Icds<C> {
    pub fn load(path: &Path, crypto: C, events: EventBus) -> Result<Self, Error> {
        let registrations = read_json(path)?.unwrap_or_default();
        Ok(Self {
            path: path.to_path_buf(),
            crypto,
            events,
            registrations: Mutex::new(registrations),
            wake: Mutex::new(HashMap::new()),
            checked_in: Mutex::new(HashMap::new()),
        })
    }

    fn save(&self, registrations: &BTreeMap<u64, Registration>) -> Result<(), Error> {
        write_private(&self.path, &serde_json::to_vec_pretty(registrations)?)
    }

    /// The registration with `node`, if it is an ICD this controller
    /// registered with.
    pub fn get(&self, node: u64) -> Option<Registration> {
        self.registrations.lock().unwrap().get(&node).cloned()
    }

    pub fn register(&self, node: u64, registration: Registration) -> Result<(), Error> {
        let mut registrations = self.registrations.lock().unwrap();
        registrations.insert(node, registration);
        self.save(&registrations)
    }

    pub fn forget(&self, node: u64) -> Result<(), Error> {
        let mut registrations = self.registrations.lock().unwrap();
        if registrations.remove(&node).is_some() {
            self.save(&registrations)?;
        }
        self.wake.lock().unwrap().remove(&node);
        self.checked_in.lock().unwrap().remove(&node);
        Ok(())
    }

    /// What wakes on `node`'s Check-Ins.
    pub fn waker(&self, node: u64) -> Arc<Notify> {
        self.wake.lock().unwrap().entry(node).or_default().clone()
    }

    /// Whether `node` checked in a moment ago, so it is awake.
    pub fn just_checked_in(&self, node: u64) -> bool {
        self.checked_in
            .lock()
            .unwrap()
            .get(&node)
            .is_some_and(|t| t.elapsed() < AWAKE_AFTER_CHECK_IN)
    }

    /// Wait for `node`'s next Check-In, at most `wait`; whether it came (or
    /// one came a moment ago).
    pub async fn wait_check_in(&self, node: u64, wait: Duration) -> bool {
        let waker = self.waker(node);
        let notified = waker.notified();
        tokio::pin!(notified);
        notified.as_mut().enable();
        if self.just_checked_in(node) {
            return true;
        }
        tokio::time::timeout(wait, notified).await.is_ok()
    }

    /// Take a Check-In's payload: the node whose key opens it, if the
    /// message is new.
    fn receive(&self, payload: &[u8]) -> Option<u64> {
        let mut registrations = self.registrations.lock().unwrap();
        let (node, counter) = registrations.iter().find_map(|(node, r)| {
            let mut buf = payload.to_vec();
            CheckIn::new(CanonAeadKeyRef::new(&r.key))
                .parse(&self.crypto, &mut buf)
                .ok()
                .map(|p| (*node, p.counter))
        })?;
        let registration = registrations.get_mut(&node)?;
        if !registration.is_new(counter) {
            log::warn!("node {node:016X}: Check-In counter {counter} replayed; dropped");
            return None;
        }
        registration.last_counter = Some(counter);
        if let Err(e) = self.save(&registrations) {
            log::warn!("ICD registrations: {e}");
        }
        Some(node)
    }
}

impl<C: Crypto> AsyncScHandler for Icds<C> {
    async fn check_in(&self, exchange: Exchange<'_>) -> Result<(), MatterError> {
        let payload = exchange.rx()?.payload().to_vec();
        match self.receive(&payload) {
            Some(node) => {
                log::info!("node {node:016X}: checked in");
                self.checked_in.lock().unwrap().insert(node, Instant::now());
                self.events.send(Event::CheckIn { node });
                self.waker(node).notify_waiters();
            }
            None => log::debug!("a Check-In no registration opens; dropped"),
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn registration(start: u32, last: Option<u32>) -> Registration {
        Registration {
            key: [0; 16],
            start_counter: start,
            last_counter: last,
            idle_mode_duration: 60,
            active_mode_duration: 300,
        }
    }

    #[test]
    fn counters_only_go_forward() {
        let r = registration(100, None);
        assert!(r.is_new(100));
        assert!(r.is_new(101));
        assert!(!r.is_new(99));

        let r = registration(100, Some(105));
        assert!(!r.is_new(105));
        assert!(!r.is_new(104));
        assert!(r.is_new(106));

        // Across the wrap.
        let r = registration(0, Some(u32::MAX));
        assert!(r.is_new(0));
        assert!(!r.is_new(u32::MAX - 1));
    }
}
