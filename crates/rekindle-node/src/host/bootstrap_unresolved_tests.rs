//! A bootstrap request the inviter cannot judge yet is no refusal (plan
//! E3.2 JN1, BR1's vocabulary): when the joiner's `MemberJoined` waits on a
//! dep the inviter neither holds nor can fetch yet, the answer is
//! `RETRY_LATER` and nothing is stored; a merge holding every dep that
//! still does not admit the join refuses it with its typed reason; once the
//! dep is held, the retried request is admitted. Offline, so a dep not in
//! the op store is not fetchable.

use rekindle_governance_runtime::bootstrap_peers::read_reply;
use rekindle_governance_runtime::deps::GovernanceOpStore as _;
use rekindle_governance_runtime::op_log::{rebuild_ops, verify_held, HeldOp};
use rekindle_governance_runtime::{
    answer_bootstrap_request, BootstrapAnswer, BootstrapJoiner, GovernanceRuntimeDeps as _,
    GovernanceRuntimeError,
};
use rekindle_secrets::derive::derive_community_pseudonym;
use rekindle_secrets::ed25519_dalek::SigningKey;
use rekindle_types::governance::{GovernanceEntry, GovernanceOp, InviteProof, OpRef};
use rekindle_types::id::PseudonymKey;
use rekindle_types::join_refusal::JoinRefusal;

use super::{create_ann, meta, offline_host, seed_governance, seed_registry_community};
use super::{signed_chain, CREATOR_RECORD};
use crate::host::Host;

const JN1_INVITE_CODE: &str = "join-code";

fn vld0(bytes: &[u8; 32]) -> String {
    use base64::Engine as _;
    format!(
        "VLD0:{}",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
    )
}

fn invite() -> GovernanceEntry {
    GovernanceEntry::InviteCreated {
        invite_id: [7; 16],
        code_hash: rekindle_secrets::invite::hash_invite_code(JN1_INVITE_CODE),
        max_uses: 100,
        expires_at: None,
        secrets_record_key: "VLD0:s".into(),
    }
}

/// The creator's chain: its meta, an invite, and that invite revoked.
fn creator_chain(creator: &SigningKey) -> Vec<HeldOp> {
    signed_chain(
        "c1",
        creator,
        CREATOR_RECORD,
        vec![
            meta("C"),
            invite(),
            GovernanceEntry::InviteRevoked { invite_id: [7; 16] },
        ],
    )
}

/// A joiner's signed `MemberJoined` on the invite, citing `dep`.
fn joiner_citing(dep: &HeldOp) -> BootstrapJoiner {
    use rekindle_codec::governance_op::{encode_op, op_unsigned_bytes};
    use rekindle_governance::op::{invite_proof_bytes, op_hash, op_signing_bytes};
    use rekindle_secrets::member_record::{dflt_opaque_record_key, sign_member_record_binding};
    use rekindle_types::dht_layout::member_record;

    let key = derive_community_pseudonym(&[0xB7; 32], "c1");
    let author = PseudonymKey(key.verifying_key().to_bytes());
    let owner = SigningKey::from_bytes(&[0xB8; 32])
        .verifying_key()
        .to_bytes();
    let record = vld0(&dflt_opaque_record_key(&owner, member_record::SUBKEY_COUNT));
    let code_hash = rekindle_secrets::invite::hash_invite_code(JN1_INVITE_CODE);
    let proof = invite_proof_bytes("c1", &author, &record, &code_hash);
    let mut op = GovernanceOp {
        device_key: author.0,
        author: author.clone(),
        seq: 0,
        prev: None,
        deps: vec![rekindle_governance_runtime::own_frontier::op_ref_of(dep)],
        issued_at: 10,
        body: GovernanceEntry::MemberJoined {
            member_record: record.clone(),
            member_record_owner: vld0(&owner),
            invite_proof: Some(InviteProof {
                sig: rekindle_secrets::invite::sign_invite_join(JN1_INVITE_CODE, &proof).to_vec(),
                code_hash,
            }),
            owner_proof: sign_member_record_binding(&[0xB8; 32], "c1", &author.0, &record).to_vec(),
            first_device: None,
        },
        sig: [0; 64],
    };
    let unsigned = op_unsigned_bytes(&op).unwrap();
    op.sig =
        rekindle_secrets::derive::sign_with_pseudonym(&key, &op_signing_bytes("c1", &unsigned));
    let hash = op_hash(&unsigned, &op.sig);
    BootstrapJoiner {
        pseudonym: hex::encode(author.0),
        member_joined: OpRef {
            op_hash: hash,
            device_key: author.0,
            author,
            author_record: record,
            author_seq: 0,
        },
        member_joined_op: encode_op(&op).unwrap(),
        route: Vec::new(),
    }
}

/// Ann's offline Host, the creator and inviter of `c1`, holding the first
/// `held` ops of her chain (rebuilt), and the whole chain.
async fn host_holding(dir: &tempfile::TempDir, held: usize) -> (std::sync::Arc<Host>, Vec<HeldOp>) {
    let host = offline_host(dir).await;
    let ann = create_ann(&host).await;
    seed_registry_community(&host, &ann.public_key).await;
    let creator = derive_community_pseudonym(&host.governance().identity_secret().unwrap(), "c1");
    let chain = creator_chain(&creator);
    let cert = seed_governance(&host, &ann.public_key, "c1", &creator, &chain[..held]).await;
    {
        let mut map = host.ctx.communities.map().write();
        let c1 = map.get_mut("c1").unwrap();
        c1.genesis_cert = Some(cert);
        c1.my_pseudonym_key = Some(hex::encode(creator.verifying_key().to_bytes()));
    }
    remerge(&host).await;
    (host, chain)
}

/// Rebuild `c1` from the op store and keep its merged state, as a rebuild
/// on login does.
async fn remerge(host: &Host) {
    let gov = host.governance();
    rebuild_ops(&gov, "c1").await.unwrap();
    let cert = gov.genesis_cert("c1").unwrap();
    let held = gov.governance_ops("c1").await.unwrap();
    let state = rekindle_governance::merge::merge(&cert, &verify_held("c1", &held).ops);
    gov.set_governance_state("c1", state);
}

async fn holds(host: &Host, joiner: &BootstrapJoiner) -> bool {
    let held = host.governance().governance_ops("c1").await.unwrap();
    held.iter()
        .any(|op| op.hash == joiner.member_joined.op_hash)
}

/// The invite is held, its creation is cited, but the op the join cites
/// is not held and not fetchable: retry later, nothing stored; once that
/// dep arrives, the retried request is admitted.
#[tokio::test]
async fn a_join_citing_an_unfetchable_dep_is_answered_retry_later_then_admitted() {
    let dir = tempfile::tempdir().unwrap();
    let (host, chain) = host_holding(&dir, 1).await;
    let gov = host.governance();
    let joiner = joiner_citing(&chain[1]);

    let answer = answer_bootstrap_request(&gov, "c1", &joiner).await;
    let Ok(BootstrapAnswer::RetryLater(bytes)) = answer else {
        panic!("a join we cannot judge is answered retry later, got {answer:?}");
    };
    assert_eq!(bytes, rekindle_types::message::CALL_RETRY_LATER);
    assert!(
        matches!(
            read_reply(&bytes, "c1"),
            Err(GovernanceRuntimeError::PeerRetryLater)
        ),
        "the joiner reads it as no refusal"
    );
    assert!(!holds(&host, &joiner).await, "nothing is stored");
    let state = gov.governance_state("c1").unwrap_or_default();
    assert!(!state.members.contains_key(&joiner.member_joined.author));

    // The dep arrives (gossip, the watch ring); the joiner retries.
    gov.store_governance_op("c1", &chain[1]).await.unwrap();
    remerge(&host).await;
    let answer = answer_bootstrap_request(&gov, "c1", &joiner).await;
    assert!(
        matches!(answer, Ok(BootstrapAnswer::Frontier(_))),
        "the retried join is admitted, got {answer:?}"
    );
    assert!(holds(&host, &joiner).await, "the admitted op is stored");
    let state = gov.governance_state("c1").unwrap();
    assert!(state.members.contains_key(&joiner.member_joined.author));
    host.shutdown().await.unwrap();
}

/// Every dep held and the invite revoked in the join's causal past: the
/// merge judges and refuses with its typed reason; nothing is stored.
#[tokio::test]
async fn a_join_with_every_dep_held_that_the_merge_refuses_is_refused_with_its_reason() {
    let dir = tempfile::tempdir().unwrap();
    let (host, chain) = host_holding(&dir, 3).await;
    let gov = host.governance();
    let joiner = joiner_citing(&chain[2]);

    let answer = answer_bootstrap_request(&gov, "c1", &joiner).await;
    let Ok(BootstrapAnswer::Refused(bytes)) = answer else {
        panic!("a judged join the merge does not admit is refused, got {answer:?}");
    };
    let reason = read_reply(&bytes, "c1");
    assert!(
        matches!(
            reason,
            Err(GovernanceRuntimeError::BootstrapRefused(r)) if r == JoinRefusal::InviteNoLongerValid
        ),
        "a revoked invite is refused as no longer valid, got {reason:?}"
    );
    assert!(!holds(&host, &joiner).await, "nothing is stored");
    host.shutdown().await.unwrap();
}
