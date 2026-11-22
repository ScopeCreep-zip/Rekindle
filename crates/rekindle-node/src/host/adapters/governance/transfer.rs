//! Community transfer on the Host (ADR 0011 "Successor community", plan
//! E3.2 S7s.4, lead decisions (16), (24)–(26), (34)). A creator offers the
//! community to a member or withdraws its offer (ops judged by every
//! reader's `successor_gate`). The recipient accepts by running decision
//! (16)'s round with the offer's other creators: each friend named is sent
//! a `CreatorInvite` naming the predecessor and answers with its pseudonym
//! for the new anchor and its creator link from its old pseudonym. The
//! round removes no one (decision (25)): it finishes once every named
//! co-creator answered; if a link is missing the transfer is abandoned,
//! never completed without it. On finish the successor is created (its
//! cert names this community and the linked new pseudonyms), the
//! continuing members' sealed invites are written (decision (34)), the
//! acceptance is published here citing the offer, and each co-creator is
//! sent its join link.

use std::collections::HashMap;

use rekindle_codec::successor_invites::seal_invite;
use rekindle_crypto::group::pseudonym::{derive_community_pseudonym, sign_with_pseudonym};
use rekindle_governance::successor::{link_holds, link_signing_bytes};
use rekindle_governance_runtime::deps::GovernanceRuntimeDeps as _;
use rekindle_governance_runtime::transfer::{self, TransferRound};
use rekindle_governance_runtime::{successor_invites, CoCreatorAcceptance};
use rekindle_transport::operations::friend::creator_round::CreatorLinkSig;
use rekindle_transport::operations::friend::CreatorAnswer;
use rekindle_types::dto::community::CommunityCreated;
use rekindle_types::governance::CreatorLink;
use rekindle_types::id::PseudonymKey;

use super::co_create::{named_co_creators, PendingCreation};
use super::create_leave::NOT_CONNECTED;
use super::HostGovernance;
use crate::host::adapters::friends::HostFriends;

/// What a pending creation that is a transfer's successor carries.
pub(crate) struct PendingTransfer {
    /// The community the successor succeeds.
    pub(super) old_anchor: String,
    pub(super) round: TransferRound,
    /// Our pseudonym in `old_anchor`.
    pub(super) me_old: PseudonymKey,
    /// The continuing members' card HPKE keys (decision (34)), read before
    /// anything was reserved.
    pub(super) cards: Vec<(PseudonymKey, [u8; 32])>,
}

impl HostGovernance {
    /// Offer `anchor` to the member `to` (intent `OfferCommunityTransfer`).
    ///
    /// # Errors
    /// We are not a creator, `to` is not another member, or the op failed.
    pub(crate) async fn offer_transfer(
        &self,
        anchor: &str,
        to: &PseudonymKey,
    ) -> Result<(), String> {
        transfer::offer(self, anchor, to)
            .await
            .map_err(|e| format!("transfer not offered: {e}"))
    }

    /// Withdraw our standing transfer offer of `anchor`.
    ///
    /// # Errors
    /// We have no standing offer, or the op failed.
    pub(crate) async fn withdraw_transfer(&self, anchor: &str) -> Result<(), String> {
        transfer::withdraw(self, anchor)
            .await
            .map_err(|e| format!("transfer offer not withdrawn: {e}"))
    }

    /// Accept the standing transfer offer of `anchor` to us, collecting a
    /// creator link from each other creator through `co_creators` (one
    /// friend per other creator). Answers the reserved successor with the
    /// co-creators it waits on; with none, it is finished at once.
    ///
    /// # Errors
    /// No standing offer names us (or the set changed), `co_creators` does
    /// not name exactly one friend per other creator (the error names the
    /// creators still to collect), a continuing member's card cannot be
    /// read, or the reservation or an invite failed.
    pub(crate) async fn accept_transfer(
        &self,
        anchor: &str,
        co_creators: &[String],
    ) -> Result<CommunityCreated, String> {
        let (round, me_old) =
            transfer::round_for(self, anchor).map_err(|e| format!("transfer not accepted: {e}"))?;
        if co_creators.len() != round.others.len() {
            let waiting: Vec<String> = round.others.iter().map(PseudonymKey::to_hex).collect();
            return Err(format!(
                "name one friend per other creator of the offer; to collect: {}",
                waiting.join(", ")
            ));
        }
        let me = (self.ctx.require_owner()).map_err(|_| "no identity is unlocked")?;
        let friends = HostFriends::of(&self.ctx);
        let named = named_co_creators(&friends, &me, co_creators)?;
        if !self.transport().is_ok_and(|node| node.is_ready()) {
            return Err(NOT_CONNECTED.into());
        }
        let state =
            (self.governance_state(anchor)).ok_or("the community's governance is not loaded")?;
        let meta = (state.metadata.clone()).ok_or("the community's governance is not loaded")?;
        let chosen: Vec<PseudonymKey> = successor_invites::default_continuing(&state, &me_old)
            .into_iter()
            .filter(|m| !round.others.contains(m))
            .collect();
        let cards = successor_invites::continuing_cards(self, anchor, &state, &chosen)
            .await
            .map_err(|e| format!("transfer not accepted: {e}"))?;
        let pending = PendingTransfer {
            old_anchor: anchor.to_owned(),
            round,
            me_old,
            cards,
        };
        let description = meta.description.unwrap_or_default();
        let admission = state.effective_admission_mode();
        let created = self
            .start_round(&meta.name, &description, admission, &named, Some(pending))
            .await?;
        if named.is_empty() {
            Box::pin(self.co_creator_settled(&created.anchor, "")).await;
        }
        Ok(created)
    }

    /// Whether `link`, in an answer to our pending creation of `anchor`,
    /// is what that creation needs: none for a plain co-creation; for a
    /// transfer, a link from one of the offer's other creators to
    /// `pseudonym` that holds (decision (24)).
    pub(crate) fn answer_link_ok(
        &self,
        anchor: &str,
        pseudonym: &[u8; 32],
        link: Option<&CreatorLinkSig>,
    ) -> bool {
        let map = self.ctx.pending_creations.0.lock();
        let Some(transfer) = map.get(anchor).and_then(|p| p.transfer.as_ref()) else {
            return link.is_none();
        };
        link.is_some_and(|l| {
            transfer.round.others.contains(&PseudonymKey(l.old))
                && link_holds(&transfer.old_anchor, anchor, &creator_link(l, pseudonym))
        })
    }

    /// The transfer's round settled: link ourselves, check every link,
    /// create the successor, write the sealed invites, publish the
    /// acceptance in the old community and send each co-creator its join
    /// link. Any failure abandons the successor (released or discarded) and
    /// withdraws the co-creators: a transfer never completes without every
    /// link.
    pub(super) async fn finish_transfer(
        &self,
        pending: PendingCreation,
        transfer: PendingTransfer,
        answers: &HashMap<String, CreatorAnswer>,
    ) {
        let anchor = pending.reserved.anchor_key().to_owned();
        let joiners: Vec<String> = (pending.named.iter())
            .filter(|n| matches!(answers.get(*n), Some(Some(_))))
            .cloned()
            .collect();
        match self.complete_transfer(pending, &transfer, answers).await {
            Ok(created) => {
                self.activate_created(&created).await;
                self.send_join_links(&created, &joiners).await;
            }
            Err(e) => {
                tracing::warn!(community = %anchor, error = %e, "transfer successor not completed");
                self.withdraw(&anchor, &joiners).await;
            }
        }
    }

    async fn complete_transfer(
        &self,
        pending: PendingCreation,
        transfer: &PendingTransfer,
        answers: &HashMap<String, CreatorAnswer>,
    ) -> Result<String, String> {
        let PendingCreation {
            reserved,
            name,
            description,
            admission,
            named,
            ..
        } = pending;
        let new_anchor = reserved.anchor_key().to_owned();
        let old = transfer.old_anchor.as_str();
        let mut links = vec![self.own_link(old, &new_anchor, &transfer.me_old)?];
        let mut accepts = Vec::new();
        for peer in &named {
            if let Some(Some(answer)) = answers.get(peer) {
                let Some(link) = &answer.link else { continue };
                links.push(creator_link(link, &answer.pseudonym));
                accepts.push(CoCreatorAcceptance {
                    pseudonym: PseudonymKey(answer.pseudonym),
                    signature: answer.signature,
                });
            }
        }
        let checked =
            transfer::check_links(old, &new_anchor, &transfer.round, &transfer.me_old, &links);
        if let Err(e) = checked {
            rekindle_governance_runtime::release_reserved(self, reserved).await;
            return Err(format!("not every creator linked: {e}"));
        }
        let id = rekindle_governance_runtime::origin::finish_successor_community(
            self, reserved, &name, admission, &accepts, old,
        )
        .await
        .map_err(|e| format!("successor create failed: {e}"))?;
        self.store_created(id.clone(), &name, &description).await?;
        let record = self.seal_invites(old, &id, &transfer.cards).await?;
        transfer::publish_acceptance(self, old, &transfer.round, &id, links, &record)
            .await
            .map_err(|e| format!("successor {id} created but the acceptance not written: {e}"))?;
        Ok(id)
    }

    /// Our creator link: our pseudonym in `old_anchor` binding our
    /// pseudonym in `new_anchor`.
    fn own_link(
        &self,
        old_anchor: &str,
        new_anchor: &str,
        me_old: &PseudonymKey,
    ) -> Result<CreatorLink, String> {
        let secret = (self.ctx.identity_secret()).ok_or("no identity is unlocked")?;
        let new = derive_community_pseudonym(&secret, new_anchor);
        let new = PseudonymKey(new.verifying_key().to_bytes());
        let signing = derive_community_pseudonym(&secret, old_anchor);
        let sig = sign_with_pseudonym(&signing, &link_signing_bytes(old_anchor, new_anchor, &new));
        Ok(CreatorLink {
            old: me_old.clone(),
            new,
            sig: sig.to_vec(),
        })
    }

    /// Mint one single-use invite to `new_anchor` per continuing member,
    /// seal each to its card key and write the sealed pages (decision
    /// (34)); the first page's key.
    pub(super) async fn seal_invites(
        &self,
        old_anchor: &str,
        new_anchor: &str,
        cards: &[(PseudonymKey, [u8; 32])],
    ) -> Result<String, String> {
        let mut sealed = Vec::with_capacity(cards.len());
        for (member, hpke_key) in cards {
            let invite = (self.create_invite(new_anchor, 1, None).await).map_err(|e| {
                format!("successor {new_anchor} created but an invite failed: {e:?}")
            })?;
            let entry = seal_invite(old_anchor, new_anchor, member, hpke_key, &invite.url)
                .map_err(|e| format!("successor {new_anchor} created but an invite failed: {e}"))?;
            sealed.push(entry);
        }
        successor_invites::write_invites(self, sealed)
            .await
            .map_err(|e| format!("successor {new_anchor} created but its invites not written: {e}"))
    }

    /// Our answer to a held transfer invite (`predecessor` set): our link
    /// from our pseudonym in `predecessor` to `pseudonym`.
    pub(super) fn answer_link(
        secret: &[u8; 32],
        predecessor: &str,
        anchor: &str,
        pseudonym: &PseudonymKey,
    ) -> CreatorLinkSig {
        let signing = derive_community_pseudonym(secret, predecessor);
        CreatorLinkSig {
            old: signing.verifying_key().to_bytes(),
            sig: sign_with_pseudonym(
                &signing,
                &link_signing_bytes(predecessor, anchor, pseudonym),
            ),
        }
    }
}

/// `link` as the op's `CreatorLink` to `pseudonym`.
fn creator_link(link: &CreatorLinkSig, pseudonym: &[u8; 32]) -> CreatorLink {
    CreatorLink {
        old: PseudonymKey(link.old),
        new: PseudonymKey(*pseudonym),
        sig: link.sig.to_vec(),
    }
}
