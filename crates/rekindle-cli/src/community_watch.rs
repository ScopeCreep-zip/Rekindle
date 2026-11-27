//! `community watch`: stream a community's membership and governance
//! events until Ctrl-C (live A/B F7-5).
//!
//! Every `Membership` and `Governance` event the Host emits is shown, so
//! a removed member sees the kick or ban arrive with the remover and the
//! reason the op gave, and a script reading `--script` JSONL gets the
//! same `SubscriptionEvent` JSON the desktop and TUI consume. The matches
//! are exhaustive: a new variant does not compile until it has a line.

use rekindle_client::fmt::abbreviate_key;
use rekindle_client::DaemonClient;
use rekindle_ipc::protocol::{CommunityRequest, IpcRequest, LifecycleRequest};
use rekindle_types::subscription_events::{
    EventCategory, GovernanceEvent, MembershipEvent, OpPublicationState, OpQuarantineReason,
    SubscriptionEvent, SubscriptionFilter,
};
use rekindle_utils::text::sanitize_for_display;

use crate::helpers;
use crate::output::OutputMode;

/// `community watch [-c X]`: one community (name or key), or all of them.
pub async fn watch(
    client: &DaemonClient,
    community: Option<&str>,
    mode: OutputMode,
) -> anyhow::Result<()> {
    // Events name a community by anchor key; the user may give a name.
    let community_scope = match community {
        Some(community) => Some(anchor(client, community).await?),
        None => None,
    };
    let filter = SubscriptionFilter {
        categories: Some(vec![EventCategory::Membership, EventCategory::Governance]),
        community_scope,
    };
    crate::watch::stream(client, vec![filter], mode, render).await
}

async fn anchor(client: &DaemonClient, community: &str) -> anyhow::Result<String> {
    let detail: rekindle_types::display::CommunityDetail = serde_json::from_value(
        helpers::send(
            client,
            IpcRequest::Community(CommunityRequest::Lifecycle(
                LifecycleRequest::CommunityInfo {
                    anchor: community.to_owned(),
                },
            )),
        )
        .await?,
    )?;
    Ok(detail.anchor)
}

/// The text line for a community event; `None` for any other event.
pub fn render(event: &SubscriptionEvent) -> Option<String> {
    let (community, text) = match event {
        SubscriptionEvent::Membership(e) => membership(e),
        SubscriptionEvent::Governance(e) => (e.community(), governance(e)),
        _ => return None,
    };
    Some(format!("{}: {text}", abbreviate_key(community)))
}

/// "kicked by Ann: spam", from the removal's optional remover and reason.
fn removal(what: &str, by: Option<&str>, by_name: Option<&str>, reason: Option<&str>) -> String {
    let by = by_name
        .map(sanitize_for_display)
        .or_else(|| by.map(abbreviate_key));
    let mut text = match by {
        Some(by) => format!("{what} by {by}"),
        None => what.to_owned(),
    };
    if let Some(reason) = reason.filter(|r| !r.is_empty()) {
        text = format!("{text}: {}", sanitize_for_display(reason));
    }
    text
}

/// "Ann (abcd1234…ef01)", or the short pseudonym alone while the member's
/// card is not read (plan E3.2 F9-2): a name is always shown beside the
/// key, so two members with one name stay apart.
fn member(pseudonym: &str, name: Option<&str>) -> String {
    match name {
        Some(name) => format!(
            "{} ({})",
            sanitize_for_display(name),
            abbreviate_key(pseudonym)
        ),
        None => abbreviate_key(pseudonym),
    }
}

fn roles(role_ids: &[String]) -> String {
    format!("[{}]", role_ids.join(", "))
}

fn membership(event: &MembershipEvent) -> (&str, String) {
    match event {
        MembershipEvent::JoinRequested {
            community,
            pseudonym,
            display_name,
            has_invite,
        } => (
            community,
            format!(
                "{} ({}) asked to join{}",
                sanitize_for_display(display_name),
                abbreviate_key(pseudonym),
                if *has_invite { " with an invite" } else { "" }
            ),
        ),
        MembershipEvent::MembersRefreshed { community } => {
            (community, "member list refreshed".into())
        }
        MembershipEvent::MemberDiscovered {
            community,
            pseudonym,
            display_name,
            ..
        } => (
            community,
            format!(
                "{} ({}) is a member",
                sanitize_for_display(display_name),
                abbreviate_key(pseudonym)
            ),
        ),
        MembershipEvent::JoinProgress {
            community,
            stage,
            status,
        } => (community, format!("join: {stage} {status}")),
        MembershipEvent::Joined {
            community,
            pseudonym,
            display_name,
            ..
        } => (
            community,
            format!("{} joined", member(pseudonym, display_name.as_deref())),
        ),
        MembershipEvent::SelfJoined { community, name } => (
            community,
            format!("we joined {}", sanitize_for_display(name)),
        ),
        MembershipEvent::JoinWithdrawn {
            community,
            reason_text,
            ..
        } => (
            community,
            format!(
                "our join was withdrawn: {}",
                sanitize_for_display(reason_text)
            ),
        ),
        MembershipEvent::MemberCardChanged {
            community,
            pseudonym,
            display_name,
            ..
        } => (
            community,
            format!(
                "{} is named on its card",
                member(pseudonym, Some(display_name))
            ),
        ),
        MembershipEvent::Left {
            community,
            pseudonym,
        } => (community, format!("{} left", abbreviate_key(pseudonym))),
        MembershipEvent::Forgotten { community } => {
            (community, "we no longer belong to it".to_owned())
        }
        MembershipEvent::Removed {
            community,
            pseudonym,
        } => (
            community,
            format!("{} was removed", abbreviate_key(pseudonym)),
        ),
        MembershipEvent::Kicked {
            community,
            target_pseudonym,
            removed_by,
            removed_by_name,
            reason,
        }
        | MembershipEvent::Banned {
            community,
            target_pseudonym,
            removed_by,
            removed_by_name,
            reason,
        } => {
            let what = if matches!(event, MembershipEvent::Banned { .. }) {
                "banned"
            } else {
                "kicked"
            };
            (
                community,
                format!(
                    "{} was {}",
                    abbreviate_key(target_pseudonym),
                    removal(
                        what,
                        removed_by.as_deref(),
                        removed_by_name.as_deref(),
                        reason.as_deref()
                    )
                ),
            )
        }
        MembershipEvent::Unbanned {
            community,
            target_pseudonym,
        } => (
            community,
            format!("{} was unbanned", abbreviate_key(target_pseudonym)),
        ),
        MembershipEvent::TimedOut {
            community,
            target_pseudonym,
            duration_seconds,
            reason,
        } => (
            community,
            format!(
                "{} was {}",
                abbreviate_key(target_pseudonym),
                removal(
                    &format!("timed out for {duration_seconds}s"),
                    None,
                    None,
                    reason.as_deref()
                )
            ),
        ),
        MembershipEvent::TimeoutRemoved {
            community,
            target_pseudonym,
        } => (
            community,
            format!("{}'s timeout was lifted", abbreviate_key(target_pseudonym)),
        ),
        MembershipEvent::TimeoutStatusChanged {
            community,
            pseudonym,
            timeout_until,
        } => (
            community,
            match timeout_until {
                Some(until) => format!(
                    "{} is timed out until {}",
                    abbreviate_key(pseudonym),
                    rekindle_client::fmt::format_timestamp(*until)
                ),
                None => format!("{} is no longer timed out", abbreviate_key(pseudonym)),
            },
        ),
        MembershipEvent::RolesChanged {
            community,
            pseudonym,
            role_ids,
        } => (
            community,
            format!(
                "{} now has roles {}",
                abbreviate_key(pseudonym),
                roles(role_ids)
            ),
        ),
        MembershipEvent::OnboardingCompleted {
            community,
            pseudonym,
            role_ids,
        } => (
            community,
            format!(
                "{} finished onboarding with roles {}",
                abbreviate_key(pseudonym),
                roles(role_ids)
            ),
        ),
        MembershipEvent::OnboardingAnswersSubmitted {
            community,
            sender_pseudonym,
            answer_count,
        } => (
            community,
            format!(
                "{} submitted {answer_count} onboarding answers",
                abbreviate_key(sender_pseudonym)
            ),
        ),
    }
}

fn governance(event: &GovernanceEvent) -> String {
    match event {
        GovernanceEvent::MetadataChanged { name, .. } => match name {
            Some(name) => format!("details changed (name: {})", sanitize_for_display(name)),
            None => "details changed".into(),
        },
        GovernanceEvent::ChannelsChanged { channels, .. } => {
            format!("channels changed ({} channels)", channels.len())
        }
        GovernanceEvent::CatchUp {
            catching_up,
            pending,
            ..
        } => {
            if *catching_up {
                format!("catching up ({} governance writes held)", pending.len())
            } else {
                "caught up".into()
            }
        }
        GovernanceEvent::OpPublication { op, state, .. } => {
            let op = abbreviate_key(op);
            match state {
                OpPublicationState::Pending => format!("op {op} pending"),
                OpPublicationState::Landed => format!("op {op} landed"),
                OpPublicationState::Failed(why) => {
                    format!("op {op} failed: {}", sanitize_for_display(why))
                }
            }
        }
        GovernanceEvent::KeyStatus { status, .. } => match status.state {
            rekindle_types::dto::community::KeyState::Current => format!(
                "community key current (generation {})",
                status.have.map_or_else(|| "none".into(), |g| g.to_string())
            ),
            rekindle_types::dto::community::KeyState::Pending => format!(
                "waiting for the community key: hold {}, need {}",
                status.have.map_or_else(|| "none".into(), |g| g.to_string()),
                status.need.map_or_else(|| "none".into(), |g| g.to_string())
            ),
        },
        GovernanceEvent::RolesChanged { roles, .. } => {
            format!("roles changed ({} roles)", roles.len())
        }
        GovernanceEvent::SegmentsChanged { segments, .. } => {
            format!("segments changed ({} segments)", segments.len())
        }
        GovernanceEvent::BansChanged { .. } => "ban list changed".into(),
        GovernanceEvent::InviteCreated {
            invite_id,
            created_by,
            max_uses,
            uses,
            ..
        } => format!(
            "invite {} created by {} ({uses}/{max_uses} uses)",
            abbreviate_key(invite_id),
            abbreviate_key(created_by)
        ),
        GovernanceEvent::InviteUsed {
            code_hash, uses, ..
        } => format!("invite {} used ({uses} uses)", abbreviate_key(code_hash)),
        GovernanceEvent::InviteRevoked { code_hash, .. } => {
            format!("invite {} revoked", abbreviate_key(code_hash))
        }
        GovernanceEvent::InviteExpired { code_hash, .. } => {
            format!("invite {} expired", abbreviate_key(code_hash))
        }
        GovernanceEvent::ChannelPermissionsChanged { channel, .. } => {
            format!("permissions changed on channel {}", abbreviate_key(channel))
        }
        GovernanceEvent::EffectivePermissionsChanged { member, .. } => {
            format!("permissions changed for {}", abbreviate_key(member))
        }
        GovernanceEvent::GovernanceRebuilt { .. } => "governance rebuilt".into(),
        GovernanceEvent::GovernanceOpQuarantined {
            op,
            author,
            reason,
            cause,
            ..
        } => {
            let why = match reason {
                OpQuarantineReason::AfterAuthorRemoval => "after its author's removal",
                OpQuarantineReason::TwoCheckpointEndorsementsInOneRound => {
                    "two checkpoint endorsements in one round"
                }
                OpQuarantineReason::CheckpointEquivocation => "checkpoint equivocation",
                OpQuarantineReason::RecordClaimed => "member record already claimed by another member",
                OpQuarantineReason::AuthorityQuarantined => {
                    "quarantined because the op its authority came from was"
                }
                OpQuarantineReason::DeviceKeyRetired => {
                    "its device key was retired or forked before it"
                }
                OpQuarantineReason::DeviceKeyInert => "its device key was endorsed past six devices",
                OpQuarantineReason::DeviceKeyOverCap => {
                    "its device key is not among its author's first six"
                }
                OpQuarantineReason::DeviceKeyNeedsReadmit => {
                    "its device key answers a fork with no re-admission"
                }
                OpQuarantineReason::DeviceKeyRecordTaken => {
                    "its device key names another device's record"
                }
                OpQuarantineReason::UnknownOpKind => {
                    "a kind of change this version cannot read; the community is quarantined"
                }
                OpQuarantineReason::UnknownOpVersion => {
                    "a newer version's change this version cannot read; the community is quarantined"
                }
            };
            match cause {
                Some(cause) => format!("op {op} by {author} quarantined: {why} (cause {cause})"),
                None => format!("op {op} by {author} quarantined: {why}"),
            }
        }
        GovernanceEvent::CheckpointSnapshotMismatch { height, digest, .. } => format!(
            "checkpoint {} at height {height} endorsed a false state snapshot",
            abbreviate_key(digest)
        ),
        GovernanceEvent::SuccessorLinkRequested {
            request,
            recipient,
            new_anchor,
            ..
        } => format!(
            "{} asks us to link into successor {} (request {})",
            abbreviate_key(recipient),
            abbreviate_key(new_anchor),
            request
        ),
        GovernanceEvent::CommunitySuccessorOffered {
            new_anchor,
            signer,
            creators,
            ..
        } => format!(
            "{} offers successor community {} (creators: {})",
            abbreviate_key(signer),
            abbreviate_key(new_anchor),
            creators
                .iter()
                .map(|c| abbreviate_key(c))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

#[cfg(test)]
#[path = "community_watch_tests.rs"]
mod tests;
