//! A community-join invite through the gate (plan E3.1 D2): held for review
//! with the record-key fingerprint as its subject, never run unreviewed,
//! and refused, not held, before an identity is unlocked (the shell keeps
//! an OS link as raw input until the unlock and submits it then).

use rekindle_ipc::protocol::{CommunityRequest, IpcRequest, IpcResponse};
use rekindle_ipc::{Caller, HostApi};
use rekindle_types::intent::{IntentClass, IntentKind, Subject};

use crate::host::fixture::temp_root;
use crate::host::{claim, identity, Host, HostConfig, HostStores, TransportSource};

const KEY: &str = "VLD0:um7m8HxBluv6XceSaB3dK9Lq0ZpWtYv1NcRe4Gh7Jf2";

fn join() -> IpcRequest {
    IpcRequest::Community(CommunityRequest::CommunityJoin {
        invite: format!("rekindle://invite/{KEY}/{KEY}/3f1a9c0b7e2d4f6a8b1c3e5d7f9a0b2c"),
    })
}

async fn locked_host(dir: &tempfile::TempDir) -> (std::sync::Arc<Host>, String) {
    let host = Host::start(HostConfig {
        root: claim(temp_root(dir)).unwrap(),
        stores: HostStores::default(),
        transport: TransportSource::Offline,
        serve_bus: false,
        clock: std::sync::Arc::new(rekindle_utils::SystemClock),
        rng: std::sync::Arc::new(rekindle_utils::OsRandom),
        idle: std::sync::Arc::new(rekindle_idle::OsIdle),
        shell: None,
    })
    .await
    .unwrap();
    let created = identity::create(host.root(), &host.identity_stores(), Some("Ann"), "pass", 1)
        .await
        .unwrap();
    let public_key = created.public_key.clone();
    drop(created);
    identity::lock(&host.identity_stores()).await;
    (host, public_key)
}

/// From the window, as the shell submits an OS link: held as a
/// TrustChanging `JoinCommunity` whose subject is the fingerprint of the
/// invite's community record key.
#[tokio::test]
async fn a_join_link_is_held_with_its_record_key_fingerprint() {
    let dir = tempfile::tempdir().unwrap();
    let (host, ann) = locked_host(&dir).await;
    let unlocked = host
        .call(
            Caller::shell(),
            IpcRequest::Unlock {
                identity: ann,
                passphrase: "pass".into(),
            },
        )
        .await;
    assert!(
        !matches!(unlocked, IpcResponse::Error { .. }),
        "{unlocked:?}"
    );
    let IpcResponse::IntentRequired(review) = host.call(Caller::shell(), join()).await else {
        panic!("a join link is held for review");
    };
    assert_eq!(review.kind, IntentKind::JoinCommunity);
    assert_eq!(review.class, IntentClass::TrustChanging);
    let fingerprint = rekindle_crypto::identity::record_key_fingerprint(KEY).unwrap();
    assert_eq!(review.subject, Subject::Named { name: fingerprint });
}

/// Before an unlock there is no one to review it: the Host refuses the
/// join rather than holding it, so the shell holds the raw link instead.
#[tokio::test]
async fn a_join_link_before_unlock_is_not_held_by_the_host() {
    let dir = tempfile::tempdir().unwrap();
    let (host, _) = locked_host(&dir).await;
    let answer = host.call(Caller::shell(), join()).await;
    assert!(matches!(answer, IpcResponse::Error { .. }), "{answer:?}");
}
