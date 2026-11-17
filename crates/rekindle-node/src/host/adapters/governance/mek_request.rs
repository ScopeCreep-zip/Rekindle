//! The `RequestMEK` responder (plan E3.2 S6; moved from the desktop's
//! `mek_rotation_orchestrators::handle_request_mek`), on both modes.
//!
//! A joiner holds no community key: the invite ships none (D11), so it
//! asks the mesh with `RequestMEK` generation 0. One peer answers, the one
//! `rekindle_secrets::rotator` elects for the requester at the cascade
//! index, and only for a requester who is a member in the merged op
//! state: a verified `MemberJoined` that is neither removed nor banned and,
//! under `ApprovalRequired`, approved (owner question Q6: no key reaches a
//! pending member). The key is sealed to the requester's pseudonym by
//! `rekindle_mek_rotation::distribute_mek` (`rekindle-secrets` ECDH wrap).
//! Rotation itself is untouched; E3.5 replaces it.

use rekindle_crypto::group::media_key::MediaEncryptionKey;
use rekindle_mek_rotation::{MekDistributeDeps, RotationRecipient};
use rekindle_secrets::rotator::{cascade_candidates, select_mek_responder};
use rekindle_types::channel_keys::KeyScope;
use rekindle_types::id::PseudonymKey;

use super::moderation::pseudonym;
use super::HostGovernance;

/// One inbound `RequestMEK`, as the wire carries it.
#[derive(Debug, Clone)]
pub struct MekRequest {
    /// A channel's id, or empty for the community key.
    pub channel_id: String,
    /// 0 asks for the current generation.
    pub needed_generation: u64,
    pub requester_pseudonym: String,
    pub cascade_index: u32,
}

/// Are we the responder `rekindle_secrets::rotator` elects for this
/// requester at this cascade index, among the online members and us?
fn elected(
    me: &PseudonymKey,
    requester: &PseudonymKey,
    online: &[RotationRecipient],
    cascade_index: u32,
) -> bool {
    let mut members: Vec<PseudonymKey> = online
        .iter()
        .filter_map(|r| pseudonym(&r.pseudonym_hex))
        .collect();
    members.push(me.clone());
    if cascade_index == 0 {
        return select_mek_responder(requester, &members).as_ref() == Some(me);
    }
    let others: Vec<PseudonymKey> = members.into_iter().filter(|m| m != requester).collect();
    let cascade = cascade_candidates(requester, &others, cascade_index as usize);
    cascade.get(cascade_index as usize) == Some(me)
}

impl HostGovernance {
    /// Is `requester` a member in the merged op state: joined, not
    /// removed, not banned, and approved where approval is required (the
    /// merge keeps a pending joiner out of `members`).
    pub(crate) fn is_key_reader(&self, community_id: &str, requester: &PseudonymKey) -> bool {
        self.ctx
            .communities
            .governance_state(community_id)
            .is_some_and(|state| {
                state.members.contains_key(requester) && !state.bans.contains(requester)
            })
    }

    /// Answer a `RequestMEK` when we are the elected responder and the
    /// requester is a member: seal the asked-for key to it.
    ///
    /// # Errors
    /// The request names no scope or no pseudonym, the requester is not a
    /// member, we hold no such key, or the delivery failed.
    pub async fn answer_mek_request<D: MekDistributeDeps>(
        &self,
        deps: &D,
        community_id: &str,
        request: &MekRequest,
    ) -> Result<(), String> {
        let scope = KeyScope::from_wire(Some(&request.channel_id))
            .ok_or_else(|| format!("RequestMEK names no key scope: {:?}", request.channel_id))?;
        let requester =
            pseudonym(&request.requester_pseudonym).ok_or("invalid requester pseudonym")?;
        let me = deps
            .my_pseudonym(community_id)
            .ok_or("no pseudonym in this community")?;
        if me == requester {
            return Ok(());
        }
        let online = deps.online_recipients(community_id, None);
        if !elected(&me, &requester, &online, request.cascade_index) {
            return Ok(());
        }
        if !self.is_key_reader(community_id, &requester) {
            return Err(format!(
                "RequestMEK from {} refused: not a member",
                request.requester_pseudonym
            ));
        }
        let Some(route_blob) = online
            .into_iter()
            .find(|r| r.pseudonym_hex == request.requester_pseudonym)
            .map(|r| r.route_blob)
        else {
            return Err("requester not in online peer set".into());
        };
        let mek = Self::requested_key(deps, community_id, scope, request.needed_generation)
            .await
            .ok_or_else(|| {
                format!(
                    "no key at generation {} (nor newer) for {community_id} {scope}",
                    request.needed_generation
                )
            })?;
        let unreached = rekindle_mek_rotation::distribute_mek(
            deps,
            community_id,
            scope,
            &mek,
            &[RotationRecipient {
                pseudonym_hex: request.requester_pseudonym.clone(),
                route_blob,
            }],
        )
        .await
        .map_err(|e| e.to_string())?;
        if !unreached.is_empty() {
            return Err("the key push did not reach the requester".into());
        }
        tracing::info!(community = community_id, requester = %request.requester_pseudonym, "RequestMEK answered");
        Ok(())
    }

    /// The inviter's half of a join (plan E3.2, D16): `joiner`'s
    /// `MemberJoined` from its `BootstrapRequest` merged, so commit the
    /// current community key, sealed to it, as a `mek_transfer` outbox row
    /// (D22): `app_call`ed by `MekTransfer` to the route it sent (never
    /// inside `bootstrapResponseV2`, D11), retried until its
    /// `MekTransferAck` or its deadline, discarded if it departs. Only the
    /// commit is awaited; the outbox sends it.
    pub async fn deliver_key_to_joiner<D>(
        &self,
        deps: &D,
        community_id: &str,
        joiner: &rekindle_governance_runtime::BootstrapJoiner,
    ) where
        D: MekDistributeDeps + Sync + ?Sized,
    {
        if let Err(error) = self.queue_key_for_joiner(deps, community_id, joiner).await {
            tracing::warn!(community = %community_id, %error, "joiner key not queued");
        }
    }

    async fn queue_key_for_joiner<D: MekDistributeDeps + Sync + ?Sized>(
        &self,
        deps: &D,
        community_id: &str,
        joiner: &rekindle_governance_runtime::BootstrapJoiner,
    ) -> Result<(), String> {
        if joiner.route.is_empty() {
            return Err("the joiner sent no route".into());
        }
        if !self.member_reads_keys(community_id, &joiner.pseudonym) {
            return Err(format!("{} is not a member", joiner.pseudonym));
        }
        let mek = deps
            .cache()
            .current(community_id, KeyScope::Community)
            .ok_or_else(|| format!("no community key for {community_id}"))?;
        let sealed = rekindle_mek_rotation::seal_mek_transfer(
            deps,
            community_id,
            KeyScope::Community,
            &mek,
            &joiner.pseudonym,
        )
        .map_err(|e| e.to_string())?;
        let transfers = self
            .ctx
            .outbox
            .mek_transfers()
            .ok_or("no identity is unlocked")?;
        // One row per (member, generation): a joiner's repeated
        // bootstrap does not queue the key twice.
        let client_id = format!(
            "MekTransfer:{community_id}:{}:{}",
            joiner.pseudonym,
            mek.generation()
        );
        transfers
            .queue(
                community_id,
                &joiner.pseudonym,
                &client_id,
                &joiner.route,
                &sealed,
            )
            .await
            .map_err(|e| e.to_string())?;
        tracing::info!(community = community_id, joiner = %joiner.pseudonym, "community key queued for the joiner (D16)");
        Ok(())
    }

    /// Whether `member_hex` is a key reader of `community_id`
    /// ([`Self::is_key_reader`]); `false` for a malformed pseudonym.
    pub(crate) fn member_reads_keys(&self, community_id: &str, member_hex: &str) -> bool {
        pseudonym(member_hex).is_some_and(|p| self.is_key_reader(community_id, &p))
    }

    /// The route `member_hex`'s presence in `community_id` carries now.
    pub(crate) fn member_route(&self, community_id: &str, member_hex: &str) -> Option<Vec<u8>> {
        self.online_members_impl(community_id)
            .into_iter()
            .find(|m| m.pseudonym_hex == member_hex && !m.route_blob.is_empty())
            .map(|m| m.route_blob)
    }

    /// The key of `scope` at exactly `generation` we hold in memory: the
    /// key a `mek_transfer` row sealed, under which its ack's
    /// key-confirmation tag must verify (review S7m part 1).
    pub(crate) fn held_key(
        &self,
        community_id: &str,
        scope: KeyScope,
        generation: u64,
    ) -> Option<MediaEncryptionKey> {
        rekindle_mek_rotation::ChannelMekCache::get(
            &crate::daemon::mek_rotation::MekCacheAdapter::new(std::sync::Arc::clone(
                &self.ctx.mek_cache,
            )),
            community_id,
            scope,
            generation,
        )
    }

    /// `app_call` a member's route with `payload` (unframed, sealed).
    pub(crate) async fn call_member(
        &self,
        route: &[u8],
        payload: Vec<u8>,
    ) -> Result<Vec<u8>, String> {
        self.app_call_peer_impl(route, payload)
            .await
            .map_err(|e| e.to_string())
    }

    /// Generation 0 is the current key, with its provenance (a rebuilt key
    /// would lose the minter's election rank). Otherwise the exact
    /// generation from memory or the vault; when that is gone and ours is
    /// newer, ours (the requester takes any generation at or above the
    /// one it needs).
    async fn requested_key<D: MekDistributeDeps>(
        deps: &D,
        community_id: &str,
        scope: KeyScope,
        generation: u64,
    ) -> Option<MediaEncryptionKey> {
        let current = deps.cache().current(community_id, scope);
        if generation == 0 {
            return current;
        }
        if let Some(mek) = deps.cache().get(community_id, scope, generation) {
            return Some(mek);
        }
        let stored = deps
            .persist()
            .load_mek_for_generation(community_id, scope, generation)
            .await
            .ok()
            .flatten()
            .and_then(|bytes| <[u8; 32]>::try_from(bytes.as_slice()).ok());
        match stored {
            Some(bytes) => Some(MediaEncryptionKey::from_bytes(bytes, generation)),
            None => current.filter(|mek| mek.generation() > generation),
        }
    }
}
