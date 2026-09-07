//! MEK cache paths.
//!
//! Backed by `rekindle_transport::crypto::mek::MekCache`, which the
//! daemon already owns and keys by `(community_id, channel_id)` with a
//! generation list per entry.
//!
//! The community-level MEK is stored under a reserved channel id rather
//! than in a second map: `MekCache` has exactly one shape, and giving
//! the community key its own store would mean two things to keep in
//! sync during rotation.

use rekindle_governance_runtime::deps::{ChannelMekSnapshot, MekSnapshot};
use rekindle_transport::crypto::mek::Mek;

use super::DaemonGovernanceAdapter;

/// Channel id under which the community-wide MEK is cached.
///
/// Not a valid channel id — channel ids are hex — so it cannot collide
/// with a real channel's entry.
pub(crate) const COMMUNITY_MEK_SLOT: &str = "__community__";

fn to_snapshot(mek: &Mek) -> MekSnapshot {
    MekSnapshot {
        generation: mek.generation(),
        key_bytes: *mek.as_bytes(),
    }
}

impl DaemonGovernanceAdapter<'_> {
    pub(super) fn community_mek_impl(&self, community_id: &str) -> Option<MekSnapshot> {
        self.ctx
            .mek_cache
            .read()
            .current(community_id, COMMUNITY_MEK_SLOT)
            .map(to_snapshot)
    }

    pub(super) fn channel_mek_impl(
        &self,
        community_id: &str,
        channel_id: &str,
    ) -> Option<MekSnapshot> {
        self.ctx
            .mek_cache
            .read()
            .current(community_id, channel_id)
            .map(to_snapshot)
    }

    /// Every channel's **current** MEK, for the bootstrap bundle.
    ///
    /// `MekCache::snapshot` yields one row per retained *generation*, so
    /// a channel that has rotated twice appears three times. The bundle
    /// wants one current key per channel, hence the dedupe through a
    /// `BTreeSet` before resolving each id to its current MEK — a
    /// straight map over the snapshot would hand the joiner the same
    /// channel several times at different generations.
    ///
    /// The community slot is filtered out: the bundle wraps per-channel
    /// keys, and the reserved id maps to no channel a joiner could use.
    pub(super) fn channel_meks_all_impl(&self, community_id: &str) -> Vec<ChannelMekSnapshot> {
        let guard = self.ctx.mek_cache.read();
        let channel_ids: std::collections::BTreeSet<String> = guard
            .snapshot(community_id)
            .into_iter()
            .map(|entry| entry.channel_id)
            .filter(|id| id != COMMUNITY_MEK_SLOT)
            .collect();
        channel_ids
            .into_iter()
            .filter_map(|channel_id| {
                guard
                    .current(community_id, &channel_id)
                    .map(|mek| ChannelMekSnapshot {
                        channel_id,
                        mek: to_snapshot(mek),
                    })
            })
            .collect()
    }

    pub(super) fn insert_community_mek_impl(&self, community_id: &str, mek: &MekSnapshot) {
        self.ctx.mek_cache.write().insert(
            community_id,
            COMMUNITY_MEK_SLOT,
            Mek::from_bytes(mek.key_bytes, mek.generation),
        );
    }

    pub(super) fn insert_channel_mek_impl(
        &self,
        community_id: &str,
        channel_id: &str,
        mek: &MekSnapshot,
    ) {
        self.ctx.mek_cache.write().insert(
            community_id,
            channel_id,
            Mek::from_bytes(mek.key_bytes, mek.generation),
        );
    }

    /// A specific historical generation, for decrypting backlog written
    /// before the current rotation.
    ///
    /// The desktop track reaches into Stronghold for this; the daemon's
    /// `MekCache` already retains prior generations per entry, so the
    /// lookup is local. A generation evicted from the cache returns
    /// `None` and the caller leaves that message undecrypted, which is
    /// the same outcome the desktop gets on a Stronghold miss.
    pub(super) fn load_historical_channel_mek_impl(
        &self,
        community_id: &str,
        channel_id: &str,
        generation: u64,
    ) -> Option<MekSnapshot> {
        self.ctx
            .mek_cache
            .read()
            .get_generation(community_id, channel_id, generation)
            .map(to_snapshot)
    }
}
