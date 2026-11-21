use rekindle_types::dht_layout::MAX_SUBKEY_SIZE;
use rekindle_types::id::PseudonymKey;

use super::*;

const OLD: &str = "VLD0:old";
const NEW: &str = "VLD0:new";

fn member(i: u8) -> (PseudonymKey, [u8; 32], [u8; 32]) {
    let secret = [i; 32];
    (
        PseudonymKey([i + 100; 32]),
        secret,
        rekindle_secrets::hpke::public_key(&secret),
    )
}

#[test]
fn an_entry_opens_only_for_its_member() {
    let (alice, alice_secret, alice_key) = member(1);
    let (_, bob_secret, _) = member(2);
    let entry = seal_invite(OLD, NEW, &alice, &alice_key, "rekindle://join/x").unwrap();
    assert_eq!(
        open_invite(OLD, NEW, &entry, &alice_secret).unwrap(),
        "rekindle://join/x"
    );
    // Another member's (or a removed member's) card secret opens nothing.
    assert_eq!(
        open_invite(OLD, NEW, &entry, &bob_secret),
        Err(InviteSealError::NotOpened)
    );
}

#[test]
fn an_entry_is_bound_to_its_member_successor_and_community() {
    let (alice, alice_secret, alice_key) = member(1);
    let (bob, _, _) = member(2);
    let entry = seal_invite(OLD, NEW, &alice, &alice_key, "invite").unwrap();
    let moved = SealedInvite {
        member: bob,
        ..entry.clone()
    };
    assert!(open_invite(OLD, NEW, &moved, &alice_secret).is_err());
    assert!(open_invite(OLD, "VLD0:other", &entry, &alice_secret).is_err());
    assert!(open_invite("VLD0:else", NEW, &entry, &alice_secret).is_err());
}

#[test]
fn an_overlong_invite_is_refused() {
    let (alice, _, key) = member(1);
    let long = "x".repeat(MAX_INVITE_LEN + 1);
    assert_eq!(
        seal_invite(OLD, NEW, &alice, &key, &long),
        Err(InviteSealError::TooLong(MAX_INVITE_LEN + 1))
    );
}

#[test]
fn a_full_first_page_fits_one_subkey_and_round_trips() {
    let longest = "i".repeat(MAX_INVITE_LEN);
    let entries: Vec<_> = (0..u8::try_from(ENTRIES_PER_PAGE).unwrap())
        .map(|i| {
            let (m, _, key) = member(i + 1);
            seal_invite(OLD, NEW, &m, &key, &longest).unwrap()
        })
        .collect();
    let page = InvitesPage {
        pages: (1..MAX_PAGES).map(|i| format!("VLD0:{i:0>43}")).collect(),
        entries,
    };
    let bytes = encode_invites_page(&page);
    assert!(bytes.len() <= MAX_SUBKEY_SIZE, "{} bytes", bytes.len());
    assert_eq!(decode_invites_page(&bytes).unwrap(), page);
}

#[test]
fn paging_splits_by_the_page_count_and_caps_the_pages() {
    let (m, _, key) = member(1);
    let one = seal_invite(OLD, NEW, &m, &key, "i").unwrap();
    let pages = paginate(vec![one.clone(); ENTRIES_PER_PAGE + 1]).unwrap();
    assert_eq!(pages.len(), 2);
    assert_eq!(pages[1].entries.len(), 1);
    assert_eq!(paginate(Vec::new()).unwrap().len(), 1);
    let over = ENTRIES_PER_PAGE * MAX_PAGES + 1;
    assert_eq!(
        paginate(vec![one; over]),
        Err(InviteSealError::TooManyMembers(over))
    );
}
