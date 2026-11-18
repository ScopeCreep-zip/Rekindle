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
    // Events name a community by governance key; the user may give a name.
    let community_scope = match community {
        Some(community) => Some(governance_key(client, community).await?),
        None => None,
    };
    let filter = SubscriptionFilter {
        categories: Some(vec![EventCategory::Membership, EventCategory::Governance]),
        community_scope,
    };
    crate::watch::stream(client, vec![filter], mode, render).await
}

async fn governance_key(client: &DaemonClient, community: &str) -> anyhow::Result<String> {
    let detail: rekindle_types::display::CommunityDetail = serde_json::from_value(
        helpers::send(
            client,
            IpcRequest::Community(CommunityRequest::Lifecycle(
                LifecycleRequest::CommunityInfo {
                    governance_key: community.to_owned(),
                },
            )),
        )
        .await?,
    )?;
    Ok(detail.governance_key)
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

fn roles(role_ids: &[u32]) -> String {
    let ids: Vec<String> = role_ids.iter().map(u32::to_string).collect();
    format!("[{}]", ids.join(", "))
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
        MembershipEvent::JoinAccepted { community, .. } => {
            (community, "your join was accepted".into())
        }
        MembershipEvent::JoinRejected { community, reason } => (
            community,
            format!("your join was refused: {}", sanitize_for_display(reason)),
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
        MembershipEvent::MemberCardChanged {
            community,
            pseudonym,
            display_name,
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
        GovernanceEvent::RolesChanged { roles, .. } => {
            format!("roles changed ({} roles)", roles.len())
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
        GovernanceEvent::GovernanceRebuilt { .. } => "governance rebuilt".into(),
        GovernanceEvent::GovernanceOpQuarantined {
            op, author, reason, ..
        } => {
            let why = match reason {
                OpQuarantineReason::AfterAuthorRemoval => "after its author's removal",
                OpQuarantineReason::TwoCheckpointEndorsementsInOneRound => {
                    "two checkpoint endorsements in one round"
                }
                OpQuarantineReason::CheckpointEquivocation => "checkpoint equivocation",
            };
            format!("op {op} by {author} quarantined: {why}")
        }
        GovernanceEvent::CheckpointSnapshotMismatch { height, digest, .. } => format!(
            "checkpoint {} at height {height} endorsed a false state snapshot",
            abbreviate_key(digest)
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::render;
    use rekindle_types::subscription_events::{MembershipEvent, SubscriptionEvent};

    fn kicked(reason: Option<&str>) -> SubscriptionEvent {
        SubscriptionEvent::Membership(MembershipEvent::Kicked {
            community: "c".repeat(64),
            target_pseudonym: "t".repeat(64),
            removed_by: Some("b".repeat(64)),
            removed_by_name: Some("Ann".into()),
            reason: reason.map(str::to_owned),
        })
    }

    /// A kick shows who removed the member and why, in text and JSONL.
    #[test]
    fn a_kick_streams_its_remover_and_reason() {
        let event = kicked(Some("spam\u{1b}[2J"));
        let line = render(&event).expect("a kick is a community event");
        assert!(line.ends_with("was kicked by Ann: spam"), "{line}");
        let json = crate::output::format::jsonl_line(&event).expect("serializes");
        let value: serde_json::Value = serde_json::from_str(&json).expect("one JSON object");
        assert_eq!(value["membership"]["kicked"]["reason"], "spam\u{1b}[2J");
        assert!(!json.contains('\n'), "one line per event: {json}");
        let bare = render(&kicked(None)).expect("a kick is a community event");
        assert!(bare.ends_with("was kicked by Ann"), "{bare}");
    }

    /// A ban without a remover name falls back to the remover's key.
    #[test]
    fn a_ban_names_the_remover_key_without_a_card() {
        let event = SubscriptionEvent::Membership(MembershipEvent::Banned {
            community: "c".repeat(64),
            target_pseudonym: "t".repeat(64),
            removed_by: Some("b".repeat(64)),
            removed_by_name: None,
            reason: None,
        });
        let line = render(&event).expect("a ban is a community event");
        assert!(line.ends_with("was banned by bbbbbbbb…bbbb"), "{line}");
    }

    /// A join before the card is read shows the key alone; the card's name
    /// follows as its own line (plan E3.2 F9-2).
    #[test]
    fn a_join_shows_the_key_until_the_card_names_the_member() {
        let joined = SubscriptionEvent::Membership(MembershipEvent::Joined {
            community: "c".repeat(64),
            pseudonym: "p".repeat(64),
            display_name: None,
            role_ids: Vec::new(),
        });
        let line = render(&joined).expect("a join is a community event");
        assert!(line.ends_with(": pppppppp…pppp joined"), "{line}");
        let card = SubscriptionEvent::Membership(MembershipEvent::MemberCardChanged {
            community: "c".repeat(64),
            pseudonym: "p".repeat(64),
            display_name: "Ann".into(),
        });
        let line = render(&card).expect("a card name is a community event");
        assert!(
            line.ends_with(": Ann (pppppppp…pppp) is named on its card"),
            "{line}"
        );
    }

    /// Events outside a community are not this watch's.
    #[test]
    fn other_events_are_not_shown() {
        let event = SubscriptionEvent::UnreadChanged {
            context: rekindle_types::subscription_events::UnreadContext::FriendRequests,
            count: 1,
        };
        assert!(render(&event).is_none());
    }
}
