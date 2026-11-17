//! The names on members' verified cards (`M`/0, signed by the pseudonym,
//! its owner pinned to the member's `MemberJoined`) as [`CommunityStore`]
//! holds them: the only source of a member's name in membership events
//! (plan E3.2 F9-2).

use rekindle_types::id::PseudonymKey;

use super::CommunityStore;

impl CommunityStore {
    /// The display name on `pseudonym`'s verified card.
    #[must_use]
    pub fn member_card_name(&self, community_id: &str, pseudonym: &PseudonymKey) -> Option<String> {
        self.member_cards
            .read()
            .get(&(community_id.to_owned(), pseudonym.0))
            .cloned()
    }

    /// Remember the display name on `pseudonym`'s verified card: whether
    /// it is news, the first name held for the member or a different one
    /// (the `MemberCardChanged` gate).
    #[must_use]
    pub fn note_member_card_name(
        &self,
        community_id: &str,
        pseudonym: &PseudonymKey,
        name: String,
    ) -> bool {
        let key = (community_id.to_owned(), pseudonym.0);
        let mut cards = self.member_cards.write();
        if cards.get(&key) == Some(&name) {
            return false;
        }
        cards.insert(key, name);
        true
    }
}

#[cfg(test)]
mod tests {
    use rekindle_types::id::PseudonymKey;

    use super::CommunityStore;

    /// The first card read and a rename are news; a re-read of the same
    /// name is not, so it announces nothing.
    #[test]
    fn a_card_name_is_news_when_first_read_and_when_changed() {
        let store = CommunityStore::new(std::sync::Arc::default());
        let bea = PseudonymKey([2; 32]);
        assert!(
            store.note_member_card_name("c", &bea, "Bea".into()),
            "first read"
        );
        assert!(
            !store.note_member_card_name("c", &bea, "Bea".into()),
            "same name"
        );
        assert!(
            store.note_member_card_name("c", &bea, "Bee".into()),
            "renamed"
        );
        assert_eq!(store.member_card_name("c", &bea).as_deref(), Some("Bee"));
        assert!(
            store.note_member_card_name("d", &bea, "Bee".into()),
            "per community"
        );
    }
}
