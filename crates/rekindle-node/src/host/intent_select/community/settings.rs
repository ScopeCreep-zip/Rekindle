//! The selection of a community settings request (plans D3.3, E3.1).

use super::{
    community, members, DaemonContext, IpcResponse, Reversibility, Selection, SettingsRequest,
    Subject,
};

/// Fill `s` for a community settings request: its profile, pictures,
/// policy, onboarding flow, welcome screen and our onboarding.
///
/// # Errors
/// The community is not joined.
pub(super) fn selection(
    ctx: &DaemonContext,
    request: &SettingsRequest,
    s: &mut Selection,
) -> Result<(), IpcResponse> {
    match request {
        SettingsRequest::CommunityUpdateProfile {
            community: target, ..
        }
        | SettingsRequest::CommunitySetAvatar {
            community: target, ..
        }
        | SettingsRequest::CommunitySetBanner {
            community: target, ..
        } => {
            let (id, name) = community(ctx, target)?;
            s.reversibility = Reversibility::Undoable;
            s.subject = Subject::Community { id, name };
        }
        SettingsRequest::CommunityPolicySet {
            community: target, ..
        } => {
            let (id, name) = community(ctx, target)?;
            s.reversibility = Reversibility::Undoable;
            s.notifies = members(ctx, &id);
            s.subject = Subject::Community { id, name };
        }
        SettingsRequest::OnboardingConfigSet {
            community: target, ..
        }
        | SettingsRequest::WelcomeScreenSet {
            community: target, ..
        } => {
            let (id, name) = community(ctx, target)?;
            s.notifies = members(ctx, &id);
            s.subject = Subject::Community { id, name };
        }
        SettingsRequest::OnboardingSubmit {
            community: target, ..
        }
        | SettingsRequest::OnboardingComplete { community: target } => {
            let (id, name) = community(ctx, target)?;
            s.subject = Subject::Community { id, name };
        }
        SettingsRequest::OnboardingConfigGet { .. }
        | SettingsRequest::WelcomeScreenGet { .. }
        | SettingsRequest::CommunityAvatarGet { .. }
        | SettingsRequest::CommunityPolicyGet { .. } => {}
    }
    Ok(())
}
