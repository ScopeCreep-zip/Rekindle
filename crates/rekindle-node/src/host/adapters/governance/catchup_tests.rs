//! A removed member cannot open a history ad sealed after the rotation
//! (plan E3.2 S10 Verify (3); ADR 0018; `catchup.rs` `peer_ad`). The
//! departure's rotation mints the next generation with
//! `MediaEncryptionKey::generate_with` (`rekindle-mek-rotation/src/rotate.rs`)
//! and wraps it to the remaining members only; the removed member is left
//! holding the replaced key. A remaining member's presence row then carries
//! its history ad sealed under the new key, as the presence writer seals
//! it (`encrypt_history_ranges_with_current_mek`: the JSON ranges under the
//! current community key). The removed member's catch-up reads no ranges
//! from it, whatever generation the row names; a member holding the new
//! key reads them.

use std::sync::Arc;

use rekindle_secrets::ed25519_dalek::{Signer, SigningKey};
use rekindle_secrets::keys::MediaEncryptionKey;
use rekindle_types::channel_keys::KeyScope;
use rekindle_types::id::{ChannelId, PseudonymKey};
use rekindle_types::presence::{EncryptedHistoryRanges, HistoryRange, MemberPresence};
use rekindle_utils::random::RandomSource;

use super::HostGovernance;
use crate::host::fixture::temp_root;
use crate::host::{claim, Host, HostConfig, HostStores, TransportSource};

const COMMUNITY: &str = "c1";

async fn offline_host(dir: &tempfile::TempDir) -> Arc<Host> {
    Host::start(HostConfig {
        root: claim(temp_root(dir)).unwrap(),
        stores: HostStores::default(),
        transport: TransportSource::Offline,
        serve_bus: false,
        clock: Arc::new(rekindle_utils::SystemClock),
        rng: Arc::new(rekindle_utils::OsRandom),
        idle: Arc::new(rekindle_idle::OsIdle),
        shell: None,
    })
    .await
    .unwrap()
}

fn ranges() -> Vec<HistoryRange> {
    vec![HistoryRange {
        channel_id: ChannelId([3; 16]),
        oldest_lamport: 1,
        newest_lamport: 40,
    }]
}

/// A remaining member's signed presence row whose history ad is `ranges`
/// sealed under `key` and labelled `generation`.
fn row(writer: &SigningKey, key: &MediaEncryptionKey, generation: u64) -> Vec<u8> {
    let mut presence = MemberPresence {
        pseudonym_key: PseudonymKey(writer.verifying_key().to_bytes()),
        route_blob: vec![9; 32],
        history_ranges_encrypted: Some(EncryptedHistoryRanges {
            mek_generation: generation,
            ciphertext: key
                .encrypt(&serde_json::to_vec(&ranges()).unwrap())
                .unwrap(),
        }),
        ..MemberPresence::default()
    };
    presence.signature = writer.sign(&presence.signing_bytes()).to_bytes().to_vec();
    serde_json::to_vec(&presence).unwrap()
}

#[tokio::test]
async fn a_removed_member_cannot_open_a_history_ad_sealed_after_the_rotation() {
    let dir = tempfile::tempdir().unwrap();
    let host = offline_host(&dir).await;
    let governance = HostGovernance::new(Arc::clone(&host.ctx));
    let rng: Arc<dyn RandomSource> = Arc::new(rekindle_utils::OsRandom);
    let replaced = MediaEncryptionKey::generate_with(6, rng.as_ref());
    // The departure's rotation: the next generation, minted as the rotator
    // mints it.
    let rotated = MediaEncryptionKey::generate_with(replaced.generation() + 1, rng.as_ref());
    let writer = SigningKey::from_bytes(&[0x42; 32]);
    let sealed_after = row(&writer, &rotated, rotated.generation());
    // The same ciphertext labelled with the generation the removed member
    // holds: the label alone must not open it.
    let relabelled = row(&writer, &rotated, replaced.generation());

    // The removed member: it holds the replaced key only.
    host.ctx
        .mek_cache
        .write()
        .insert(COMMUNITY, KeyScope::Community, replaced.clone());
    assert!(
        governance.peer_ad(COMMUNITY, &sealed_after).is_none(),
        "the removed member reads no ranges from an ad sealed after the rotation"
    );
    assert!(
        governance.peer_ad(COMMUNITY, &relabelled).is_none(),
        "nor from the same ad labelled with the replaced generation"
    );
    // Its own pre-rotation reads still work: the opener is not refusing
    // everything.
    assert!(
        governance
            .peer_ad(COMMUNITY, &row(&writer, &replaced, replaced.generation()))
            .is_some(),
        "an ad sealed under the key it holds opens"
    );

    // A remaining member, holding the rotated key, reads the same row.
    host.ctx
        .mek_cache
        .write()
        .insert(COMMUNITY, KeyScope::Community, rotated);
    let (pseudonym, _, ads) = governance
        .peer_ad(COMMUNITY, &sealed_after)
        .expect("a remaining member opens the ad");
    assert_eq!(pseudonym, hex::encode(writer.verifying_key().to_bytes()));
    assert_eq!(ads.len(), 1);
    assert_eq!(ads[0].newest_lamport, 40);
    host.shutdown().await.unwrap();
}
