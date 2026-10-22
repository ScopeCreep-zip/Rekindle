//! A community's onboarding flow and welcome screen on both modes (plan
//! E3.1 D3).
//!
//! The flow and the screen are governance entries (`OnboardingConfig`,
//! `WelcomeScreen`) every reader validates: `MANAGE_COMMUNITY`, the mode,
//! and the bounds (`rekindle_governance::validate::bounds`), which
//! `write_entry` refuses by field before anything is written.
//!
//! Answering writes the self-assignable roles the chosen options grant
//! ourselves and sends `SubmitOnboardingAnswers` to the mesh; each member
//! holding `MANAGE_ROLES` validates the answers and grants the rest
//! (`membership_events::process_onboarding_answers`, B2e).

use std::fmt;

use rekindle_codec::community::envelope::{CommunityEnvelope, ControlPayload};
use rekindle_governance_runtime::deps::GovernanceRuntimeDeps;
use rekindle_governance_runtime::membership_events::onboarding::validate_onboarding_answers;
use rekindle_governance_runtime::membership_events::MembershipEventDeps;
use rekindle_governance_runtime::{write_entry, GovernanceRuntimeError};
use rekindle_types::dto::onboarding::{OnboardingConfigDto, WelcomeScreenDto};
use rekindle_types::governance::GovernanceEntry;
use rekindle_types::id::{PseudonymKey, RoleId};
use rekindle_types::presence::OnboardingAnswer;

use super::{onboarding_map, HostGovernance};

/// Why an onboarding command did not run.
#[derive(Debug)]
pub enum OnboardingRefusal {
    /// The request is malformed, the community has no onboarding, the
    /// answers do not fit its questions, or its rules are unacknowledged.
    Invalid(String),
    /// We hold no pseudonym in this community, or its governance has not
    /// merged.
    NotAMember,
    /// The write was refused or failed.
    Failed(GovernanceRuntimeError),
}

impl OnboardingRefusal {
    /// The IPC status the refusal answers with.
    #[must_use]
    pub fn code(&self) -> u16 {
        match self {
            Self::Invalid(_)
            | Self::Failed(
                GovernanceRuntimeError::OutOfBounds { .. } | GovernanceRuntimeError::Malformed,
            ) => 400,
            Self::NotAMember => 404,
            Self::Failed(GovernanceRuntimeError::PermissionDenied) => 403,
            Self::Failed(_) => 500,
        }
    }
}

impl fmt::Display for OnboardingRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(why) => f.write_str(why),
            Self::NotAMember => f.write_str("no pseudonym for this community"),
            Self::Failed(e) => write!(f, "onboarding write: {e}"),
        }
    }
}

impl From<GovernanceRuntimeError> for OnboardingRefusal {
    fn from(e: GovernanceRuntimeError) -> Self {
        Self::Failed(e)
    }
}

impl HostGovernance {
    /// The merged onboarding flow, or the empty default when none is set.
    pub fn onboarding_config(&self, community_id: &str) -> OnboardingConfigDto {
        self.governance_state_impl(community_id)
            .and_then(|g| g.onboarding.as_ref().map(onboarding_map::config_view))
            .unwrap_or_default()
    }

    /// The merged welcome screen, or the empty default when none is set.
    pub fn welcome_screen(&self, community_id: &str) -> WelcomeScreenDto {
        self.governance_state_impl(community_id)
            .and_then(|g| g.welcome_screen.as_ref().map(onboarding_map::welcome_view))
            .unwrap_or_default()
    }

    /// Write `config` as the community's onboarding flow.
    ///
    /// # Errors
    /// A channel id is malformed, a field is out of bounds, we lack
    /// `MANAGE_COMMUNITY`, or the write failed.
    pub async fn set_onboarding_config(
        &self,
        community_id: &str,
        config: &OnboardingConfigDto,
    ) -> Result<(), OnboardingRefusal> {
        let lamport = GovernanceRuntimeDeps::next_governance_lamport(self, community_id)
            .map_err(GovernanceRuntimeError::from)?;
        let entry =
            onboarding_map::config_entry(config, lamport).map_err(OnboardingRefusal::Invalid)?;
        write_entry(self, community_id, entry).await?;
        Ok(())
    }

    /// Write `screen` as the community's welcome screen.
    ///
    /// # Errors
    /// As [`Self::set_onboarding_config`].
    pub async fn set_welcome_screen(
        &self,
        community_id: &str,
        screen: &WelcomeScreenDto,
    ) -> Result<(), OnboardingRefusal> {
        let lamport = GovernanceRuntimeDeps::next_governance_lamport(self, community_id)
            .map_err(GovernanceRuntimeError::from)?;
        let entry =
            onboarding_map::welcome_entry(screen, lamport).map_err(OnboardingRefusal::Invalid)?;
        write_entry(self, community_id, entry).await?;
        Ok(())
    }

    /// Our pseudonym in the community, as hex and as a key.
    fn my_pseudonym(
        &self,
        community_id: &str,
    ) -> Result<(String, PseudonymKey), OnboardingRefusal> {
        let hex_str = self
            .community_membership_impl(community_id)
            .and_then(|m| m.my_pseudonym_hex)
            .ok_or(OnboardingRefusal::NotAMember)?;
        let bytes: [u8; 32] = hex::decode(&hex_str)
            .ok()
            .and_then(|b| b.try_into().ok())
            .ok_or(OnboardingRefusal::NotAMember)?;
        Ok((hex_str, PseudonymKey(bytes)))
    }

    /// Answer the community's onboarding questions: the self-assignable
    /// roles the chosen options grant are written now, in role-id order,
    /// and the answers go to the mesh for the members who grant the rest.
    ///
    /// # Errors
    /// The community has no onboarding, its rules are gated and
    /// unacknowledged, the answers do not fit its questions, or a write
    /// failed.
    pub async fn submit_onboarding_answers(
        &self,
        community_id: &str,
        answers: Vec<OnboardingAnswer>,
        acknowledged_rules: bool,
    ) -> Result<(), OnboardingRefusal> {
        let state = self
            .governance_state_impl(community_id)
            .ok_or(OnboardingRefusal::NotAMember)?;
        let (_, me) = self.my_pseudonym(community_id)?;
        let onboarding = state
            .onboarding
            .as_ref()
            .filter(|o| o.enabled)
            .ok_or_else(|| OnboardingRefusal::Invalid("this community has no onboarding".into()))?;
        if onboarding.mode == "gated" && !acknowledged_rules {
            return Err(OnboardingRefusal::Invalid(
                "this community is gated: acknowledge its rules first".into(),
            ));
        }
        let mut roles: Vec<RoleId> = validate_onboarding_answers(&onboarding.questions, &answers)
            .map_err(|r| OnboardingRefusal::Invalid(format!("onboarding answers: {r:?}")))?
            .into_iter()
            .collect();
        roles.sort_by_key(|r| r.0);
        let held = state.role_assignments.get(&me).cloned().unwrap_or_default();
        for role_id in roles {
            let self_assignable = state.roles.get(&role_id).is_some_and(|r| r.self_assignable);
            if !self_assignable || held.contains(&role_id) {
                continue;
            }
            let lamport = GovernanceRuntimeDeps::next_governance_lamport(self, community_id)
                .map_err(GovernanceRuntimeError::from)?;
            let entry = GovernanceEntry::RoleAssignment {
                target: me.clone(),
                role_id,
                lamport,
            };
            write_entry(self, community_id, entry).await?;
        }
        let envelope =
            CommunityEnvelope::Control(ControlPayload::SubmitOnboardingAnswers { answers });
        GovernanceRuntimeDeps::send_to_mesh(self, community_id, &envelope)?;
        Ok(())
    }

    /// Our onboarding of the community is done: stored, and pushed to the
    /// personal sync record (the shell's seam; E5.2 on the daemon).
    ///
    /// # Errors
    /// We are not a member, no identity is unlocked, or the row could not
    /// be written.
    pub async fn mark_onboarding_complete(
        &self,
        community_id: &str,
    ) -> Result<(), OnboardingRefusal> {
        let (me_hex, me) = self.my_pseudonym(community_id)?;
        let mut roles: Vec<u32> = self
            .governance_state_impl(community_id)
            .and_then(|g| g.role_assignments.get(&me).cloned())
            .map(|held| held.into_iter().map(RoleId::to_legacy_u32).collect())
            .unwrap_or_default();
        roles.sort_unstable();
        let failed = |e: String| OnboardingRefusal::Failed(GovernanceRuntimeError::Adapter(e));
        let owner = self
            .ctx
            .require_owner()
            .map_err(|_| failed("no identity is unlocked".into()))?;
        let db = self.ctx.db.current().map_err(|e| failed(e.to_string()))?;
        let (community, pseudonym) = (community_id.to_owned(), me_hex.clone());
        db.call(move |conn| {
            rekindle_db::repo::members::set_onboarding_complete(
                conn, &owner, &community, &pseudonym,
            )
        })
        .await
        .map_err(|e| failed(format!("onboarding completion not stored: {e}")))?;
        self.ctx
            .communities
            .note_onboarding_completion(community_id, &me_hex, &roles, true);
        MembershipEventDeps::push_onboarding_complete_to_sync(self, community_id);
        Ok(())
    }
}
