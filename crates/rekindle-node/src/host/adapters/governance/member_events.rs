//! Membership verdicts from the merged governance (plan E3.2 S9, A/B
//! F7b-1/F7b-2), on both modes: who joined, left, was kicked, banned or
//! unbanned is what the merge of the signed, permission-checked op DAG
//! says, read as the difference between the state a frontend was last
//! shown and the state just merged. Each transition is published once
//! (a later projection of the same state differs in nothing), with the
//! full payload the merge holds: the remover and the reason, the leaver's
//! own departure to its own frontends.
//!
//! A joiner waiting for approval (`JoinRequested`) and a timeout set or
//! lifted (`TimedOut`, `TimeoutRemoved`, each with its
//! `TimeoutStatusChanged`) are read the same way, ours included: our own
//! windows show our timeout from the same event.
//!
//! The gossip notices (`Kick`, `Ban`, `MemberLeave`, `MemberJoined`, ...)
//! publish no membership event (`rekindle-transport`
//! `control_into_event`): a gossip frame is a hint, not authenticated for
//! policy, and it carries neither the remover nor the reason, so a frontend
//! taking the first event it saw got a removal with no reason (F7b-2) and a
//! member whose gossip missed a leave never saw it (F7b-1). Matrix makes
//! the same split: membership is a state event in the room DAG
//! (`m.room.member`), never an ephemeral EDU.
//!
//! Our own kick or ban is not published here: the own-removal path
//! (`own_removal.rs`) announces it once, with the remover's name kept
//! across the teardown.

use rekindle_governance::diff::{GovernanceDiff, MemberChange};
use rekindle_types::id::PseudonymKey;
use rekindle_types::subscription_events::{MembershipEvent, SubscriptionEvent, SystemEvent};

use super::HostGovernance;

impl HostGovernance {
    /// The membership events of `diff`, the merged state against the
    /// state last projected (`None` on the first projection, which is a
    /// baseline and announces nothing).
    pub(super) fn member_transitions_of(
        &self,
        community_id: &str,
        diff: Option<&GovernanceDiff>,
        me: Option<&PseudonymKey>,
    ) -> Vec<MembershipEvent> {
        let Some(diff) = diff else {
            return Vec::new();
        };
        let communities = &self.ctx.communities;
        member_transitions(
            community_id,
            diff,
            me,
            |p| communities.member_card_name(community_id, p),
            |p| {
                Some(super::community_profile::shown_name(
                    communities.removal_name_of(community_id, p),
                    p,
                ))
            },
        )
    }

    /// Publish `events` (from [`Self::member_transitions_of`]).
    ///
    /// A `Joined` member goes on the stored roster first: a frontend
    /// re-reads the member list on `Joined` (`get_community_members` reads
    /// `community_members`), and the merge is the authority on membership
    /// (Matrix's `m.room.member` state event), so the list it reads must
    /// already hold the member. The write is queued on the database's one
    /// connection before the event leaves, so every read that follows the
    /// event sees it. Without it the row waited for the joiner's gossip
    /// `MemberJoined` or the presence poll, which publish nothing a window
    /// re-reads on, and an open window never listed the joiner (walk B61).
    ///
    /// Each `Joined` also counts in the community's raid window
    /// (architecture §20.6): the merge is where both modes learn of a
    /// join, so the alert rests on signed, permission-checked joins, not
    /// on a gossip notice anyone can send.
    pub(super) fn publish_member_transitions(&self, events: Vec<MembershipEvent>) {
        self.store_joined_rows(&events);
        let alerts = self.observe_raid_joins(&events);
        for event in events {
            self.ctx.publish_event(SubscriptionEvent::Membership(event));
        }
        for alert in alerts {
            self.ctx.publish_event(SubscriptionEvent::System(alert));
        }
    }

    /// The key rotation each merged departure in `events` requires (ADR
    /// 0018): a member who left still holds the current keys. Kicks and
    /// bans rotate from the moderation path; ours rotates nothing of ours.
    /// Every member asks, and the election inside picks the rotator
    /// (`blake3(departed || own)`), on both modes. The request is a plain
    /// departure (`MekRotationRequest::departure`), which holds no sends:
    /// a voluntary leave keeps sending under the old generation until the
    /// new one is confirmed (ADR 0018, 2026-10-24 amendment).
    pub(super) fn rotate_for_departures(
        &self,
        community_id: &str,
        events: &[MembershipEvent],
        me: Option<&PseudonymKey>,
    ) {
        use rekindle_governance_runtime::GovernanceRuntimeDeps;
        for departed in departures_to_rotate(events, me) {
            self.spawn_text_mek_rotation_for_ban(community_id, departed);
        }
    }

    /// The `RaidDetected` alerts the `Joined`s in `events` raise.
    fn observe_raid_joins(&self, events: &[MembershipEvent]) -> Vec<SystemEvent> {
        let now_secs = self.ctx.clock.now_secs();
        events
            .iter()
            .filter_map(|e| match e {
                MembershipEvent::Joined {
                    community,
                    pseudonym,
                    ..
                } => {
                    let alert = self
                        .ctx
                        .communities
                        .observe_raid_join(community, pseudonym, now_secs)?;
                    tracing::warn!(
                        community = %community,
                        joins = alert.joins_in_window,
                        threshold = alert.max_joins_per_interval,
                        interval_s = alert.join_interval_seconds,
                        "raid threshold exceeded — alerting moderators (architecture §20.6)"
                    );
                    Some(SystemEvent::RaidDetected {
                        community: community.clone(),
                        joins_in_window: alert.joins_in_window,
                        max_joins_per_interval: alert.max_joins_per_interval,
                        join_interval_seconds: alert.join_interval_seconds,
                    })
                }
                _ => None,
            })
            .collect()
    }

    /// Store an unnamed roster row for each `Joined` in `events`: its name
    /// is cached from the joiner's verified card alone (plan E3.4 S6).
    fn store_joined_rows(&self, events: &[MembershipEvent]) {
        let joined: Vec<(String, String)> = events
            .iter()
            .filter_map(|e| match e {
                MembershipEvent::Joined {
                    community,
                    pseudonym,
                    ..
                } => Some((community.clone(), pseudonym.clone())),
                _ => None,
            })
            .collect();
        if joined.is_empty() {
            return;
        }
        let Some((owner, db)) = self.owner_and_db() else {
            return;
        };
        let joined_at = self.ctx.clock.now_secs().cast_signed();
        db.fire("joined members", move |conn| {
            for (community, pseudonym) in &joined {
                rekindle_db::repo::members::insert_if_absent(
                    conn, &owner, community, pseudonym, joined_at,
                )?;
            }
            Ok(())
        });
    }
}

/// The membership events of `diff` (the merged state's
/// [`rekindle_governance::state::GovernanceState::diff`] from the state last projected), in its
/// order: members by pseudonym. `card_name_of` gives the name on a
/// member's verified card, the only source of a joiner's name (plan E3.2
/// F9-2: the stored `member_profiles` rows are not all signed by the
/// member, e.g. a gossip `MemberJoined` hint is the sender's word); `None`
/// while the card is not read, and its name follows as
/// `MemberCardChanged`. `name_of` gives a remover's card or profile name.
/// Our own kick or ban (`me`) is left to the own-removal path.
pub(super) fn member_transitions(
    community_id: &str,
    diff: &GovernanceDiff,
    me: Option<&PseudonymKey>,
    card_name_of: impl Fn(&PseudonymKey) -> Option<String>,
    name_of: impl Fn(&PseudonymKey) -> Option<String>,
) -> Vec<MembershipEvent> {
    let community = || community_id.to_owned();
    let mut out = Vec::new();
    for change in &diff.membership {
        let p = change.member();
        match change {
            MemberChange::JoinRequested { has_invite, .. } => {
                out.push(MembershipEvent::JoinRequested {
                    community: community(),
                    pseudonym: p.to_hex(),
                    display_name: card_name_of(p).unwrap_or_default(),
                    has_invite: *has_invite,
                });
            }
            MemberChange::TimedOut {
                duration_seconds,
                reason,
                until,
                ..
            } => out.extend([
                MembershipEvent::TimedOut {
                    community: community(),
                    target_pseudonym: p.to_hex(),
                    duration_seconds: *duration_seconds,
                    reason: reason.clone(),
                },
                MembershipEvent::TimeoutStatusChanged {
                    community: community(),
                    pseudonym: p.to_hex(),
                    timeout_until: Some(*until),
                },
            ]),
            MemberChange::TimeoutLifted { .. } => out.extend([
                MembershipEvent::TimeoutRemoved {
                    community: community(),
                    target_pseudonym: p.to_hex(),
                },
                MembershipEvent::TimeoutStatusChanged {
                    community: community(),
                    pseudonym: p.to_hex(),
                    timeout_until: None,
                },
            ]),
            MemberChange::Joined { .. } => out.push(MembershipEvent::Joined {
                community: community(),
                pseudonym: p.to_hex(),
                display_name: card_name_of(p),
                // The legacy numeric role list: a member's roles are 16-byte
                // ids, published by `RolesChanged` once assigned.
                role_ids: Vec::new(),
            }),
            MemberChange::Unbanned { .. } => out.push(MembershipEvent::Unbanned {
                community: community(),
                target_pseudonym: p.to_hex(),
            }),
            MemberChange::Banned { .. } | MemberChange::Kicked { .. } if me == Some(p) => {}
            MemberChange::Banned { by, reason, .. } => out.push(MembershipEvent::Banned {
                community: community(),
                target_pseudonym: p.to_hex(),
                removed_by_name: by.as_ref().and_then(&name_of),
                removed_by: by.as_ref().map(PseudonymKey::to_hex),
                reason: reason.clone(),
            }),
            MemberChange::Kicked { by, reason, .. } => out.push(MembershipEvent::Kicked {
                community: community(),
                target_pseudonym: p.to_hex(),
                removed_by_name: by.as_ref().and_then(&name_of),
                removed_by: by.as_ref().map(PseudonymKey::to_hex),
                reason: reason.clone(),
            }),
            MemberChange::Left { .. } => out.push(MembershipEvent::Left {
                community: community(),
                pseudonym: p.to_hex(),
            }),
        }
    }
    out
}

/// The members whose merged `Left` in `events` needs a key rotation: every
/// leaver but us. A `Kicked` or `Banned` is the moderation path's rotation,
/// and a gossip notice never reaches here (only merged transitions do).
fn departures_to_rotate<'a>(
    events: &'a [MembershipEvent],
    me: Option<&PseudonymKey>,
) -> Vec<&'a str> {
    let me = me.map(PseudonymKey::to_hex);
    events
        .iter()
        .filter_map(|event| match event {
            MembershipEvent::Left { pseudonym, .. }
                if me.as_deref() != Some(pseudonym.as_str()) =>
            {
                Some(pseudonym.as_str())
            }
            _ => None,
        })
        .collect()
}

#[cfg(test)]
#[path = "member_events_tests.rs"]
mod tests;
