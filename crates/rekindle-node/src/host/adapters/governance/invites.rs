//! A community's invites on the Host (plan E3.1 D1), both modes: mint one
//! (`rekindle_governance_runtime::invites::create_invite`) and keep its code
//! so its redemptions are counted and its link can be shown again, list the
//! live ones from merged governance, and revoke one by its invite id.

use rekindle_types::dto::community::{CommunityInviteCreated, CommunityInviteView};
use rekindle_types::subscription_events::{GovernanceEvent, SubscriptionEvent};

use super::HostGovernance;

/// The canonical link for an invite: all three parts the joiner needs.
fn invite_url(
    governance_key: &str,
    secrets_record_key: &str,
    code: &str,
) -> Result<String, String> {
    use rekindle_types::key_format;
    let link = rekindle_types::invite::InviteLink {
        governance_key: key_format::record_key(governance_key)
            .map_err(|e| format!("invite governance key: {e}"))?,
        secrets_record_key: key_format::record_key(secrets_record_key)
            .map_err(|e| format!("invite secrets key: {e}"))?,
        invite_code: key_format::hex16_id(code).map_err(|e| format!("invite code: {e}"))?,
    };
    Ok(link.to_url())
}

impl HostGovernance {
    /// Mint an invite of `community` (`InviteCreated` in governance) and
    /// store its code as ours.
    ///
    /// # Errors
    /// The mint or its governance write failed, or the code was not stored.
    pub(crate) async fn create_invite(
        &self,
        community: &str,
        max_uses: u32,
        expires_seconds: Option<u64>,
    ) -> Result<CommunityInviteCreated, String> {
        let invite = rekindle_governance_runtime::invites::create_invite(
            self,
            community,
            max_uses,
            expires_seconds,
        )
        .await
        .map_err(|e| format!("invite create: {e}"))?;
        let url = invite_url(community, &invite.secrets_record_key, &invite.code)?;
        let owner = self
            .ctx
            .require_owner()
            .map_err(|_| "no identity is unlocked")?;
        let db = self.ctx.db.current().map_err(|e| e.to_string())?;
        let (cid, code, hash) = (
            community.to_owned(),
            invite.code.clone(),
            invite.code_hash.clone(),
        );
        let now = self.ctx.clock.now_ms_i64();
        db.call(move |c| {
            rekindle_db::repo::community_invites::insert(c, &owner, &cid, &code, &hash, now)
        })
        .await
        .map_err(|e| format!("invite minted but not stored: {e}"))?;
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
                max_uses: (max_uses != 0).then_some(max_uses),
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
    /// with no revocation and not expired, ours with their use count and
    /// link.
    ///
    /// # Errors
    /// Our stored invites could not be read.
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
        let mut live: Vec<_> = gov
            .invites
            .iter()
            .filter(|(_, inv)| inv.expires_at.is_none_or(|exp| exp > now))
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
                    max_uses: (inv.max_uses != 0).then_some(inv.max_uses),
                    uses: own.map_or(0, |o| o.uses),
                    expires_at: inv.expires_at,
                    created_at: own.map(|o| o.created_at),
                    url: own
                        .and_then(|o| invite_url(community, &inv.secrets_record_key, &o.code).ok()),
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
