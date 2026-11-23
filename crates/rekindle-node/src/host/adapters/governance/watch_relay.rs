//! Plan GI3: Mutual Aid §14.3, the watch relay's receive half, through the
//! one Host ingest both modes call (the daemon's `ingest_gossip`, the
//! desktop's relay dispatch).
//!
//! Veilid reserves `member_watch_limit` signed watch slots plus
//! `public_watch_limit` anonymous ones per record, so in a community of any
//! size most members hold no watch on a given record. Members that hold one
//! relay the change — Plumtree's lazy push (Leitão et al., SRDS'07):
//! `WatchRelay` is the `IHAVE`, our read plus hash check the `GRAFT`. The
//! relay carries a BLAKE3 `content_hash` and no value, on purpose: gossip is
//! unencrypted at the envelope layer, so shipping the value would leak it to
//! every hop. The hash proves that what we read is what the observer saw.

use super::HostGovernance;
use rekindle_governance_runtime::GovernanceRuntimeDeps;

impl HostGovernance {
    /// A peer holding a watch slot reports that `record_key`'s `subkey`
    /// changed to a value hashing to `content_hash`. Skipped when our own
    /// watch covers the record (its change arrives directly); otherwise the
    /// subkey is read from the network and applied when its hash matches.
    pub async fn on_watch_relay(
        &self,
        record_key: &str,
        subkey: u32,
        content_hash: &str,
        observer_pseudonym: &str,
    ) {
        if self.own_watch_covers(record_key) {
            tracing::trace!(
                record_key,
                subkey,
                "watch relay: our own watch covers this record"
            );
            return;
        }
        let port = match self.record_port() {
            Ok(port) => port,
            Err(error) => {
                tracing::debug!(record_key, %error, "watch relay: no record pool");
                return;
            }
        };
        let Ok(Some(value)) = port.read_once(record_key, subkey, true).await else {
            return;
        };
        self.apply_relayed_value(
            record_key,
            subkey,
            content_hash,
            observer_pseudonym,
            value.data,
        )
        .await;
    }

    /// Apply a relayed subkey `value` read from the network, only when it
    /// hashes to the observer's `content_hash`; whether it was applied. The
    /// value goes where a watched change of ours would: the community's
    /// rebuild when the record is a governance one, else the friends' Host
    /// (an invite inbox, a participant record, a friend's profile), and the
    /// daemon's subscription overlay when it runs.
    pub(crate) async fn apply_relayed_value(
        &self,
        record_key: &str,
        subkey: u32,
        content_hash: &str,
        observer_pseudonym: &str,
        value: Vec<u8>,
    ) -> bool {
        if blake3::hash(&value).to_hex().as_str() != content_hash {
            tracing::debug!(record_key, subkey, observer = %observer_pseudonym, "watch relay: content hash mismatch, dropping");
            return false;
        }
        if !self.on_watched_change(record_key, &[subkey]).await {
            crate::host::adapters::friends::HostFriends::of(&self.ctx)
                .on_watched_change(record_key, &[subkey], Some(&value))
                .await;
        }
        if let Some(sub) = self.ctx.subscriptions.read().as_ref() {
            sub.on_value_change(record_key, vec![subkey], Some(value));
        }
        true
    }

    /// Whether a watch of ours in this session covers `record_key`.
    fn own_watch_covers(&self, record_key: &str) -> bool {
        (self.ctx.communities.map().read().values())
            .any(|community| community.watched_records.contains(record_key))
    }
}

#[cfg(test)]
#[path = "watch_relay_tests.rs"]
mod tests;
