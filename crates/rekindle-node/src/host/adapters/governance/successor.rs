//! Successor communities on the Host (ADR 0011 "Successor community",
//! plan E3.2 S7s.4): a creator re-creates the community under a new anchor
//! whose cert names it as `predecessor` and names that anchor here (both
//! links); a member follows an offered successor by joining it after
//! re-checking both links; a window reads the offers made this session.

use rekindle_governance_runtime::deps::GovernanceRuntimeDeps as _;
use rekindle_governance_runtime::successor;
use rekindle_governance_runtime::GovernanceRuntimeEvent;
use rekindle_types::dto::community::{CommunityCreated, CommunityJoined, SuccessorOffer};
use rekindle_types::id::PseudonymKey;
use rekindle_types::invite::InviteLink;

use super::join_bootstrap::JoinFailure;
use super::HostGovernance;

impl HostGovernance {
    /// Publish a successor of `anchor`: create it (same name, admission
    /// and description; we are its sole creator), store it as a create
    /// does, then write the `CommunitySuccessor` op here.
    ///
    /// # Errors
    /// Not connected, not a creator, or a create or write failed (the new
    /// community then exists, unnamed here, and the error says so).
    pub(crate) async fn publish_successor(&self, anchor: &str) -> Result<CommunityCreated, String> {
        if !self.transport().is_ok_and(|node| node.is_ready()) {
            return Err(super::create_leave::NOT_CONNECTED.into());
        }
        let meta = (self.governance_state(anchor))
            .and_then(|s| s.metadata)
            .ok_or("the community's governance is not loaded")?;
        let new_anchor = successor::create_successor(self, anchor)
            .await
            .map_err(|e| format!("successor create failed: {e}"))?;
        let description = meta.description.unwrap_or_default();
        let created = (self.store_created(new_anchor.clone(), &meta.name, &description)).await?;
        successor::publish(self, anchor, &new_anchor)
            .await
            .map_err(|e| format!("successor {new_anchor} created but not named here: {e}"))?;
        Ok(created)
    }

    /// Follow `link`'s community, a successor of `anchor`: both links are
    /// re-checked from the new community's side, then it is joined.
    ///
    /// # Errors
    /// A link fails or the far cert cannot be read, or the join failed.
    pub(crate) async fn follow_successor(
        &self,
        anchor: &str,
        link: &InviteLink,
    ) -> Result<CommunityJoined, JoinFailure> {
        successor::check_follow(self, anchor, link.anchor.as_str())
            .await
            .map_err(|e| JoinFailure::from(format!("successor not followed: {e}")))?;
        self.join(link).await
    }

    /// The successors of `anchor` offered this session.
    pub(crate) fn successor_offers(&self, anchor: &str) -> Vec<SuccessorOffer> {
        (self.successor_judgements(anchor).offers().into_iter())
            .filter_map(|offer| match offer {
                GovernanceRuntimeEvent::CommunitySuccessorOffered {
                    community_id,
                    new_anchor,
                    signer,
                    reason,
                    creators,
                } => Some(SuccessorOffer {
                    community: community_id,
                    new_anchor,
                    signer: signer.to_hex(),
                    reason,
                    creators: creators.iter().map(PseudonymKey::to_hex).collect(),
                }),
                _ => None,
            })
            .collect()
    }
}
