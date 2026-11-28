//! The events' control messages off the mesh (plan E3.1 E4), on both
//! hosts, and the rows they and our own commands store. Readers validate:
//! an `EventCreated` must name its sender as creator and the sender must
//! hold `CREATE_EVENTS`; an update or a deletion needs `MANAGE_EVENTS`; an
//! `EventRsvpChanged` is accepted only for the sender's own pseudonym.

use rekindle_codec::community::envelope::ControlPayload;
use rekindle_governance::permissions::{compute_permissions, has_capability};
use rekindle_governance_runtime::GovernanceRuntimeError;
use rekindle_types::event::{is_event_id, EventInfo, RsvpStatus};
use rekindle_types::id::PseudonymKey;
use rekindle_types::permissions;

use super::community_events::adapter;
use super::HostGovernance;

impl HostGovernance {
    /// Handle one event control message from `sender_pseudonym`: `true`
    /// when it was accepted and stored, so the shell shows it.
    pub async fn on_event_control(
        &self,
        community_id: &str,
        sender_pseudonym: &str,
        payload: &ControlPayload,
    ) -> bool {
        let stored = match payload {
            ControlPayload::EventCreated { event } => {
                if !is_event_id(&event.id)
                    || event.creator_pseudonym != sender_pseudonym
                    || !self.sender_holds(
                        community_id,
                        sender_pseudonym,
                        permissions::CREATE_EVENTS,
                    )
                {
                    tracing::warn!(community = %community_id, "event dropped: malformed, not the sender's, or no CREATE_EVENTS");
                    return false;
                }
                self.store_event_row(community_id, event).await
            }
            ControlPayload::EventUpdated { event } => {
                if !is_event_id(&event.id)
                    || !self.sender_holds(
                        community_id,
                        sender_pseudonym,
                        permissions::MANAGE_EVENTS,
                    )
                {
                    tracing::warn!(community = %community_id, "event update dropped: malformed or no MANAGE_EVENTS");
                    return false;
                }
                self.store_event_row(community_id, event).await
            }
            ControlPayload::EventDeleted { event_id } => {
                if !is_event_id(event_id)
                    || !self.sender_holds(
                        community_id,
                        sender_pseudonym,
                        permissions::MANAGE_EVENTS,
                    )
                {
                    tracing::warn!(community = %community_id, "event deletion dropped: malformed or no MANAGE_EVENTS");
                    return false;
                }
                self.remove_event_row(community_id, event_id).await
            }
            ControlPayload::EventRsvpChanged {
                event_id,
                pseudonym_key,
                status,
            } => {
                if !is_event_id(event_id) || pseudonym_key != sender_pseudonym {
                    tracing::warn!(community = %community_id, "RSVP dropped: malformed or not the sender's own");
                    return false;
                }
                let wire = RsvpStatus::from_request(status).as_wire_str();
                if let Some(cs) = self.ctx.communities.map().write().get_mut(community_id) {
                    super::event_rsvps::apply(cs, event_id, pseudonym_key, wire);
                }
                self.store_rsvp(community_id, event_id, pseudonym_key, wire)
                    .await
            }
            _ => return false,
        };
        match stored {
            Ok(()) => true,
            Err(e) => {
                tracing::warn!(community = %community_id, error = %e, "event change not stored");
                false
            }
        }
    }

    /// Whether `sender_pseudonym` holds `perm` in the merged state.
    pub(super) fn sender_holds(
        &self,
        community_id: &str,
        sender_pseudonym: &str,
        perm: u64,
    ) -> bool {
        let Some(state) = self.governance_state_impl(community_id) else {
            return false;
        };
        let Some(sender) = hex::decode(sender_pseudonym)
            .ok()
            .and_then(|b| <[u8; 32]>::try_from(b).ok())
        else {
            return false;
        };
        let effective = compute_permissions(
            &PseudonymKey(sender),
            None,
            &state,
            self.ctx.clock.now_secs(),
        );
        has_capability(effective, perm)
    }

    /// Store `event`'s row.
    pub(super) async fn store_event_row(
        &self,
        community_id: &str,
        event: &EventInfo,
    ) -> Result<(), GovernanceRuntimeError> {
        let owner = self
            .ctx
            .require_owner()
            .map_err(|_| adapter("no identity is unlocked"))?;
        let db = self.ctx.db.current().map_err(adapter)?;
        let (cid, event) = (community_id.to_owned(), event.clone());
        db.call(move |c| rekindle_db::repo::community_events::upsert(c, &owner, &cid, &event))
            .await
            .map_err(|e| adapter(format!("event not stored: {e}")))
    }

    /// Drop the row of `event_id` and its RSVPs.
    pub(super) async fn remove_event_row(
        &self,
        community_id: &str,
        event_id: &str,
    ) -> Result<(), GovernanceRuntimeError> {
        let owner = self
            .ctx
            .require_owner()
            .map_err(|_| adapter("no identity is unlocked"))?;
        let db = self.ctx.db.current().map_err(adapter)?;
        let (cid, id) = (community_id.to_owned(), event_id.to_owned());
        db.call(move |c| rekindle_db::repo::community_events::remove(c, &owner, &cid, &id))
            .await
            .map_err(|e| adapter(format!("event not deleted: {e}")))
    }

    /// Store one member's answer to `event_id`.
    pub(super) async fn store_rsvp(
        &self,
        community_id: &str,
        event_id: &str,
        pseudonym_key: &str,
        status: &str,
    ) -> Result<(), GovernanceRuntimeError> {
        let owner = self
            .ctx
            .require_owner()
            .map_err(|_| adapter("no identity is unlocked"))?;
        let db = self.ctx.db.current().map_err(adapter)?;
        let (cid, id, who, status) = (
            community_id.to_owned(),
            event_id.to_owned(),
            pseudonym_key.to_owned(),
            status.to_owned(),
        );
        db.call(move |c| {
            rekindle_db::repo::community_events::upsert_rsvp(c, &owner, &cid, &id, &who, &status)
        })
        .await
        .map_err(|e| adapter(format!("RSVP not stored: {e}")))
    }
}
