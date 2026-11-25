//! Plan E3.2 S7d-keys K5 (ADR 0011, ADR 0022 rule 4): an identity exported
//! from its device and imported on a fresh one. Person `p` (node 1) is a
//! member with history; node 2 imports `p`'s archive (`IdentityImport`:
//! the identity vault only, an empty database and no device vault) and
//! unlocks it. Its unlock stage finds the community in the vault, walks it
//! from the anchor alone, records the membership from the walked state and
//! enrols a device key of its own as its first write; then it writes an op
//! the other members see as `p`'s.

use std::time::Duration;

use rekindle_ipc::protocol::IpcRequest;

use crate::devices::{device_status, lists, sees_role, self_assign, self_assignable_role, unlock};
use crate::harness::{eventually, Community, PASSPHRASE};

const P: usize = 1;
const FRESH: usize = 2;

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn an_imported_identity_catches_up_its_community_and_enrols() {
    let mut community = Community::create(3, 2213).await;
    community.join(P).await;
    let p = community.pseudonym(P).await;
    let founder = community.pseudonym(0).await;
    let role = self_assignable_role(&community, "history").await;
    tokio::time::sleep(Duration::from_secs(60)).await;

    let exported = community.nodes[P]
        .call_held(IpcRequest::IdentityExport {
            passphrase: PASSPHRASE.into(),
        })
        .await
        .expect("p's identity exports");
    let archive = exported["archive"].as_str().expect("an archive").to_owned();
    let p_key = community.nodes[P].public_key.clone().expect("p's identity");
    community.nodes[FRESH].ok(IpcRequest::Lock).await;
    community.nodes[FRESH]
        .ok(IpcRequest::IdentityImport {
            archive,
            passphrase: PASSPHRASE.into(),
        })
        .await;
    community.nodes[FRESH].public_key = Some(p_key);
    unlock(&community, FRESH).await;

    let caught_up = eventually(&community.sim, Duration::from_secs(300), || async {
        community.nodes[FRESH]
            .call(IpcRequest::Community(
                rekindle_ipc::protocol::CommunityRequest::Lifecycle(
                    rekindle_ipc::protocol::LifecycleRequest::CommunityInfo {
                        anchor: community.anchor.clone(),
                    },
                ),
            ))
            .await
            .is_ok_and(|info| info["our_pseudonym"].as_str() == Some(p.as_str()))
    })
    .await;
    assert!(caught_up, "the fresh device holds the community as p");
    assert!(
        lists(&community, FRESH, &founder, true).await,
        "the walked roster holds the founder"
    );
    assert!(
        device_status(&community, FRESH, "enrolled").await,
        "the fresh device enrolled its own key"
    );
    self_assign(&community, FRESH, &role)
        .await
        .expect("the fresh device writes, on the walked role");
    assert!(
        sees_role(&community, 0, &p, &role).await,
        "another member sees the fresh device's op as p's"
    );
}
