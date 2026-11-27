use rekindle_types::id::PseudonymKey;

use super::CommunityStore;
use rekindle_governance_runtime::CardText;

fn named(name: &str) -> CardText {
    CardText {
        display_name: Some(name.to_owned()),
        custom_status: None,
        game: None,
    }
}

/// The first card read and a rename are news; a re-read of the same
/// name is not, so it announces nothing.
#[test]
fn a_card_name_is_news_when_first_read_and_when_changed() {
    let store = CommunityStore::new(std::sync::Arc::default());
    let bea = PseudonymKey([2; 32]);
    assert!(
        store.note_member_card("c", &bea, named("Bea")),
        "first read"
    );
    assert!(
        !store.note_member_card("c", &bea, named("Bea")),
        "same name"
    );
    assert!(store.note_member_card("c", &bea, named("Bee")), "renamed");
    assert_eq!(store.member_card_name("c", &bea).as_deref(), Some("Bee"));
    assert!(
        store.note_member_card("d", &bea, named("Bee")),
        "per community"
    );
}
