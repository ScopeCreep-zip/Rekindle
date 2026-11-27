//! The names on members' verified cards (`M`/0, signed by the pseudonym,
//! its owner pinned to the member's `MemberJoined`) as [`CommunityStore`]
//! holds them: the only source of a member's name in membership events
//! (plan E3.2 F9-2).

use rekindle_governance_runtime::CardText;
use rekindle_types::id::PseudonymKey;

use super::CommunityStore;

impl CommunityStore {
    /// What `pseudonym`'s verified card says.
    #[must_use]
    pub fn member_card(&self, community_id: &str, pseudonym: &PseudonymKey) -> Option<CardText> {
        self.member_cards
            .read()
            .get(&(community_id.to_owned(), pseudonym.0))
            .cloned()
    }

    /// The display name on `pseudonym`'s verified card, when it names one.
    #[must_use]
    pub fn member_card_name(&self, community_id: &str, pseudonym: &PseudonymKey) -> Option<String> {
        self.member_card(community_id, pseudonym)?.display_name
    }

    /// Replace every held card with `cards`, the ones cached on the
    /// identity's `community_members` rows (a login's restore).
    pub fn restore_member_cards(
        &self,
        cards: impl IntoIterator<Item = (String, PseudonymKey, CardText)>,
    ) {
        let mut held = self.member_cards.write();
        held.clear();
        held.extend(cards.into_iter().map(|(c, p, card)| ((c, p.0), card)));
    }

    /// Remember what `pseudonym`'s verified card says: whether it is news,
    /// the first card held for the member or a changed one (the
    /// `MemberCardChanged` gate).
    #[must_use]
    pub fn note_member_card(
        &self,
        community_id: &str,
        pseudonym: &PseudonymKey,
        card: CardText,
    ) -> bool {
        let key = (community_id.to_owned(), pseudonym.0);
        let mut cards = self.member_cards.write();
        if cards.get(&key) == Some(&card) {
            return false;
        }
        cards.insert(key, card);
        true
    }
}

#[cfg(test)]
#[path = "cards_tests.rs"]
mod tests;
