//! Community transfer on the Host (ADR 0011 "Successor community", plan
//! E3.2 S7s.4, lead decisions (24)–(26), (34), (46)). A creator offers the
//! community to a member or withdraws its offer (ops judged by every
//! reader's `successor_gate`). The recipient accepts through the link
//! round inside the community: its `SuccessorLinkRequest` asks the offer's
//! other creators by their pseudonyms here, so they need not be its
//! friends, and each answers with a `SuccessorLink` op (intent
//! `AnswerSuccessorLink`). The round removes no one (decision (25)); the
//! transfer never completes without every link. On finish the successor is
//! created (its cert names this community and the linked new pseudonyms),
//! the continuing members' sealed invites are written (decision (34)), the
//! co-creators among them, and the acceptance is published here citing
//! the offer and the link ops.

use rekindle_codec::successor_invites::seal_invite;
use rekindle_crypto::group::pseudonym::{derive_community_pseudonym, sign_with_pseudonym};
use rekindle_governance::genesis::creator_accept_signing_bytes;
use rekindle_governance::successor::link_signing_bytes;
use rekindle_governance_runtime::deps::GovernanceRuntimeDeps as _;
use rekindle_governance_runtime::transfer::{self, TransferRound};
use rekindle_governance_runtime::{successor_invites, CoCreatorAcceptance};
use rekindle_types::dto::community::CommunityCreated;
use rekindle_types::governance::{CreatorLink, OpHash, SuccessorLink};
use rekindle_types::id::PseudonymKey;

use super::co_create::PendingCreation;
use super::create_leave::NOT_CONNECTED;
use super::HostGovernance;

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

    /// Accept the standing transfer offer of `anchor` to us (intent
    /// `AcceptCommunityTransfer`, lead decision (46)). The first call
    /// reserves the successor and asks the offer's other creators, inside
    /// this community, for their links; it answers with the creators still
    /// to link (by their pseudonyms here). A later call, once every one has
    /// linked, creates the successor, writes the continuing members' sealed
    /// invites (decision (34); the co-creators follow through theirs) and
    /// publishes the acceptance citing the offer and the link ops. With no
    /// other creator the first call finishes at once. The round removes no
    /// one (decision (25)).
    ///
    /// # Errors
    /// No standing offer names us (or the set changed), a continuing
    /// member's card cannot be read, or the reservation, the request or
    /// the successor's creation failed.
    pub(crate) async fn accept_transfer(&self, anchor: &str) -> Result<CommunityCreated, String> {
        let (round, me_old) =
            transfer::round_for(self, anchor).map_err(|e| format!("transfer not accepted: {e}"))?;
        if let Some(new_anchor) = self.pending_transfer(anchor, &round) {
            return Box::pin(self.collect_or_finish(anchor, &new_anchor, &round, &me_old)).await;
        }
        if !self.transport().is_ok_and(|node| node.is_ready()) {
            return Err(NOT_CONNECTED.into());
        }
        let state =
            (self.governance_state(anchor)).ok_or("the community's governance is not loaded")?;
        let meta = (state.metadata.clone()).ok_or("the community's governance is not loaded")?;
        let chosen = successor_invites::default_continuing(&state, &me_old);
        let cards = successor_invites::continuing_cards(self, anchor, &state, &chosen)
            .await
            .map_err(|e| format!("transfer not accepted: {e}"))?;
        let others = round.others.clone();
        let pending = PendingTransfer {
            old_anchor: anchor.to_owned(),
            round: round.clone(),
            me_old: me_old.clone(),
            cards,
        };
        let description = meta.description.unwrap_or_default();
        let admission = state.effective_admission_mode();
        let created = self
            .start_round(&meta.name, &description, admission, &[], Some(pending))
            .await?;
        if others.is_empty() {
            return Box::pin(self.collect_or_finish(anchor, &created.anchor, &round, &me_old))
                .await;
        }
        if let Err(e) = transfer::request_links(self, anchor, &round, &created.anchor).await {
            self.drop_pending_transfer(&created.anchor).await;
            return Err(format!("the link request was not written: {e}"));
        }
        Ok(CommunityCreated {
            pending_co_creators: others.iter().map(PseudonymKey::to_hex).collect(),
            ..created
        })
    }

    /// The reserved successor of our pending acceptance of `round`'s
    /// offer of `old_anchor`, if any.
    fn pending_transfer(&self, old_anchor: &str, round: &TransferRound) -> Option<String> {
        (self.ctx.pending_creations.0.lock().iter())
            .find(|(_, p)| {
                p.transfer
                    .as_ref()
                    .is_some_and(|t| t.old_anchor == old_anchor && t.round.offer == round.offer)
            })
            .map(|(anchor, _)| anchor.clone())
    }

    /// Release a pending transfer's reserved successor.
    async fn drop_pending_transfer(&self, new_anchor: &str) {
        let pending = self.ctx.pending_creations.0.lock().remove(new_anchor);
        if let Some(pending) = pending {
            rekindle_governance_runtime::release_reserved(self, pending.reserved).await;
        }
    }

    /// Answer with the creators still to link, or finish the transfer once
    /// every one has.
    async fn collect_or_finish(
        &self,
        old_anchor: &str,
        new_anchor: &str,
        round: &TransferRound,
        me_old: &PseudonymKey,
    ) -> Result<CommunityCreated, String> {
        let links = if round.others.is_empty() {
            Vec::new()
        } else {
            let state = (self.governance_state(old_anchor))
                .ok_or("the community's governance is not loaded")?;
            let collected =
                transfer::collected_links(&state, old_anchor, round, me_old, new_anchor)
                    .ok_or("our link request is not in the community's op log yet")?;
            if !collected.missing.is_empty() {
                let name = state.metadata.map(|m| m.name).unwrap_or_default();
                return Ok(CommunityCreated {
                    anchor: new_anchor.to_owned(),
                    registry_key: String::new(),
                    name,
                    pending_co_creators: collected
                        .missing
                        .iter()
                        .map(PseudonymKey::to_hex)
                        .collect(),
                });
            }
            collected.links
        };
        let pending = (self.ctx.pending_creations.0.lock().remove(new_anchor))
            .ok_or("the reserved successor is gone")?;
        let mut pending = pending;
        let transfer =
            (pending.transfer.take()).ok_or("the reserved successor is not a transfer")?;
        let created = self.complete_transfer(pending, &transfer, links).await?;
        self.activate_created(&created.anchor).await;
        Ok(created)
    }

    async fn complete_transfer(
        &self,
        pending: PendingCreation,
        transfer: &PendingTransfer,
        collected: Vec<(OpHash, SuccessorLink)>,
    ) -> Result<CommunityCreated, String> {
        let PendingCreation {
            reserved,
            name,
            description,
            admission,
            ..
        } = pending;
        let new_anchor = reserved.anchor_key().to_owned();
        let old = transfer.old_anchor.as_str();
        let mut links = vec![self.own_link(old, &new_anchor, &transfer.me_old)?];
        let (mut accepts, mut link_ops) = (Vec::new(), Vec::new());
        for (op, l) in collected {
            let Ok(signature) = <[u8; 64]>::try_from(l.creator_sig.as_slice()) else {
                continue;
            };
            accepts.push(CoCreatorAcceptance {
                pseudonym: l.link.new.clone(),
                signature,
            });
            links.push(l.link);
            link_ops.push(op);
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
        let created = self.store_created(id.clone(), &name, &description).await?;
        let record = self.seal_invites(old, &id, &transfer.cards).await?;
        let published = transfer::publish_acceptance(
            self,
            old,
            &transfer.round,
            &id,
            links,
            &link_ops,
            &record,
        );
        published
            .await
            .map_err(|e| format!("successor {id} created but the acceptance not written: {e}"))?;
        Ok(created)
    }

    /// Answer the link request `request` of a transfer of `anchor` that
    /// asks us (intent `AnswerSuccessorLink`, lead decision (46)): our
    /// pseudonym in its reserved successor, linked from ours here and
    /// accepting the successor's creator role.
    ///
    /// # Errors
    /// No unanswered request with that hash asks us, or the op failed.
    pub(crate) async fn answer_successor_link(
        &self,
        anchor: &str,
        request: OpHash,
    ) -> Result<(), String> {
        let state =
            (self.governance_state(anchor)).ok_or("the community's governance is not loaded")?;
        let me_old = (self.community_membership(anchor))
            .and_then(|m| m.my_pseudonym_hex)
            .map(|hex| PseudonymKey::from_hex_lossy(&hex))
            .ok_or("no pseudonym in the community")?;
        let (_, asked) = (transfer::requests_asking(&state, &me_old).into_iter())
            .find(|(h, _)| *h == request)
            .ok_or("no unanswered link request with that hash asks us")?;
        let new_anchor = asked.new_anchor.as_str();
        let link = self.own_link(anchor, new_anchor, &me_old)?;
        let secret = (self.ctx.identity_secret()).ok_or("no identity is unlocked")?;
        let signing = derive_community_pseudonym(&secret, new_anchor);
        let creator_sig = sign_with_pseudonym(
            &signing,
            &creator_accept_signing_bytes(new_anchor, &link.new),
        );
        transfer::answer_link(self, anchor, request, link, creator_sig)
            .await
            .map_err(|e| format!("link not answered: {e}"))
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
}
