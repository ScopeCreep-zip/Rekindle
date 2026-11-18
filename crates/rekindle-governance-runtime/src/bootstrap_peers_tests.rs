//! Plan E3.2 S7c.8 (i): a partial inviter answers `bootstrapPartial` and
//! the joiner asks the members its walk shows, through their `M`/6 routes
//! read from a real pool.

use std::sync::Arc;

use rekindle_codec::community::envelope::{CommunityEnvelope, ControlPayload};
use rekindle_governance::op::OpStamp;
use rekindle_governance::state::{GovernanceState, MemberState};
use rekindle_record_pool::test_support::{pool_on, MemDht};
use rekindle_records::port::RecordPort;
use rekindle_secrets::ed25519_dalek::SigningKey;
use rekindle_types::governance::{OpHash, OpRef};
use rekindle_types::id::PseudonymKey;

use super::{candidates, find_frontier, MAX_BOOTSTRAP_PEERS};
use crate::error::GovernanceRuntimeError;
use crate::member_card::OwnRecord;
use crate::member_routes::write_routes;

const ANCHOR: &str = "community";
const INVITER_ROUTE: &[u8] = b"route-inviter";

fn encode(payload: ControlPayload) -> Vec<u8> {
    rekindle_codec::capnp_envelope::encode_community_envelope(&CommunityEnvelope::Control(payload))
        .unwrap()
}

fn partial() -> Vec<u8> {
    encode(ControlPayload::BootstrapPartial {
        anchor: ANCHOR.into(),
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
        anchor: ANCHOR.into(),
        frontier: vec![head()],
    })
}

/// A member whose `M`/6 holds `route`: its pseudonym and record.
async fn member(pool: &dyn RecordPort, seed: u8, route: &[u8]) -> (PseudonymKey, String) {
    let (lease, record, writer) =
        RecordPort::create(pool, &rekindle_records::schema::member_record(), None)
            .await
            .unwrap();
    pool.release(lease).await;
    let pseudonym = SigningKey::from_bytes(&[seed; 32]);
    let own = OwnRecord {
        community_id: ANCHOR,
        record: &record,
        writer: &writer,
        pseudonym: &pseudonym,
    };
    write_routes(pool, &own, route, 1_000).await.unwrap();
    (PseudonymKey(pseudonym.verifying_key().to_bytes()), record)
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
    let found = find_frontier(&call, &*pool, ANCHOR, INVITER_ROUTE, &asked, b"request").await;
    assert_eq!(found.unwrap(), vec![head()]);

    // Nobody whole among those asked: a typed error, not a refusal.
    let found = find_frontier(
        &call,
        &*pool,
        ANCHOR,
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
    let found = find_frontier(&refusing, &*pool, ANCHOR, INVITER_ROUTE, &whole, b"r").await;
    assert!(
        matches!(found, Err(GovernanceRuntimeError::BootstrapRefused(_))),
        "{found:?}"
    );
    let silent = caller(vec![(b"route-whole", frontier())]);
    let found = find_frontier(&silent, &*pool, ANCHOR, INVITER_ROUTE, &whole, b"r").await;
    assert!(
        matches!(found, Err(GovernanceRuntimeError::Adapter(_))),
        "{found:?}"
    );
}

/// Oldest join first, never us, the inviter or a banned member, at most
/// `MAX_BOOTSTRAP_PEERS`.
#[test]
fn members_are_asked_oldest_join_first_within_the_bound() {
    let key = |n: u8| PseudonymKey([n; 32]);
    let mut state = GovernanceState::default();
    for n in 1..=8u8 {
        state.members.insert(
            key(n),
            MemberState {
                member_record: format!("VLD0:m{n}"),
                member_record_owner: String::new(),
                joined: OpStamp {
                    lamport: u64::from(10 - n),
                    hash: OpHash([n; 32]),
                },
            },
        );
    }
    state.bans.insert(key(7));
    // 8 is us, 6 the inviter, 7 banned: 5, 4, 3, 2 joined earliest.
    let picked: Vec<_> = candidates(&state, &key(8), "VLD0:m6")
        .into_iter()
        .map(|(k, _)| k)
        .collect();
    assert_eq!(picked.len(), MAX_BOOTSTRAP_PEERS);
    assert_eq!(picked, vec![key(5), key(4), key(3), key(2)]);
}
