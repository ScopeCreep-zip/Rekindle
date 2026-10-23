//! Attachment upload limits through the Host request (plan E3.1 E3d): the
//! file an `AttachmentUpload` names is refused before the Host reads it
//! unless it is an absolute path to a regular file under the attachment
//! cap, and a refused upload caches nothing.

use rekindle_files::MAX_FILE_SIZE_BYTES;
use rekindle_ipc::message::SecurityLevel;
use rekindle_ipc::protocol::{CommunityRequest, IpcRequest, IpcResponse, PinRequest};
use rekindle_ipc::{Caller, ConnId, HostApi};

use super::{create_ann, offline_host, seed_registry_community};

fn shell() -> Caller {
    Caller {
        conn: ConnId::Bus(1),
        clearance: SecurityLevel::Admin,
        verified_name: None,
        static_key: None,
    }
}

fn upload(path: &std::path::Path) -> IpcRequest {
    IpcRequest::Community(CommunityRequest::Pins(PinRequest::AttachmentUpload {
        community: "c1".into(),
        channel_id: "general".into(),
        path: path.display().to_string(),
    }))
}

/// The refusal's message, which must be a 400.
fn refusal(answer: IpcResponse) -> String {
    match answer {
        IpcResponse::Error {
            code: 400, message, ..
        } => message,
        other => panic!("expected a 400 refusal, got {other:?}"),
    }
}

#[tokio::test]
async fn an_upload_over_the_cap_or_of_another_kind_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let host = offline_host(&dir).await;
    let owner = create_ann(&host).await.public_key;
    seed_registry_community(&host, &owner).await;
    let files = tempfile::tempdir().unwrap();

    // One byte over the cap, sparse so nothing is written.
    let big = files.path().join("big.bin");
    std::fs::File::create(&big)
        .unwrap()
        .set_len(MAX_FILE_SIZE_BYTES + 1)
        .unwrap();
    let message = refusal(host.call(shell(), upload(&big)).await);
    assert!(message.contains("cap"), "{message}");

    // A folder is not a file.
    let message = refusal(host.call(shell(), upload(files.path())).await);
    assert!(message.contains("not a regular file"), "{message}");

    // A link is not followed, even to a file under the cap.
    #[cfg(unix)]
    {
        let small = files.path().join("small.txt");
        std::fs::write(&small, b"hello").unwrap();
        let link = files.path().join("link.txt");
        std::os::unix::fs::symlink(&small, &link).unwrap();
        let message = refusal(host.call(shell(), upload(&link)).await);
        assert!(message.contains("not a regular file"), "{message}");
    }

    // A relative path and a missing file.
    let message = refusal(
        host.call(shell(), upload(std::path::Path::new("small.txt")))
            .await,
    );
    assert!(message.contains("absolute"), "{message}");
    let message = refusal(host.call(shell(), upload(&files.path().join("gone"))).await);
    assert!(message.contains("no readable file"), "{message}");

    assert!(host.ctx.files.holder_of(uuid::Uuid::nil()).is_none());
    host.shutdown().await.unwrap();
}
