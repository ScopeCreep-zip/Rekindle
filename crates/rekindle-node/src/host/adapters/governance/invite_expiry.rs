//! Invite expiry as a Host timer (plan E3.1 C2b), one per unlock on both
//! modes. It sleeps until the soonest expiry among the invites we authored
//! (on the Host clock, so the netsim drives it), then deletes each expired
//! invite's row and journals `InviteExpired`, so every frontend drops it.
//! A joiner refuses an expired invite on its own clock (`join_flow`); this
//! timer is the inviter's side. It reads no record (D4): the invites come
//! from merged governance, and an invite created later is planned at the
//! next [`REPLAN`] at the latest.

use std::collections::HashSet;
use std::time::Duration;

use rekindle_types::subscription_events::{GovernanceEvent, SubscriptionEvent};
use tokio_util::sync::CancellationToken;

use super::HostGovernance;

/// The longest sleep, so an invite created meanwhile is planned.
const REPLAN: Duration = Duration::from_secs(60);

/// An invite we authored that expires.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct ExpiringInvite {
    pub community: String,
    pub code_hash: String,
    pub expires_at: u64,
}

pub(super) async fn run(host: HostGovernance, stop: CancellationToken) {
    let mut done: HashSet<ExpiringInvite> = HashSet::new();
    loop {
        let now = host.ctx.clock.now_secs();
        let mut next: Option<u64> = None;
        for invite in host.our_expiring_invites() {
            if invite.expires_at <= now {
                if done.insert(invite.clone()) {
                    host.expire_invite(&invite).await;
                }
            } else {
                next = Some(next.map_or(invite.expires_at, |n| n.min(invite.expires_at)));
            }
        }
        let wait = next.map_or(REPLAN, |at| Duration::from_secs(at - now).min(REPLAN));
        if stop
            .run_until_cancelled(tokio::time::sleep(wait))
            .await
            .is_none()
        {
            return;
        }
    }
}

impl HostGovernance {
    /// Every invite we authored with an expiry, from merged governance.
    pub(crate) fn our_expiring_invites(&self) -> Vec<ExpiringInvite> {
        let map = self.ctx.communities.map().read();
        let mut out = Vec::new();
        for c in map.values() {
            let (Some(gov), Some(mine)) = (&c.governance_state, &c.my_pseudonym_key) else {
                continue;
            };
            for invite in gov.invites.values() {
                let Some(expires_at) = invite.expires_at else {
                    continue;
                };
                if hex::encode(invite.creator_pseudonym.0) == *mine {
                    out.push(ExpiringInvite {
                        community: c.id.clone(),
                        code_hash: invite.code_hash.clone(),
                        expires_at,
                    });
                }
            }
        }
        out
    }

    /// Delete the expired invite's row (the desktop lists that table) and
    /// journal `InviteExpired`.
    pub(crate) async fn expire_invite(&self, invite: &ExpiringInvite) {
        if let (Ok(owner), Ok(db)) = (self.ctx.require_owner(), self.ctx.db.current()) {
            let (cid, hash) = (invite.community.clone(), invite.code_hash.clone());
            if let Err(e) = db
                .call(move |c| rekindle_db::repo::community_invites::delete(c, &owner, &cid, &hash))
                .await
            {
                tracing::warn!(community = %invite.community, error = %e, "expired invite row not deleted");
            }
        }
        self.ctx.publish_event(SubscriptionEvent::Governance(
            GovernanceEvent::InviteExpired {
                community: invite.community.clone(),
                code_hash: invite.code_hash.clone(),
            },
        ));
    }
}
