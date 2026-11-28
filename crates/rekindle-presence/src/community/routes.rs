//! Verified M/6 versions, by device record. Unusable routes retain their
//! sequence watermark so a lagging replica cannot resurrect an older route.

use std::collections::BTreeMap;
use std::sync::Arc;

use tokio::sync::Mutex;
use tokio::time::Instant;

use super::reread_cap::RereadCap;

/// An inner-signature verified route and its DHT sequence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteVersion {
    pub seq: u64,
    pub blob: Vec<u8>,
}

#[derive(Debug, Clone)]
struct CachedRoute {
    highest: RouteVersion,
    usable: bool,
}

/// The member routes of one community, indexed by device record.
#[derive(Debug, Clone, Default)]
pub struct MemberRouteCache {
    by_record: BTreeMap<String, CachedRoute>,
    rereads: RereadCap,
    reads: BTreeMap<String, Arc<Mutex<()>>>,
}

impl MemberRouteCache {
    /// The current usable route of `record`.
    #[must_use]
    pub fn get(&self, record: &str) -> Option<&RouteVersion> {
        self.by_record
            .get(record)
            .filter(|r| r.usable)
            .map(|r| &r.highest)
    }

    /// Whether `version` is still the highest verified version.
    #[must_use]
    pub fn is_current(&self, record: &str, version: &RouteVersion) -> bool {
        self.by_record
            .get(record)
            .is_some_and(|r| r.highest == *version)
    }

    /// Accept only an advance or the same verified version. Observing a
    /// live version separately from verifying it keeps stale probes from
    /// changing usability.
    pub fn verify(&mut self, record: &str, version: &RouteVersion) -> bool {
        if let Some(held) = self.by_record.get(record) {
            if version.seq <= held.highest.seq {
                return held.highest == *version;
            }
        }
        self.rereads.observed(record, version.seq);
        self.by_record.insert(
            record.to_owned(),
            CachedRoute {
                highest: version.clone(),
                usable: false,
            },
        );
        true
    }

    /// Change usability only for the version actually observed or tried.
    pub fn set_usable(&mut self, record: &str, version: &RouteVersion, usable: bool) -> bool {
        let Some(held) = self
            .by_record
            .get_mut(record)
            .filter(|held| held.highest == *version)
        else {
            return false;
        };
        held.usable = usable;
        true
    }

    /// Mark every route the transport declared dead as unusable.
    pub fn forget_blobs(&mut self, mut dead: impl FnMut(&[u8]) -> bool) {
        for held in self.by_record.values_mut() {
            if held.usable && dead(&held.highest.blob) {
                held.usable = false;
            }
        }
    }

    /// Keep the records that are still eligible in the merged view.
    pub fn retain(&mut self, mut keep: impl FnMut(&str) -> bool) {
        self.by_record.retain(|record, _| keep(record));
        self.rereads.retain(&mut keep);
        self.reads.retain(|record, _| keep(record));
    }

    /// Single-flight cold and failure-triggered reads of this record.
    pub fn read_lock(&mut self, record: &str) -> Arc<Mutex<()>> {
        Arc::clone(self.reads.entry(record.to_owned()).or_default())
    }

    /// Whether a forced read may start under D8.
    #[must_use]
    pub fn may_reread(&self, record: &str, now: Instant) -> bool {
        self.rereads.may_reread(record, now)
    }

    /// Record the result of a forced read for D8. A stale result never
    /// competes with the watermark or resets the backoff.
    pub fn finish_reread(&mut self, record: &str, usable: bool, now: Instant) {
        let seq = self.by_record.get(record).map(|route| route.highest.seq);
        self.rereads.reread(record, seq, usable, now);
    }
}

#[cfg(test)]
#[path = "routes_reread_tests.rs"]
mod reread_tests;

#[cfg(test)]
#[path = "routes_tests.rs"]
mod tests;
