//! The selection of a community lifecycle and invite request (plans D3.3, E3.1).

use super::{
    community, leave_escalations, members, DaemonContext, Fact, IntentKind, IpcResponse,
    LifecycleRequest, Notifies, Reversibility, Selection, Subject,
};

/// `selection` for the community lifecycle and invite requests.
pub(super) fn selection(
    ctx: &DaemonContext,
    request: &LifecycleRequest,
    s: &mut Selection,
) -> Result<(), IpcResponse> {
    match request {
        LifecycleRequest::CommunityCreate { name, .. } => {
            s.subject = Subject::Community {
                id: String::new(),
                name: name.clone(),
            };
            s.reversibility = Reversibility::ReversibleByInverse(IntentKind::LeaveCommunity);
        }
        LifecycleRequest::CommunityJoin { invite } => {
            // The invite names a record key, not a community we know: the
            // review shows its fingerprint (`deep_links` in the shell).
            let link = rekindle_types::invite::InviteLink::parse(invite)
                .map_err(|e| IpcResponse::error(400, format!("community invite: {e}")))?;
            let fingerprint =
                rekindle_crypto::identity::record_key_fingerprint(link.governance_key.as_str())
                    .ok_or_else(|| {
                        IpcResponse::error(400, "community invite key is not a record key")
                    })?;
            s.subject = Subject::Named { name: fingerprint };
            s.facts.push(Fact::PresenceVisible);
            s.reversibility = Reversibility::ReversibleByInverse(IntentKind::LeaveCommunity);
            s.notifies = Notifies::Subject;
        }
        LifecycleRequest::CommunityLeave { governance_key } => {
            let (id, name) = community(ctx, governance_key)?;
            s.escalations = leave_escalations(ctx, &id)?;
            s.facts.push(Fact::RejoinNeedsInvite);
            s.reversibility = Reversibility::NeedsOthers;
            s.notifies = members(ctx, &id);
            s.subject = Subject::Community { id, name };
        }
        LifecycleRequest::CommunityUpdateInfo { governance_key, .. } => {
            let (id, name) = community(ctx, governance_key)?;
            s.reversibility = Reversibility::Undoable;
            s.notifies = members(ctx, &id);
            s.subject = Subject::Community { id, name };
        }
        LifecycleRequest::CommunityApprove {
            governance_key,
            member_pseudonym,
        }
        | LifecycleRequest::CommunityReject {
            governance_key,
            member_pseudonym,
            ..
        }
        | LifecycleRequest::CommunityTransferOwnership {
            governance_key,
            new_owner_pseudonym: member_pseudonym,
        } => {
            let (id, _) = community(ctx, governance_key)?;
            s.subject = Subject::Named {
                name: member_pseudonym.clone(),
            };
            s.notifies = members(ctx, &id);
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
