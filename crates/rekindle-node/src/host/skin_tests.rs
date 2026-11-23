//! The skin requests through `HostApi::call` on an offline host (plan
//! E7.7.2): an id naming no built-in and no library row is refused, a
//! package the helper refuses is refused with the helper's typed error,
//! and import, select, set-login, list and remove round-trip. The helper
//! is a stand-in script answering a frame the real pipeline produced.

use rekindle_ipc::protocol::{IpcRequest, IpcResponse};
use rekindle_ipc::{Caller, HostApi};
use rekindle_skin::helper::{frame, HelperOpts};
use rekindle_skin::{builtin, conform, validate};
use rekindle_types::dto::skin_library::{SkinEntry, SkinLibrary};
use rekindle_types::intent::{Escalation, IntentClass, IntentProof};

use super::{create_ann, offline_host, status_of};
use crate::host::Host;

const UNKNOWN: &str = "pkg:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

/// Run `request`, committing its review (if any) with the proof its
/// class asks; the review's class and escalations come back with it.
pub(super) async fn run(
    host: &Host,
    request: IpcRequest,
) -> (IpcResponse, Option<(IntentClass, Vec<Escalation>)>) {
    match host.call(Caller::shell(), request).await {
        IpcResponse::IntentRequired(review) => {
            let proof = match review.class {
                IntentClass::Consequential => IntentProof::Commit,
                IntentClass::TrustChanging => IntentProof::Acknowledged,
                other => panic!("unexpected class {other:?}"),
            };
            let commit = IpcRequest::IntentCommit {
                intent_id: review.intent_id,
                proof,
            };
            let answer = host.call(Caller::shell(), commit).await;
            (answer, Some((review.class, review.escalations)))
        }
        answer => (answer, None),
    }
}

pub(super) fn data<T: serde::de::DeserializeOwned>(answer: IpcResponse) -> T {
    match answer {
        IpcResponse::Ok { data, .. } => serde_json::from_value(data).unwrap(),
        other => panic!("expected Ok, got {other:?}"),
    }
}

/// A stand-in helper that reads the package and answers `answer`'s frame.
/// A child `sh` writes the script (see `rekindle-skin/tests/helper.rs`:
/// a write fd held here would make a sibling's exec fail with ETXTBSY).
pub(super) fn stand_in(dir: &tempfile::TempDir, answer: &frame::HelperAnswer) -> HelperOpts {
    let frame_path = dir.path().join("answer.bin");
    std::fs::write(&frame_path, frame::encode(answer)).unwrap();
    let path = dir.path().join("rekindle-skin-helper");
    let body = format!(
        "#!/bin/sh\ncat >/dev/null\ncat '{}'\n",
        frame_path.display()
    );
    let written = std::process::Command::new("/bin/sh")
        .args(["-c", r#"printf '%s' "$1" > "$2" && chmod 755 "$2""#, "sh"])
        .arg(body)
        .arg(&path)
        .status()
        .unwrap();
    assert!(written.success());
    HelperOpts {
        path,
        timeout: std::time::Duration::from_secs(10),
        footprint_cap: rekindle_skin::helper::FOOTPRINT_BYTES,
    }
}

#[tokio::test]
async fn an_id_naming_no_builtin_and_no_row_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let host = offline_host(&dir).await;
    create_ann(&host).await;
    for id in ["builtin:nope", UNKNOWN] {
        let (answer, review) = run(&host, IpcRequest::SkinSelect { id: id.into() }).await;
        assert_eq!(status_of(&answer), Some(404), "select {id}: {answer:?}");
        assert!(review.is_none(), "choosing a skin is Routine");
        let (answer, _) = run(&host, IpcRequest::SkinSetLogin { id: id.into() }).await;
        assert_eq!(status_of(&answer), Some(404), "set-login {id}: {answer:?}");
    }
    let (answer, review) = run(&host, IpcRequest::SkinRemove { id: UNKNOWN.into() }).await;
    assert_eq!(status_of(&answer), Some(404));
    assert_eq!(review.unwrap().0, IntentClass::Consequential);
    let library: SkinLibrary = data(host.call(Caller::shell(), IpcRequest::SkinList).await);
    assert_eq!(library.active, builtin::DEFAULT);
    assert_eq!(library.login, builtin::DEFAULT);
    assert!(!host.ctx.root.skins().exists(), "nothing written");
}

#[tokio::test]
async fn a_package_the_helper_refuses_is_refused_with_its_typed_error() {
    let dir = tempfile::tempdir().unwrap();
    let host = offline_host(&dir).await;
    create_ann(&host).await;
    let bad = b"not a zip".to_vec();
    let refusal = validate::run(&bad).unwrap_err();
    host.ctx
        .skin
        .helper
        .set(stand_in(&dir, &Err(refusal.clone())))
        .unwrap();
    let (answer, review) = run(&host, IpcRequest::SkinImport { bytes: bad }).await;
    assert_eq!(review.unwrap().0, IntentClass::TrustChanging);
    match answer {
        IpcResponse::Error {
            code: 400, message, ..
        } => {
            assert_eq!(message, format!("skin refused: {refusal}"));
        }
        other => panic!("expected the helper's refusal, got {other:?}"),
    }
    let library: SkinLibrary = data(host.call(Caller::shell(), IpcRequest::SkinList).await);
    // The committed packs and the conforming alternate.
    assert_eq!(
        library.skins.len(),
        builtin::PACKS.len() + 1,
        "nothing stored"
    );
}

/// Plan SK14: the conforming alternate is listed right after Symbiosis,
/// counts no short target where Symbiosis counts four, and is selectable
/// and settable as the device login skin.
#[tokio::test]
async fn the_conforming_alternate_is_listed_selectable_and_a_login_skin() {
    let dir = tempfile::tempdir().unwrap();
    let host = offline_host(&dir).await;
    create_ann(&host).await;
    let library: SkinLibrary = data(host.call(Caller::shell(), IpcRequest::SkinList).await);
    let at = library
        .skins
        .iter()
        .position(|s| s.id == builtin::DEFAULT)
        .expect("Symbiosis is listed");
    let (symbiosis, alternate) = (&library.skins[at], &library.skins[at + 1]);
    assert_eq!(alternate.id, conform::ID);
    assert_eq!(alternate.name, conform::NAME);
    assert_eq!((alternate.short_targets, alternate.conforming), (0, true));
    assert_eq!((symbiosis.short_targets, symbiosis.conforming), (4, false));
    assert_eq!(alternate.credits, symbiosis.credits);

    let (answer, _) = run(
        &host,
        IpcRequest::SkinSelect {
            id: conform::ID.into(),
        },
    )
    .await;
    let library: SkinLibrary = data(answer);
    assert_eq!(library.active, conform::ID);
    assert!(library.trial.is_none(), "a built-in is stored at once");

    let (answer, review) = run(
        &host,
        IpcRequest::SkinSetLogin {
            id: conform::ID.into(),
        },
    )
    .await;
    assert!(review.is_none(), "a built-in login skin is Routine");
    assert_eq!(data::<SkinLibrary>(answer).login, conform::ID);
    let library: SkinLibrary = data(host.call(Caller::shell(), IpcRequest::SkinList).await);
    assert_eq!(
        library.login,
        conform::ID,
        "login.json names it and reads back"
    );
}

#[tokio::test]
async fn import_select_set_login_and_remove_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let host = offline_host(&dir).await;
    create_ann(&host).await;
    let canonical = validate::run(builtin::PACKS[0].bytes).unwrap();
    host.ctx
        .skin
        .helper
        .set(stand_in(&dir, &Ok(canonical.clone())))
        .unwrap();
    let (answer, _) = run(
        &host,
        IpcRequest::SkinImport {
            bytes: canonical.bytes.clone(),
        },
    )
    .await;
    let entry: SkinEntry = data(answer);
    assert_eq!(entry.id, format!("pkg:{}", canonical.address()));
    // Plan SK14: an imported pack's count comes from its bytes as served.
    let shorts = conform::short_targets_of(&canonical.bytes).unwrap();
    assert_eq!(
        (entry.short_targets, entry.conforming),
        (shorts, shorts == 0)
    );
    // Plan E7.7.3d: the import answers the manifest's credits.
    let credited = entry.credits.clone().expect("Symbiosis states its credits");
    assert_eq!(credited.authors, ["Xfire, Inc."]);

    let (answer, _) = run(
        &host,
        IpcRequest::SkinSelect {
            id: entry.id.clone(),
        },
    )
    .await;
    let library: SkinLibrary = data(answer);
    assert_eq!(library.trial.map(|t| t.id), Some(entry.id.clone()));
    assert!(library.skins.iter().any(|s| s.id == entry.id));
    // `SkinList` carries the credits of the built-ins and the library.
    for s in &library.skins {
        assert!(s.credits.is_some(), "{} lacks its credits", s.id);
    }
    let library_row = library.skins.iter().find(|s| s.id == entry.id).unwrap();
    assert_eq!(library_row.credits.as_ref(), Some(&credited));
    assert_eq!(
        library_row.short_targets, shorts,
        "the row's count, from its bytes"
    );
    let (answer, review) = run(
        &host,
        IpcRequest::SkinKeep {
            id: entry.id.clone(),
            keep: true,
        },
    )
    .await;
    assert!(review.is_none(), "keeping a skin is Routine");
    let library: SkinLibrary = data(answer);
    assert_eq!(library.active, entry.id);
    assert!(library.trial.is_none());
    let (answer, _) = run(
        &host,
        IpcRequest::SkinRemove {
            id: entry.id.clone(),
        },
    )
    .await;
    assert_eq!(status_of(&answer), Some(409), "the skin in use stays");

    // A library skin on the login screen is held for acknowledgement.
    let (answer, review) = run(
        &host,
        IpcRequest::SkinSetLogin {
            id: entry.id.clone(),
        },
    )
    .await;
    let (class, escalations) = review.unwrap();
    assert_eq!(class, IntentClass::TrustChanging);
    assert_eq!(escalations, vec![Escalation::VisibleBeforeUnlock]);
    let library: SkinLibrary = data(answer);
    let hash = library.login.strip_prefix("dev:").unwrap().to_owned();
    assert_ne!(hash, canonical.address(), "a stripped copy, re-hashed");
    let copy = std::fs::read(host.ctx.root.skins().join(format!("{hash}.rkskin"))).unwrap();
    let pkg = rekindle_skin::package::read(&copy).unwrap();
    let m = rekindle_skin::manifest::parse(&pkg).unwrap();
    assert!(m.author.is_none() && m.source.is_none() && m.sounds.is_empty());

    // A built-in on the login screen is Routine; the copy goes.
    let (answer, review) = run(
        &host,
        IpcRequest::SkinSetLogin {
            id: builtin::DEFAULT.into(),
        },
    )
    .await;
    assert!(review.is_none());
    assert_eq!(data::<SkinLibrary>(answer).login, builtin::DEFAULT);
    assert!(!host
        .ctx
        .root
        .skins()
        .join(format!("{hash}.rkskin"))
        .exists());

    run(
        &host,
        IpcRequest::SkinSelect {
            id: builtin::DEFAULT.into(),
        },
    )
    .await;
    let (answer, _) = run(
        &host,
        IpcRequest::SkinRemove {
            id: entry.id.clone(),
        },
    )
    .await;
    let library: SkinLibrary = data(answer);
    assert!(library.skins.iter().all(|s| s.id != entry.id));
}

#[tokio::test]
async fn a_skin_is_chosen_while_the_network_is_detached() {
    let dir = tempfile::tempdir().unwrap();
    let host = offline_host(&dir).await;
    create_ann(&host).await;
    host.ctx
        .lifecycle
        .transition(crate::daemon::DaemonState::Detached)
        .unwrap();
    let canonical = validate::run(builtin::PACKS[1].bytes).unwrap();
    host.ctx
        .skin
        .helper
        .set(stand_in(&dir, &Ok(canonical)))
        .unwrap();
    let other = format!("{}{}", builtin::PREFIX, builtin::PACKS[1].id);
    let (answer, _) = run(&host, IpcRequest::SkinSelect { id: other.clone() }).await;
    assert_eq!(
        data::<SkinLibrary>(answer).active,
        other,
        "chosen while detached"
    );
    let (answer, _) = run(&host, IpcRequest::SkinSetLogin { id: other.clone() }).await;
    assert_eq!(
        data::<SkinLibrary>(answer).login,
        other,
        "login skin set while detached"
    );
}
