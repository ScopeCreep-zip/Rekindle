//! Where the membership control messages enter the Host (plan E3.1 B2e),
//! on both hosts, and the background halves of their handling: the gossip
//! peers a JoinAccepted's member list bootstraps and the presence tick a
//! slot grant asks for.

use std::sync::Arc;

use rekindle_codec::community::envelope::ControlPayload;
use rekindle_governance_runtime::membership_events::{self, JoinAcceptedInput, MemberUpsertRow};
use rekindle_governance_runtime::GovernanceRuntimeDeps;

use super::HostGovernance;
use crate::host::community::OnlineMember;

/// The pause between two registry reads of a peer bootstrap.
const BOOTSTRAP_PACE: std::time::Duration = std::time::Duration::from_millis(200);

impl HostGovernance {
    /// Handle one membership control message from `sender_pseudonym`;
    /// `false` when `payload` is not one.
    pub async fn on_membership_control(
        &self,
        community_id: &str,
        sender_pseudonym: &str,
        payload: &ControlPayload,
    ) -> bool {
        match payload {
            ControlPayload::MemberRolesChanged {
                pseudonym_key,
                role_ids,
            } => membership_events::process_member_roles_changed(
                self,
                community_id,
                pseudonym_key,
                role_ids,
            ),
            ControlPayload::JoinAccepted {
                mek_encrypted,
                mek_generation,
                members,
                member_registry_key,
                slot_index,
                wrapped_slot_seed,
            } => {
                let input = JoinAcceptedInput {
                    mek_wire_bytes: mek_encrypted,
                    mek_generation: *mek_generation,
                    members,
                    member_registry_key: member_registry_key.as_deref(),
                    slot_index: *slot_index,
                    wrapped_slot_seed: wrapped_slot_seed.as_deref(),
                };
                membership_events::process_join_accepted(
                    self,
                    community_id,
                    sender_pseudonym,
                    input,
                )
                .await;
            }
            ControlPayload::AdminKeypairGrant {
                wrapped_owner_keypair,
                wrapped_slot_seed,
            } => membership_events::process_admin_keypair_grant(
                self,
                community_id,
                sender_pseudonym,
                wrapped_owner_keypair,
                wrapped_slot_seed,
            ),
            ControlPayload::SlotKeypairGrant {
                slot_index,
                segment_index,
                wrapped_slot_keypair,
            } => membership_events::process_slot_keypair_grant(
                self,
                community_id,
                sender_pseudonym,
                *slot_index,
                *segment_index,
                wrapped_slot_keypair,
            ),
            ControlPayload::SubmitOnboardingAnswers { answers } => {
                membership_events::process_onboarding_answers(
                    self,
                    community_id,
                    sender_pseudonym,
                    answers,
                )
                .await;
            }
            ControlPayload::MemberJoinRequest {
                pseudonym_key,
                display_name,
                claimed_subkey_index,
                route_blob,
                invite_code,
                ..
            } => membership_events::process_peer_assisted_join(
                self,
                community_id,
                pseudonym_key,
                display_name,
                *claimed_subkey_index,
                route_blob.as_deref(),
                invite_code.as_deref(),
            ),
            ControlPayload::MemberJoined {
                pseudonym_key,
                display_name,
                role_ids,
                status,
                route_blob,
            } if pseudonym_key == sender_pseudonym => self.on_member_joined(
                community_id,
                pseudonym_key,
                display_name,
                role_ids,
                status,
                route_blob.as_deref(),
            ),
            ControlPayload::MemberLeave { pseudonym_key }
            | ControlPayload::MemberRemoved { pseudonym_key }
                if pseudonym_key == sender_pseudonym =>
            {
                self.forget_member(community_id, pseudonym_key);
            }
            ControlPayload::MemberJoined { .. }
            | ControlPayload::MemberLeave { .. }
            | ControlPayload::MemberRemoved { .. } => {
                tracing::debug!(community = %community_id, sender = %sender_pseudonym, "membership announcement about another member dropped");
            }
            _ => return false,
        }
        true
    }

    /// Read each listed member's presence row from the registry, verify its
    /// signature, and add the online ones to the community's gossip overlay
    /// (a peer only once it has a route).
    pub(super) fn spawn_peer_bootstrap_impl(
        &self,
        community_id: &str,
        members: Vec<MemberUpsertRow>,
    ) {
        let Ok(transport) = self.transport() else {
            return;
        };
        let ctx = Arc::clone(&self.ctx);
        let community = community_id.to_string();
        let scope = self.scope();
        let stop = scope.token();
        scope.spawn_or_drop("peer bootstrap", async move {
            let (registry, mine) = ctx.communities.registry_and_pseudonym(&community);
            let Some(registry) = registry else {
                return;
            };
            let mut found = 0u32;
            for member in members.iter().filter(|m| mine.as_deref() != Some(&m.pseudonym_hex)) {
                if stop.run_until_cancelled(tokio::time::sleep(BOOTSTRAP_PACE)).await.is_none() {
                    return;
                }
                let Ok(Some(raw)) = rekindle_transport::broadcast::dht_writes::read_once_str(
                    transport.as_ref(),
                    &registry,
                    member.subkey_index,
                    false,
                )
                .await
                else {
                    continue;
                };
                let Some(presence) = verified_presence(&raw) else {
                    continue;
                };
                if presence.status == "offline" {
                    continue;
                }
                let online = OnlineMember {
                    location: presence.session.location.clone(),
                    last_active: presence.session.last_active,
                    route_blob: presence.route_blob,
                    status: presence.status,
                    last_seen: ctx.clock.now_secs(),
                };
                if ctx.communities.note_online_member(&community, &member.pseudonym_hex, online) {
                    found += 1;
                }
            }
            if found > 0 {
                tracing::info!(community = %community, peers = found, "gossip peers bootstrapped from the JoinAccepted member list");
            }
        });
    }

    /// One presence poll right after a slot grant: the shell runs its own
    /// polls until E3.4; the daemon runs its presence adapter's tick.
    pub(super) fn spawn_presence_poll_tick_impl(&self, community_id: &str) {
        if let Some(shell) = &self.ctx.shell {
            shell.spawn_presence_poll_tick(community_id);
            return;
        }
        let adapter = Arc::new(crate::daemon::presence_adapter::DaemonPresenceAdapter::new(
            Arc::clone(&self.ctx),
        ));
        let community = community_id.to_string();
        self.scope().spawn_or_drop("presence poll tick", async move {
            if let Err(e) = rekindle_presence::community::presence_poll_tick(adapter, &community).await {
                tracing::debug!(community = %community, error = %e, "presence poll after a slot grant failed");
            }
        });
    }
}

/// A registry presence row signed by the pseudonym it claims (architecture
/// §26 W26), or `None`.
fn verified_presence(raw: &[u8]) -> Option<rekindle_types::presence::MemberPresence> {
    if raw.is_empty() {
        return None;
    }
    let presence: rekindle_types::presence::MemberPresence = serde_json::from_slice(raw).ok()?;
    let signature: [u8; 64] = presence.signature.as_slice().try_into().ok()?;
    rekindle_secrets::derive::verify_pseudonym_signature(
        &presence.pseudonym_key.0,
        &presence.signing_bytes(),
        &signature,
    )
    .ok()?;
    Some(presence)
}
