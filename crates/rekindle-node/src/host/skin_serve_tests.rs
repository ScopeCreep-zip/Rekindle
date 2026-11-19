//! `SkinGet`, `SkinFile` and `SkinChanged` through `HostApi::call` on an
//! offline host (plan E7.7.2 "`skin://` scheme", "Apply"): before unlock
//! the login skin applies and only its files are served; a library
//! package is served `no-store`, a built-in `immutable`; an unknown
//! address or a path outside the scheme's shape is refused; choosing a
//! skin announces it to every window; role tag colours follow the shown
//! skin, an imported one too (plan E7.2 slice 4).

use std::collections::BTreeMap;

use rekindle_governance::state::{GovernanceState, RoleState};
use rekindle_ipc::protocol::{IpcRequest, IpcResponse};
use rekindle_ipc::{Caller, HostApi};
use rekindle_skin::import::walk::color;
use rekindle_skin::role_tag::{self, SkinGrounds};
use rekindle_skin::serve;
use rekindle_skin::{builtin, draft, validate};
use rekindle_types::dto::skin::{OriginalLicence, SkinBase, SkinContrast, SkinCredits};
use rekindle_types::dto::skin_draft::SkinDraft;
use rekindle_types::dto::skin_library::{SkinApplied, SkinEntry, SkinServed};
use rekindle_types::id::RoleId;
use rekindle_types::subscription_events::{
    GovernanceEvent, StreamFrame, SubscriptionEvent, SystemEvent,
};

use super::skin_requests::{data, run, stand_in};
use super::{create_ann, offline_host, status_of};

const UNKNOWN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

fn file(path: String) -> IpcRequest {
    IpcRequest::SkinFile { path }
}

#[tokio::test]
async fn the_login_skin_applies_before_unlock_and_is_served_immutable() {
    let dir = tempfile::tempdir().unwrap();
    let host = offline_host(&dir).await;
    let canonical = validate::run(builtin::PACKS[0].bytes).unwrap();
    host.ctx
        .skin_helper
        .set(stand_in(&dir, &Ok(canonical.clone())))
        .unwrap();
    let applied: SkinApplied = data(
        host.call(Caller::shell(), IpcRequest::SkinGet { id: None })
            .await,
    );
    let a = canonical.address();
    assert_eq!(applied.id, builtin::DEFAULT);
    assert_eq!(applied.tokens_href, serve::tokens_href(&a));
    assert_eq!(applied.base, SkinBase::Dark);
    // The TUI's palette subset rides the same answer (plan E7.7.2).
    let resolved = serve::stylesheet(&canonical.bytes, &a).unwrap();
    assert_eq!(applied.palette, resolved.palette);
    assert!(applied.palette.iter().any(|c| c.role == "accent.primary"));
    let css: SkinServed = data(
        host.call(Caller::shell(), file(format!("/{a}/tokens.css")))
            .await,
    );
    assert!(css.content_type.starts_with("text/css"));
    assert_eq!(css.cache_control, serve::Cache::Immutable.header());
    assert!(String::from_utf8(css.bytes)
        .unwrap()
        .starts_with("@layer skin {"));
    for bad in [
        format!("/{UNKNOWN}/tokens.css"),
        format!("/{a}/{UNKNOWN}.png"),
        format!("/{a}/../{a}/tokens.css"),
        format!("/{a}/manifest.json"),
    ] {
        let answer = host.call(Caller::shell(), file(bad.clone())).await;
        assert_eq!(status_of(&answer), Some(404), "{bad}");
    }
}

#[tokio::test]
async fn a_library_skin_is_announced_and_served_no_store() {
    let dir = tempfile::tempdir().unwrap();
    let host = offline_host(&dir).await;
    create_ann(&host).await;
    let canonical = validate::run(builtin::PACKS[0].bytes).unwrap();
    // A distinct library package: the stripped login copy of the built-in
    // is a valid canonical package with another address.
    let library = rekindle_skin::login::strip(&canonical.bytes).unwrap();
    host.ctx
        .skin_helper
        .set(stand_in(&dir, &Ok(library.clone())))
        .unwrap();
    let (answer, _) = run(
        &host,
        IpcRequest::SkinImport {
            bytes: library.bytes.clone(),
        },
    )
    .await;
    let entry: SkinEntry = data(answer);
    let mut events = host.ctx.journal.subscribe(None);
    run(
        &host,
        IpcRequest::SkinSelect {
            id: entry.id.clone(),
        },
    )
    .await;
    let announced = loop {
        let frame = tokio::time::timeout(std::time::Duration::from_secs(5), events.next())
            .await
            .expect("SkinChanged within 5 s")
            .expect("an event");
        match frame {
            StreamFrame::Event(s) => {
                if let SubscriptionEvent::System(SystemEvent::SkinChanged { skin }) = s.event {
                    break skin;
                }
            }
            StreamFrame::Resumed(_) => {}
            StreamFrame::InvalidSession => panic!("invalid session"),
        }
    };
    assert_eq!(announced.id, entry.id);
    let a = library.address();
    assert_eq!(announced.tokens_href, serve::tokens_href(&a));
    let got: SkinApplied = data(
        host.call(Caller::shell(), IpcRequest::SkinGet { id: None })
            .await,
    );
    assert_eq!(got, announced);
    let css: SkinServed = data(
        host.call(Caller::shell(), file(format!("/{a}/tokens.css")))
            .await,
    );
    assert_eq!(css.cache_control, "no-store");

    // Locked, the library is out of reach.
    let lock = host.call(Caller::shell(), IpcRequest::Lock).await;
    assert!(matches!(lock, IpcResponse::Ok { .. }), "{lock:?}");
    let answer = host
        .call(Caller::shell(), file(format!("/{a}/tokens.css")))
        .await;
    assert_eq!(status_of(&answer), Some(404));
}

/// An imported dark skin whose role-tag grounds are #404040: no built-in
/// dark skin has one above #1e1e1e.
fn grey_skin() -> SkinDraft {
    let grey = color([0x40; 3]);
    SkinDraft {
        name: "Slate".to_owned(),
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
        tokens: role_tag::GROUNDS
            .iter()
            .map(|g| ((*g).to_owned(), grey.clone()))
            .collect::<BTreeMap<_, _>>(),
        from: None,
        regions: BTreeMap::new(),
    }
}

#[tokio::test]
async fn role_colours_are_resent_readable_on_an_imported_skin() {
    let dir = tempfile::tempdir().unwrap();
    let host = offline_host(&dir).await;
    create_ann(&host).await;
    // #c0c0c0 reads on every built-in dark skin's tag (5.2:1 over #1e1e1e)
    // but not on its tag over #404040 (3.5:1).
    let mut gov = GovernanceState::default();
    gov.roles.insert(
        RoleId([7; 16]),
        RoleState {
            name: "Mods".into(),
            permissions: 0,
            position: 1,
            color: 0x00c0_c0c0,
            hoist: false,
            mentionable: false,
            self_assignable: false,
            exclusion_group: None,
            stamp: rekindle_governance::fixture::stamp(1),
        },
    );
    host.ctx.communities.set_governance_state("c1", gov);
    assert_eq!(
        host.governance().role_list("c1")[0].tag_colours.dark,
        0x00c0_c0c0
    );
    let library = draft::package(&grey_skin(), &draft::Assets::default()).unwrap();
    host.ctx
        .skin_helper
        .set(stand_in(&dir, &Ok(library.clone())))
        .unwrap();
    let (answer, _) = run(
        &host,
        IpcRequest::SkinImport {
            bytes: library.bytes.clone(),
        },
    )
    .await;
    let entry: SkinEntry = data(answer);
    let mut events = host.ctx.journal.subscribe(None);
    run(&host, IpcRequest::SkinSelect { id: entry.id }).await;
    let (community, roles) = loop {
        let frame = tokio::time::timeout(std::time::Duration::from_secs(5), events.next())
            .await
            .expect("RolesChanged within 5 s")
            .expect("an event");
        if let StreamFrame::Event(s) = frame {
            if let SubscriptionEvent::Governance(GovernanceEvent::RolesChanged {
                community,
                roles,
            }) = s.event
            {
                break (community, roles);
            }
        }
    };
    let shown = SkinGrounds::of_package(&library.bytes).unwrap();
    let want = role_tag::colours(0x00c0_c0c0, Some(&shown));
    assert_ne!(want.dark, 0x00c0_c0c0);
    assert_eq!(community, "c1");
    assert_eq!(roles[0].tag_colours, want);
    assert_eq!(host.governance().role_list("c1")[0].tag_colours, want);
}
