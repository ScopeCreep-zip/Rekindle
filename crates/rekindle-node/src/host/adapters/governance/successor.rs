//! Successor communities on the Host (ADR 0011 "Successor community",
//! plan E3.2 S7s.4): a creator re-creates the community under a new anchor
//! whose cert names it as `predecessor` and names that anchor here (both
//! links); a member follows an offered successor by joining it after
//! re-checking both links; a window reads the offers made this session.

use rekindle_governance_runtime::deps::GovernanceRuntimeDeps as _;
use rekindle_governance_runtime::GovernanceRuntimeEvent;
use rekindle_governance_runtime::{successor, successor_invites};
use rekindle_types::dto::community::{CommunityCreated, CommunityJoined, SuccessorOffer};
use rekindle_types::id::PseudonymKey;
use rekindle_types::invite::InviteLink;

use super::join_bootstrap::JoinFailure;
use super::HostGovernance;

impl HostGovernance {
    /// Publish a successor of `anchor`: read the continuing members' cards
    /// (`continuing`, default the roster without departed members; lead
    /// decision (34)), create it (same name, admission and description; we
    /// are its sole creator), store it as a create does, mint one
    /// single-use invite per continuing member, seal each to its card
    /// `hpkeKey` into sealed pages, then write the `CommunitySuccessor` op
    /// here naming the first page.
    ///
    /// # Errors
    /// Not connected, not a creator, a continuing member's card cannot be
    /// read (nothing is created), or a create, mint or write failed (the
    /// new community then exists, unnamed here, and the error says so).
    pub(crate) async fn publish_successor(
        &self,
        anchor: &str,
        continuing: Option<&[PseudonymKey]>,
    ) -> Result<CommunityCreated, String> {
        if !self.transport().is_ok_and(|node| node.is_ready()) {
            return Err(super::create_leave::NOT_CONNECTED.into());
        }
        let state =
            (self.governance_state(anchor)).ok_or("the community's governance is not loaded")?;
        let meta = (state.metadata.clone()).ok_or("the community's governance is not loaded")?;
        let me = self.successor_pseudonym(anchor)?;
        let chosen = continuing.map_or_else(
            || successor_invites::default_continuing(&state, &me),
            <[PseudonymKey]>::to_vec,
        );
        let cards = successor_invites::continuing_cards(self, anchor, &state, &chosen)
            .await
            .map_err(|e| format!("successor not published: {e}"))?;
        let new_anchor = successor::create_successor(self, anchor)
            .await
            .map_err(|e| format!("successor create failed: {e}"))?;
        let description = meta.description.unwrap_or_default();
        let created = (self.store_created(new_anchor.clone(), &meta.name, &description)).await?;
        let record = self.seal_invites(anchor, &new_anchor, &cards).await?;
        successor::publish(self, anchor, &new_anchor, &record)
            .await
            .map_err(|e| format!("successor {new_anchor} created but not named here: {e}"))?;
        Ok(created)
    }

    /// Follow `new_anchor`, a successor of `anchor`: both links are
    /// re-checked from the new community's side, our sealed invite is
    /// opened from the successor's `invitesRecord` with our card HPKE
    /// secret (lead decision (34)), then the new community is joined with it.
    ///
    /// # Errors
    /// A link fails or the far cert cannot be read, the publisher did not
    /// continue us (the error says so), our invite does not open or does not
    /// open `new_anchor`, or the join failed.
    pub(crate) async fn follow_successor(
        &self,
        anchor: &str,
        new_anchor: &str,
    ) -> Result<CommunityJoined, JoinFailure> {
        let fail = |e: String| JoinFailure::from(format!("successor not followed: {e}"));
        let record = (successor::check_follow(self, anchor, new_anchor).await)
            .map_err(|e| fail(e.to_string()))?;
        let me = self.successor_pseudonym(anchor).map_err(fail)?;
        let secret = self.member_hpke_secret(anchor).map_err(fail)?;
        let invite =
            successor_invites::open_own_invite(self, anchor, new_anchor, &record, &me, &secret)
                .await
                .map_err(|e| fail(e.to_string()))?;
        let link = InviteLink::parse(&invite).map_err(|e| fail(format!("sealed invite: {e}")))?;
        if link.anchor.as_str() != new_anchor {
            return Err(fail("the sealed invite does not open the successor".into()));
        }
        self.join(&link).await
    }

    /// Our pseudonym in `anchor`.
    fn successor_pseudonym(&self, anchor: &str) -> Result<PseudonymKey, String> {
        (self.community_membership(anchor))
            .and_then(|m| m.my_pseudonym_hex)
            .map(|hex| PseudonymKey::from_hex_lossy(&hex))
            .ok_or_else(|| format!("no pseudonym in {anchor}"))
    }

    /// Our member card's HPKE secret for `anchor`, from the vault.
    fn member_hpke_secret(&self, anchor: &str) -> Result<zeroize::Zeroizing<[u8; 32]>, String> {
        let vault = self.ctx.vault.lock();
        let vault = vault.as_ref().ok_or("no identity is unlocked")?;
        rekindle_vault::typed::community_keys::load_member_hpke_secret(vault, anchor)
            .map_err(|e| format!("member card secret: {e}"))?
            .ok_or_else(|| format!("no member card secret for {anchor}"))
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
