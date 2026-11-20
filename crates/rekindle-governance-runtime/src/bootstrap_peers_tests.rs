//! Plan E3.2 S7c.8 (i): a partial inviter answers `bootstrapPartial` and
//! the joiner asks the live members its walk shows, in the documented hash
//! order, through their `M`/6 routes read from a real pool, within the cap.

use std::sync::Arc;

use parking_lot::Mutex;
use std::time::Duration;

use rekindle_codec::community::envelope::{CommunityEnvelope, ControlPayload};
use rekindle_governance::op::OpStamp;
use rekindle_governance::state::{GovernanceState, MemberState};
use rekindle_record_pool::test_support::{pool_on, MemDht};
use rekindle_records::port::RecordPort;
use rekindle_secrets::ed25519_dalek::SigningKey;
use rekindle_types::governance::{OpHash, OpRef};
use rekindle_types::id::PseudonymKey;
use rekindle_types::presence::SessionStatus;

use super::{bootstrap_peers_cap, candidates, find_frontier, BootstrapPeer, MAX_BOOTSTRAP_PEERS};
use crate::error::GovernanceRuntimeError;
use crate::member_card::{write_beat, OwnProfile, OwnRecord};
use crate::member_routes::write_routes;
use rekindle_codec::member_record::BEAT_PERIOD_MS;
use rekindle_presence::community::BEAT_LIVENESS_MS;

const BOOTSTRAP_COMMUNITY: &str = "community";
const INVITER_ROUTE: &[u8] = b"route-inviter";
/// Now, on the joiner's clock (ms).
const JOINER_NOW_MS: u64 = 1_800_000_000_000;
const ASKING_AT: (&str, u64) = (BOOTSTRAP_COMMUNITY, JOINER_NOW_MS);

fn encode(payload: ControlPayload) -> Vec<u8> {
    rekindle_codec::capnp_envelope::encode_community_envelope(&CommunityEnvelope::Control(payload))
        .unwrap()
}

fn partial() -> Vec<u8> {
    encode(ControlPayload::BootstrapPartial {
        anchor: BOOTSTRAP_COMMUNITY.into(),
    })
}

fn head() -> OpRef {
    OpRef {
        op_hash: OpHash([7; 32]),
        author: PseudonymKey([8; 32]),
        author_record: "VLD0:head".into(),
        author_seq: 3,
    }
}

fn frontier() -> Vec<u8> {
    encode(ControlPayload::BootstrapResponseV2 {
        anchor: BOOTSTRAP_COMMUNITY.into(),
        frontier: vec![head()],
    })
}

/// A member whose `M`/6 holds `route` and whose beat was written online
/// now: its candidate entry.
async fn member(pool: &dyn RecordPort, seed: u8, route: &[u8]) -> BootstrapPeer {
    member_beating(pool, seed, route, (SessionStatus::Online, JOINER_NOW_MS)).await
}

/// A member whose `M`/6 holds `route` and whose beat says `status`,
/// written at `beat_ms`.
async fn member_beating(
    pool: &dyn RecordPort,
    seed: u8,
    route: &[u8],
    (status, beat_ms): (SessionStatus, u64),
) -> BootstrapPeer {
    let (lease, record, writer) =
        RecordPort::create(pool, &rekindle_records::schema::member_record(), None)
            .await
            .unwrap();
    let owner = pool.owner_of(lease).unwrap();
    pool.release(lease).await;
    let pseudonym = SigningKey::from_bytes(&[seed; 32]);
    let own = OwnRecord {
        community_id: BOOTSTRAP_COMMUNITY,
        record: &record,
        writer: &writer,
        pseudonym: &pseudonym,
    };
    write_routes(pool, &own, route, 1_000).await.unwrap();
    let profile = OwnProfile {
        display_name: String::new(),
        status,
        beat_seq: 1,
        now_ms: beat_ms,
        route: Vec::new(),
    };
    write_beat(pool, &own, &profile).await.unwrap();
    BootstrapPeer {
        pseudonym: PseudonymKey(pseudonym.verifying_key().to_bytes()),
        record,
        owner,
    }
}

/// Answers by route: each route's reply, or an unreachable route.
fn caller(
    answers: Vec<(&'static [u8], Vec<u8>)>,
) -> impl Fn(Vec<u8>, Vec<u8>) -> std::future::Ready<Result<Vec<u8>, GovernanceRuntimeError>> {
    move |route, _request| {
        std::future::ready(
            answers
                .iter()
                .find(|(r, _)| *r == route.as_slice())
                .map(|(_, reply)| reply.clone())
                .ok_or_else(|| GovernanceRuntimeError::Adapter("app_call: timeout".into())),
        )
    }
}

/// The inviter is partial, the first member asked is unreachable, the
/// second refuses (behind on the invite op), the third is partial and the
/// fourth answers: the join's walk continues from the fourth's frontier.
#[tokio::test]
async fn a_partial_inviters_joiner_bootstraps_from_another_member() {
    let pool = pool_on(Arc::new(MemDht::default()));
    let gone = member(&*pool, 1, b"route-gone").await;
    let behind = member(&*pool, 2, b"route-behind").await;
    let also_partial = member(&*pool, 3, b"route-partial").await;
    let whole = member(&*pool, 4, b"route-whole").await;
    let call = caller(vec![
        (INVITER_ROUTE, partial()),
        (b"route-behind", b"NAK".to_vec()),
        (b"route-partial", partial()),
        (b"route-whole", frontier()),
    ]);
    let asked = [gone, behind, also_partial, whole];
    let found = find_frontier(&call, &*pool, ASKING_AT, INVITER_ROUTE, &asked, b"request").await;
    assert_eq!(found.unwrap(), vec![head()]);

    // Nobody whole among those asked: a typed error, not a refusal.
    let found = find_frontier(
        &call,
        &*pool,
        ASKING_AT,
        INVITER_ROUTE,
        &asked[..3],
        b"request",
    )
    .await;
    assert!(
        matches!(found, Err(GovernanceRuntimeError::BootstrapUnavailable(3))),
        "{found:?}"
    );
}

/// The inviter's own NAK is the refusal that withdraws the join, and its
/// silence leaves the join standing from the DHT; neither asks others.
#[tokio::test]
async fn only_a_partial_inviter_sends_the_joiner_elsewhere() {
    let pool = pool_on(Arc::new(MemDht::default()));
    let whole = [member(&*pool, 4, b"route-whole").await];
    let refusing = caller(vec![
        (INVITER_ROUTE, b"NAK".to_vec()),
        (b"route-whole", frontier()),
    ]);
    let found = find_frontier(&refusing, &*pool, ASKING_AT, INVITER_ROUTE, &whole, b"r").await;
    assert!(
        matches!(found, Err(GovernanceRuntimeError::BootstrapRefused(_))),
        "{found:?}"
    );
    let silent = caller(vec![(b"route-whole", frontier())]);
    let found = find_frontier(&silent, &*pool, ASKING_AT, INVITER_ROUTE, &whole, b"r").await;
    assert!(
        matches!(found, Err(GovernanceRuntimeError::Adapter(_))),
        "{found:?}"
    );
}

/// The documented order (architecture §7.1, `select_mek_responder` with
/// us as the requester): lowest `blake3(joiner ‖ member)` first, never
/// us, the inviter or a banned member, and not the join order.
#[test]
fn members_are_asked_in_the_responder_hash_order() {
    let key = |n: u8| PseudonymKey([n; 32]);
    let mut state = GovernanceState::default();
    for n in 1..=8u8 {
        state.members.insert(
            key(n),
            MemberState {
                member_record: format!("VLD0:m{n}"),
                member_record_owner: format!("owner{n}"),
                joined: OpStamp {
                    lamport: u64::from(10 - n),
                    hash: OpHash([n; 32]),
                },
            },
        );
    }
    state.bans.insert(key(7));
    // 8 is us, 6 the inviter, 7 banned.
    let picked: Vec<_> = candidates(&state, &key(8), "VLD0:m6")
        .into_iter()
        .map(|c| c.pseudonym)
        .collect();
    let mut want = vec![key(1), key(2), key(3), key(4), key(5)];
    want.sort_by_key(|m| rekindle_secrets::rotator::election_hash(&key(8).0, &m.0));
    assert_eq!(picked, want);
    assert_eq!(
        picked.first(),
        rekindle_secrets::rotator::select_mek_responder(&key(8), &want).as_ref()
    );
    let by_join = vec![key(5), key(4), key(3), key(2), key(1)];
    assert_ne!(picked, by_join, "not oldest join first");
}

/// Replies by route, recording each route called in order.
fn recording(
    answers: Vec<(&'static [u8], Vec<u8>)>,
    log: Arc<Mutex<Vec<Vec<u8>>>>,
) -> impl Fn(Vec<u8>, Vec<u8>) -> std::future::Ready<Result<Vec<u8>, GovernanceRuntimeError>> {
    let inner = caller(answers);
    move |route, request| {
        log.lock().push(route.clone());
        inner(route, request)
    }
}

/// A member whose beat is one beat period past the 15-minute rule, or
/// whose fresh beat says `Offline`, is skipped without a call: the member
/// one beat period inside the rule after them answers.
#[tokio::test]
async fn a_member_with_a_stale_beat_is_not_asked() {
    let pool = pool_on(Arc::new(MemDht::default()));
    let expired = (
        SessionStatus::Online,
        JOINER_NOW_MS - BEAT_LIVENESS_MS - BEAT_PERIOD_MS,
    );
    let stale = member_beating(&*pool, 1, b"route-stale", expired).await;
    let offline = (SessionStatus::Offline, JOINER_NOW_MS);
    let gone = member_beating(&*pool, 2, b"route-offline", offline).await;
    let inside = (
        SessionStatus::Online,
        JOINER_NOW_MS - BEAT_LIVENESS_MS + BEAT_PERIOD_MS,
    );
    let live = member_beating(&*pool, 3, b"route-whole", inside).await;
    let log = Arc::new(Mutex::new(Vec::new()));
    let call = recording(
        vec![
            (INVITER_ROUTE, partial()),
            (b"route-stale", frontier()),
            (b"route-offline", frontier()),
            (b"route-whole", frontier()),
        ],
        log.clone(),
    );
    let found = find_frontier(
        &call,
        &*pool,
        ASKING_AT,
        INVITER_ROUTE,
        &[stale, gone, live],
        b"r",
    )
    .await;
    assert_eq!(found.unwrap(), vec![head()]);
    assert_eq!(
        *log.lock(),
        vec![INVITER_ROUTE.to_vec(), b"route-whole".to_vec()]
    );
}

/// Every live member is partial and each call runs 10 s (an `app_call`
/// may run to its 11 s deadline): the cap ends the asking after the
/// third call, short of [`MAX_BOOTSTRAP_PEERS`], and the join stands.
#[tokio::test(start_paused = true)]
async fn the_cap_bounds_the_attempts() {
    const ONE_CALL: Duration = Duration::from_secs(10);
    assert_eq!(bootstrap_peers_cap(), Duration::from_secs(24));
    let pool = pool_on(Arc::new(MemDht::default()));
    let mut asked = Vec::new();
    for seed in 1..=6u8 {
        asked.push(member(&*pool, seed, &[seed]).await);
    }
    let calls = Arc::new(Mutex::new(0usize));
    let counted = calls.clone();
    let call = move |route: Vec<u8>, _request: Vec<u8>| {
        let calls = counted.clone();
        async move {
            if route.as_slice() != INVITER_ROUTE {
                *calls.lock() += 1;
                tokio::time::sleep(ONE_CALL).await;
            }
            Ok::<_, GovernanceRuntimeError>(partial())
        }
    };
    let started = tokio::time::Instant::now();
    let found = find_frontier(&call, &*pool, ASKING_AT, INVITER_ROUTE, &asked, b"r").await;
    assert!(
        matches!(found, Err(GovernanceRuntimeError::BootstrapUnavailable(3))),
        "{found:?}"
    );
    assert_eq!(*calls.lock(), 3);
    // The cap, not the count, ended it.
    const { assert!(3 < MAX_BOOTSTRAP_PEERS) };
    assert!(started.elapsed() <= bootstrap_peers_cap() + Duration::from_secs(1));
}

/// Plan E3.2 join step 5: the inviter's `bootstrapRefused` for our anchor
/// decodes to its reason; one for another anchor is a bare NAK, whose
/// reason is unnamed (`NotAdmitted`).
#[test]
fn a_bootstrap_refused_reply_decodes_to_its_reason() {
    use rekindle_types::join_refusal::JoinRefusal;
    for reason in [
        JoinRefusal::InviteUsedUp,
        JoinRefusal::InviteNoLongerValid,
        JoinRefusal::InviteUnknown,
        JoinRefusal::NotAdmitted,
    ] {
        let reply = crate::encode_refusal("VLD0:anchor", reason).expect("encodes");
        match super::read_reply(&reply, "VLD0:anchor") {
            Err(GovernanceRuntimeError::BootstrapRefused(got)) => assert_eq!(got, reason),
            other => panic!("{reason:?}: expected the refusal, got {other:?}"),
        }
        match super::read_reply(&reply, "VLD0:other") {
            Err(GovernanceRuntimeError::BootstrapRefused(got)) => {
                assert_eq!(
                    got,
                    JoinRefusal::NotAdmitted,
                    "another anchor's refusal is a NAK"
                );
            }
            other => panic!("expected a NAK, got {other:?}"),
        }
    }
}
