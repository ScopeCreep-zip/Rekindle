//! The clearance refusal names its real reason (plan D1, live R5-4): a
//! client's clearance lasts from unlock to lock, so a refused client finds
//! either no identity unlocked or a key the identity does not trust.

use rekindle_ipc::message::SecurityLevel;
use rekindle_ipc::protocol::{IpcRequest, IpcResponse};
use rekindle_ipc::{Caller, ConnId, HostApi};

use crate::host::gate::tests::host_with_identity as locked_host;
use crate::host::Host;

/// A bus client with `key`, at the clearance the registry gives it.
async fn client(host: &Host, conn: u64, key: [u8; 32]) -> Caller {
    let clearance = host
        .ctx
        .registry
        .read()
        .await
        .lookup(&key)
        .map_or(SecurityLevel::Open, |found| found.security_level);
    Caller {
        conn: ConnId::Bus(conn),
        clearance,
        verified_name: None,
        static_key: Some(key),
    }
}

fn refusal(answer: IpcResponse) -> (u32, String, String) {
    match answer {
        IpcResponse::Error {
            code,
            message,
            remediation,
        } => (code, message, remediation.unwrap_or_default()),
        other => panic!("expected a refusal, got {other:?}"),
    }
}

#[tokio::test]
async fn a_client_refused_on_a_locked_daemon_is_told_no_identity_is_unlocked() {
    let dir = tempfile::tempdir().unwrap();
    let (host, ann) = locked_host(&dir).await;
    let cli = [0x31; 32];

    // A daemon starts locked: the refusal says so, not that the client
    // lacks trust.
    let (code, message, remediation) = refusal(
        host.call(client(&host, 1, cli).await, IpcRequest::FriendList)
            .await,
    );
    assert_eq!(code, 403);
    assert!(message.contains("this daemon holds none"), "{message}");
    assert!(remediation.contains("rekindle unlock"), "{remediation}");

    // The unlock registers the unlocking client (this offline host takes
    // its unlock from the shell, so the client is registered as a bus
    // unlock registers it), and it keeps its clearance while the identity
    // stays unlocked.
    let unlock = IpcRequest::Unlock {
        identity: ann,
        passphrase: rekindle_test_fixtures::PASSPHRASE.into(),
    };
    let answer = host.call(Caller::shell(), unlock).await;
    assert!(!matches!(answer, IpcResponse::Error { .. }), "{answer:?}");
    crate::daemon::clients::register_unlocking_client(&host.ctx, &cli)
        .await
        .unwrap();
    let raised = client(&host, 2, cli).await;
    assert_eq!(raised.clearance, SecurityLevel::Admin);
    let answer = host.call(raised, IpcRequest::FriendList).await;
    assert!(!matches!(answer, IpcResponse::Error { .. }), "{answer:?}");

    // Another key on the unlocked identity is untrusted, and told that.
    let (code, message, _) = refusal(
        host.call(client(&host, 3, [0x32; 32]).await, IpcRequest::FriendList)
            .await,
    );
    assert_eq!(code, 403);
    assert!(
        message.contains("does not trust this client's key"),
        "{message}"
    );

    // A lock (as a restart does) ends every client's clearance; the
    // refusal names the lock again.
    let answer = host.call(Caller::shell(), IpcRequest::Lock).await;
    assert!(!matches!(answer, IpcResponse::Error { .. }), "{answer:?}");
    let lowered = client(&host, 4, cli).await;
    assert_eq!(lowered.clearance, SecurityLevel::Open);
    let (_, message, _) = refusal(host.call(lowered, IpcRequest::FriendList).await);
    assert!(message.contains("this daemon holds none"), "{message}");
}
