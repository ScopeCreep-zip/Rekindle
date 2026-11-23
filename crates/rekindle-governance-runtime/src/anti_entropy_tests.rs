//! A frontier whose heads did not all fetch stays unread: the next
//! inspect reads it again although its seq has not moved. A frontier with
//! our heads and another roster checksum is warned once and never rebuilds
//! our state (D15, review S7m part 4 Q2); an installed state lagging our
//! op set's merge is rebuilt once.

use super::*;
use crate::deps::MockGovernanceRuntimeDeps;
use rekindle_governance::checksum::roster_checksum;
use rekindle_records::lease::LeaseId;
use rekindle_types::governance::{Frontier, OpRef, RosterChecksum};
use rekindle_types::id::PseudonymKey;

fn frontier() -> Vec<u8> {
    rekindle_codec::governance_op::encode_frontier(&Frontier {
        fork_proof: Vec::new(),
        fork_refs: Vec::new(),
        heads: vec![OpRef {
            op_hash: OpHash([7; 32]),
            device_key: [1; 32],
            author: PseudonymKey([1; 32]),
            author_record: "VLD0:m".into(),
            author_seq: 0,
        }],
        roster_checksum: RosterChecksum([0; 32]),
    })
    .expect("frontier encodes")
}

/// `deps` for a record at frontier seq 5 whose one head never fetches,
/// read `reads` times.
fn deps(reads: usize) -> MockGovernanceRuntimeDeps {
    let mut deps = MockGovernanceRuntimeDeps::new();
    let mut ops = crate::deps::MockGovernanceOpStore::new();
    deps.expect_acquire_record()
        .returning(|_, _, _| Ok(LeaseId(1)));
    deps.expect_release_record().returning(|_| ());
    deps.expect_inspect_dht_record_update_get_seqs()
        .returning(|_| {
            let mut seqs = vec![None; member_record::SUBKEY_COUNT as usize];
            seqs[member_record::FRONTIER as usize] = Some(5);
            Ok(seqs)
        });
    deps.expect_read_hinted_value()
        .times(reads)
        .returning(|_, _| Ok(Some((5, frontier()))));
    ops.expect_fetch_governance_op().returning(|_, _| Ok(None));
    let ops: std::sync::Arc<dyn crate::deps::GovernanceOpStore> = std::sync::Arc::new(ops);
    deps.expect_op_store()
        .returning(move || std::sync::Arc::clone(&ops));
    deps
}

#[tokio::test]
async fn a_head_that_did_not_fetch_leaves_the_frontier_unread() {
    let deps = deps(2);
    let seen = SeenFrontiers::new();
    let mut known = BTreeSet::new();
    for _ in 0..2 {
        let stored = inspect_one(&deps, "A", "VLD0:m", &seen, &mut known, FrontierRead::Sweep)
            .await
            .expect("inspected");
        assert_eq!(stored, 0);
    }
    assert!(seen.get("VLD0:m").is_none(), "{seen:?}");
    assert!(known.is_empty(), "an unfetched head is not known");
}

/// An op whose encoding hashes to `OpRef.op_hash` (`held_from_encoded`
/// checks the hash, not the signature) and the frontier naming it.
fn fetchable() -> (Vec<u8>, Vec<u8>) {
    let op = rekindle_types::governance::GovernanceOp {
        device_key: [1; 32],
        author: PseudonymKey([1; 32]),
        seq: 0,
        prev: None,
        deps: Vec::new(),
        issued_at: 0,
        body: rekindle_types::governance::GovernanceEntry::CommunityMeta {
            name: Some("a".into()),
            description: None,
            icon_hash: None,
            banner_hash: None,
        },
        sig: [0; 64],
    };
    let unsigned = rekindle_codec::governance_op::op_unsigned_bytes(&op);
    let hash = rekindle_governance::op::op_hash(&unsigned, &op.sig);
    let encoded = rekindle_codec::governance_op::encode_op(&op).expect("op encodes");
    let frontier = rekindle_codec::governance_op::encode_frontier(&Frontier {
        fork_proof: Vec::new(),
        fork_refs: Vec::new(),
        heads: vec![OpRef {
            op_hash: hash,
            device_key: [1; 32],
            author: PseudonymKey([1; 32]),
            author_record: "VLD0:m".into(),
            author_seq: 0,
        }],
        roster_checksum: RosterChecksum([0; 32]),
    })
    .expect("frontier encodes");
    (encoded, frontier)
}

/// A head that did not fetch on the first read is fetched on the next read
/// at the same seq and stored; only then is the seq read, so a third
/// inspect does not read the frontier again. Before d1f6493b the first
/// read marked the head known and the seq read, so the op was never
/// stored.
#[tokio::test]
async fn a_head_that_fetches_on_a_later_read_is_stored_and_then_the_seq_is_read() {
    let (encoded, frontier) = fetchable();
    let mut deps = MockGovernanceRuntimeDeps::new();
    let mut ops = crate::deps::MockGovernanceOpStore::new();
    deps.expect_acquire_record()
        .returning(|_, _, _| Ok(LeaseId(1)));
    deps.expect_release_record().returning(|_| ());
    deps.expect_inspect_dht_record_update_get_seqs()
        .returning(|_| {
            let mut seqs = vec![None; member_record::SUBKEY_COUNT as usize];
            seqs[member_record::FRONTIER as usize] = Some(5);
            Ok(seqs)
        });
    deps.expect_read_hinted_value()
        .times(2)
        .returning(move |_, _| Ok(Some((5, frontier.clone()))));
    let fetches = std::sync::atomic::AtomicUsize::new(0);
    ops.expect_fetch_governance_op().returning(move |_, _| {
        let first = fetches.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0;
        Ok((!first).then(|| encoded.clone()))
    });
    ops.expect_store_governance_op()
        .times(1)
        .returning(|_, _| Ok(()));
    let ops: std::sync::Arc<dyn crate::deps::GovernanceOpStore> = std::sync::Arc::new(ops);
    deps.expect_op_store()
        .returning(move || std::sync::Arc::clone(&ops));
    let seen = SeenFrontiers::new();
    let mut known = BTreeSet::new();
    let mut stored = Vec::new();
    for _ in 0..3 {
        stored.push(
            inspect_one(&deps, "A", "VLD0:m", &seen, &mut known, FrontierRead::Sweep)
                .await
                .expect("inspected"),
        );
    }
    assert_eq!(stored, [0, 1, 0]);
    assert_eq!(seen.get("VLD0:m"), Some(5));
}

/// The roster checksum a peer's frontier carries in [`converging`].
enum Peer {
    /// The checksum of the merge of the op set it shares with us.
    Merged,
    /// Another checksum: its merge disagrees with ours.
    Other,
}

/// `deps` holding one signed op of ours, whose record's frontier names
/// exactly that op as its head with `peer`'s checksum; the installed state
/// is that op set's merge, or with `stale` the empty state of a rebuild
/// still pending; the rebuild entry point ([`crate::rebuild_governance`])
/// is reached `rebuilds` times, seen through its first call (the
/// joined-community check, which ends it here). Returns `deps` and the op
/// set's hashes, our `known`. The community is whole.
fn converging(
    peer: &Peer,
    stale: bool,
    rebuilds: usize,
) -> (MockGovernanceRuntimeDeps, BTreeSet<OpHash>, Frontier) {
    converging_as(peer, stale, rebuilds, false, 5)
}

/// [`converging`], partial (joined from a hint, backfill not ended) when
/// `partial` is set, with the inspect reporting the frontier at
/// `inspect_seq` (the read still returns the value at seq 5).
fn converging_as(
    peer: &Peer,
    stale: bool,
    rebuilds: usize,
    partial: bool,
    inspect_seq: u64,
) -> (MockGovernanceRuntimeDeps, BTreeSet<OpHash>, Frontier) {
    use rekindle_secrets::ed25519_dalek::SigningKey;
    let key = SigningKey::from_bytes(&[9; 32]);
    let cert = rekindle_types::governance::GenesisCert {
        name: "c".into(),
        policy: rekindle_types::governance::AdmissionMode::Open,
        initial_roles: Vec::new(),
        creator_pseudonym: PseudonymKey(key.verifying_key().to_bytes()),
        creator_member_record: "VLD0:m".into(),
        creator_member_record_owner: "o".into(),
        created_at: 1,
        additional_creators: Vec::new(),
        predecessor: None,
        creator_device: None,
    };
    let state = rekindle_governance::state::GovernanceState::default();
    let (op, encoded) = op_log::build_op(
        "A",
        &cert,
        &state,
        &op_log::VerifiedSet::default(),
        crate::test_member::signer(&key, &crate::test_member::device_of(&key)),
        rekindle_governance::fixture::meta("a"),
        op_log::Deps::Frontier,
        5,
    )
    .expect("op builds");
    let merged = rekindle_governance::merge::merge(&cert, std::slice::from_ref(&op));
    assert_ne!(
        roster_checksum(&merged),
        roster_checksum(&state),
        "the op changes the roster, so a stale state's checksum differs"
    );
    let checksum = match peer {
        Peer::Merged => roster_checksum(&merged),
        Peer::Other => RosterChecksum([0xEE; 32]),
    };
    let state = if stale { state } else { merged };
    let held = op_log::held(&op, "VLD0:m".into(), encoded, op_log::OpFate::Applied);
    let known = BTreeSet::from([op.hash()]);
    let served = Frontier {
        fork_proof: Vec::new(),
        fork_refs: Vec::new(),
        heads: vec![op.op_ref("VLD0:m")],
        roster_checksum: checksum,
    };
    let bytes = rekindle_codec::governance_op::encode_frontier(&served).expect("frontier encodes");
    let mut deps = MockGovernanceRuntimeDeps::new();
    let mut ops = crate::deps::MockGovernanceOpStore::new();
    deps.expect_acquire_record()
        .returning(|_, _, _| Ok(LeaseId(1)));
    deps.expect_release_record().returning(|_| ());
    deps.expect_inspect_dht_record_update_get_seqs()
        .returning(move |_| {
            let mut seqs = vec![None; member_record::SUBKEY_COUNT as usize];
            seqs[member_record::FRONTIER as usize] = Some(inspect_seq);
            Ok(seqs)
        });
    deps.expect_read_hinted_value()
        .times(1)
        .returning(move |_, _| Ok(Some((5, bytes.clone()))));
    deps.expect_genesis_cert()
        .returning(move |_| Some(cert.clone()));
    // The op set and the installed state are read under the lock every
    // installer holds (plan S7m4b).
    let rebuild = std::sync::Arc::new(tokio::sync::Mutex::new(()));
    deps.expect_rebuild_lock()
        .returning(move |_| std::sync::Arc::clone(&rebuild));
    deps.expect_governance_state()
        .returning(move |_| Some(state.clone()));
    ops.expect_governance_ops()
        .returning(move |_| Ok(vec![held.clone()]));
    ops.expect_governance_partial().returning(move |_| {
        Ok(partial.then_some(crate::partial::PartialJoin {
            height: 1,
            digest: [1; 32],
            state_root: [2; 32],
        }))
    });
    let ops: std::sync::Arc<dyn crate::deps::GovernanceOpStore> = std::sync::Arc::new(ops);
    deps.expect_op_store()
        .returning(move || std::sync::Arc::clone(&ops));
    let verdicts = std::sync::Arc::new(crate::verify_cache::OpVerifyCache::default());
    deps.expect_op_verify_cache()
        .returning(move |_| std::sync::Arc::clone(&verdicts));
    deps.expect_list_community_governance_targets()
        .times(rebuilds)
        .returning(Vec::new);
    (deps, known, served)
}

/// Equal heads with another roster checksum: a peer's merge disagrees
/// with ours over the same op set. Rebuilding the same merge cannot change
/// ours, so nothing is rebuilt (a peer cannot trigger rebuilds), and the
/// seq is read.
#[tokio::test]
async fn a_peer_mismatch_with_equal_heads_does_not_rebuild() {
    let (deps, mut known, _) = converging(&Peer::Other, false, 0);
    let seen = SeenFrontiers::new();
    let stored = inspect_one(&deps, "A", "VLD0:m", &seen, &mut known, FrontierRead::Sweep)
        .await
        .expect("inspected");
    assert_eq!(stored, 0);
    assert_eq!(seen.get("VLD0:m"), Some(5));
}

/// Plan E3.2 S7c.8 (1): a partial community compares no D15 roster
/// checksum, so equal heads with another checksum do not rebuild it.
#[tokio::test]
async fn a_partial_community_compares_no_roster_checksum() {
    let (deps, mut known, _) = converging_as(&Peer::Other, false, 0, true, 5);
    let seen = SeenFrontiers::new();
    let stored = inspect_one(&deps, "A", "VLD0:m", &seen, &mut known, FrontierRead::Sweep)
        .await
        .expect("inspected");
    assert_eq!(stored, 0);
    assert_eq!(seen.get("VLD0:m"), Some(5));
}

/// The inspect and the read are separate fanouts: an inspect that reports
/// the frontier at seq 6 while the read settles on the value at seq 5 marks
/// read seq 5, the seq of the bytes stored, so the next pass reads seq 6.
/// Before 8bcc16881 the sweep marked the inspect's seq 6 read against the
/// seq-5 bytes and never read seq 6.
#[tokio::test]
async fn a_read_older_than_the_inspect_marks_read_the_seq_it_returned() {
    let (deps, mut known, _) = converging_as(&Peer::Merged, false, 0, false, 6);
    let seen = SeenFrontiers::new();
    let stored = inspect_one(&deps, "A", "VLD0:m", &seen, &mut known, FrontierRead::Sweep)
        .await
        .expect("inspected");
    assert_eq!(stored, 0);
    assert_eq!(seen.get("VLD0:m"), Some(5), "the read returned seq 5");
    assert!(seen.unread("VLD0:m", 6), "seq 6 is still to be read");
}

/// Equal heads with our own checksum: nothing to rebuild.
#[tokio::test]
async fn equal_heads_with_our_checksum_do_not_rebuild() {
    let (deps, mut known, _) = converging(&Peer::Merged, false, 0);
    let seen = SeenFrontiers::new();
    let stored = inspect_one(&deps, "A", "VLD0:m", &seen, &mut known, FrontierRead::Sweep)
        .await
        .expect("inspected");
    assert_eq!(stored, 0);
    assert_eq!(seen.get("VLD0:m"), Some(5));
}

/// Equal heads with the checksum of our op set's merge while our installed
/// state lags it: our own state is behind our own merge, so it is rebuilt,
/// once.
#[tokio::test]
async fn a_lagging_installed_state_is_rebuilt_once() {
    let (deps, mut known, _) = converging(&Peer::Merged, true, 1);
    let seen = SeenFrontiers::new();
    let stored = inspect_one(&deps, "A", "VLD0:m", &seen, &mut known, FrontierRead::Sweep)
        .await
        .expect("inspected");
    assert_eq!(stored, 0);
    assert_eq!(seen.get("VLD0:m"), Some(5));
}

/// The frontier we write carries the checksum of the merge of the op set
/// its heads come from, not the installed state's.
#[test]
fn our_frontier_carries_the_checksum_of_its_heads_merge() {
    use rekindle_secrets::ed25519_dalek::SigningKey;
    let key = SigningKey::from_bytes(&[9; 32]);
    let cert = rekindle_types::governance::GenesisCert {
        name: "c".into(),
        policy: rekindle_types::governance::AdmissionMode::Open,
        initial_roles: Vec::new(),
        creator_pseudonym: PseudonymKey(key.verifying_key().to_bytes()),
        creator_member_record: "VLD0:m".into(),
        creator_member_record_owner: "o".into(),
        created_at: 1,
        additional_creators: Vec::new(),
        predecessor: None,
        creator_device: None,
    };
    let (op, encoded) = op_log::build_op(
        "A",
        &cert,
        &rekindle_governance::state::GovernanceState::default(),
        &op_log::VerifiedSet::default(),
        crate::test_member::signer(&key, &crate::test_member::device_of(&key)),
        rekindle_governance::fixture::meta("a"),
        op_log::Deps::Frontier,
        5,
    )
    .expect("op builds");
    let held = [op_log::held(
        &op,
        "VLD0:m".into(),
        encoded,
        op_log::OpFate::Applied,
    )];
    let set = op_log::verify_held("A", &held);
    let frontier = op_log::own_frontier(&cert, &set, &[], None);
    assert_eq!(frontier.heads, vec![op.op_ref("VLD0:m")]);
    assert_eq!(
        frontier.roster_checksum,
        roster_checksum(&rekindle_governance::merge::merge(&cert, &set.ops))
    );
}

/// VU3's hint contract: a ring change or a hint that names the frontier
/// force-reads it with no inspect first, even when the seq last read is the
/// one a lagging replica's inspect would still report (`seen` holds 5), so
/// the change is never turned into "nothing newer".
#[tokio::test]
async fn a_hinted_frontier_read_never_inspects_first() {
    let mut deps = MockGovernanceRuntimeDeps::new();
    let mut ops = crate::deps::MockGovernanceOpStore::new();
    deps.expect_acquire_record()
        .returning(|_, _, _| Ok(LeaseId(1)));
    deps.expect_release_record().returning(|_| ());
    deps.expect_inspect_dht_record_update_get_seqs().never();
    deps.expect_get_dht_value().never();
    deps.expect_read_hinted_value()
        .times(1)
        .returning(|_, subkey| {
            assert_eq!(subkey, member_record::FRONTIER);
            Ok(Some((6, frontier())))
        });
    ops.expect_fetch_governance_op().returning(|_, _| Ok(None));
    let ops: std::sync::Arc<dyn crate::deps::GovernanceOpStore> = std::sync::Arc::new(ops);
    deps.expect_op_store()
        .returning(move || std::sync::Arc::clone(&ops));
    let seen = SeenFrontiers::new();
    seen.mark_read("VLD0:m", 5);
    let mut known = BTreeSet::new();
    let stored = inspect_one(
        &deps,
        "A",
        "VLD0:m",
        &seen,
        &mut known,
        FrontierRead::Hinted,
    )
    .await
    .expect("read");
    assert_eq!(stored, 0, "its head did not fetch");
    assert_eq!(
        seen.get("VLD0:m"),
        Some(5),
        "an incomplete read stays unread"
    );
}

/// Review S7m part 4 Q2: the same bogus frontier checked again (the
/// sweep, then the ring or a hint) warns once and rebuilds nothing; the
/// divergence memo holds it, and the second check reuses the cached merge.
#[tokio::test]
async fn a_repeated_peer_mismatch_warns_once_and_never_rebuilds() {
    let (deps, mut known, served) = converging(&Peer::Other, false, 0);
    let seen = SeenFrontiers::new();
    inspect_one(&deps, "A", "VLD0:m", &seen, &mut known, FrontierRead::Sweep)
        .await
        .expect("inspected");
    check_convergence(&deps, "A", "VLD0:m", &served, &seen)
        .await
        .expect("checked");
    let heads = op_set_digest(served.heads.iter().map(|r| &r.op_hash));
    assert!(
        !seen.note_divergence("VLD0:m", heads, served.roster_checksum),
        "the first check noted (and warned about) this tuple; the repeat was silent"
    );
    assert!(seen.merged(&op_set_digest(&known)).is_some());
}

/// A lagging installed state checked twice for one op set is rebuilt once:
/// a rebuild that did not close the lag is not repeated.
#[tokio::test]
async fn a_lag_the_rebuild_did_not_close_is_not_rebuilt_again() {
    let (deps, mut known, served) = converging(&Peer::Merged, true, 1);
    let seen = SeenFrontiers::new();
    inspect_one(&deps, "A", "VLD0:m", &seen, &mut known, FrontierRead::Sweep)
        .await
        .expect("inspected");
    check_convergence(&deps, "A", "VLD0:m", &served, &seen)
        .await
        .expect("checked");
}

/// The sweep and a ring catch-up hold one `SeenFrontiers`: a frontier the
/// ring read at seq 5 is not read again by the sweep's inspect at seq 5.
#[tokio::test]
async fn the_sweep_and_the_ring_share_their_reads() {
    let (deps, mut known, _) = converging(&Peer::Merged, false, 0);
    let ring = SeenFrontiers::new();
    let sweep = ring.clone();
    inspect_one(&deps, "A", "VLD0:m", &ring, &mut known, FrontierRead::Sweep)
        .await
        .expect("inspected");
    // `converging` allows one frontier read: a second would fail the mock.
    let stored = inspect_one(
        &deps,
        "A",
        "VLD0:m",
        &sweep,
        &mut known,
        FrontierRead::Sweep,
    )
    .await
    .expect("inspected");
    assert_eq!(stored, 0);
}
