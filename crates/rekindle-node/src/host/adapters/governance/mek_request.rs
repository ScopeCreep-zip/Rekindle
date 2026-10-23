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

/// The most `RequestMEK`s parked per community (gossip sender rate limits
/// bound how fast a non-member can refill it).
const PARKED_MEK_REQUESTS: usize = 32;

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
            self.park_mek_request(community_id, request);
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
            // Its route reaches our overlay with a later presence row.
            self.park_mek_request(community_id, request);
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
        rekindle_mek_rotation::distribute_mek(
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
        .map_err(|e| e.to_string())
    }

    /// Hold `request` until its requester's ops merge: one per requester,
    /// the oldest dropped past `PARKED_MEK_REQUESTS`.
    fn park_mek_request(&self, community_id: &str, request: &MekRequest) {
        tracing::debug!(
            community = community_id,
            requester = %request.requester_pseudonym,
            "RequestMEK parked",
        );
        let mut map = self.ctx.communities.map().write();
        let Some(community) = map.get_mut(community_id) else {
            return;
        };
        let parked = &mut community.parked_mek_requests;
        parked.retain(|r| r.requester_pseudonym != request.requester_pseudonym);
        if parked.len() >= PARKED_MEK_REQUESTS {
            parked.remove(0);
        }
        parked.push(request.clone());
    }

    /// The overlay of `community_id` gained a reachable peer: catch
    /// ourselves up (`catch_up_on_peer`) and answer what we parked for it.
    pub fn on_mesh_peers_gained<D>(&self, deps: std::sync::Arc<D>, community_id: &str)
    where
        D: MekDistributeDeps + Send + Sync + 'static,
    {
        use rekindle_governance_runtime::GovernanceRuntimeDeps as _;
        self.catch_up_on_peer(community_id);
        let host = self.clone();
        let community = community_id.to_owned();
        self.scope()
            .spawn_or_drop("parked key requests", async move {
                host.answer_parked_mek_requests(deps.as_ref(), &community)
                    .await;
            });
    }

    /// Answer the parked requests whose requester is now a member; run
    /// after a gossiped op merged. The rest stay parked.
    pub async fn answer_parked_mek_requests<D: MekDistributeDeps>(
        &self,
        deps: &D,
        community_id: &str,
    ) {
        let parked = {
            let mut map = self.ctx.communities.map().write();
            let Some(community) = map.get_mut(community_id) else {
                return;
            };
            std::mem::take(&mut community.parked_mek_requests)
        };
        // The membership read takes the store's lock: outside the write.
        let (ready, waiting): (Vec<MekRequest>, Vec<MekRequest>) =
            parked.into_iter().partition(|r| {
                pseudonym(&r.requester_pseudonym)
                    .is_some_and(|p| self.is_key_reader(community_id, &p))
            });
        for request in &waiting {
            self.park_mek_request(community_id, request);
        }
        for request in ready {
            if let Err(error) = self.answer_mek_request(deps, community_id, &request).await {
                tracing::debug!(community = community_id, %error, "parked RequestMEK not answered");
            }
        }
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
