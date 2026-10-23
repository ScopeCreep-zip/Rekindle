//! The moderation commands on both modes (plan E3.1 F2): ban, unban, the
//! ban list, timeout, lifting a timeout, and removing a member.
//!
//! Bans and timeouts are governance entries every reader validates after
//! the merge (`rekindle_governance::validate::validate_write`: the
//! permission bit, the creator's immunity and the strict rank rule). Each
//! write is checked here with that same function before it is written, so
//! a refusal answers at once instead of being dropped silently by every
//! peer. The entry is written by `rekindle_governance_runtime::moderation`
//! and projected onto the store (`projection::prune_banned`).
//!
//! A removal is the durable kick `MemberRemoved` (plan E3.2), which every
//! reader's merge applies by dropping the target from `members`; then the
//! `Kick` notice to the mesh, the member forgotten in our store and roster,
//! and the departure rotation (ADR 0018; the ballot itself is E3.5 behind
//! `spawn_text_mek_rotation_for_ban`, which the runtime's removal starts).

use std::fmt;

use rekindle_codec::community::envelope::{CommunityEnvelope, ControlPayload};
use rekindle_governance::state::GovernanceState;
use rekindle_governance::state::ModReportState;
use rekindle_governance::validate::validate_write;
use rekindle_governance_runtime::deps::GovernanceRuntimeDeps;
use rekindle_governance_runtime::moderation;
use rekindle_governance_runtime::GovernanceRuntimeError;
use rekindle_types::dto::community::{CommunityBanView, ModReportView};
use rekindle_types::governance::{GovernanceEntry, ModReportTarget};
use rekindle_types::id::PseudonymKey;
use rekindle_types::permissions::KICK_MEMBERS;

use super::HostGovernance;

/// Why a moderation command did not run.
#[derive(Debug)]
pub enum ModerationRefusal {
    /// Readers would drop the entry: we lack the permission, the target is
    /// the creator, or the target does not rank strictly below us.
    NotPermitted(&'static str),
    /// We hold no pseudonym in this community, or its governance has not
    /// merged.
    NotAMember,
    /// The target is not a pseudonym.
    BadTarget,
    /// A channel or message id is malformed.
    BadMessage,
    /// The write failed.
    Failed(GovernanceRuntimeError),
}

impl ModerationRefusal {
    /// The IPC status the refusal answers with.
    #[must_use]
    pub fn code(&self) -> u16 {
        match self {
            Self::NotPermitted(_) => 403,
            Self::NotAMember => 404,
            Self::BadTarget | Self::BadMessage => 400,
            Self::Failed(_) => 500,
        }
    }
}

impl fmt::Display for ModerationRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotPermitted(what) => write!(f, "not permitted: {what}"),
            Self::NotAMember => f.write_str("no pseudonym for this community"),
            Self::BadTarget => f.write_str("the target is not a pseudonym"),
            Self::BadMessage => f.write_str("not a channel or message id"),
            Self::Failed(e) => write!(f, "moderation write: {e}"),
        }
    }
}

impl From<GovernanceRuntimeError> for ModerationRefusal {
    fn from(e: GovernanceRuntimeError) -> Self {
        match e {
            GovernanceRuntimeError::PermissionDenied => Self::NotPermitted("missing permission"),
            other => Self::Failed(other),
        }
    }
}

/// A moderation action, as the reader rule sees it.
#[derive(Debug, Clone, Copy)]
pub enum ModerationAction {
    Ban,
    Unban,
    Timeout(u64),
    RemoveTimeout,
    Remove,
}

impl ModerationAction {
    fn what(self) -> &'static str {
        match self {
            Self::Ban => "ban members",
            Self::Unban => "lift bans",
            Self::Timeout(_) => "time out members",
            Self::RemoveTimeout => "lift timeouts",
            Self::Remove => "remove members",
        }
    }
}

/// Whether `me` may take `action` against `target`: for an entry, the
/// reader rule over a probe of it; for a removal, `KICK_MEMBERS` with the
/// creator's immunity and the strict rank rule the ban carries.
///
/// # Errors
/// Readers would refuse it.
pub fn permitted(
    state: &GovernanceState,
    me: &PseudonymKey,
    target: &PseudonymKey,
    action: ModerationAction,
    now: u64,
) -> Result<(), ModerationRefusal> {
    let target = target.clone();
    let ok = match action {
        ModerationAction::Ban => validate_write(
            me,
            &GovernanceEntry::BanEntry {
                target,
                reason: None,
            },
            state,
        ),
        ModerationAction::Unban => {
            validate_write(me, &GovernanceEntry::UnbanEntry { target }, state)
        }
        ModerationAction::Timeout(duration_seconds) => validate_write(
            me,
            &GovernanceEntry::TimeoutEntry {
                target,
                duration_seconds,
                reason: None,
                started_at: now,
            },
            state,
        ),
        ModerationAction::RemoveTimeout => {
            validate_write(me, &GovernanceEntry::RemoveTimeoutEntry { target }, state)
        }
        ModerationAction::Remove => {
            let perms = rekindle_governance::permissions::compute_permissions(me, None, state, now);
            state.creator.as_ref() == Some(me)
                || (perms & KICK_MEMBERS != 0
                    && state.creator.as_ref() != Some(&target)
                    && state.member_max_position(&target) < state.member_max_position(me))
        }
    };
    if ok {
        Ok(())
    } else {
        Err(ModerationRefusal::NotPermitted(action.what()))
    }
}

/// A 64-hex pseudonym.
pub(super) fn pseudonym(hex_str: &str) -> Option<PseudonymKey> {
    let bytes: [u8; 32] = hex::decode(hex_str).ok()?.try_into().ok()?;
    Some(PseudonymKey(bytes))
}

/// A short form of a pseudonym for a member we hold no name for.
fn short(pseudonym: &str) -> String {
    format!("{}…", rekindle_utils::text::prefix(pseudonym, 12))
}

impl HostGovernance {
    /// Our pseudonym, the merged state and the target, once `action` is
    /// one readers accept.
    fn moderate(
        &self,
        community_id: &str,
        target_hex: &str,
        action: ModerationAction,
    ) -> Result<(), ModerationRefusal> {
        let target = pseudonym(target_hex).ok_or(ModerationRefusal::BadTarget)?;
        let state = self
            .governance_state_impl(community_id)
            .ok_or(ModerationRefusal::NotAMember)?;
        let me = self
            .community_membership_impl(community_id)
            .and_then(|m| m.my_pseudonym_hex)
            .and_then(|hex| pseudonym(&hex))
            .ok_or(ModerationRefusal::NotAMember)?;
        permitted(&state, &me, &target, action, self.ctx.clock.now_secs())
    }

    /// Ban `target`; the departure rotation follows (ADR 0018).
    ///
    /// # Errors
    /// Readers would refuse it, or the write failed.
    pub async fn ban(
        &self,
        community_id: &str,
        target: &str,
        reason: Option<&str>,
    ) -> Result<(), ModerationRefusal> {
        self.moderate(community_id, target, ModerationAction::Ban)?;
        moderation::ban_member(self, community_id, target, reason).await?;
        Ok(())
    }

    /// Lift a ban.
    ///
    /// # Errors
    /// Readers would refuse it, or the write failed.
    pub async fn unban(&self, community_id: &str, target: &str) -> Result<(), ModerationRefusal> {
        self.moderate(community_id, target, ModerationAction::Unban)?;
        moderation::unban_member(self, community_id, target).await?;
        Ok(())
    }

    /// Time `target` out for `duration_seconds` from now (the clock port).
    ///
    /// # Errors
    /// Readers would refuse it, or the write failed.
    pub async fn timeout(
        &self,
        community_id: &str,
        target: &str,
        duration_seconds: u64,
        reason: Option<&str>,
    ) -> Result<(), ModerationRefusal> {
        self.moderate(
            community_id,
            target,
            ModerationAction::Timeout(duration_seconds),
        )?;
        moderation::timeout_member(self, community_id, target, duration_seconds, reason).await?;
        Ok(())
    }

    /// Lift a timeout early.
    ///
    /// # Errors
    /// Readers would refuse it, or the write failed.
    pub async fn remove_timeout(
        &self,
        community_id: &str,
        target: &str,
    ) -> Result<(), ModerationRefusal> {
        self.moderate(community_id, target, ModerationAction::RemoveTimeout)?;
        moderation::remove_timeout(self, community_id, target).await?;
        Ok(())
    }

    /// Remove `target` without barring return: the `MemberRemoved` op and
    /// the departure rotation, the `Kick` notice to the mesh, and the
    /// member forgotten here.
    ///
    /// # Errors
    /// Not permitted, the op could not be written, or the notice could not
    /// be queued.
    pub async fn remove_member(
        &self,
        community_id: &str,
        target: &str,
    ) -> Result<(), ModerationRefusal> {
        self.moderate(community_id, target, ModerationAction::Remove)?;
        moderation::remove_member(self, community_id, target).await?;
        let notice = CommunityEnvelope::Control(ControlPayload::Kick {
            target_pseudonym: target.to_owned(),
        });
        self.send_to_mesh(community_id, &notice)?;
        self.forget_member(community_id, target);
        tracing::info!(community = %community_id, member = %target, "removed community member");
        Ok(())
    }

    /// The community's bans, by pseudonym, named from the roster.
    ///
    /// # Errors
    /// No identity is unlocked or the roster could not be read.
    pub async fn ban_list(&self, community_id: &str) -> Result<Vec<CommunityBanView>, String> {
        let mut bans: Vec<String> = self
            .governance_state_impl(community_id)
            .map(|g| g.bans.iter().map(|p| hex::encode(p.0)).collect())
            .unwrap_or_default();
        bans.sort();
        if bans.is_empty() {
            return Ok(Vec::new());
        }
        let owner = self
            .ctx
            .require_owner()
            .map_err(|_| "no identity is unlocked")?;
        let db = self.ctx.db.current().map_err(|e| e.to_string())?;
        let cid = community_id.to_owned();
        let rows = db
            .call(move |conn| rekindle_db::repo::members::roster(conn, &owner, &cid))
            .await
            .map_err(|e| format!("ban list: {e}"))?;
        Ok(bans
            .into_iter()
            .map(|pseudonym_key| {
                let display_name = rows
                    .iter()
                    .find(|r| r.pseudonym_key == pseudonym_key)
                    .and_then(|r| r.display_name.clone())
                    .filter(|n| !n.is_empty())
                    .unwrap_or_else(|| short(&pseudonym_key));
                CommunityBanView {
                    pseudonym_key,
                    display_name,
                }
            })
            .collect())
    }

    /// The community's moderator queue (plan E3.1, D26), in report-id
    /// order.
    ///
    /// # Errors
    /// We lack `MANAGE_MESSAGES`.
    pub fn mod_reports(
        &self,
        community_id: &str,
    ) -> Result<Vec<ModReportView>, GovernanceRuntimeError> {
        self.require_permission(community_id, rekindle_types::permissions::MANAGE_MESSAGES)?;
        let Some(gov) = self.governance_state_impl(community_id) else {
            return Ok(Vec::new());
        };
        Ok(gov.mod_reports.iter().map(mod_report_view).collect())
    }
}

fn mod_report_view((id, r): (&[u8; 16], &ModReportState)) -> ModReportView {
    let (channel_id, message_id, member) = match &r.target {
        ModReportTarget::Message {
            channel_id,
            message_id,
        } => (
            Some(hex::encode(channel_id.0)),
            Some(message_id.clone()),
            None,
        ),
        ModReportTarget::Member { pseudonym } => (None, None, Some(hex::encode(pseudonym.0))),
    };
    ModReportView {
        report_id: hex::encode(id),
        reporter: hex::encode(r.reporter.0),
        channel_id,
        message_id,
        member,
        reason: r.reason.clone(),
        lamport: r.stamp.lamport,
    }
}

#[cfg(test)]
#[path = "moderation_tests.rs"]
mod tests;
