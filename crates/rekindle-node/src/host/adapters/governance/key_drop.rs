//! The key drop (plan RV2d, owning steps RV2 + E3.5; D16, ADR 0011/0012):
//! the durable copy of a joiner's first community key.
//!
//! D16 delivers the key by `app_call` alone (`outbox/mek_transfer.rs`); a
//! per-pair path that is fresh but unreachable left a joiner with no text
//! key for over a minute, so its posts and typing were refused. As RV2b
//! gives a friend's first contact a durable copy, the inviter also writes
//! every `MekTransfer` envelope it still owes a joiner to its own member
//! record's key drop (`M`/7), and the joiner watches that subkey of the
//! record its invite names until it holds a key.
//!
//! - **Who writes:** only `K_M(inviter)` writes `M(inviter)` (Veilid's
//!   owner rule), bound to the inviter by its `MemberJoined` (ADR 0011);
//!   the joiner opens the record its invite's code-authenticated secrets
//!   name, whose key fixes the owner (`member_routes.rs`).
//! - **Who reads:** each envelope is the D16 seal, an ECDH wrap to the
//!   joiner's community pseudonym under the sender's. It names no
//!   recipient: the joiner tries each, one not wrapped for it fails the
//!   unwrap quietly, and one that unwraps goes through the call path's own
//!   receive (`receive_mek_transfer`: member, not removed, sender not
//!   departed, generation, convergence rule), so the reader validates as
//!   today's transfer does and a duplicate is idempotent.
//! - **No polling:** one watch on `M(inviter)`/7 (Veilid delivers each
//!   change of a watched subkey, 0.5.8 `storage_manager/watch_value.rs`
//!   `inbound_value_changed`), plus one read when it is armed, since the
//!   drop may predate the watch. Held only while we hold no key.
//! - **At rest:** a row receipted by its call leaves the drop at once.
//! - **Evidence:** none. A landing from the drop is not route-rotation
//!   evidence: RV2b's source 6 is a friend pair's, and a community member's
//!   route comes from its `M`/6, not a per-pair route state.

use rekindle_codec::community::envelope::{CommunityEnvelope, ControlPayload, MekTransferPayload};
use rekindle_governance_runtime::GovernanceRuntimeDeps as _;
use rekindle_records::port::OpenIntent;
use rekindle_transport::broadcast::dht_writes;
use rekindle_types::channel_keys::KeyScope;
use rekindle_types::dht_layout::member_record::MAX_SUBKEY_VALUE_LEN;

pub(crate) use rekindle_types::dht_layout::member_record::KEY_DROP;

use super::HostGovernance;

/// Frame `entries` as the drop's value: each `len (u16 BE) ‖ envelope`, in
/// order, as many as fit one subkey.
#[must_use]
pub(crate) fn encode(entries: &[Vec<u8>]) -> Vec<u8> {
    let mut out = Vec::new();
    for entry in entries {
        let Ok(len) = u16::try_from(entry.len()) else {
            continue;
        };
        if out.len() + 2 + entry.len() > MAX_SUBKEY_VALUE_LEN {
            break;
        }
        out.extend_from_slice(&len.to_be_bytes());
        out.extend_from_slice(entry);
    }
    out
}

/// The envelopes of a drop's value; a truncated tail is dropped.
#[must_use]
pub(crate) fn decode(mut value: &[u8]) -> Vec<&[u8]> {
    let mut entries = Vec::new();
    while let Some(head) = value.get(..2) {
        let len = usize::from(u16::from_be_bytes([head[0], head[1]]));
        let Some(entry) = value.get(2..2 + len) else {
            break;
        };
        entries.push(entry);
        value = &value[2 + len..];
    }
    entries
}

/// The `MekTransfer` an envelope carries, for `community`.
fn transfer_of(envelope: &[u8], community: &str) -> Option<MekTransferPayload> {
    match rekindle_codec::capnp_envelope::try_decode_community_envelope(envelope)
        .ok()
        .flatten()
        .and_then(CommunityEnvelope::into_control)
    {
        Some(ControlPayload::MekTransfer(p)) if p.community_id == community => Some(p),
        _ => None,
    }
}

impl HostGovernance {
    /// Write our key drop in `community_id`: the envelopes of every
    /// unsettled `mek_transfer` row of the community but `exclude`'s (a row
    /// just receipted, not settled yet). Nothing is written while there is
    /// nothing to add or take away.
    pub(crate) async fn refresh_key_drop(&self, community_id: &str, exclude: Option<&str>) {
        let Some(transfers) = self.ctx.outbox.mek_transfers() else {
            return;
        };
        // Read the rows and write the subkey under the community's one
        // writer lock, so a later row set is never overwritten by an older.
        let Some(writer) = (self.ctx.communities.map().read())
            .get(community_id)
            .map(|c| std::sync::Arc::clone(&c.key_drop_writer))
        else {
            return;
        };
        let _writing = writer.lock().await;
        let entries: Vec<Vec<u8>> = match transfers.sealed(community_id).await {
            Ok(rows) => rows
                .into_iter()
                .filter(|(member, _)| Some(member.as_str()) != exclude)
                .map(|(_, sealed)| sealed)
                .collect(),
            Err(error) => {
                tracing::warn!(community = community_id, %error, "key drop rows not read");
                return;
            }
        };
        if entries.is_empty() && exclude.is_none() {
            return;
        }
        // Newest generation first (D16 history bundle): the bound is one
        // subkey value (`MAX_SUBKEY_VALUE_LEN`, Veilid's 32,768 B), so what
        // fits always holds the key a joiner sends under; an older one past
        // the bound stays a queued row the call path delivers.
        let mut entries = entries;
        entries.sort_by_key(|entry| {
            std::cmp::Reverse(transfer_of(entry, community_id).map_or(0, |t| t.generation))
        });
        let value = encode(&entries);
        let left = entries.len() - decode(&value).len();
        if left > 0 {
            tracing::warn!(
                community = community_id,
                left,
                "key drop full: the older keys past it go by call only"
            );
        }
        if let Err(error) = self.write_key_drop(community_id, value).await {
            tracing::warn!(community = community_id, %error, "key drop not written");
        } else {
            tracing::info!(
                community = community_id,
                entries = entries.len(),
                "key drop written (RV2d)"
            );
        }
    }

    async fn write_key_drop(&self, community_id: &str, value: Vec<u8>) -> Result<(), String> {
        let keys = self.own_record_keys(community_id)?;
        let own = keys.own(community_id);
        let node = self.transport().map_err(|e| e.to_string())?;
        let lease = dht_writes::acquire_str(&node, own.record, Some(own.writer), OpenIntent::Hold)
            .await
            .map_err(|e| e.to_string())?;
        let written =
            dht_writes::set_own_leased_str(&node, lease, KEY_DROP, value, Some(own.writer)).await;
        dht_writes::release(&node, lease).await;
        match written {
            Ok(None) => Ok(()),
            Ok(Some(newer)) => Err(format!("a newer drop at seq {} held", newer.seq)),
            Err(e) => Err(e.to_string()),
        }
    }

    /// [`Self::refresh_key_drop`] without `member`, off the caller's task.
    pub(crate) fn spawn_key_drop_refresh(&self, community_id: &str, member: &str) {
        let host = self.clone();
        let (community, member) = (community_id.to_owned(), member.to_owned());
        self.scope().spawn_or_drop("key drop refresh", async move {
            host.refresh_key_drop(&community, Some(&member)).await;
        });
    }

    fn holds_community_key(&self, community_id: &str) -> bool {
        self.current_key_generation(community_id, KeyScope::Community)
            .is_some()
    }

    /// The community whose key drop watch holds `record`.
    pub(super) fn community_of_key_drop(&self, record: &str) -> Option<String> {
        let map = self.ctx.communities.map().read();
        let mut ids: Vec<&String> = map
            .values()
            .filter(|c| c.key_drop.as_ref().is_some_and(|(r, _)| r == record))
            .map(|c| &c.id)
            .collect();
        ids.sort_unstable();
        ids.first().map(|id| (*id).clone())
    }

    /// Watch `inviter_record`'s key drop while we hold no key of
    /// `community_id`, and read it once.
    pub(crate) async fn arm_key_drop(&self, community_id: &str, inviter_record: &str) {
        if self.holds_community_key(community_id)
            || self.own_member_record(community_id).as_deref() == Some(inviter_record)
        {
            return;
        }
        let Ok(node) = self.transport() else {
            return;
        };
        let lease =
            match dht_writes::acquire_str(&node, inviter_record, None, OpenIntent::Check).await {
                Ok(lease) => lease,
                Err(e) => {
                    tracing::debug!(community_id, error = %e, "key drop record did not open");
                    return;
                }
            };
        // A refused watch stays set in the pool, which re-arms it (D4).
        if let Err(e) = dht_writes::watch_leased(&node, lease, &[KEY_DROP]).await {
            tracing::debug!(community_id, error = %e, "key drop watch not yet set");
        }
        let surplus = {
            let mut map = self.ctx.communities.map().write();
            match map.get_mut(community_id) {
                Some(c) if c.key_drop.is_none() => {
                    c.key_drop = Some((inviter_record.to_owned(), lease));
                    None
                }
                _ => Some(lease),
            }
        };
        if let Some(lease) = surplus {
            dht_writes::release(&node, lease).await;
        }
        self.read_key_drop(community_id).await;
    }

    /// Arm the key drop watch again after an unlock (the watch is not
    /// persisted): a community we hold no key for watches its inviter's
    /// drop, the inviter named by the merged state itself.
    pub(crate) async fn rearm_key_drop(&self, community_id: &str) {
        if self.holds_community_key(community_id) {
            return;
        }
        if let Some(record) = self.inviter_record_of(community_id).await {
            self.arm_key_drop(community_id, &record).await;
        } else {
            tracing::debug!(community_id, "key drop not re-armed: no inviter known");
        }
    }

    /// Our inviter's member record in `community_id`: our `MemberJoined`
    /// (from our op store) proves the invite by its code hash, the merged
    /// `InviteCreated` with that hash names its creator, and the creator's
    /// `MemberJoined` names its record (ADR 0011).
    async fn inviter_record_of(&self, community_id: &str) -> Option<String> {
        let joined = {
            let map = self.ctx.communities.map().read();
            let c = map.get(community_id)?;
            let me = super::watch_ring::my_pseudonym(c)?;
            c.governance_state.as_ref()?.members.get(&me)?.joined.hash
        };
        let (owner, db) = self.owner_and_db()?;
        let community = community_id.to_owned();
        let stored = db
            .call(move |conn| {
                rekindle_db::repo::governance_ops::get_op(conn, &owner, &community, &joined.0)
            })
            .await
            .ok()??;
        let op = rekindle_codec::governance_op::decode_op(&stored.op)
            .ok()?
            .op;
        let rekindle_types::governance::GovernanceEntry::MemberJoined {
            invite_proof: Some(proof),
            ..
        } = op.body
        else {
            return None;
        };
        let map = self.ctx.communities.map().read();
        let state = map.get(community_id)?.governance_state.as_ref()?;
        let inviter = state
            .invites
            .values()
            .find(|invite| invite.code_hash == proof.code_hash)?
            .creator_pseudonym
            .clone();
        Some(state.members.get(&inviter)?.member_record.clone())
    }

    /// Release `community_id`'s key drop watch, off the caller's task, once
    /// we hold a key of it by any path.
    pub(crate) fn spawn_key_drop_release(&self, community_id: &str) {
        let armed = (self.ctx.communities.map().read())
            .get(community_id)
            .is_some_and(|c| c.key_drop.is_some());
        if !armed || !self.holds_community_key(community_id) {
            return;
        }
        let host = self.clone();
        let community = community_id.to_owned();
        self.scope().spawn_or_drop("key drop release", async move {
            host.release_key_drop(&community).await;
        });
    }

    /// Release `community_id`'s key drop watch.
    pub(crate) async fn release_key_drop(&self, community_id: &str) {
        let lease = (self.ctx.communities.map().write())
            .get_mut(community_id)
            .and_then(|c| c.key_drop.take())
            .map(|(_, lease)| lease);
        if let (Some(lease), Ok(node)) = (lease, self.transport()) {
            dht_writes::release(&node, lease).await;
        }
    }

    /// Read the watched key drop of `community_id` and take the key wrapped
    /// for us, through the call path's receive; once we hold a key, by
    /// either path, the watch is released.
    pub(super) async fn read_key_drop(&self, community_id: &str) {
        let lease = {
            let map = self.ctx.communities.map().read();
            map.get(community_id)
                .and_then(|c| c.key_drop.as_ref().map(|(_, lease)| *lease))
        };
        let (Some(lease), Ok(node)) = (lease, self.transport()) else {
            return;
        };
        if !self.holds_community_key(community_id) {
            match dht_writes::read_hinted_leased(&node, lease, KEY_DROP).await {
                Ok(Some((_, value))) => self.take_dropped_key(community_id, &value),
                Ok(None) => {}
                Err(e) => tracing::debug!(community_id, error = %e, "key drop not read"),
            }
        }
        if self.holds_community_key(community_id) {
            self.release_key_drop(community_id).await;
        }
    }

    pub(crate) fn take_dropped_key(&self, community_id: &str, value: &[u8]) {
        let Some(secret) = self.identity_secret_impl() else {
            return;
        };
        // Oldest generation first: the first installs as our key and each
        // newer one raises it, so no older key arrives as history behind a
        // catch-up that passed its entries (D16, `key_history.rs`).
        let mut transfers: Vec<MekTransferPayload> = decode(value)
            .into_iter()
            .filter_map(|envelope| transfer_of(envelope, community_id))
            .collect();
        transfers.sort_by_key(|t| t.generation);
        for transfer in transfers {
            let Some(scope) = KeyScope::from_wire(transfer.channel_id.as_deref()) else {
                continue;
            };
            let projected = rekindle_transport::payload::rpc::MekTransferPayload {
                scope,
                generation: transfer.generation,
                rotator_pseudonym_hex: transfer.sender_pseudonym.clone(),
                wrapped_mek: transfer.wrapped_mek.clone(),
            };
            // Another joiner's copy does not unwrap under our pseudonym.
            if rekindle_transport::operations::mek::unwrap_mek_transfer_payload(
                &projected,
                &secret,
                community_id,
            )
            .is_err()
            {
                continue;
            }
            match self.receive_mek_transfer(&transfer) {
                Ok(_) => tracing::info!(
                    community = community_id,
                    "community key taken from the inviter's key drop (RV2d)"
                ),
                Err(refused) => tracing::warn!(
                    community = community_id,
                    reason = refused.reason(),
                    "key drop entry refused"
                ),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_drop_round_trips_its_envelopes_and_drops_a_truncated_tail() {
        let value = super::encode(&[b"one".to_vec(), b"two!".to_vec()]);
        assert_eq!(super::decode(&value), vec![&b"one"[..], &b"two!"[..]]);
        assert_eq!(super::decode(&value[..value.len() - 1]), vec![&b"one"[..]]);
    }

    #[test]
    fn a_drop_holds_only_what_fits_one_subkey() {
        let big = vec![0u8; super::MAX_SUBKEY_VALUE_LEN / 2];
        let value = super::encode(&[big.clone(), big.clone(), big]);
        assert_eq!(super::decode(&value).len(), 1);
        assert!(value.len() <= super::MAX_SUBKEY_VALUE_LEN);
    }
}
