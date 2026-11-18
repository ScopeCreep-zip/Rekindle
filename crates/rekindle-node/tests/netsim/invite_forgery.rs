//! Plan E3.2 S10 (found, security): the invite proof proves the invite
//! code. A party that read the op log but never held the code (a kicked
//! member, an I16 outsider) knows the invite's public `code_hash` and the
//! `InviteCreated` op, nothing more. Its `MemberJoined` citing that live
//! invite, with the best proof it can make (its own pseudonym's signature
//! over the proof bytes), reaches every member on the `governanceOp @74`
//! ingest and is admitted by none: the proof must be the invite join key's,
//! derived from the code (`rekindle_secrets::invite::sign_invite_join`).

use std::time::Duration;

use rekindle_codec::governance_op::decode_op;
use rekindle_governance::op::invite_proof_bytes;
use rekindle_governance_runtime::deps::GovernanceRuntimeDeps as _;
use rekindle_secrets::ed25519_dalek::{Signer, SigningKey};
use rekindle_secrets::member_record::{dflt_opaque_record_key, sign_member_record_binding};
use rekindle_types::dht_layout::member_record;
use rekindle_types::governance::{GovernanceEntry, GovernanceOp, InviteProof, OpRef};
use rekindle_types::id::PseudonymKey;

use crate::harness::{eventually, Community};
use crate::scenarios::{signed, vld0};

/// The `OpRef` of the `InviteCreated` with `code_hash` in node 0's store.
async fn invite_ref(community: &Community, code_hash: &str) -> OpRef {
    let held = (community.nodes[0].host.governance())
        .governance_ops(&community.anchor)
        .await
        .expect("the op store");
    held.iter()
        .find(|h| {
            decode_op(&h.encoded).is_ok_and(|d| {
                matches!(&d.op.body, GovernanceEntry::InviteCreated { code_hash: c, .. } if c == code_hash)
            })
        })
        .map(|h| OpRef {
            op_hash: h.hash,
            author: h.author.clone(),
            author_record: h.author_record.clone(),
            author_seq: h.seq,
        })
        .expect("the invite op is held")
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn an_op_log_reader_without_the_code_is_admitted_nowhere() {
    let mut community = Community::of(2, 237).await;
    community.invite().await;
    let link = rekindle_types::invite::InviteLink::parse(&community.link).expect("an invite link");
    // All the forger takes from the invite is what the op log shows.
    let code_hash = rekindle_secrets::invite::hash_invite_code(link.invite_code.as_str());
    assert!(
        eventually(&community.sim, Duration::from_secs(120), || async {
            community.nodes.iter().all(|node| {
                (node.host.governance().governance_state(&community.anchor))
                    .is_some_and(|state| state.invites.values().any(|i| i.code_hash == code_hash))
            })
        })
        .await,
        "every member merged the invite"
    );
    let anchor = community.anchor.clone();
    let invite = invite_ref(&community, &code_hash).await;

    let forger = rekindle_secrets::derive::derive_community_pseudonym(&[0xC3; 32], &anchor);
    let author = PseudonymKey(forger.verifying_key().to_bytes());
    let owner = SigningKey::from_bytes(&[0xC4; 32])
        .verifying_key()
        .to_bytes();
    let record = vld0(&dflt_opaque_record_key(&owner, member_record::SUBKEY_COUNT));
    let proof = invite_proof_bytes(&anchor, &author, &record, &code_hash);
    let joined = GovernanceOp {
        author: author.clone(),
        seq: 0,
        prev: None,
        deps: vec![invite],
        issued_at: 0,
        body: GovernanceEntry::MemberJoined {
            member_record: record.clone(),
            member_record_owner: vld0(&owner),
            invite_proof: Some(InviteProof {
                code_hash: code_hash.clone(),
                sig: forger.sign(&proof).to_bytes().to_vec(),
            }),
            owner_proof: sign_member_record_binding(&[0xC4; 32], &anchor, &author.0, &record)
                .to_vec(),
        },
        sig: [0; 64],
    };
    let (joined, _) = signed(&anchor, &forger, joined);
    for node in &community.nodes {
        (node.host.governance())
            .on_governance_op(&anchor, joined.clone())
            .await;
    }
    community.sim.quiesce().await;

    for (i, node) in community.nodes.iter().enumerate() {
        let state = (node.host.governance().governance_state(&anchor)).expect("a state");
        assert!(
            !state.members.contains_key(&author),
            "node {i} admitted a join without the invite code"
        );
    }
}
