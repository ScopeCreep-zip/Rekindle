use super::*;
use rekindle_types::id::ChannelId;

#[test]
fn classify_role_definition_marks_roles() {
    let entry = GovernanceEntry::RoleDefinition {
        role_id: rekindle_types::id::RoleId([0u8; 16]),
        name: "test".into(),
        permissions: 0,
        position: 0,
        color: 0,
        hoist: false,
        mentionable: false,
        self_assignable: false,
        exclusion_group: None,
    };
    let affects = classify_entry(&entry);
    assert!(affects.roles);
    assert!(!affects.channels_or_categories);
}

#[test]
fn classify_role_archived_marks_roles() {
    let entry = GovernanceEntry::RoleArchived {
        role_id: rekindle_types::id::RoleId([0u8; 16]),
    };
    let affects = classify_entry(&entry);
    assert!(affects.roles);
    assert!(!affects.channels_or_categories);
}

#[test]
fn classify_channel_created_marks_channels() {
    let entry = GovernanceEntry::ChannelCreated {
        channel_id: ChannelId([0u8; 16]),
        name: "general".into(),
        channel_type: "text".into(),
        category_id: None,
        position: 0,
        parent_voice_channel_id: None,
    };
    let affects = classify_entry(&entry);
    assert!(!affects.roles);
    assert!(affects.channels_or_categories);
}

#[test]
fn classify_category_updated_marks_channels() {
    let entry = GovernanceEntry::CategoryUpdated {
        category_id: rekindle_types::id::CategoryId([0u8; 16]),
        name: Some("renamed".into()),
        position: None,
    };
    let affects = classify_entry(&entry);
    assert!(!affects.roles);
    assert!(affects.channels_or_categories);
}

#[test]
fn out_of_bounds_entries_are_refused_by_field() {
    use rekindle_types::channel::MAX_SLOWMODE_SECONDS;
    use rekindle_types::key_format::MAX_DESCRIPTION_LEN;
    use rekindle_types::member::MAX_TIMEOUT_SECONDS;
    let slowmode = |seconds| GovernanceEntry::ChannelUpdated {
        channel_id: ChannelId([0u8; 16]),
        name: None,
        topic: None,
        forum_tags: None,
        position: None,
        slowmode_seconds: Some(seconds),
        nsfw: None,
        category_id: None,
    };
    let timeout = |duration_seconds| GovernanceEntry::TimeoutEntry {
        target: PseudonymKey([0u8; 32]),
        duration_seconds,
        reason: None,
        started_at: 0,
    };
    let meta = |description: String| GovernanceEntry::CommunityMeta {
        name: None,
        description: Some(description),
        icon_hash: None,
        banner_hash: None,
    };
    let field = |entry: &GovernanceEntry| match refuse_out_of_bounds(entry) {
        Err(GovernanceRuntimeError::OutOfBounds { field }) => Some(field),
        Err(other) => panic!("unexpected {other}"),
        Ok(()) => None,
    };
    assert_eq!(
        field(&slowmode(MAX_SLOWMODE_SECONDS + 1)),
        Some("slowmode_seconds")
    );
    assert_eq!(field(&slowmode(MAX_SLOWMODE_SECONDS)), None);
    assert_eq!(field(&timeout(0)), Some("duration_seconds"));
    assert_eq!(
        field(&timeout(MAX_TIMEOUT_SECONDS + 1)),
        Some("duration_seconds")
    );
    assert_eq!(field(&timeout(MAX_TIMEOUT_SECONDS)), None);
    let long = "x".repeat(MAX_DESCRIPTION_LEN + 1);
    assert_eq!(field(&meta(long)), Some("description"));
    assert_eq!(field(&meta("ok\n".into())), None);
}

/// `write_entry` refuses before touching anything: the mock has no
/// expectations, so any state lookup or DHT call would panic.
#[tokio::test]
async fn write_entry_returns_out_of_bounds_before_any_io() {
    let deps = crate::deps::MockGovernanceRuntimeDeps::new();
    let entry = GovernanceEntry::TimeoutEntry {
        target: PseudonymKey([0u8; 32]),
        duration_seconds: 0,
        reason: None,
        started_at: 0,
    };
    let refused = write_entry(&deps, "community", entry).await;
    assert!(matches!(
        refused,
        Err(GovernanceRuntimeError::OutOfBounds {
            field: "duration_seconds"
        })
    ));
}

/// A write reads the state and op set only under the rebuild lock
/// (register S7l, HIGH-2): while a rebuild holds it, the write reads
/// nothing, so a rebuild that read an older op set cannot install over
/// the write's state.
#[tokio::test]
async fn a_write_waits_for_a_running_rebuild_before_reading_the_state() {
    let held = std::sync::Arc::new(tokio::sync::Mutex::new(()));
    let read = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let mut deps = crate::deps::MockGovernanceRuntimeDeps::new();
    let lock = std::sync::Arc::clone(&held);
    deps.expect_rebuild_lock()
        .returning(move |_| std::sync::Arc::clone(&lock));
    let seen = std::sync::Arc::clone(&read);
    deps.expect_governance_state().returning(move |_| {
        seen.store(true, std::sync::atomic::Ordering::SeqCst);
        None
    });
    let deps = std::sync::Arc::new(deps);
    let rebuilding = held.lock().await;
    let writer = std::sync::Arc::clone(&deps);
    let write = tokio::spawn(async move {
        write_entry(
            writer.as_ref(),
            "community",
            GovernanceEntry::MemberLeft { heads: Vec::new() },
        )
        .await
    });
    for _ in 0..50 {
        tokio::task::yield_now().await;
    }
    assert!(
        !read.load(std::sync::atomic::Ordering::SeqCst),
        "the write read the state while a rebuild held the lock"
    );
    drop(rebuilding);
    let written = write.await.unwrap();
    assert!(read.load(std::sync::atomic::Ordering::SeqCst));
    assert!(matches!(
        written,
        Err(GovernanceRuntimeError::GovernanceStateMissing(_))
    ));
}

#[test]
fn classify_role_assignment_affects_nothing() {
    let entry = GovernanceEntry::RoleAssignment {
        target: PseudonymKey([0u8; 32]),
        role_id: rekindle_types::id::RoleId([0u8; 16]),
    };
    let affects = classify_entry(&entry);
    assert!(!affects.roles);
    assert!(!affects.channels_or_categories);
}

/// An honest member's op written through the real write path is judged
/// `Effective` by every reader (ADR 0022 rules 1, 6): read back from the
/// state the write installs, its device key is the one `p` endorsed, in
/// the merged `DeviceView`, never the pseudonym.
#[tokio::test]
async fn an_op_written_through_the_write_path_judges_effective_in_the_merged_view() {
    use crate::deps::{CommunityMembership, MockGovernanceRuntimeDeps, OpOutcome};
    use parking_lot::Mutex;
    use rekindle_governance::device_judge::{judge_op_at, KeyJudgement};
    use rekindle_secrets::ed25519_dalek::SigningKey;
    use std::sync::Arc;

    const COMMUNITY: &str = crate::test_world::ANCHOR;
    let secret = [9u8; 32];
    let p = derive::derive_community_pseudonym(&secret, COMMUNITY);
    let d = SigningKey::from_bytes(&[8; 32]);
    let author = PseudonymKey(p.verifying_key().to_bytes());
    let (record, owner) = ("VLD0:m", "o");
    let cert = GenesisCert {
        name: "c".into(),
        policy: rekindle_types::governance::AdmissionMode::Open,
        initial_roles: Vec::new(),
        creator_pseudonym: author.clone(),
        creator_member_record: record.into(),
        creator_member_record_owner: owner.into(),
        created_at: 1,
        additional_creators: Vec::new(),
        predecessor: None,
        creator_device: Some(device_signer::first_endorsement(
            &p, COMMUNITY, &d, record, owner,
        )),
    };
    // A held genesis op seeds the cert's device in a merge (the write
    // lands on the record that endorsement binds); with none held, the
    // fixture seeds it as that merge would.
    let mut state = merge::merge(&cert, &[]);
    state.devices.seed_genesis(COMMUNITY, &cert);

    let mut deps = MockGovernanceRuntimeDeps::new();
    let mut ops = crate::deps::MockGovernanceOpStore::new();
    let lock = Arc::new(tokio::sync::Mutex::new(()));
    deps.expect_rebuild_lock()
        .returning(move |_| Arc::clone(&lock));
    deps.expect_governance_state()
        .returning(move |_| Some(state.clone()));
    // The write path judges successors in the state it installs (SS4h).
    deps.expect_successor_judgements()
        .returning(|_| Arc::default());
    let hex = author.to_hex();
    deps.expect_community_membership().returning(move |_| {
        Some(CommunityMembership {
            anchor: Some(COMMUNITY.into()),
            member_registry_key: None,
            my_pseudonym_hex: Some(hex.clone()),
            my_subkey_index: None,
            my_segment_index: None,
            slot_keypair: None,
            slot_seed_hex: None,
            channel_ids: Vec::new(),
            mek_generation: 0,
        })
    });
    let pinned = cert.clone();
    deps.expect_genesis_cert()
        .returning(move |_| Some(pinned.clone()));
    ops.expect_governance_ops().returning(|_| Ok(Vec::new()));
    deps.expect_op_verify_cache()
        .returning(|_| Arc::new(crate::verify_cache::OpVerifyCache::default()));
    deps.expect_identity_secret()
        .returning(move || Some(secret));
    let device = d.to_bytes();
    deps.expect_device_op_key()
        .returning(move |_| Ok(Some(Zeroizing::new(device))));
    deps.expect_clock()
        .returning(|| Arc::new(rekindle_utils::SystemClock));
    let stored = Arc::new(Mutex::new(Vec::new()));
    let into = Arc::clone(&stored);
    ops.expect_store_signed_governance_op()
        .returning(move |_, op| {
            into.lock().push(op.clone());
            Ok(())
        });
    let ops: std::sync::Arc<dyn crate::deps::GovernanceOpStore> = std::sync::Arc::new(ops);
    deps.expect_op_store()
        .returning(move || std::sync::Arc::clone(&ops));
    deps.expect_gossip_governance_op().return_const(());
    let merged = Arc::new(Mutex::new(None));
    let installed = Arc::clone(&merged);
    deps.expect_set_governance_state()
        .returning(move |_, state| *installed.lock() = Some(state));
    deps.expect_emit_event().return_const(());
    deps.expect_publish_governance_op()
        .returning(|_, _, _| Ok(OpOutcome::Landed));

    let entry = rekindle_governance::fixture::meta("renamed");
    let outcome = write_entry(&deps, COMMUNITY, entry).await.unwrap();
    assert!(matches!(outcome, OpOutcome::Landed));

    let held = stored.lock().clone();
    let read = op_log::verify_held(COMMUNITY, &held);
    let [op] = read.ops.as_slice() else {
        panic!("one op written, read back verified");
    };
    assert_eq!(op.op().author, author, "p stays the author");
    let merged = merged.lock().take().expect("the write installed a state");
    let md = (merged.devices.member(&author).cloned()).unwrap_or_default();
    let record = md.latest_join.as_ref().map(|(_, r, _)| r.as_str());
    assert_eq!(
        judge_op_at(COMMUNITY, &md, record, op.op()),
        KeyJudgement::Effective,
        "an honest member's own op is effective in the merged view"
    );
}
