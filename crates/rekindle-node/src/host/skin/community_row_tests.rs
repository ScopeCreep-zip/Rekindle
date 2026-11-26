//! Plan E7.7: the Communities row's Open, Leave and New. Open and Leave act
//! on the window's selected community (`SkinSelection::Community`), are
//! refused with the Host's reason while none is selected, and their
//! capability holds only with one selected, its refusal (CR1) in the same words.

use rekindle_types::dto::skin_action::{Capability as C, SkinAction as A};
use rekindle_types::dto::skin_run::{ShellOp, ShellPanel, SkinRun, SkinSelection};

use super::actions::{map, Mapped, Selected, Selections};
use super::capability::{allows, refusal, Selected as CapSelected, SkinWorld};
use super::community::SELECT_COMMUNITY;
use super::reads::Reads;
use super::signals::Inputs;

fn run(action: &A, community: Option<&str>) -> Result<SkinRun, String> {
    let selected = Selected {
        community,
        ..Selected::default()
    };
    match map(action, selected, &Reads::default()) {
        Ok(Mapped::Run(r)) => Ok(r),
        Ok(other) => panic!("{action:?}: {other:?}"),
        Err(e) => Err(format!("{e:?}")),
    }
}

#[test]
fn open_and_leave_act_on_the_selected_community() {
    let ck = "communitykey";
    assert_eq!(
        run(&A::OpenCommunity, Some(ck)),
        Ok(SkinRun::Shell(ShellOp::OpenCommunity {
            community: ck.into()
        }))
    );
    assert_eq!(
        run(&A::LeaveCommunity, Some(ck)),
        Ok(SkinRun::Shell(ShellOp::LeaveCommunity {
            community: ck.into()
        }))
    );
    assert_eq!(
        run(&A::StartCommunity, None),
        Ok(SkinRun::Shell(ShellOp::OpenPanel(
            ShellPanel::CommunityStart
        )))
    );
}

#[test]
fn open_and_leave_refused_with_the_reason_while_nothing_is_selected() {
    for action in [A::OpenCommunity, A::LeaveCommunity] {
        let err = run(&action, None).expect_err("refused");
        assert!(err.contains(SELECT_COMMUNITY), "{action:?}: {err}");
        assert!(err.contains("409"), "{action:?}: {err}");
    }
    assert_eq!(SELECT_COMMUNITY, "Select a community first.");
}

#[test]
fn the_selection_names_the_community_and_enables_open_and_leave() {
    let selections = Selections::default();
    let picked = SkinSelection::Community {
        community: "ck".into(),
        channel: None,
        server: None,
    };
    assert_eq!(selections.community("main"), None);
    selections.set("main", Some(picked.clone()));
    assert_eq!(selections.community("main").as_deref(), Some("ck"));
    let (inputs, world) = (Inputs::default(), SkinWorld::default());
    let a_friend = SkinSelection::Friend {
        public_key: "k".into(),
    };
    for cap in [C::OpenCommunity, C::LeaveCommunity] {
        assert!(
            !allows(cap, &inputs, &world, CapSelected::Nothing),
            "{cap:?}"
        );
        let friend = CapSelected::of(Some(&a_friend), None);
        assert!(!allows(cap, &inputs, &world, friend), "{cap:?}");
        let community = CapSelected::of(Some(&picked), None);
        assert!(allows(cap, &inputs, &world, community), "{cap:?}");
        // CR1: the one source of a refusal's words is the press's own.
        assert_eq!(
            refusal(cap, &inputs, &world, CapSelected::Nothing).as_deref(),
            Some(SELECT_COMMUNITY),
            "{cap:?}"
        );
        assert_eq!(refusal(cap, &inputs, &world, community), None, "{cap:?}");
    }
}
