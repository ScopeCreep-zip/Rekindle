//! The selection of a role and moderation request (plans D3.3, E3.1).

use super::{
    community, member_selection, members, role, DaemonContext, Fact, IntentKind, IpcResponse,
    MemberRequest, Reversibility, Selection, Subject,
};

/// `selection` for the role and moderation requests.
pub(super) fn selection(
    ctx: &DaemonContext,
    request: &MemberRequest,
    s: &mut Selection,
) -> Result<(), IpcResponse> {
    match request {
        MemberRequest::RoleCreate {
            community: target,
            name,
            ..
        } => {
            let (id, _) = community(ctx, target)?;
            s.subject = Subject::Role {
                id: String::new(),
                name: name.clone(),
            };
            s.reversibility = Reversibility::ReversibleByInverse(IntentKind::DeleteRole);
            s.notifies = members(ctx, &id);
        }
        MemberRequest::RoleUpdate {
            community: target,
            role_id,
            ..
        }
        | MemberRequest::RoleDelete {
            community: target,
            role_id,
        } => {
            let (id, _) = community(ctx, target)?;
            let (name, holders) = role(ctx, &id, *role_id)?;
            if matches!(request, MemberRequest::RoleDelete { .. }) {
                s.facts.push(Fact::MembersAffected { count: holders });
            } else {
                s.reversibility = Reversibility::Undoable;
            }
            s.subject = Subject::Role {
                id: role_id.to_string(),
                name,
            };
            s.notifies = members(ctx, &id);
        }
        MemberRequest::RoleSelfAssign {
            community: target,
            role_id,
        }
        | MemberRequest::RoleSelfUnassign {
            community: target,
            role_id,
        } => {
            let (id, _) = community(ctx, target)?;
            let (name, _) = role(ctx, &id, *role_id)?;
            s.subject = Subject::Role {
                id: role_id.to_string(),
                name,
            };
            s.reversibility = if matches!(request, MemberRequest::RoleSelfAssign { .. }) {
                Reversibility::ReversibleByInverse(IntentKind::SelfUnassignRole)
            } else {
                Reversibility::ReversibleByInverse(IntentKind::SelfAssignRole)
            };
            s.notifies = members(ctx, &id);
        }
        MemberRequest::RoleAssign { .. }
        | MemberRequest::RoleUnassign { .. }
        | MemberRequest::Kick { .. }
        | MemberRequest::Ban { .. }
        | MemberRequest::Unban { .. }
        | MemberRequest::Timeout { .. }
        | MemberRequest::TimeoutRemove { .. }
        | MemberRequest::MemberBlock { .. }
        | MemberRequest::MemberUnblock { .. } => member_selection(ctx, request, s)?,
        MemberRequest::AutomodRuleSet {
            community: target, ..
        }
        | MemberRequest::AutomodRuleDelete {
            community: target, ..
        } => {
            let (id, name) = community(ctx, target)?;

            s.subject = Subject::Community { id, name };
        }
        _ => {}
    }
    Ok(())
}
