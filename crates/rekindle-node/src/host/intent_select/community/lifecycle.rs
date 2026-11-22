//! The selection of a community lifecycle and invite request (plans D3.3, E3.1).

use super::{
    community, leave_escalations, members, pseudonym, unavailable, DaemonContext, Fact, IntentKind,
    IpcResponse, LifecycleRequest, Notifies, Reversibility, Selection, Subject,
};

/// `selection` for the community lifecycle and invite requests.
pub(super) fn selection(
    ctx: &DaemonContext,
    request: &LifecycleRequest,
    s: &mut Selection,
) -> Result<(), IpcResponse> {
    match request {
        LifecycleRequest::CommunityCreate {
            name, co_creators, ..
        } => {
            s.subject = Subject::Community {
                id: String::new(),
                name: name.clone(),
            };
            s.reversibility = Reversibility::ReversibleByInverse(IntentKind::LeaveCommunity);
            // Named co-creators are told and hold creator rights from the
            // genesis (plan E3.2 S7s.3); the review counts them.
            if !co_creators.is_empty() {
                s.notifies =
                    Notifies::Members(u32::try_from(co_creators.len()).unwrap_or(u32::MAX));
            }
        }
        LifecycleRequest::CreatorRemove { anchor, .. }
        | LifecycleRequest::CommunityCreateCancel { anchor } => {
            // A pending creation has no community to name yet: its anchor.
            s.subject = Subject::Named {
                name: anchor.clone(),
            };
            s.reversibility = Reversibility::Never;
            s.notifies = Notifies::Subject;
        }
        LifecycleRequest::CreatorInviteAnswer { anchor, .. } => {
            s.subject = Subject::Named {
                name: anchor.clone(),
            };
            s.facts.push(Fact::PresenceVisible);
            s.reversibility = Reversibility::NeedsOthers;
            s.notifies = Notifies::Subject;
        }
        LifecycleRequest::CommunityJoin { invite } => {
            // The invite names a record key, not a community we know: the
            // review shows its fingerprint (`deep_links` in the shell).
            let link = rekindle_types::invite::InviteLink::parse(invite)
                .map_err(|e| IpcResponse::error(400, format!("community invite: {e}")))?;
            let fingerprint =
                crate::host::adapters::governance::invite_fingerprint(link.anchor.as_str())
                    .ok_or_else(|| {
                        IpcResponse::error(400, "community invite key is not a record key")
                    })?;
            s.subject = Subject::Named { name: fingerprint };
            s.facts.push(Fact::PresenceVisible);
            s.reversibility = Reversibility::ReversibleByInverse(IntentKind::LeaveCommunity);
            s.notifies = Notifies::Subject;
        }
        LifecycleRequest::CommunityLeave { anchor } => {
            let (id, name) = community(ctx, anchor)?;
            s.escalations = leave_escalations(ctx, &id)?;
            s.facts.push(Fact::RejoinNeedsInvite);
            s.reversibility = Reversibility::NeedsOthers;
            s.notifies = members(ctx, &id);
            s.subject = Subject::Community { id, name };
        }
        LifecycleRequest::CommunityForget { anchor } => {
            // Only our own copy goes: no one is told (plan E3.2 S9 R3-5).
            let (id, name) = community(ctx, anchor)?;
            s.facts.push(Fact::RejoinNeedsInvite);
            s.reversibility = Reversibility::Never;
            s.subject = Subject::Community { id, name };
        }
        LifecycleRequest::CommunityUpdateInfo { anchor, .. } => {
            let (id, name) = community(ctx, anchor)?;
            s.reversibility = Reversibility::Undoable;
            s.notifies = members(ctx, &id);
            s.subject = Subject::Community { id, name };
        }
        LifecycleRequest::CommunityApprove {
            anchor,
            member_pseudonym,
        }
        | LifecycleRequest::CommunityReject {
            anchor,
            member_pseudonym,
            ..
        }
        | LifecycleRequest::CommunityOfferTransfer {
            anchor,
            transfer_to: member_pseudonym,
        } => {
            let (id, _) = community(ctx, anchor)?;
            s.subject = Subject::Named {
                name: member_pseudonym.clone(),
            };
            s.notifies = members(ctx, &id);
        }
        LifecycleRequest::CommunityWithdrawTransfer { anchor } => {
            let (id, name) = community(ctx, anchor)?;
            s.notifies = members(ctx, &id);
            s.subject = Subject::Community { id, name };
        }
        LifecycleRequest::CommunityAcceptTransfer { anchor } => {
            // ADR 0019 TrustChanging; decision (25): the round removes no
            // one, so the review names the creators who have not linked
            // yet (lead decision (46): read from the op log).
            let (id, name) = community(ctx, anchor)?;
            let state = (ctx.communities.governance_state(&id))
                .ok_or_else(|| unavailable("the community's governance", "not loaded"))?;
            let me = pseudonym(&ctx.my_pseudonym(&id))
                .ok_or_else(|| unavailable("our pseudonym", "not in the community"))?;
            let missing = rekindle_governance_runtime::transfer::links_missing(&state, &id, &me)
                .map_err(|e| IpcResponse::error(409, e.to_string()))?;
            s.facts.push(Fact::TransferLinksMissing {
                creators: missing
                    .iter()
                    .map(rekindle_types::id::PseudonymKey::to_hex)
                    .collect(),
            });
            s.notifies = members(ctx, &id);
            s.subject = Subject::Community { id, name };
        }
        LifecycleRequest::CommunityAnswerSuccessorLink { anchor, .. } => {
            // ADR 0019 TrustChanging: our pseudonym here is linked to our
            // pseudonym in the successor, visibly in the op log.
            let (id, name) = community(ctx, anchor)?;
            s.facts.push(Fact::PresenceVisible);
            s.reversibility = Reversibility::Never;
            s.notifies = members(ctx, &id);
            s.subject = Subject::Community { id, name };
        }
        LifecycleRequest::CommunityPublishSuccessor { anchor, .. } => {
            // ADR 0019 Destructive: the new creator set, who leaves it, and
            // that no history is copied (plan E3.2 S7s.4).
            let (id, name) = community(ctx, anchor)?;
            let state = (ctx.communities.governance_state(&id))
                .ok_or_else(|| unavailable("the community's governance", "not loaded"))?;
            let me = pseudonym(&ctx.my_pseudonym(&id))
                .ok_or_else(|| unavailable("our pseudonym", "not in the community"))?;
            if !rekindle_governance::permissions::holds_creator_seat(&me, &state) {
                return Err(IpcResponse::error(
                    403,
                    "only a creator of this community can publish its successor",
                ));
            }
            s.facts
                .extend(rekindle_governance::intent_facts::successor_publish_facts(
                    &state, &me,
                ));
            s.reversibility = Reversibility::Never;
            s.notifies = members(ctx, &id);
            s.subject = Subject::Community { id, name };
        }
        LifecycleRequest::CommunityFollowSuccessor {
            anchor, new_anchor, ..
        } => {
            // TrustChanging, as a join: the signer and that both links
            // held when offered; the handler re-checks them.
            let (id, _) = community(ctx, anchor)?;
            let offers = crate::daemon::dispatch::adapter(ctx).successor_offers(&id);
            let offer = (offers.into_iter())
                .find(|o| o.new_anchor == *new_anchor)
                .ok_or_else(|| {
                    IpcResponse::error(409, "this successor has not been offered here")
                })?;
            let fingerprint = crate::host::adapters::governance::invite_fingerprint(new_anchor)
                .ok_or_else(|| IpcResponse::error(400, "the successor is not a record key"))?;
            s.subject = Subject::Named { name: fingerprint };
            s.facts.push(Fact::SuccessorLinked {
                signer: offer.signer,
            });
            s.facts.push(Fact::PresenceVisible);
            s.reversibility = Reversibility::ReversibleByInverse(IntentKind::LeaveCommunity);
            s.notifies = Notifies::Subject;
        }
        LifecycleRequest::InviteCreate {
            community: target, ..
        }
        | LifecycleRequest::InviteRevoke {
            community: target, ..
        } => {
            let (id, name) = community(ctx, target)?;
            if matches!(request, LifecycleRequest::InviteCreate { .. }) {
                s.reversibility =
                    Reversibility::ReversibleByInverse(IntentKind::RevokeCommunityInvite);
            }

            s.subject = Subject::Community { id, name };
        }
        _ => {}
    }
    Ok(())
}
