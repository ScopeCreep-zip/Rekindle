//! Our per-community profile and the community's policy on the Host (plan
//! E3.1 D4), both modes.
//!
//! The profile (bio, pronouns, theme color, badges, avatar and banner
//! hashes) is ours alone: it is kept on the community in the store. Our
//! per-community name and custom status are our card's (`M`/0, plan E3.4
//! S6): set here, written to the card, read back verified into the card
//! cache, and announced as `MemberCardChanged`. The identity's name is
//! never written to a community.
//! The policy is a `CommunityPolicy` governance entry: readers validate
//! `MANAGE_COMMUNITY` at merge, and the merge projects it into the store.

use rekindle_governance::state::CommunityPolicyState;
use rekindle_governance_runtime::member_card::{refresh_own_card, write_card, OwnCardText};
use rekindle_governance_runtime::{CardText, GovernanceRuntimeDeps, GovernanceRuntimeError};
use rekindle_types::dto::community::CommunityPolicyView;
use rekindle_types::governance::GovernanceEntry;
use rekindle_types::id::PseudonymKey;
use rekindle_types::permissions;
use rekindle_types::subscription_events::{MembershipEvent, SubscriptionEvent};

use super::HostGovernance;

/// Our profile in one community, as the user set it.
#[derive(Debug, Clone, Default)]
pub(crate) struct ProfileEdit {
    pub bio: Option<String>,
    pub pronouns: Option<String>,
    pub theme_color: Option<u32>,
    pub badges: Vec<String>,
    pub avatar_ref: Option<String>,
    pub banner_ref: Option<String>,
    /// Our card's custom status (plan E3.4 S6).
    pub custom_status: Option<String>,
}

/// A new community policy.
#[derive(Debug, Clone)]
pub(crate) struct PolicyEdit {
    pub policy_text: Option<String>,
    pub max_joins_per_interval: u32,
    pub join_interval_seconds: u32,
}

impl HostGovernance {
    /// Make `edit` our profile in `community`; its custom status goes to
    /// our card.
    ///
    /// # Errors
    /// The community is not joined, or the card write failed.
    pub(crate) async fn update_profile(
        &self,
        community: &str,
        edit: ProfileEdit,
    ) -> Result<(), String> {
        {
            let mut map = self.ctx.communities.map().write();
            let c = map.get_mut(community).ok_or("unknown community")?;
            c.my_bio = edit.bio;
            c.my_pronouns = edit.pronouns;
            c.my_theme_color = edit.theme_color;
            c.my_badges = edit.badges;
            c.my_avatar_ref = edit.avatar_ref;
            c.my_banner_ref = edit.banner_ref;
        }
        let mut text = self.own_card_text(community);
        text.custom_status = edit.custom_status.filter(|s| !s.is_empty());
        self.publish_own_card(community, &text).await
    }

    /// Make `name` our name in `community` (`""` clears it: readers show
    /// our short pseudonym fingerprint), on our card (plan E3.4 S6).
    ///
    /// # Errors
    /// The community is not joined, or the card write failed.
    pub(crate) async fn set_display_name(&self, community: &str, name: &str) -> Result<(), String> {
        let mut text = self.own_card_text(community);
        let name = name.trim();
        text.display_name = (!name.is_empty()).then(|| name.to_owned());
        self.publish_own_card(community, &text).await
    }

    /// What our card says now, from the card cache.
    fn own_card_text(&self, community: &str) -> OwnCardText {
        let card = GovernanceRuntimeDeps::my_pseudonym(self, community)
            .and_then(|me| self.ctx.communities.member_card(community, &me))
            .unwrap_or_default();
        OwnCardText {
            display_name: card.display_name,
            custom_status: card.custom_status,
        }
    }

    /// Write `text` to our card, then read it back into the card cache.
    async fn publish_own_card(&self, community: &str, text: &OwnCardText) -> Result<(), String> {
        let keys = self.own_record_keys(community)?;
        let hpke = {
            let guard = self.ctx.vault.lock();
            let vault = guard.as_ref().ok_or("no identity is unlocked")?;
            rekindle_vault::typed::community_keys::load_member_hpke_secret(vault, community)
                .map_err(|e| format!("member hpke secret: {e}"))?
                .ok_or("no member hpke secret is stored")?
        };
        let port = self.record_port().map_err(|e| e.to_string())?;
        write_card(&*port, &keys.own(community), text, &hpke).await?;
        refresh_own_card(self, community).await
    }

    /// Note a verified card; news goes to every frontend as
    /// `MemberCardChanged` (plan E3.2 F9-2), its name never empty: the
    /// card's, else the short pseudonym fingerprint (plan E3.4 S6).
    pub(super) fn note_member_card_impl(
        &self,
        community: &str,
        pseudonym: &PseudonymKey,
        card: CardText,
    ) {
        if !self
            .ctx
            .communities
            .note_member_card(community, pseudonym, card.clone())
        {
            return;
        }
        self.ctx.publish_event(SubscriptionEvent::Membership(
            MembershipEvent::MemberCardChanged {
                community: community.to_owned(),
                pseudonym: pseudonym.to_hex(),
                display_name: shown_name(card.display_name, pseudonym),
                custom_status: card.custom_status,
            },
        ));
    }

    /// `community`'s merged policy; the defaults before any merged.
    ///
    /// # Errors
    /// We lack `VIEW_CHANNELS`.
    pub(crate) fn community_policy(
        &self,
        community: &str,
    ) -> Result<CommunityPolicyView, GovernanceRuntimeError> {
        self.require_permission(community, permissions::VIEW_CHANNELS)?;
        Ok(policy_view(
            self.governance_state_impl(community)
                .and_then(|s| s.community_policy),
        ))
    }

    /// Write `edit` as `community`'s policy.
    ///
    /// # Errors
    /// We lack `MANAGE_COMMUNITY`, or the entry could not be written.
    pub(crate) async fn set_community_policy(
        &self,
        community: &str,
        edit: PolicyEdit,
    ) -> Result<(), GovernanceRuntimeError> {
        self.require_permission(community, permissions::MANAGE_COMMUNITY)?;
        rekindle_governance_runtime::apply::write_entry(self, community, policy_entry(edit))
            .await
            .map(drop)
    }
}

/// The name a community renders for `pseudonym`: its card's, else its
/// short pseudonym fingerprint; never empty (plan E3.4 S6).
pub(crate) fn shown_name(card_name: Option<String>, pseudonym: &PseudonymKey) -> String {
    card_name
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| rekindle_crypto::identity::short_key_fingerprint(&pseudonym.0))
}

/// The governance entry that makes `edit` the policy.
fn policy_entry(edit: PolicyEdit) -> GovernanceEntry {
    GovernanceEntry::CommunityPolicy {
        policy_text: edit.policy_text,
        max_joins_per_interval: edit.max_joins_per_interval,
        join_interval_seconds: edit.join_interval_seconds,
    }
}

/// The merged policy as the frontends read it; the defaults before any.
fn policy_view(policy: Option<CommunityPolicyState>) -> CommunityPolicyView {
    policy.map_or(
        CommunityPolicyView {
            policy_text: None,
            max_joins_per_interval: CommunityPolicyState::DEFAULT_MAX_JOINS_PER_INTERVAL,
            join_interval_seconds: CommunityPolicyState::DEFAULT_JOIN_INTERVAL_SECONDS,
        },
        |p| CommunityPolicyView {
            policy_text: p.policy_text,
            max_joins_per_interval: p.max_joins_per_interval,
            join_interval_seconds: p.join_interval_seconds,
        },
    )
}

#[cfg(test)]
mod tests {
    //! A policy set is the policy read back, through the governance merge.

    use rekindle_governance::fixture::{at, merge_entries, Stamped};
    use rekindle_types::id::PseudonymKey;

    use super::*;

    /// A member whose card names nobody renders as its short pseudonym
    /// fingerprint, stated by the Host, never empty and never another name
    /// (plan E3.4 S6); a card name renders as itself.
    #[test]
    fn an_unnamed_member_renders_as_its_short_fingerprint() {
        let me = PseudonymKey([0xab; 32]);
        assert_eq!(shown_name(None, &me), "abab abab");
        assert_eq!(shown_name(Some(String::new()), &me), "abab abab");
        assert_eq!(shown_name(Some("Kit".into()), &me), "Kit");
    }

    fn edit() -> PolicyEdit {
        PolicyEdit {
            policy_text: Some("Be kind".into()),
            max_joins_per_interval: 5,
            join_interval_seconds: 120,
        }
    }

    fn meta(lamport: u64) -> Stamped {
        at(
            lamport,
            GovernanceEntry::CommunityMeta {
                name: Some("C".into()),
                description: None,
                icon_hash: None,
                banner_hash: None,
            },
        )
    }

    #[test]
    fn the_defaults_are_read_before_any_policy() {
        let view = policy_view(None);
        assert_eq!(view.policy_text, None);
        assert_eq!(
            view.max_joins_per_interval,
            CommunityPolicyState::DEFAULT_MAX_JOINS_PER_INTERVAL
        );
        assert_eq!(
            view.join_interval_seconds,
            CommunityPolicyState::DEFAULT_JOIN_INTERVAL_SECONDS
        );
    }

    #[test]
    fn a_policy_set_is_read_back() {
        let creator = PseudonymKey([1; 32]);
        let state = merge_entries(
            &creator,
            &[(creator.clone(), vec![meta(1), at(2, policy_entry(edit()))])],
        );
        let view = policy_view(state.community_policy);
        assert_eq!(view.policy_text.as_deref(), Some("Be kind"));
        assert_eq!(view.max_joins_per_interval, 5);
        assert_eq!(view.join_interval_seconds, 120);
    }

    #[test]
    fn a_policy_without_manage_community_is_not_read() {
        let creator = PseudonymKey([1; 32]);
        let member = PseudonymKey([2; 32]);
        let state = merge_entries(
            &creator,
            &[
                (creator.clone(), vec![meta(1)]),
                (member, vec![at(2, policy_entry(edit()))]),
            ],
        );
        assert_eq!(policy_view(state.community_policy).policy_text, None);
    }
}
