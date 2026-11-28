//! The host over a temp root with no node (plan D3.1).

use rekindle_ipc::message::SecurityLevel;
use rekindle_ipc::protocol::{IpcRequest, IpcResponse};

use super::*;
use crate::host::fixture::temp_root;
use crate::host::shell_hooks;
use rekindle_ipc::{Caller, ConnId, HostApi};
use rekindle_secrets::ed25519_dalek::SigningKey;

use governance_seed::meta;
pub(in crate::host) use governance_seed::{seed_governance, signed_chain, CREATOR_RECORD};

#[path = "attachment_limit_tests.rs"]
mod attachment_limit;
#[path = "audit_export_tests.rs"]
mod audit_export;
#[path = "automod_malformed_tests.rs"]
mod automod_malformed;
#[path = "bootstrap_unresolved_tests.rs"]
mod bootstrap_unresolved;
#[path = "channel_change_tests.rs"]
pub(crate) mod channel_change_tests;
#[path = "channel_gap_tests.rs"]
pub(crate) mod channel_gap_tests;
#[path = "channel_held_tests.rs"]
mod channel_held_tests;
#[path = "channel_ingest_tests.rs"]
mod channel_ingest_tests;
#[path = "channel_send_tests.rs"]
mod channel_send_tests;
#[path = "channel_view_tests.rs"]
mod channel_view_tests;
#[path = "checkpoint_rows_tests.rs"]
mod checkpoint_rows;
#[path = "checkpoint_state_tests.rs"]
mod checkpoint_state;
#[path = "community_info_tests.rs"]
mod community_info;
#[path = "expression_upload_tests.rs"]
mod expression_upload_tests;
#[path = "friend_code_wait_tests.rs"]
mod friend_code_wait;
#[path = "frontier_tests.rs"]
mod frontier_tests;
#[path = "governance_seed_tests.rs"]
mod governance_seed;
#[path = "held_threads_tests.rs"]
mod held_threads;
#[path = "invite_bounds_tests.rs"]
mod invite_bounds;
#[path = "login_stage_tests.rs"]
mod login_stage;
#[path = "mek_transfer_tests.rs"]
mod mek_transfer_tests;
#[path = "membership_tests.rs"]
mod membership;
#[path = "onboarding_bounds_tests.rs"]
mod onboarding_bounds;
#[path = "op_publications_tests.rs"]
mod op_publications;
#[path = "own_frontier_record_tests.rs"]
mod own_frontier_record;
#[path = "own_member_record_tests.rs"]
mod own_member_record;
#[path = "own_records_events_tests.rs"]
mod own_records_events;
#[path = "own_removal_tests.rs"]
mod own_removal;
#[path = "partial_tests.rs"]
mod partial;
#[path = "quick_wipe_tests.rs"]
mod quick_wipe;
#[path = "shell_hooks_tests.rs"]
mod shell_hooks_tests;
#[path = "signed_here_tests.rs"]
mod signed_here;
#[cfg(unix)]
#[path = "skin_apply_failed_tests.rs"]
mod skin_apply_failed;
#[cfg(unix)]
#[path = "skin_draft_tests.rs"]
mod skin_draft;
#[cfg(unix)]
#[path = "skin_tests.rs"]
mod skin_requests;
#[cfg(unix)]
#[path = "skin_serve_tests.rs"]
mod skin_serve;
#[cfg(unix)]
#[path = "skin_trial_tests.rs"]
mod skin_trial;
#[cfg(all(feature = "webdriver-test", debug_assertions))]
#[path = "webdriver_tests.rs"]
mod webdriver_tests;
#[path = "withdrawal_tests.rs"]
mod withdrawal;

// The offline Host and its seeds live in the public test fixture (plan
// GI3), which the desktop's relay tests call too.
use crate::host::test_fixtures::offline_host_with;
pub(crate) use crate::host::test_fixtures::{create_ann, offline_host, seed_registry_community};

fn status_of(response: &IpcResponse) -> Option<u32> {
    match response {
        IpcResponse::Error { code, .. } => Some(*code),
        _ => None,
    }
}

/// One create makes the identity and unlocks it (plan E1.1), offline: a
/// request then reaches that identity, and a logout takes it out again.
#[tokio::test]
async fn a_create_unlocks_until_logout() {
    let dir = tempfile::tempdir().unwrap();
    let host = offline_host(&dir).await;
    assert_eq!(host.mode(), HostMode::Shell);

    let view = create_ann(&host).await;
    assert_eq!(view.display_name, "Ann");
    assert_eq!(host.ctx.lifecycle.state(), DaemonState::Operational);
    assert_eq!(
        host.ctx.require_owner().ok().as_deref(),
        Some(view.public_key.as_str())
    );
    let shown = host.call(Caller::shell(), IpcRequest::IdentityShow).await;
    let IpcResponse::Ok { data, .. } = shown else {
        panic!("{shown:?}")
    };
    let shown: Option<rekindle_types::dto::identity::IdentityShown> =
        serde_json::from_value(data).unwrap();
    assert_eq!(shown.map(|s| s.public_key), Some(view.public_key));

    let out = host.call(Caller::shell(), IpcRequest::Logout).await;
    assert!(status_of(&out).is_none(), "{out:?}");
    assert_eq!(host.ctx.lifecycle.state(), DaemonState::Locked);
    assert!(host.ctx.session.read().is_none());
    assert!(host.ctx.db.current().is_err());
    let shown = host.call(Caller::shell(), IpcRequest::IdentityShow).await;
    assert!(matches!(shown, IpcResponse::Ok { data, .. } if data.is_null()));
}

/// One host per data root: a second claim of a held root is refused.
#[tokio::test]
async fn a_held_root_cannot_be_claimed_again() {
    let dir = tempfile::tempdir().unwrap();
    let _host = offline_host(&dir).await;
    assert!(claim(temp_root(&dir)).is_err());
}

/// Whether `state/` carries the platform's backup-exclusion mark.
fn state_is_excluded(root: &DataRoot) -> bool {
    #[cfg(target_os = "macos")]
    {
        use rekindle_utils::paths::backup::{TIME_MACHINE_XATTR, TIME_MACHINE_XATTR_VALUE};
        let out = std::process::Command::new("xattr")
            .args(["-px", TIME_MACHINE_XATTR])
            .arg(&root.state)
            .output()
            .unwrap();
        let hex = TIME_MACHINE_XATTR_VALUE
            .iter()
            .fold(String::new(), |mut hex, b| {
                use std::fmt::Write as _;
                let _ = write!(hex, "{b:02X}");
                hex
            });
        let said: String = String::from_utf8_lossy(&out.stdout)
            .split_whitespace()
            .collect();
        said == hex
    }
    #[cfg(target_os = "linux")]
    {
        root.state
            .join(rekindle_utils::paths::backup::DEJA_DUP_MARKER)
            .is_file()
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = root;
        true
    }
}

/// A claimed root keeps `state/` out of OS backups (plan E7.1, threat
/// model I14), a fresh root and one the layout check resets alike.
#[tokio::test]
async fn a_claimed_root_keeps_state_out_of_backups() {
    let dir = tempfile::tempdir().unwrap();
    let root = temp_root(&dir);
    drop(claim(root.clone()).unwrap());
    assert!(state_is_excluded(&root));
    // Another version's layout is reset, which empties `state/`.
    std::fs::write(root.layout_version(), "1").unwrap();
    drop(claim(root.clone()).unwrap());
    assert!(state_is_excluded(&root));
}

/// An own governance op lands in the op store at once (plan E3.2): the
/// warm restore reads it beside the other authors' ops, which stay.
#[tokio::test]
async fn an_own_governance_op_lands_in_the_op_store() {
    use rekindle_governance_runtime::deps::GovernanceOpStore as _;

    let dir = tempfile::tempdir().unwrap();
    let host = offline_host(&dir).await;
    let owner = create_ann(&host).await.public_key;
    seed_registry_community(&host, &owner).await;
    let other = SigningKey::from_bytes(&[1; 32]);
    let me = SigningKey::from_bytes(&[2; 32]);
    let theirs = signed_chain("c1", &other, CREATOR_RECORD, vec![meta("theirs")]);
    seed_governance(&host, &owner, "c1", &other, &theirs).await;

    let mine = signed_chain("c1", &me, "VLD0:my-record", vec![meta("mine")]);
    host.governance()
        .store_governance_op("c1", &mine[0])
        .await
        .unwrap();
    let held = host.governance().governance_ops("c1").await.unwrap();
    let mut authors: Vec<[u8; 32]> = held.iter().map(|op| op.author.0).collect();
    authors.sort_unstable();
    let mut expected = vec![
        other.verifying_key().to_bytes(),
        me.verifying_key().to_bytes(),
    ];
    expected.sort_unstable();
    assert_eq!(authors, expected);
    host.shutdown().await.unwrap();
}

/// Hydration recovers the stored registry owner keypair (plan E3.1 B2d):
/// a community whose store entry lacks it gets the vault's.
#[tokio::test]
async fn hydration_recovers_the_stored_registry_keypair() {
    use rekindle_governance_runtime::deps::SlotRecovery;

    let dir = tempfile::tempdir().unwrap();
    let host = offline_host(&dir).await;
    let owner = create_ann(&host).await.public_key;
    seed_registry_community(&host, &owner).await;
    rekindle_vault::typed::community_keys::persist_registry_keypair(
        host.ctx.vault.lock().as_ref().unwrap(),
        "c1",
        "registry-keypair",
    );

    let governance = host.governance();
    assert_eq!(
        governance.list_missing_registry_keypairs(),
        vec!["c1".to_string()]
    );
    governance.recover_registry_keypair_from_keystore("c1");
    let stored = host.ctx.communities.map().read()["c1"]
        .registry_owner_keypair
        .clone();
    assert_eq!(
        stored.as_ref().map(|key| key.as_str()),
        Some("registry-keypair")
    );
    assert!(governance.list_missing_registry_keypairs().is_empty());
    host.shutdown().await.unwrap();
}

/// The members a join discovered are stored (plan E3.1 B2d): known to the
/// community and in its `community_members` rows with their profiles,
/// unnamed (a name is the verified card's alone). Our own row is never the
/// scan's (plan E3.4 S6): its card fields stay what `set_card` wrote and its
/// profile what `set_own_profile` wrote, whatever the registry says of us.
#[tokio::test]
async fn discovered_registry_members_are_persisted() {
    use rekindle_db::repo::members;
    use rekindle_governance_runtime::{DiscoveredMember, GovernanceRuntimeDeps};
    use rekindle_types::presence::MemberPresence;

    let dir = tempfile::tempdir().unwrap();
    let host = offline_host(&dir).await;
    let owner = create_ann(&host).await.public_key;
    seed_registry_community(&host, &owner).await;
    let me = host.governance().me_in("c1").unwrap();
    let me_hex = hex::encode(me.0);
    let (o, m) = (owner.clone(), me_hex.clone());
    host.ctx
        .db
        .current()
        .unwrap()
        .call(move |conn| {
            members::set_card(conn, &o, "c1", &m, (Some("Ann"), Some("here")), 1)?;
            let profile = members::OwnProfile {
                bio: Some("mine".into()),
                ..Default::default()
            };
            members::set_own_profile(conn, &o, "c1", &m, &profile)
        })
        .await
        .unwrap();
    let bea = MemberPresence {
        pseudonym_key: rekindle_types::id::PseudonymKey([7; 32]),
        display_name: Some("Bea".into()),
        bio: Some("Bea's bio".into()),
        ..Default::default()
    };
    let stale_me = MemberPresence {
        pseudonym_key: me,
        display_name: Some("Old".into()),
        bio: Some("stale".into()),
        ..Default::default()
    };
    let found = |slot_index, presence| DiscoveredMember {
        segment_index: 0,
        slot_index,
        presence,
    };
    host.governance()
        .persist_discovered_registry_members("c1", vec![found(3, bea), found(4, stale_me)]);

    let bea_hex = hex::encode([7; 32]);
    assert!(host.ctx.communities.known_members("c1").contains(&bea_hex));
    let rows = host
        .ctx
        .db
        .current()
        .unwrap()
        .call(move |conn| members::load_profiles(conn, &owner))
        .await
        .unwrap();
    assert_eq!(rows.len(), 2, "{rows:?}");
    let row = |key: &str| rows.iter().find(|r| r.pseudonym_key == key).unwrap();
    let bea_row = row(&bea_hex);
    assert_eq!(bea_row.bio.as_deref(), Some("Bea's bio"));
    assert_eq!(bea_row.display_name, None, "the registry names nobody");
    let mine = row(&me_hex);
    assert_eq!(mine.display_name.as_deref(), Some("Ann"));
    assert_eq!(mine.custom_status.as_deref(), Some("here"));
    assert_eq!(mine.bio.as_deref(), Some("mine"));
    host.shutdown().await.unwrap();
}

#[path = "runtime_projection_tests.rs"]
mod projection;

/// A status picked while the node is detached is taken and kept (plan E7.13,
/// audit 2026-11-18): the identity is unlocked, so the status is ours to
/// show; our presence publishes a new beat with it once the network is ready.
#[tokio::test]
async fn a_status_picked_while_detached_is_kept() {
    use rekindle_types::presence::SessionStatus;
    let dir = tempfile::tempdir().unwrap();
    let host = offline_host(&dir).await;
    create_ann(&host).await;
    host.ctx
        .lifecycle
        .transition(DaemonState::Detached)
        .unwrap();

    let set = host
        .call(
            Caller::shell(),
            IpcRequest::PresenceSet {
                status: SessionStatus::Away,
            },
        )
        .await;
    assert!(
        status_of(&set).is_none(),
        "a detached status is taken: {set:?}"
    );
    let got = host.call(Caller::shell(), IpcRequest::PresenceGet).await;
    let IpcResponse::Ok { data, .. } = got else {
        panic!("{got:?}")
    };
    let view: rekindle_types::dto::presence::OwnPresenceView =
        serde_json::from_value(data).unwrap();
    assert_eq!(view.status, SessionStatus::Away);
}

/// A rename made while the node is detached is taken (walk 23 B12): the
/// name needs an unlocked identity, not the network; the own-records stage
/// publishes it with the profile when the network is up.
#[tokio::test]
async fn a_rename_while_detached_is_kept() {
    let dir = tempfile::tempdir().unwrap();
    let host = offline_host(&dir).await;
    create_ann(&host).await;
    host.ctx
        .lifecycle
        .transition(DaemonState::Detached)
        .unwrap();

    let set = host
        .call(
            Caller::shell(),
            IpcRequest::ProfileSetNickname {
                display_name: "Ann Two".into(),
            },
        )
        .await;
    assert!(
        status_of(&set).is_none(),
        "a detached rename is taken: {set:?}"
    );
    let name = (host.ctx.session.read().as_ref()).map(|s| s.identity.display_name.clone());
    assert_eq!(name.as_deref(), Some("Ann Two"));
}
