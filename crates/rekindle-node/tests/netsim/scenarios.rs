//! Plan C9.4's first scenarios. Each is a failing test until the step that
//! fixes it lands, marked `#[ignore = "<step>"]` until then; that step
//! un-ignores it.

use std::time::Duration;

use base64::Engine as _;
use rekindle_codec::capnp_envelope::encode_community_envelope;
use rekindle_codec::community::envelope::{CommunityEnvelope, ControlPayload, MekTransferPayload};
use rekindle_codec::governance_op::{encode_op, op_unsigned_bytes};
use rekindle_governance::op::{invite_proof_bytes, op_hash, op_signing_bytes};
use rekindle_governance_runtime::deps::GovernanceOpStore as _;
use rekindle_governance_runtime::deps::GovernanceRuntimeDeps;
use rekindle_ipc::protocol::{ChannelMessageRequest, MemberRequest};
use rekindle_ipc::protocol::{CommunityRequest, IpcRequest};
use rekindle_secrets::ed25519_dalek::{Signer, SigningKey};
use rekindle_secrets::member_record::{dflt_opaque_record_key, sign_member_record_binding};
use rekindle_types::config::SafetyProfile;
use rekindle_types::dht_layout::member_record;
use rekindle_types::governance::{GovernanceEntry, GovernanceOp, InviteProof, OpHash};
use rekindle_types::id::PseudonymKey;
use rekindle_types::permissions::{ADMINISTRATOR, KICK_MEMBERS};

use crate::harness::{eventually, Community};
use crate::moderation::listed;

/// `bytes` as a `VLD0:` key text, as Veilid prints a public or opaque key.
pub(crate) fn vld0(bytes: &[u8; 32]) -> String {
    format!(
        "VLD0:{}",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
    )
}

/// `op` signed by `key` over the anchor-bound signing bytes, encoded as the
/// op log and `governanceOp @74` carry it, with its hash.
pub(crate) fn signed(anchor: &str, key: &SigningKey, mut op: GovernanceOp) -> (Vec<u8>, OpHash) {
    let unsigned = op_unsigned_bytes(&op);
    op.sig = key.sign(&op_signing_bytes(anchor, &unsigned)).to_bytes();
    (
        encode_op(&op).expect("encodes"),
        op_hash(&unsigned, &op.sig),
    )
}

/// S4 (fixed in E3.2): an op at the lowest lamport never becomes the
/// creator. Before E3.2 a member holding the slot seed wrote a lamport-0
/// entry into a free governance slot and the merge's lowest-lamport genesis
/// branch made the forger the creator. Now the creator is the `GenesisCert`'s
/// (`rekindle-governance/src/merge`), and an op's lamport is derived,
/// `1 + max(deps)` (`rekindle-governance/src/op.rs` `verify_with`), so the
/// lowest an op can reach is a dep-less one: lamport 1, the creator's own
/// op #0's. Anyone holding an invite builds such an op, a `MemberJoined`
/// with a valid invite proof and owner proof (so it passes the receive
/// bound and lands in the op store), issued at time 0, and a dep-less
/// `CommunityMeta` renaming the community; both reach the members on the
/// `governanceOp @74` ingest (`daemon/handler/gossip_ingest.rs`). Every
/// member keeps the genesis author as creator and the name.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_lamport_zero_entry_never_becomes_the_creator() {
    let mut community = Community::create(2, 41).await;
    let creator = community.info(0).await["creator_pseudonyms"].clone();
    let name = community.info(0).await["name"].clone();
    community.join(1).await;
    community.invite().await;
    let link = rekindle_types::invite::InviteLink::parse(&community.link).expect("an invite link");
    let code_hash = rekindle_secrets::invite::hash_invite_code(link.invite_code.as_str());
    // Every member holds the invite, so the forgery passes the receive bound.
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
    let forger =
        rekindle_secrets::derive::derive_community_pseudonym(&[0xAB; 32], &community.anchor);
    let author = PseudonymKey(forger.verifying_key().to_bytes());
    let owner = SigningKey::from_bytes(&[0xEF; 32]);
    let owner_public = owner.verifying_key().to_bytes();
    let record = vld0(&dflt_opaque_record_key(
        &owner_public,
        member_record::SUBKEY_COUNT,
    ));
    let joined = GovernanceOp {
        device_key: author.0,
        author: author.clone(),
        seq: 0,
        prev: None,
        deps: Vec::new(),
        issued_at: 0,
        body: GovernanceEntry::MemberJoined {
            member_record: record.clone(),
            member_record_owner: vld0(&owner_public),
            invite_proof: Some(InviteProof {
                code_hash: code_hash.clone(),
                sig: rekindle_secrets::invite::sign_invite_join(
                    link.invite_code.as_str(),
                    &invite_proof_bytes(&anchor, &author, &record, &code_hash),
                )
                .to_vec(),
            }),
            owner_proof: sign_member_record_binding(&[0xEF; 32], &anchor, &author.0, &record)
                .to_vec(),
            first_device: None,
        },
        sig: [0; 64],
    };
    let (joined, joined_hash) = signed(&anchor, &forger, joined);
    let renamed = GovernanceOp {
        device_key: author.0,
        author: author.clone(),
        seq: 1,
        prev: Some(joined_hash),
        deps: Vec::new(),
        issued_at: 0,
        body: GovernanceEntry::CommunityMeta {
            name: Some("Taken".into()),
            description: None,
            icon_hash: None,
            banner_hash: None,
        },
        sig: [0; 64],
    };
    let (renamed, _) = signed(&anchor, &forger, renamed);
    for node in &community.nodes {
        let governance = node.host.governance();
        governance.on_governance_op(&anchor, joined.clone()).await;
        governance.on_governance_op(&anchor, renamed.clone()).await;
    }
    community.sim.quiesce().await;

    for (i, node) in community.nodes.iter().enumerate() {
        let held = node
            .host
            .governance()
            .governance_ops(&anchor)
            .await
            .unwrap();
        assert!(
            held.iter().any(|op| op.hash == joined_hash),
            "node {i}'s op store holds the forged op, so the merge reads it"
        );
        let view = community.info(i).await;
        assert_eq!(
            view["creator_pseudonyms"], creator,
            "node {i}: the creator stays the genesis author"
        );
        assert_eq!(view["name"], name, "node {i}: the name is unchanged");
    }
}

/// The strong-removal chain (fixed in E3.2; plan E3.2 S3 decision (a)).
/// Under the hierarchy (target rank < writer rank) a mutual kick of two
/// managers is refused on both sides, so the Verify item is the chain: the
/// creator A removes the moderator B concurrently with B removing the
/// member C. B's removal of C is invalidated (p2panda-auth transitive
/// invalidation), so on every remaining node B is gone and C stays, and
/// C's later post is read, while B's is not: every reader's ingest gate
/// drops a departed author's post (`GateReason::NotMember`, register FX1).
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_removal_concurrent_with_the_removers_own_is_void_everywhere() {
    let community = Community::of(3, 42).await;
    let (bea, cal) = (community.pseudonym(1).await, community.pseudonym(2).await);
    let role = community.nodes[0]
        .ok(IpcRequest::Community(CommunityRequest::Members(
            MemberRequest::RoleCreate {
                community: community.anchor.clone(),
                name: "mod".into(),
                permissions: KICK_MEMBERS,
                color: 0,
                hoist: false,
                mentionable: false,
                self_assignable: false,
                exclusion_group: None,
            },
        )))
        .await;
    let role_id = role.as_str().expect("a role id").to_owned();
    community.nodes[0]
        .ok(IpcRequest::Community(CommunityRequest::Members(
            MemberRequest::RoleAssign {
                community: community.anchor.clone(),
                member_pseudonym: bea.clone(),
                role_id: role_id.clone(),
            },
        )))
        .await;
    assert!(
        community.holds_role(1, &role_id).await,
        "the moderator learns its role"
    );
    let limit = Duration::from_secs(120);
    assert!(
        eventually(&community.sim, limit, || async {
            listed(&community, 1, &cal).await.is_some()
        })
        .await,
        "the moderator lists the member it removes"
    );

    let kick = |by: usize, target: &str| {
        community.nodes[by].call(IpcRequest::Community(CommunityRequest::Members(
            MemberRequest::Kick {
                community: community.anchor.clone(),
                target_pseudonym: target.to_string(),
                reason: None,
            },
        )))
    };
    let (by_ann, by_bea) = tokio::join!(kick(0, &bea), kick(1, &cal));
    assert!(by_ann.is_ok() && by_bea.is_ok(), "{by_ann:?} / {by_bea:?}");

    for reader in [0, 2] {
        assert!(
            eventually(&community.sim, limit, || async {
                listed(&community, reader, &bea).await.is_none()
                    && listed(&community, reader, &cal).await.is_some()
            })
            .await,
            "node {reader}: the moderator is removed and its removal of the member is void"
        );
    }

    let _ = community.post(1, "bea after the kicks").await;
    community
        .post(2, "cal after the kicks")
        .await
        .expect("the member still posts");
    assert!(community.reads(0, "cal after the kicks", limit).await);
    for reader in [0, 2] {
        assert!(!community.reads(reader, "bea after the kicks", limit).await);
    }
}

/// Two concurrent manual rotations produce one generation (fixed in E3.5):
/// after both, every node reads every later post.
#[tokio::test(flavor = "current_thread", start_paused = true)]
#[ignore = "E3.5"]
async fn concurrent_rotations_converge_on_one_generation() {
    let community = Community::of(3, 43).await;
    let bea = community.pseudonym(1).await;
    let role = community.nodes[0]
        .ok(IpcRequest::Community(CommunityRequest::Members(
            MemberRequest::RoleCreate {
                community: community.anchor.clone(),
                name: "admin".into(),
                permissions: ADMINISTRATOR,
                color: 0,
                hoist: false,
                mentionable: false,
                self_assignable: false,
                exclusion_group: None,
            },
        )))
        .await;
    let role_id = role.as_str().expect("a role id").to_owned();
    community.nodes[0]
        .ok(IpcRequest::Community(CommunityRequest::Members(
            MemberRequest::RoleAssign {
                community: community.anchor.clone(),
                member_pseudonym: bea,
                role_id: role_id.clone(),
            },
        )))
        .await;
    assert!(
        community.holds_role(1, &role_id).await,
        "the administrator learns its role"
    );
    let general = community.info(0).await["channels"][0]["id"]
        .as_str()
        .expect("the general channel's id")
        .to_string();

    let rotate = |by: usize| {
        community.nodes[by].call(IpcRequest::Community(CommunityRequest::Messages(
            ChannelMessageRequest::MekRotate {
                community: community.anchor.clone(),
                channel: general.clone(),
            },
        )))
    };
    let (a, b) = tokio::join!(rotate(0), rotate(1));
    assert!(a.is_ok() && b.is_ok(), "{a:?} / {b:?}");
    community.sim.quiesce().await;

    let generations: Vec<String> = {
        let mut out = Vec::new();
        for node in &community.nodes {
            out.push(
                node.ok(IpcRequest::Community(CommunityRequest::Messages(
                    ChannelMessageRequest::MekList {
                        community: community.anchor.clone(),
                    },
                )))
                .await
                .to_string(),
            );
        }
        out
    };
    assert!(
        generations.windows(2).all(|w| w[0] == w[1]),
        "one generation everywhere: {generations:?}"
    );
    for (i, body) in ["zero after", "one after", "two after"].iter().enumerate() {
        community.post(i, body).await.expect("posted");
    }
    for reader in 0..3 {
        for body in ["zero after", "one after", "two after"] {
            assert!(
                community.reads(reader, body, Duration::from_secs(60)).await,
                "node {reader} reads {body:?}"
            );
        }
    }
}

/// S7 (fixed in E3.5): anyone who reaches a member can inject a community
/// key. The wrap is unsigned and unwrapped against whatever sender the
/// payload names (`rekindle-mek-rotation/src/receive.rs`), so an outsider
/// with its own key and a member's route installs a newer generation.
/// E3.5 signs the wrap and binds it to a ballot.
#[tokio::test(flavor = "current_thread", start_paused = true)]
#[ignore = "E3.5"]
async fn an_outsider_cannot_inject_a_community_key() {
    let community = Community::of(2, 44).await;
    let before = community.nodes[0]
        .ok(IpcRequest::Community(CommunityRequest::Messages(
            ChannelMessageRequest::MekList {
                community: community.anchor.clone(),
            },
        )))
        .await;
    let victim = PseudonymKey::from_hex_lossy(&community.pseudonym(0).await);

    let outsider = community.sim.add_node();
    community.sim.set_ready(outsider.index, true);
    let blob = community
        .sim
        .route_blobs(0)
        .into_iter()
        .next()
        .expect("the victim has a route");
    let target = outsider.ports.routes.import(&blob).unwrap();
    let forger = rekindle_secrets::derive::derive_community_pseudonym(&[0xCD; 32], "outsider");
    let key = rekindle_crypto::group::media_key::MediaEncryptionKey::generate(99);
    let wrapped = rekindle_crypto::group::mek_distribution::wrap_mek(
        &forger,
        &victim.0,
        &key.to_wire_bytes(),
    )
    .unwrap();
    let transfer = encode_community_envelope(&CommunityEnvelope::control(
        ControlPayload::MekTransfer(MekTransferPayload {
            community_id: community.anchor.clone(),
            channel_id: None,
            generation: 99,
            sender_pseudonym: hex::encode(forger.verifying_key().to_bytes()),
            wrapped_mek: wrapped,
            transfer_id: vec![1; 16],
        }),
    ))
    .unwrap();
    let _ = outsider
        .ports
        .messages
        .call(&target, &SafetyProfile::default_dht(), transfer)
        .await;
    community.sim.quiesce().await;

    let after = community.nodes[0]
        .ok(IpcRequest::Community(CommunityRequest::Messages(
            ChannelMessageRequest::MekList {
                community: community.anchor.clone(),
            },
        )))
        .await;
    assert_eq!(after, before, "the forged key is refused");
}

/// The trace is the seed's (C9.3): one seed, one trace; another seed,
/// another trace.
#[test]
fn one_seed_one_trace() {
    fn run(seed: u64) -> [u8; 32] {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .start_paused(true)
            .build()
            .unwrap();
        runtime.block_on(async {
            let community = Community::of(2, seed).await;
            community.post(0, "traced").await.expect("posted");
            assert!(community.reads(1, "traced", Duration::from_secs(60)).await);
            community.sim.trace().hash()
        })
    }
    assert_eq!(run(5), run(5));
    assert_ne!(run(5), run(6));
}
