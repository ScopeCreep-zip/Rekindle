//! The skin editor's requests through `HostApi::call` (plan E7.7.5): a
//! draft's report needs no review, a failing pair blocks export with the
//! named ratio, "Start from the current skin" answers the shown skin's
//! tokens, and an exported draft re-imports byte-identically.

use std::collections::BTreeMap;

use rekindle_ipc::protocol::{IpcRequest, IpcResponse};
use rekindle_ipc::{Caller, HostApi};
use rekindle_skin::import::walk::color;
use rekindle_skin::{builtin, draft, validate};
use rekindle_types::dto::skin::{
    Insets, OriginalLicence, RegionFit, SkinBase, SkinContrast, SkinCredits,
};
use rekindle_types::dto::skin_draft::{SkinDraft, SkinDraftRegion, SkinDraftReport};
use rekindle_types::dto::skin_library::SkinEntry;

use super::skin_requests::{data, run, stand_in};
use super::{create_ann, offline_host, status_of};

fn a_draft(accent_text: [u8; 3]) -> SkinDraft {
    SkinDraft {
        name: "Night".to_owned(),
        description: String::new(),
        author: "Ana".to_owned(),
        license: "MIT".to_owned(),
        credits: SkinCredits {
            authors: Vec::new(),
            contributors: Vec::new(),
            url: None,
            licence: OriginalLicence::NoneStated,
        },
        base: SkinBase::Dark,
        contrast: SkinContrast::Standard,
        tokens: BTreeMap::from([("accent.text".to_owned(), color(accent_text))]),
        from: None,
        regions: BTreeMap::new(),
    }
}

#[tokio::test]
async fn a_draft_is_reported_without_a_review_and_a_failing_pair_blocks_export() {
    let dir = tempfile::tempdir().unwrap();
    let host = offline_host(&dir).await;
    create_ann(&host).await;
    let failing = a_draft([0x16, 0x7b, 0xc0]);
    let (answer, review) = run(
        &host,
        IpcRequest::SkinValidateDraft {
            draft: failing.clone(),
        },
    )
    .await;
    assert!(review.is_none(), "validation is not an intent");
    let report: SkinDraftReport = data(answer);
    assert!(!report.exportable);
    let bad = report.pairs.iter().find(|p| !p.pass).unwrap();
    assert!(bad.suggestion.is_some());
    // The helper is the gate: it refuses the package, naming the ratio.
    let refusal = validate::run(
        &draft::package(&failing, &draft::Assets::default())
            .unwrap()
            .bytes,
    )
    .unwrap_err();
    host.ctx
        .skin_helper
        .set(stand_in(&dir, &Err(refusal.clone())))
        .unwrap();
    let path = dir.path().join("night.rkskin");
    let export = IpcRequest::SkinExportDraft {
        draft: failing,
        path: path.display().to_string(),
    };
    let (answer, review) = run(&host, export).await;
    assert!(review.is_none());
    match answer {
        IpcResponse::Error {
            code: 400, message, ..
        } => assert_eq!(message, format!("skin refused: {refusal}")),
        other => panic!("expected the named refusal, got {other:?}"),
    }
    assert!(!path.exists(), "nothing written");
}

#[tokio::test]
async fn a_draft_starts_from_the_shown_skin_and_exports_for_a_byte_identical_import() {
    let dir = tempfile::tempdir().unwrap();
    let host = offline_host(&dir).await;
    create_ann(&host).await;
    let shown = validate::run(builtin::PACKS[0].bytes).unwrap();
    host.ctx
        .skin_helper
        .set(stand_in(&dir, &Ok(shown.clone())))
        .unwrap();
    let start: SkinDraft = data(
        host.call(Caller::shell(), IpcRequest::SkinDraftStart { id: None })
            .await,
    );
    let from = start.from.clone().expect("the shown skin's id");
    assert_eq!(start, draft::from_package(&from, &shown.bytes).unwrap());
    assert!(
        matches!(start.credits.licence, OriginalLicence::Text(_)),
        "the licence it was made under carries forward"
    );
    // An export writes the helper's canonical bytes to the chosen file.
    let path = dir.path().join("start.rkskin");
    let export = IpcRequest::SkinExportDraft {
        draft: start,
        path: path.display().to_string(),
    };
    let address: String = data(host.call(Caller::shell(), export).await);
    assert_eq!(address, shown.address());
    let written = std::fs::read(&path).unwrap();
    assert_eq!(written, shown.bytes);
    let (answer, _) = run(&host, IpcRequest::SkinImport { bytes: written }).await;
    let entry: SkinEntry = data(answer);
    assert_eq!(entry.id, format!("pkg:{address}"));
}

#[tokio::test]
async fn a_draft_past_its_bounds_or_a_relative_path_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let host = offline_host(&dir).await;
    create_ann(&host).await;
    let relative = IpcRequest::SkinExportDraft {
        draft: a_draft([0x34, 0x91, 0xd8]),
        path: "night.rkskin".to_owned(),
    };
    let answer = host.call(Caller::shell(), relative).await;
    assert_eq!(status_of(&answer), Some(400), "{answer:?}");
    let unknown = SkinDraft {
        tokens: BTreeMap::from([("no.such".to_owned(), color([1, 2, 3]))]),
        ..a_draft([0, 0, 0])
    };
    let answer = host
        .call(
            Caller::shell(),
            IpcRequest::SkinValidateDraft { draft: unknown },
        )
        .await;
    assert_eq!(status_of(&answer), Some(400), "{answer:?}");
}

/// Region import goes through the Host: a relative image path is refused
/// before the gate, an unreadable one names itself, and the JSON Schema
/// export writes the token file's schema.
#[tokio::test]
async fn a_region_import_reads_its_image_in_the_host_and_the_schema_exports() {
    let dir = tempfile::tempdir().unwrap();
    let host = offline_host(&dir).await;
    create_ann(&host).await;
    let with = |path: String| {
        let mut d = a_draft([0x34, 0x91, 0xd8]);
        d.regions.insert(
            "scrollbar.track".to_owned(),
            SkinDraftRegion {
                path,
                fit: RegionFit::NineSlice(Insets {
                    top: 2,
                    right: 2,
                    bottom: 2,
                    left: 2,
                }),
                text_backing: None,
            },
        );
        IpcRequest::SkinDraftCheck { draft: d }
    };
    let answer = host
        .call(Caller::shell(), with("track.png".to_owned()))
        .await;
    assert_eq!(status_of(&answer), Some(400), "{answer:?}");
    let missing = dir.path().join("missing.png").display().to_string();
    match host.call(Caller::shell(), with(missing)).await {
        IpcResponse::Error { message, .. } => {
            assert!(message.contains("skin region unreadable"), "{message}");
        }
        other => panic!("expected the read failure, got {other:?}"),
    }
    let path = dir.path().join("skin.schema.json");
    let schema = IpcRequest::SkinExportSchema {
        path: path.display().to_string(),
    };
    let (answer, review) = run(&host, schema).await;
    assert!(review.is_none(), "the save dialog is its confirmation");
    let () = data(answer);
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        rekindle_skin::generate::json_schema()
    );
}
