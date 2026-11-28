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

use rekindle_codec::member_record::{card::decode_member_card, RecordScope};
use rekindle_governance::state::CommunityPolicyState;
use rekindle_governance_runtime::member_card::{
    card_of, refresh_own_card, write_card, OwnCardText,
};
use rekindle_governance_runtime::{CardText, GovernanceRuntimeDeps, GovernanceRuntimeError};
use rekindle_types::dht_layout::member_record::CARD;
use rekindle_types::dto::community::CommunityPolicyView;
use rekindle_types::governance::GovernanceEntry;
use rekindle_types::id::PseudonymKey;
use rekindle_types::permissions;
use rekindle_types::presence::{GameInfo, PresenceSharingPolicy};
use rekindle_types::subscription_events::{
    GameActivity, MembershipEvent, PresenceEvent, PresenceSnapshot, SubscriptionEvent,
};

use super::HostGovernance;
use crate::host::community::CommunityStore;

/// A new community policy.
#[derive(Debug, Clone)]
pub(crate) struct PolicyEdit {
    pub policy_text: Option<String>,
    pub max_joins_per_interval: u32,
    pub join_interval_seconds: u32,
}

impl HostGovernance {
    /// What our card says now: its name and custom status from the card
    /// cache, and the game we share there now ([`Self::shared_card_game`]).
    pub(super) fn own_card_text(&self, community: &str) -> OwnCardText {
        let card = self.own_cached_card(community).unwrap_or_default();
        OwnCardText {
            display_name: card.display_name,
            custom_status: card.custom_status,
            game: self.shared_card_game(community),
        }
    }

    /// Our card as the card cache holds it (what a verified read returned).
    fn own_cached_card(&self, community: &str) -> Option<CardText> {
        GovernanceRuntimeDeps::my_pseudonym(self, community)
            .and_then(|me| self.ctx.communities.member_card(community, &me))
    }

    /// The game our card carries in `community`: the game we show, through
    /// the community's sharing consent (`presence::card_game::shared_game`).
    fn shared_card_game(&self, community: &str) -> Option<GameInfo> {
        let policy = {
            let map = self.ctx.communities.map().read();
            map.get(community).map(|c| c.presence_policy.clone())
        }?;
        crate::host::adapters::presence::card_game::shared_game(
            &self.ctx.own_presence.view(),
            &policy,
        )
    }

    /// Store the local consent and wake the beat and card writers after the
    /// edit commits.
    ///
    /// # Errors
    /// The identity is locked, the community is missing or the write failed.
    pub(crate) async fn edit_presence_policy(
        &self,
        community: &str,
        update: impl FnOnce(&mut PresenceSharingPolicy) + Send + 'static,
    ) -> Result<(), String> {
        let (owner, db) = self.owner_and_db().ok_or("no identity is unlocked")?;
        let changed = crate::host::session::record_presence_policy(
            &db,
            owner,
            self.ctx.communities.clone(),
            community.to_owned(),
            update,
        )
        .await?;
        if changed {
            crate::host::adapters::presence::view::location_policy_changed(&self.ctx, community);
            self.ctx.own_presence.card_game.notify_one();
            crate::host::adapters::presence::refresh_beat(&self.ctx, community);
        }
        Ok(())
    }

    /// Rewrite our card in every community we beat in whose card cache
    /// shows another game than the one we share there now (plan E3.4 S6;
    /// the coalesced writer, `presence::card_game::run`, calls it).
    pub(crate) async fn rewrite_moved_card_games(&self) {
        for community in self.sync_targets() {
            if self.own_departed(&community) {
                continue;
            }
            // The card also carries our name and custom status. Read those
            // fields after any profile edit that is writing the same card.
            let lock = self.ctx.communities.profile_write_lock(&community);
            let _held = lock.lock().await;
            let shared = self.shared_card_game(&community);
            if self.own_cached_card(&community).and_then(|c| c.game) == shared {
                continue;
            }
            let text = self.own_card_text(&community);
            if let Err(error) = self.publish_own_card(&community, &text).await {
                tracing::warn!(community, %error, "card game not rewritten");
            }
        }
    }

    /// Write `text` to our card, then read it back into the card cache.
    pub(super) async fn publish_own_card(
        &self,
        community: &str,
        text: &OwnCardText,
    ) -> Result<(), String> {
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

    /// Note a verified card; news is cached on the member's
    /// `community_members` row (the row's only name writer, so the name
    /// survives a restart) and goes to every frontend as
    /// `MemberCardChanged` (plan E3.2 F9-2), its name never empty: the
    /// card's, else the short pseudonym fingerprint (plan E3.4 S6).
    pub(super) fn note_member_card_impl(
        &self,
        community: &str,
        pseudonym: &PseudonymKey,
        card: CardText,
    ) {
        let before = self.ctx.communities.member_card(community, pseudonym);
        if !self
            .ctx
            .communities
            .note_member_card(community, pseudonym, card.clone())
        {
            return;
        }
        let ours = GovernanceRuntimeDeps::my_pseudonym(self, community).as_ref() == Some(pseudonym);
        if let Some(snapshot) = game_moved(before.as_ref(), &card).filter(|_| !ours) {
            self.ctx.publish_event(SubscriptionEvent::Presence(
                PresenceEvent::CommunityMemberChanged {
                    community: community.to_owned(),
                    pseudonym: pseudonym.to_hex(),
                    snapshot,
                },
            ));
        }
        if let Some((owner, db)) = self.owner_and_db() {
            let (cid, pk) = (community.to_owned(), pseudonym.to_hex());
            let (name, status) = (card.display_name.clone(), card.custom_status.clone());
            let joined_at = self.ctx.clock.now_secs().cast_signed();
            db.fire("member card", move |conn| {
                rekindle_db::repo::members::set_card(
                    conn,
                    &owner,
                    &cid,
                    &pk,
                    (name.as_deref(), status.as_deref()),
                    joined_at,
                )
            });
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

    /// A ring change naming the card (subkey 0) of `record` (plan E3.4
    /// "Card": a card is re-read only when its seq advances, and a watch
    /// notification is that advance): read it through the ring lease,
    /// forced through the hinted read (a ValueChange carries only its first
    /// subkey's value, veilid-core 0.5.8 `veilid_state.rs:334-336`), verify
    /// it under the record's member and note it. A card that does not
    /// verify is not noted.
    pub(super) async fn ring_card_moved(&self, community: &str, record: &str) {
        let lease = {
            let map = self.ctx.communities.map().read();
            map.get(community)
                .and_then(|c| c.watch_ring.get(record).copied())
        };
        let (Some(lease), Some(author)) = (lease, self.author_of_record(community, record)) else {
            return;
        };
        let Ok(Some((_, bytes))) = self.read_hinted_value(lease, CARD).await else {
            return;
        };
        let at = RecordScope {
            community_id: community,
            member_record: record,
        };
        match decode_member_card(&bytes, at, &author.0) {
            Ok(signed) => self.note_member_card_impl(community, &author, card_of(&signed)),
            Err(error) => tracing::debug!(community, %error, "ring member card refused"),
        }
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

impl HostGovernance {
    /// How the member list names this member from the Host's verified card view.
    #[must_use]
    pub fn member_shown_name(&self, community_id: &str, member_hex: &str) -> String {
        member_shown_name(&self.ctx.communities, community_id, member_hex)
    }
}

/// The name a community renders for `pseudonym`: its card's, else its
/// short pseudonym fingerprint; never empty (plan E3.4 S6).
#[must_use]
pub fn shown_name(card_name: Option<String>, pseudonym: &PseudonymKey) -> String {
    card_name
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| rekindle_crypto::identity::short_key_fingerprint(&pseudonym.0))
}

/// The name `community_id` renders for the member whose pseudonym is
/// `member_hex`: [`shown_name`] over their verified card, as the member
/// list names its rows and so as a message's sender header shows them. A
/// typist and a forward's original author (plan E3.3) are named by it.
pub(crate) fn member_shown_name(
    communities: &CommunityStore,
    community_id: &str,
    member_hex: &str,
) -> String {
    let pseudonym = PseudonymKey::from_hex_lossy(member_hex);
    shown_name(
        communities.member_card_name(community_id, &pseudonym),
        &pseudonym,
    )
}

/// The member-list observation a card read makes when its game moved from
/// `before` (plan E3.4 S6): game only (the beat owns the status, `None` is
/// "not observed"), built as the gossip `PresenceUpdate` receiver built it
/// (`GameActivity::from_parts` of the name and numeric id, no elapsed time,
/// server address or icon), so a stopped game reads `Idle` and clears.
fn game_moved(before: Option<&CardText>, after: &CardText) -> Option<PresenceSnapshot> {
    if before.and_then(|c| c.game.as_ref()) == after.game.as_ref() {
        return None;
    }
    let game = after.game.as_ref();
    Some(PresenceSnapshot::game(GameActivity::from_parts(
        game.map(|g| g.game_name.clone()),
        game.and_then(|g| g.game_id.as_deref()?.parse().ok()),
        None,
        None,
        None,
    )))
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
#[path = "community_profile_tests.rs"]
mod tests;
