//! Registry I/O and the materialised roster.
//!
//! This is the half the daemon was missing. `scan_segment_raw` is the
//! read the presence orchestrator runs each tick; the crate then
//! W26-verifies and ban-filters every row via `parse_and_classify_row`
//! and hands the survivors back to `persist_discovered_member_rows`,
//! which lands them in the runtime map. That round trip is what makes a
//! member count answerable without a DHT read, and what the v1.0 member
//! index was substituting for.

use std::collections::HashMap;

use rekindle_presence::deps::{DiscoveredMemberRow, PresenceError, RowWrite};

use crate::daemon::community_runtime::MemberRecord;

use super::DaemonPresenceAdapter;

impl DaemonPresenceAdapter {
    /// Read every occupied subkey of one segment.
    ///
    /// Uses `inspect_present_subkeys` to learn which subkeys hold a
    /// value, then fetches only those — rather than sweeping
    /// `0..max_subkey` blind. On a segment holding five members that is
    /// five reads instead of 255, and the orchestrator's contract only
    /// asks for non-empty rows.
    ///
    /// `skip_subkey` is our own slot: we already know what we wrote
    /// there, and re-reading it would let a stale network copy of our
    /// own presence overwrite the fresh local one.
    pub(super) async fn scan_segment_impl(
        &self,
        registry_key: &str,
        max_subkey: u32,
        skip_subkey: Option<u32>,
    ) -> Vec<(u32, Vec<u8>)> {
        let Some(node) = self.transport() else {
            return Vec::new();
        };
        // One borrow per segment for the whole scan: a table hit while the
        // community holds the record (plan C7.7g).
        let lease = match rekindle_transport::broadcast::dht_writes::acquire_str(
            node.as_ref(),
            registry_key,
            None,
        )
        .await
        {
            Ok(lease) => lease,
            Err(error) => {
                tracing::debug!(registry = %registry_key, %error, "presence scan: segment not open");
                return Vec::new();
            }
        };
        let rows = scan_leased(node.as_ref(), lease, registry_key, max_subkey, skip_subkey).await;
        rekindle_transport::broadcast::dht_writes::release(node.as_ref(), lease).await;
        rows
    }

    /// Write our own presence row into our slot.
    pub(super) async fn write_presence_impl(
        &self,
        registry_key: &str,
        subkey_index: u32,
        presence_json: Vec<u8>,
        writer_keypair_str: &str,
    ) -> Result<RowWrite, PresenceError> {
        let node = self.transport().ok_or(PresenceError::NotAttached)?;

        // A table hit while the community holds its registry with our slot
        // writer; never a re-open that would replace it (V5, plan C7.7g).
        let lease = rekindle_transport::broadcast::dht_writes::acquire_str(
            node.as_ref(),
            registry_key,
            Some(writer_keypair_str),
        )
        .await
        .map_err(|e| PresenceError::Dht(format!("open registry: {e}")))?;

        // A plain (online-only) write, never held for re-push: a heartbeat
        // that flushes minutes later advertises liveness we did not have,
        // and every reader's staleness check believes it. A miss fails now
        // and the next tick writes again.
        let written = rekindle_transport::broadcast::dht_writes::set_leased_str(
            node.as_ref(),
            lease,
            subkey_index,
            presence_json,
            Some(writer_keypair_str),
        )
        .await;
        rekindle_transport::broadcast::dht_writes::release(node.as_ref(), lease).await;
        // A supersede goes back to the writer, which decides from whose row
        // it is (plan C7.16); it used to be reported as written.
        match written {
            Ok(None) => Ok(RowWrite::Stored),
            Ok(Some(newer)) => Ok(RowWrite::Superseded {
                seq: newer.seq,
                data: newer.data,
            }),
            Err(e) => Err(PresenceError::Dht(format!("write presence: {e}"))),
        }
    }

    /// Land the poll's validated roster in the runtime map.
    ///
    /// Wholesale replacement, matching `set_members`: the poll re-derives
    /// the full set each tick, so a member whose row vanished — departed,
    /// banned, slot reclaimed — must vanish here too.
    pub(super) fn persist_members_impl(
        &self,
        community_id: &str,
        rows: Vec<DiscoveredMemberRow>,
        banned_pseudonyms: &[String],
    ) {
        let members: HashMap<String, MemberRecord> = rows
            .into_iter()
            // Defence in depth: the orchestrator already drops banned
            // rows, but the roster is what every downstream count and
            // lookup reads, so it filters again rather than trusting an
            // upstream pass.
            .filter(|row| !banned_pseudonyms.contains(&row.pseudonym_key))
            .map(|row| {
                let record = MemberRecord {
                    display_name: row.display_name,
                    subkey_index: u32::try_from(row.subkey_index).unwrap_or(0),
                    segment_index: u32::try_from(row.segment_index).unwrap_or(0),
                    bio: row.bio,
                    pronouns: row.pronouns,
                    theme_color: row.theme_color.and_then(|c| u32::try_from(c).ok()),
                    badges: serde_json::from_str(&row.badges_json).unwrap_or_default(),
                    avatar_ref: row.avatar_ref,
                    banner_ref: row.banner_ref,
                };
                (row.pseudonym_key, record)
            })
            .collect();

        tracing::debug!(
            community = %community_id,
            members = members.len(),
            "presence poll: roster refreshed"
        );
        self.ctx
            .community_runtime
            .set_members(community_id, members);
    }

    /// Pseudonyms in `candidates` not already in the roster.
    ///
    /// The orchestrator uses the returned set to emit one
    /// `MemberDiscovered` per genuinely new peer, so returning
    /// everything would re-announce the whole community every tick.
    pub(super) fn extend_known_members_impl(
        &self,
        community_id: &str,
        candidates: Vec<String>,
    ) -> Vec<String> {
        let known = self.ctx.community_runtime.members(community_id);
        candidates
            .into_iter()
            .filter(|pseudonym| !known.contains_key(pseudonym))
            .collect()
    }
}

/// [`DaemonPresenceAdapter::scan_segment_impl`] on the segment's lease.
async fn scan_leased(
    node: &rekindle_transport::broadcast::node::TransportNode,
    lease: rekindle_transport::broadcast::dht_writes::LeaseId,
    registry_key: &str,
    max_subkey: u32,
    skip_subkey: Option<u32>,
) -> Vec<(u32, Vec<u8>)> {
    let occupied = match rekindle_transport::broadcast::dht_writes::inspect_leased_present_subkeys(
        node, lease,
    )
    .await
    {
        Ok(subkeys) => subkeys,
        Err(error) => {
            // Not an empty segment — an unreadable one. Falling
            // back to a blind sweep would turn one failed round
            // trip into 255.
            tracing::debug!(
                registry = %registry_key,
                %error,
                "presence scan: inspect failed; skipping this segment for now"
            );
            return Vec::new();
        }
    };

    let mut rows = Vec::new();
    for subkey in occupied {
        if subkey > max_subkey || Some(subkey) == skip_subkey {
            continue;
        }
        if let Ok(Some(raw)) =
            rekindle_transport::broadcast::dht_writes::get_leased(node, lease, subkey, false).await
        {
            if !raw.is_empty() {
                rows.push((subkey, raw));
            }
        }
    }
    rows
}
