//! Plan C9.4's first scenarios. Each is a failing test until the step that
//! fixes it lands, marked `#[ignore = "<step>"]` until then; that step
//! un-ignores it.

use std::time::Duration;

use base64::Engine as _;
use rekindle_codec::capnp_envelope::encode_community_envelope;
use rekindle_codec::community::envelope::{CommunityEnvelope, ControlPayload, MekTransferPayload};
use rekindle_ipc::protocol::{ChannelMessageRequest, MemberRequest};
use rekindle_ipc::protocol::{CommunityRequest, IpcRequest};
use rekindle_types::config::SafetyProfile;
use rekindle_types::governance::GovernanceEntry;
use rekindle_types::id::PseudonymKey;
use rekindle_types::permissions::{ADMINISTRATOR, KICK_MEMBERS};

use crate::harness::{eventually, Community};
use crate::moderation::listed;

/// A member's slot seed, as anyone holding an invite reads it: the link's
/// code decrypts the secrets record.
fn slot_seed_from_invite(community: &Community) -> [u8; 32] {
    let link = rekindle_types::invite::InviteLink::parse(&community.link).expect("an invite link");
    let stored = community
        .sim
        .network_get(link.secrets_record_key.as_str(), 0)
        .expect("the invite secrets are published");
    let encrypted = base64::engine::general_purpose::STANDARD
        .decode(String::from_utf8(stored.data).expect("base64 text"))
        .expect("base64");
    let secrets: rekindle_types::invite::InviteSecrets = serde_json::from_slice(
        &rekindle_secrets::invite::decrypt_invite_secrets(link.invite_code.as_str(), &encrypted)
            .expect("the code opens the secrets"),
    )
    .expect("invite secrets");
    hex::decode(secrets.slot_seed)
        .expect("hex")
        .try_into()
        .expect("32 bytes")
}

/// S4 (fixed in E3.2): a member holding the slot seed writes, into a free
/// slot of the governance record, a validly signed entry at lamport 0.
/// Genesis entries start at lamport 1, so the merge's lowest-lamport
/// genesis branch (`rekindle-governance/src/merge/mod.rs:85-90`) makes the
/// forger the creator. E3.2 pins the creator by the `GenesisCert`.
#[tokio::test(flavor = "current_thread", start_paused = true)]
#[ignore = "E3.2"]
async fn a_lamport_zero_entry_never_becomes_the_creator() {
    let mut community = Community::create(2, 41).await;
    let creator = community.info(0).await["owner_pseudonym"].clone();
    community.invite().await;

    let seed = slot_seed_from_invite(&community);
    let free_slot = 254;
    let writer = rekindle_crypto::keypair_text::slot_keypair_text(&seed, free_slot).unwrap();
    let forger = rekindle_secrets::derive::derive_community_pseudonym(
        &[0xAB; 32],
        &community.governance_key,
    );
    let author = PseudonymKey(forger.verifying_key().to_bytes());
    let (_, page) = rekindle_governance_runtime::overflow::build_signed_payload(
        &[GovernanceEntry::CommunityMeta {
            name: Some("Taken".into()),
            description: None,
            icon_hash: None,
            banner_hash: None,
        }],
        None,
        &forger,
        &author,
    )
    .unwrap();
    community
        .sim
        .forge_write(&community.governance_key, free_slot, page, &writer)
        .unwrap();

    // A joiner merges the governance record from the DHT, forged page included.
    community.join(1).await;
    let joined_view = community.info(1).await;
    assert_eq!(
        joined_view["owner_pseudonym"], creator,
        "the creator stays the genesis author"
    );
    assert_ne!(joined_view["name"], "Taken");
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
                community: community.governance_key.clone(),
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
    let role_id = u32::try_from(role.as_u64().expect("a role id")).unwrap();
    community.nodes[0]
        .ok(IpcRequest::Community(CommunityRequest::Members(
            MemberRequest::RoleAssign {
                community: community.governance_key.clone(),
                member_pseudonym: bea.clone(),
                role_id,
            },
        )))
        .await;
    assert!(
        community.holds_role(1, role_id).await,
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
                community: community.governance_key.clone(),
                target_pseudonym: target.to_string(),
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
                community: community.governance_key.clone(),
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
    let role_id = u32::try_from(role.as_u64().expect("a role id")).unwrap();
    community.nodes[0]
        .ok(IpcRequest::Community(CommunityRequest::Members(
            MemberRequest::RoleAssign {
                community: community.governance_key.clone(),
                member_pseudonym: bea,
                role_id,
            },
        )))
        .await;
    assert!(
        community.holds_role(1, role_id).await,
        "the administrator learns its role"
    );
    let general = community.info(0).await["channels"][0]["id"]
        .as_str()
        .expect("the general channel's id")
        .to_string();

    let rotate = |by: usize| {
        community.nodes[by].call(IpcRequest::Community(CommunityRequest::Messages(
            ChannelMessageRequest::MekRotate {
                community: community.governance_key.clone(),
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
                        community: community.governance_key.clone(),
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
                community: community.governance_key.clone(),
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
    let transfer = encode_community_envelope(&CommunityEnvelope::Control(
        ControlPayload::MekTransfer(MekTransferPayload {
            community_id: community.governance_key.clone(),
            channel_id: None,
            generation: 99,
            sender_pseudonym: hex::encode(forger.verifying_key().to_bytes()),
            wrapped_mek: wrapped,
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
                community: community.governance_key.clone(),
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
