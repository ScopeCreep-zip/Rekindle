//! The audit log exported to a file (plan E7.15 S3): `AuditExport` is
//! Protected (ADR 0019, `IntentKind::AuditExport`), so the Host holds it
//! for the user's review and writes nothing until the review commits with
//! the passphrase; then it writes the entries after `since` to the path the
//! frontend picked and answers the path and the entry count.

use rekindle_ipc::protocol::{IpcRequest, IpcResponse};
use rekindle_ipc::{Caller, HostApi};
use rekindle_types::dto::audit::{AuditEntryView, AuditExported};
use rekindle_types::intent::{IntentClass, IntentProof};

use super::{create_ann, offline_host};

fn export(path: &std::path::Path) -> IpcRequest {
    IpcRequest::AuditExport {
        since: 0,
        path: path.to_str().expect("a UTF-8 temp path").to_owned(),
    }
}

#[tokio::test]
async fn audit_export_is_reviewed_then_writes_the_log_to_the_picked_path() {
    let dir = tempfile::tempdir().unwrap();
    let host = offline_host(&dir).await;
    create_ann(&host).await;
    let out = dir.path().join("audit.json");

    // Held for review: nothing is written yet.
    let IpcResponse::IntentRequired(review) = host.call(Caller::shell(), export(&out)).await else {
        panic!("AuditExport was not held for review");
    };
    assert_eq!(review.class, IntentClass::Protected);
    assert!(
        !out.exists(),
        "nothing is written before the review commits"
    );

    // A wrong passphrase is refused and still writes nothing.
    let wrong = host
        .call(
            Caller::shell(),
            IpcRequest::IntentCommit {
                intent_id: review.intent_id,
                proof: IntentProof::Passphrase("not the passphrase".to_string().into()),
            },
        )
        .await;
    assert!(matches!(wrong, IpcResponse::Error { .. }), "{wrong:?}");
    assert!(!out.exists(), "a refused review writes nothing");

    // Reviewed with the passphrase: the file holds every entry.
    let IpcResponse::IntentRequired(review) = host.call(Caller::shell(), export(&out)).await else {
        panic!("AuditExport was not held for review");
    };
    let done = host
        .call(
            Caller::shell(),
            IpcRequest::IntentCommit {
                intent_id: review.intent_id,
                proof: IntentProof::Passphrase("pass".to_string().into()),
            },
        )
        .await;
    let IpcResponse::Ok { data, .. } = done else {
        panic!("the reviewed export failed: {done:?}");
    };
    let answered: AuditExported = serde_json::from_value(data).expect("an AuditExported");
    let written: Vec<AuditEntryView> =
        serde_json::from_slice(&std::fs::read(&out).expect("the export file")).unwrap();

    let db = host.ctx.require_db().expect("an open db");
    let owner = host.ctx.require_owner().expect("an unlocked owner");
    let stored = crate::host::audit::export(&db, &owner, 0).await.unwrap();
    assert!(!stored.is_empty(), "creating Ann logs entries");
    assert_eq!(
        answered,
        AuditExported {
            path: out.to_str().unwrap().to_owned(),
            entries: stored.len() as u64,
        }
    );
    assert_eq!(
        written.iter().map(|e| e.cursor).collect::<Vec<_>>(),
        stored.iter().map(|e| e.cursor).collect::<Vec<_>>(),
        "the file holds the stored entries in order"
    );
    assert_eq!(
        written.iter().map(|e| e.mac.clone()).collect::<Vec<_>>(),
        stored
            .iter()
            .map(|e| hex::encode(e.mac))
            .collect::<Vec<_>>(),
        "each entry keeps its MAC"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&out).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "the export is owner-only");
    }
}

#[tokio::test]
async fn audit_export_refuses_a_relative_path() {
    let dir = tempfile::tempdir().unwrap();
    let host = offline_host(&dir).await;
    create_ann(&host).await;
    let refused = host
        .call(
            Caller::shell(),
            IpcRequest::AuditExport {
                since: 0,
                path: "audit.json".into(),
            },
        )
        .await;
    assert!(
        matches!(refused, IpcResponse::Error { code: 400, .. }),
        "{refused:?}"
    );
}
