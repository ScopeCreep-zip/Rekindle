//! The one community store (plan E3.1 A1): its accessors, and a Shell-mode
//! Host reading what the GUI wrote into the shared map.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use rekindle_governance::state::{GovernanceState, RoleState};
use rekindle_ipc::protocol::MemberRequest;
use rekindle_ipc::protocol::{CommunityRequest, IpcRequest};
use rekindle_transport::session::CommunityMembership;
use rekindle_types::id::{PseudonymKey, RoleId};
use rekindle_types::intent::{Fact, IntentKind, Notifies, Subject};

use super::{new_map, CommunityState, CommunityStore, MemberRecord};
use crate::host::fixture::temp_root;
use crate::host::{claim, identity, Host, HostConfig, HostStores, TransportSource};

/// The keepalive starts once per community per session; lock (`clear`)
/// and leave (`remove`) let the next session start it again (C7.8b).
#[test]
fn the_keepalive_starts_once_until_cleared_or_removed() {
    let store = CommunityStore::new(new_map());
    assert!(store.start_keepalive_once("c1"));
    assert!(!store.start_keepalive_once("c1"));
    assert!(store.start_keepalive_once("c2"), "per community");
    store.remove("c1");
    assert!(store.start_keepalive_once("c1"), "leave resets it");
    store.clear();
    assert!(store.start_keepalive_once("c2"), "lock resets it");
}

#[test]
fn governance_state_round_trips_and_is_absent_until_set() {
    let store = CommunityStore::new(new_map());
    assert!(store.governance_state("c1").is_none());
    store.set_governance_state("c1", GovernanceState::default());
    assert!(store.governance_state("c1").is_some());
    assert!(store.governance_state("c2").is_none());
    let map = store.map().read();
    let entry = map.get("c1").expect("the first merge makes the entry");
    assert_eq!(entry.id, "c1");
    assert_eq!(entry.governance_key.as_deref(), Some("c1"));
}

#[test]
fn overflow_chain_keeps_order_and_skips_repeats() {
    let store = CommunityStore::new(new_map());
    store.register_overflow_keys("c1", &["page-1".into(), "page-2".into()]);
    store.register_overflow_keys("c1", &["page-2".into(), "page-3".into()]);
    assert_eq!(
        store.overflow_keys("c1"),
        vec!["page-1", "page-2", "page-3"]
    );
}

/// `set_members` replaces the roster wholesale and keeps `known_members`
/// equal to its keys, which the member count reads.
#[test]
fn set_members_replaces_the_roster_and_the_known_members() {
    let store = CommunityStore::new(new_map());
    let roster = |keys: &[&str]| {
        keys.iter()
            .map(|k| ((*k).to_string(), MemberRecord::default()))
            .collect::<HashMap<_, _>>()
    };
    store.set_members("c1", roster(&["aa", "bb"]));
    assert_eq!(store.member_count("c1"), 2);
    store.set_members("c1", roster(&["bb"]));
    assert_eq!(store.members("c1").len(), 1);
    assert_eq!(store.known_members("c1"), HashSet::from(["bb".to_string()]));
    assert_eq!(store.member_count("c1"), 1);
    assert_eq!(store.member_count("c2"), 0);
    store.set_member_roles("c1", HashMap::from([("bb".to_string(), vec![3])]));
    assert_eq!(store.member_roles("c1")["bb"], vec![3]);
    store.mark_channel_synced("c1", "ch");
    assert!(store.map().read()["c1"].synced_channels.contains("ch"));
}

fn membership(me: &PseudonymKey) -> CommunityMembership {
    CommunityMembership {
        governance_key: "gov-key".into(),
        pseudonym_key: hex::encode(me.0),
        role_ids: vec![0],
        registry_key: "reg-key".into(),
        slot_index: 0,
        community_name: "Team".into(),
        slot_seed: None,
        channel_record_keys: HashMap::new(),
        segment_index: Some(0),
        lamport_counter: 0,
        mek_generation: 0,
    }
}

/// A Shell-mode Host selects `RoleDelete` from the community state the GUI
/// wrote into `AppState.communities` (the Host's map): the role's name and
/// holders and the members it reaches. Before A1 the Host read a map only
/// the daemon filled, and refused (503, governance not loaded).
#[tokio::test]
async fn a_shell_host_selects_a_role_delete_from_gui_written_state() {
    let dir = tempfile::tempdir().unwrap();
    let stores = HostStores::default();
    let gui = stores.clone();
    let host = Host::start(HostConfig {
        root: claim(temp_root(&dir)).unwrap(),
        stores,
        transport: TransportSource::Offline,
        serve_bus: false,
        clock: Arc::new(rekindle_utils::SystemClock),
        rng: Arc::new(rekindle_utils::OsRandom),
        idle: Arc::new(rekindle_idle::OsIdle),
        shell: None,
    })
    .await
    .unwrap();
    assert_eq!(host.mode(), crate::host::HostMode::Shell);
    let made = identity::create(host.root(), &host.identity_stores(), Some("Ann"), "pass", 1)
        .await
        .unwrap();
    let owner = made.public_key.clone();
    drop(made);
    let me = PseudonymKey([1; 32]);
    let db = gui.db.current().unwrap();
    let mut session =
        crate::host::session::load(&db, &gui.vault, &owner, &rekindle_utils::OsRandom, 0)
            .await
            .unwrap();
    session.join_community(membership(&me));
    *gui.session.write() = Some(session);

    // What the GUI's governance rebuild writes.
    let role = RoleId::from_legacy_u32(5);
    let mut governance = GovernanceState::default();
    governance.roles.insert(
        role,
        RoleState {
            name: "mods".into(),
            permissions: 0,
            position: 1,
            color: 0,
            hoist: false,
            mentionable: false,
            self_assignable: false,
            exclusion_group: None,
            lamport: 1,
        },
    );
    governance
        .role_assignments
        .insert(PseudonymKey([2; 32]), HashSet::from([role]));
    let mut community = CommunityState::new("gov-key");
    community.governance_state = Some(governance);
    community.known_members = HashSet::from([hex::encode(me.0), hex::encode([2u8; 32])]);
    gui.communities
        .write()
        .insert("gov-key".to_string(), community);

    let request = IpcRequest::Community(CommunityRequest::Members(MemberRequest::RoleDelete {
        community: "gov-key".into(),
        role_id: 5,
    }));
    let selection = crate::host::intent_select::select(&host.ctx, &request, IntentKind::DeleteRole)
        .await
        .expect("the GUI's state is the Host's");
    assert_eq!(
        selection.subject,
        Subject::Role {
            id: "5".into(),
            name: "mods".into()
        }
    );
    assert!(selection
        .facts
        .contains(&Fact::MembersAffected { count: 1 }));
    assert_eq!(selection.notifies, Notifies::Members(2));
}

fn thread_by(creator: &PseudonymKey) -> rekindle_governance::state::ThreadState {
    rekindle_governance::state::ThreadState {
        parent_channel_id: rekindle_types::id::ChannelId([1; 16]),
        name: "side".into(),
        thread_type: "public".into(),
        record_key: None,
        invited: Vec::new(),
        forum_tag: None,
        auto_archive_seconds: 3600,
        creator: creator.clone(),
        created_lamport: 2,
        archived_lamport: None,
        unarchived_lamport: None,
    }
}

/// A timed-out member's `ThreadCreated` received by the store is held from
/// the projection, and stays held once the timeout ends; another member's
/// post in another thread passes the ingest gate.
#[test]
fn a_timed_out_members_thread_is_held_and_another_members_post_admitted() {
    use rekindle_channel::automod::AutoModCompiledCache;
    use rekindle_channel::ingest::{gate, BlockList, GateItem, RateLimits, Verdict};
    use rekindle_types::id::ThreadId;

    let (late, other) = (PseudonymKey([0xbb; 32]), PseudonymKey([0xcc; 32]));
    let store = CommunityStore::new(new_map());
    store.project_governance("c1", GovernanceState::default(), 100);
    let mut merged = GovernanceState::default();
    merged.timeouts.insert(
        late.clone(),
        rekindle_governance::state::TimeoutState {
            duration_seconds: 60,
            started_at: 100,
            lamport: 1,
        },
    );
    merged.threads.insert(ThreadId([2; 16]), thread_by(&late));
    merged.threads.insert(ThreadId([3; 16]), thread_by(&other));
    store.project_governance("c1", merged.clone(), 120);
    let gov = store.governance_state("c1").expect("merged");
    assert!(!gov.threads.contains_key(&ThreadId([2; 16])), "held");
    assert!(gov.threads.contains_key(&ThreadId([3; 16])));

    store.project_governance("c1", merged, 500);
    let gov = store.governance_state("c1").expect("merged");
    assert!(!gov.threads.contains_key(&ThreadId([2; 16])), "stays held");

    let (thread, sender) = (hex::encode([3u8; 16]), hex::encode(other.0));
    let item = GateItem {
        community_id: "c1",
        channel_id: &thread,
        sender_pseudonym: &sender,
        body: "reply",
        sent_at_secs: 120,
        now_secs: 120,
    };
    let verdict = gate(
        &item,
        &gov,
        &AutoModCompiledCache::compile(&gov),
        &BlockList::default(),
        &mut RateLimits::default(),
    );
    assert_eq!(verdict, Verdict::Admit);
}
