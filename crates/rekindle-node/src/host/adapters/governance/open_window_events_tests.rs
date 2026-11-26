//! A merge reaches a window that is already open (plan E3.2, OWR1): the
//! first projection of a community (a join's first merge) announces the
//! merged roster and our own verdicts, and an ingested role assignment that
//! grants us a permission names us with the new denied map, under the same
//! pseudonym form the details serve as `myPseudonymKey`.

use rekindle_governance::op::OpStamp;
use rekindle_governance::state::{GovernanceState, MemberState, RoleState};
use rekindle_governance_runtime::deps::GovernanceRuntimeDeps;
use rekindle_transport::session::CommunityMembership;
use rekindle_types::dto::community_capability::{CommunityCapability, DeniedCapabilities};
use rekindle_types::id::{PseudonymKey, RoleId};
use rekindle_types::permissions::BAN_MEMBERS;
use rekindle_types::subscription_events::{
    GovernanceEvent, MembershipEvent, StreamFrame, SubscriptionEvent,
};

use crate::host::test_fixtures::{create_ann, offline_host};

const C: &str = "c1";
const BANNERS: RoleId = RoleId([5; 16]);

fn key(byte: u8) -> PseudonymKey {
    PseudonymKey([byte; 32])
}

/// Members 1 (us) and 2, and a "Banners" role holding Ban Members.
fn merged() -> GovernanceState {
    let mut state = GovernanceState::default();
    for m in [1, 2] {
        state.members.insert(
            key(m),
            MemberState {
                member_record: "record".into(),
                member_record_owner: "owner".into(),
                joined: OpStamp::MIN,
            },
        );
    }
    state.roles.insert(
        BANNERS,
        RoleState {
            name: "Banners".into(),
            permissions: BAN_MEMBERS,
            position: 1,
            color: 0,
            hoist: false,
            mentionable: false,
            self_assignable: false,
            exclusion_group: None,
            stamp: OpStamp::MIN,
        },
    );
    state
}

/// What a Host joined to `C` as member 1 publishes when `next` is merged
/// over `prev` (`None`: the community's first projection), and the
/// `myPseudonymKey` its details serve (`views::community_view`).
async fn merge_events(
    prev: Option<GovernanceState>,
    next: GovernanceState,
) -> (Vec<SubscriptionEvent>, Option<String>) {
    let dir = tempfile::tempdir().unwrap();
    let host = offline_host(&dir).await;
    create_ann(&host).await;
    host.ctx.communities.install_membership(
        &CommunityMembership {
            anchor: C.into(),
            pseudonym_key: key(1).to_hex(),
            registry_key: String::new(),
            slot_index: 0,
            community_name: "Cove".into(),
            slot_seed: None,
            segment_index: Some(0),
            mek_generation: 0,
        },
        None,
    );
    if let Some(prev) = prev {
        host.ctx.communities.set_governance_state(C, prev);
    }
    let mut stream = host.ctx.journal.subscribe(None);
    host.governance().set_governance_state(C, next);
    let mut events = Vec::new();
    while let Ok(Some(frame)) =
        tokio::time::timeout(std::time::Duration::from_millis(200), stream.next()).await
    {
        if let StreamFrame::Event(s) = frame {
            events.push(s.event);
        }
    }
    let mine = (host.ctx.communities.map().read().get(C)).and_then(|c| c.my_pseudonym_key.clone());
    (events, mine)
}

/// Each `EffectivePermissionsChanged` of `C`: its member and denied map.
fn permission_events(events: &[SubscriptionEvent]) -> Vec<(String, DeniedCapabilities)> {
    events
        .iter()
        .filter_map(|e| match e {
            SubscriptionEvent::Governance(GovernanceEvent::EffectivePermissionsChanged {
                community,
                member,
                denied,
            }) if community == C => Some((member.clone(), denied.clone())),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn an_ingested_role_assign_for_us_names_us_with_ban_members_allowed() {
    let prev = merged();
    let mut next = prev.clone();
    next.role_assignments
        .entry(key(1))
        .or_default()
        .insert(BANNERS);
    let (events, mine) = merge_events(Some(prev), next).await;
    let named = permission_events(&events);
    assert_eq!(named.len(), 1, "one verdict, ours: {named:?}");
    let (member, denied) = &named[0];
    assert_eq!(
        Some(member),
        mine.as_ref(),
        "the details' myPseudonymKey form"
    );
    assert!(!denied.contains_key(&CommunityCapability::BanMembers));
    assert!(denied.contains_key(&CommunityCapability::ManageRoles));
}

#[tokio::test]
async fn a_first_projection_names_us_with_the_merged_verdicts() {
    let mut next = merged();
    next.role_assignments
        .entry(key(1))
        .or_default()
        .insert(BANNERS);
    let (events, mine) = merge_events(None, next).await;
    let named = permission_events(&events);
    assert_eq!(named.len(), 1, "ours alone: {named:?}");
    let (member, denied) = &named[0];
    assert_eq!(
        Some(member),
        mine.as_ref(),
        "the details' myPseudonymKey form"
    );
    assert!(!denied.contains_key(&CommunityCapability::BanMembers));
}

#[tokio::test]
async fn a_first_projection_refreshes_the_roster_of_open_windows() {
    let (events, _) = merge_events(None, merged()).await;
    assert!(
        events.iter().any(|e| matches!(
            e,
            SubscriptionEvent::Membership(MembershipEvent::MembersRefreshed { community })
                if community == C
        )),
        "no MembersRefreshed: {events:?}"
    );
}
