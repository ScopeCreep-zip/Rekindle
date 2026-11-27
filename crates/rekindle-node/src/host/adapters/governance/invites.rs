//! A community's invites on the Host (plan E3.1 D1), both modes: mint one
//! (`rekindle_governance_runtime::invites::create_invite`) and keep its code
//! so its link can be shown again, list the live ones from merged
//! governance with the uses the merge counts, and revoke one by its id.

use std::fmt;

use rekindle_governance_runtime::GovernanceRuntimeError;
use rekindle_types::dto::community::{CommunityInviteCreated, CommunityInviteView};
use rekindle_types::subscription_events::{GovernanceEvent, SubscriptionEvent};

use super::HostGovernance;

/// The fingerprint of an invite to the community `anchor`: what the
/// joiner's review names (`intent_select`'s `CommunityJoin`) and what the
/// inviter's invite list shows beside each invite, one function so the two
/// screens can be compared out of band like a safety number (plan E7.15).
/// `None` when `anchor` is not a record key.
pub(crate) fn invite_fingerprint(anchor: &str) -> Option<String> {
    rekindle_crypto::identity::record_key_fingerprint(anchor)
}

/// Why an invite was not minted.
#[derive(Debug)]
pub(crate) enum InviteRefusal {
    /// The governance runtime refused it: an out-of-range `max_uses` or
    /// expiry (`OutOfBounds`), a missing permission, or a failed write.
    Runtime(GovernanceRuntimeError),
    /// The minted invite's link or stored code failed.
    Failed(String),
}

impl InviteRefusal {
    /// The IPC status the refusal answers with: the request's own bounds
    /// are the caller's error (400), not the daemon's (500).
    #[must_use]
    pub(crate) fn code(&self) -> u32 {
        match self {
            Self::Runtime(
                GovernanceRuntimeError::OutOfBounds { .. } | GovernanceRuntimeError::Malformed,
            ) => 400,
            Self::Runtime(GovernanceRuntimeError::PermissionDenied) => 403,
            Self::Runtime(_) | Self::Failed(_) => 500,
        }
    }
}

impl fmt::Display for InviteRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Runtime(e) => write!(f, "invite create: {e}"),
            Self::Failed(why) => f.write_str(why),
        }
    }
}

impl From<String> for InviteRefusal {
    fn from(why: String) -> Self {
        Self::Failed(why)
    }
}

/// The canonical link for an invite: every part the joiner needs, with the
/// op and frontier seq it was handed out at (plan IV1b).
fn invite_url(
    anchor: &str,
    invite: &rekindle_governance_runtime::invites::MintedInvite,
) -> Result<String, String> {
    use rekindle_types::key_format;
    let link = rekindle_types::invite::InviteLink {
        anchor: key_format::record_key(anchor).map_err(|e| format!("invite anchor: {e}"))?,
        secrets_record_key: key_format::record_key(&invite.secrets_record_key)
            .map_err(|e| format!("invite secrets key: {e}"))?,
        invite_code: key_format::hex16_id(&invite.code).map_err(|e| format!("invite code: {e}"))?,
        invite_op: invite.invite_op,
        invite_op_seq: invite.invite_op_seq,
        frontier_seq: invite.frontier_seq,
    };
    Ok(link.to_url())
}

impl HostGovernance {
    /// Mint an invite of `community` (`InviteCreated` in governance) and
    /// store its code as ours.
    ///
    /// # Errors
    /// The runtime refused the mint (`max_uses` outside `1..=100` among
    /// its bounds) or its governance write failed, or the code was not
    /// stored.
    pub(crate) async fn create_invite(
        &self,
        community: &str,
        max_uses: u32,
        expires_seconds: Option<u64>,
    ) -> Result<CommunityInviteCreated, InviteRefusal> {
        let invite = rekindle_governance_runtime::invites::create_invite(
            self,
            community,
            max_uses,
            expires_seconds,
        )
        .await
        .map_err(InviteRefusal::Runtime)?;
        let url = invite_url(community, &invite)?;
        let owner = self
            .ctx
            .require_owner()
            .map_err(|_| InviteRefusal::Failed("no identity is unlocked".into()))?;
        let db = self.ctx.db.current().map_err(|e| e.to_string())?;
        let (cid, code, hash, link) = (
            community.to_owned(),
            invite.code.clone(),
            invite.code_hash.clone(),
            url.clone(),
        );
        let now = self.ctx.clock.now_ms_i64();
        let inbox_owner_key = rekindle_vault::VaultKey::JoinInboxOwnerKeypair {
            invite_id: hex::encode(invite.invite_id),
        };
        self.ctx
            .vault
            .lock()
            .as_ref()
            .ok_or_else(|| InviteRefusal::Failed("the vault is locked".into()))?
            .put(&inbox_owner_key, invite.join_inbox.owner_keypair.as_bytes())
            .map_err(|e| format!("join inbox owner not stored: {e}"))?;
        let inbox = rekindle_db::repo::community_invites::JoinInbox {
            key: invite.join_inbox.record_key.clone(),
            invite_id: hex::encode(invite.invite_id),
            slots: u8::try_from(max_uses).map_err(|e| e.to_string())?,
            expires_at: invite.expires_at,
        };
        let stored_inbox = inbox.clone();
        let stored = db
            .call(move |c| {
                rekindle_db::repo::community_invites::insert(
                    c,
                    &owner,
                    &cid,
                    (&code, &hash, &link),
                    now,
                    Some(&stored_inbox),
                )
            })
            .await;
        if let Err(e) = stored {
            if let Some(vault) = self.ctx.vault.lock().as_ref() {
                if let Err(error) = vault.delete(&inbox_owner_key) {
                    tracing::warn!(%error, "unlisted join inbox owner could not be forgotten");
                }
            }
            return Err(format!("invite minted but not stored: {e}").into());
        }
        self.arm_join_inbox(community, &invite.code_hash, inbox)
            .await;
        let created_by = self
            .ctx
            .communities
            .map()
            .read()
            .get(community)
            .and_then(|c| c.my_pseudonym_key.clone())
            .unwrap_or_default();
        self.ctx.publish_event(SubscriptionEvent::Governance(
            GovernanceEvent::InviteCreated {
                community: community.to_owned(),
                invite_id: hex::encode(invite.invite_id),
                code_hash: invite.code_hash.clone(),
                created_by,
                max_uses,
                uses: 0,
                expires_at: invite.expires_at,
                created_at: u64::try_from(now).unwrap_or(0),
            },
        ));
        Ok(CommunityInviteCreated {
            invite_id: hex::encode(invite.invite_id),
            code_hash: invite.code_hash,
            url,
            expires_at: invite.expires_at,
        })
    }

    /// The live invites of `community`, by invite id: merged governance
    /// with no revocation and not expired, each with the use count the
    /// merge gates (the joins that redeemed it), ours with their link,
    /// and every one with the fingerprint the joiner's review shows.
    ///
    /// # Errors
    /// Our stored invites could not be read, or `community` is not a
    /// record key.
    pub(crate) async fn list_invites(
        &self,
        community: &str,
    ) -> Result<Vec<CommunityInviteView>, String> {
        let owner = self
            .ctx
            .require_owner()
            .map_err(|_| "no identity is unlocked")?;
        let db = self.ctx.db.current().map_err(|e| e.to_string())?;
        let cid = community.to_owned();
        let ours = db
            .call(move |c| rekindle_db::repo::community_invites::list(c, &owner, &cid))
            .await
            .map_err(|e| format!("our invites: {e}"))?;
        let now = self.ctx.clock.now_secs();
        let Some(gov) = self.ctx.communities.governance_state(community) else {
            return Ok(Vec::new());
        };
        let fingerprint =
            invite_fingerprint(community).ok_or("the community id is not a record key")?;
        // A revoke held behind our unlanded ops is in effect here at once.
        let barriers = self.ctx.communities.unlock_barriers();
        let mut live: Vec<_> = gov
            .invites
            .iter()
            .filter(|(_, inv)| inv.expires_at.is_none_or(|exp| exp > now))
            .filter(|(id, _)| !barriers.withdraws_invite(community, id))
            .collect();
        live.sort_by_key(|(id, _)| **id);
        Ok(live
            .into_iter()
            .map(|(id, inv)| {
                let own = ours.iter().find(|o| o.code_hash == inv.code_hash);
                CommunityInviteView {
                    invite_id: hex::encode(id),
                    code_hash: inv.code_hash.clone(),
                    created_by: hex::encode(inv.creator_pseudonym.0),
                    max_uses: inv.max_uses,
                    uses: inv.uses,
                    expires_at: inv.expires_at,
                    created_at: own.map(|o| o.created_at),
                    url: own.map(|o| o.url.clone()),
                    fingerprint: fingerprint.clone(),
                    inbox_jammed: own
                        .and_then(|o| o.join_inbox.as_ref())
                        .and_then(|inbox| {
                            self.ctx
                                .communities
                                .map()
                                .read()
                                .get(community)?
                                .join_inboxes
                                .get(&inbox.key)
                                .cloned()
                        })
                        .is_some_and(|watch| watch.is_jammed()),
                }
            })
            .collect())
    }

    /// Revoke the invite `invite_id` of `community` (`InviteRevoked` in
    /// governance), drop our row of it and journal `InviteRevoked`.
    ///
    /// # Errors
    /// The invite is not live, or the governance write failed.
    pub(crate) async fn revoke_invite(
        &self,
        community: &str,
        invite_id: [u8; 16],
    ) -> Result<(), String> {
        let code_hash = self
            .ctx
            .communities
            .governance_state(community)
            .and_then(|gov| gov.invites.get(&invite_id).map(|inv| inv.code_hash.clone()))
            .ok_or("no live invite has this id")?;
        rekindle_governance_runtime::invites::revoke_invite(self, community, invite_id)
            .await
            .map_err(|e| format!("invite revoke: {e}"))?;
        self.retire_ended_join_inboxes(community);
        if let (Ok(owner), Ok(db)) = (self.ctx.require_owner(), self.ctx.db.current()) {
            let (cid, hash) = (community.to_owned(), code_hash.clone());
            if let Err(e) = db
                .call(move |c| rekindle_db::repo::community_invites::delete(c, &owner, &cid, &hash))
                .await
            {
                tracing::warn!(community, error = %e, "revoked invite row not deleted");
            }
        }
        self.ctx.publish_event(SubscriptionEvent::Governance(
            GovernanceEvent::InviteRevoked {
                community: community.to_owned(),
                code_hash,
            },
        ));
        Ok(())
    }
}
