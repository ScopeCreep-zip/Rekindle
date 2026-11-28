//! A follower opens only its own sealed invite (plan E3.2 SS3c, lead
//! decision (34)): a member the publisher left out is told plainly, and an
//! entry read against another successor anchor does not open.

use std::collections::BTreeMap;

use rekindle_codec::successor_invites::{seal_invite, InvitesPage, SealedInvite};
use rekindle_types::id::PseudonymKey;

use super::Follow;
use crate::error::GovernanceRuntimeError;

const OLD: &str = "VLD0:old";
const NEW: &str = "VLD0:new";
const FIRST_PAGE: &str = "VLD0:first";
const SECOND_PAGE: &str = "VLD0:second";

fn member(i: u8) -> (PseudonymKey, [u8; 32]) {
    (PseudonymKey([i + 100; 32]), [i; 32])
}

fn sealed(i: u8, new_anchor: &str) -> SealedInvite {
    let (pseudonym, secret) = member(i);
    let key = rekindle_secrets::hpke::public_key(&secret);
    seal_invite(
        OLD,
        new_anchor,
        &pseudonym,
        &key,
        &format!("rekindle://join/{i}"),
    )
    .unwrap()
}

/// Two pages: the first names the second; `first` and `second` are their
/// entries.
fn pages(first: Vec<SealedInvite>, second: Vec<SealedInvite>) -> BTreeMap<String, InvitesPage> {
    BTreeMap::from([
        (
            FIRST_PAGE.to_owned(),
            InvitesPage {
                pages: vec![SECOND_PAGE.to_owned()],
                entries: first,
            },
        ),
        (
            SECOND_PAGE.to_owned(),
            InvitesPage {
                pages: Vec::new(),
                entries: second,
            },
        ),
    ])
}

async fn follow(
    pages: &BTreeMap<String, InvitesPage>,
    i: u8,
    new_anchor: &str,
) -> Result<String, GovernanceRuntimeError> {
    let (me, secret) = member(i);
    let follow = Follow {
        old_anchor: OLD,
        new_anchor,
        me: &me,
        hpke_secret: &secret,
    };
    let read = |key: String| async move {
        pages
            .get(&key)
            .cloned()
            .ok_or(GovernanceRuntimeError::Adapter(key))
    };
    follow.open(FIRST_PAGE, read).await
}

#[tokio::test]
async fn a_follower_opens_its_own_entry_on_a_further_page() {
    let pages = pages(vec![sealed(1, NEW)], vec![sealed(2, NEW)]);
    assert_eq!(follow(&pages, 2, NEW).await.unwrap(), "rekindle://join/2");
}

/// A member the publisher did not continue has no entry and gets the plain
/// "did not continue you" refusal, not a crypto or read error.
#[tokio::test]
async fn a_member_with_no_entry_is_told_it_was_not_continued() {
    let pages = pages(vec![sealed(1, NEW)], vec![sealed(2, NEW)]);
    match follow(&pages, 3, NEW).await {
        Err(GovernanceRuntimeError::OpRefused(why)) => {
            assert!(why.contains("did not continue you"), "{why}");
        }
        other => panic!("expected the plain refusal, got {other:?}"),
    }
}

/// An entry sealed for one successor does not open when followed as a link
/// naming another anchor.
#[tokio::test]
async fn an_entry_followed_through_another_anchor_is_refused() {
    let pages = pages(vec![sealed(1, NEW)], Vec::new());
    assert!(matches!(
        follow(&pages, 1, "VLD0:other").await,
        Err(GovernanceRuntimeError::Crypto(_))
    ));
}
