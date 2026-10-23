//! A community's scheduled events on the Host (plan E3.1 E4), both modes.
//!
//! An event is a member-authored `EventCreated` governance entry, written
//! again in full (LWW per event id) by an edit or a cancel, and retired by
//! `EventArchived`; an answer is the member's own `EventRsvp` entry. Each
//! write is also sent to the mesh as its control message (the fast path)
//! and stored here at once. The list answers merged governance laid over
//! the stored rows, which carry what governance does not (`maxAttendees`,
//! `createdAt`) and what gossip brought before the next merge.

use rekindle_codec::community::envelope::{CommunityEnvelope, ControlPayload};
use rekindle_governance_runtime::{GovernanceRuntimeDeps as _, GovernanceRuntimeError};
use rekindle_types::event::{
    EventEdit, EventInfo, EventRsvp, EventStatus, NewEvent, RsvpStatus,
    MAX_EVENT_DESCRIPTION_CHARS, MAX_EVENT_NAME_CHARS,
};
use rekindle_types::governance::GovernanceEntry;
use rekindle_types::id::{ChannelId, EventId, PseudonymKey};
use rekindle_types::permissions;
use rekindle_types::subscription_events::{SocialEvent, SubscriptionEvent};

use super::HostGovernance;

pub(super) fn adapter(what: impl std::fmt::Display) -> GovernanceRuntimeError {
    GovernanceRuntimeError::Adapter(what.to_string())
}

/// The `evt_<32 hex>` id of `id`.
pub(crate) fn event_key(id: &EventId) -> String {
    format!("evt_{}", hex::encode(id.0))
}

/// The `EventId` an `evt_<32 hex>` id names.
pub(super) fn event_id_of(key: &str) -> Option<EventId> {
    let bytes = hex::decode(key.strip_prefix("evt_")?).ok()?;
    Some(EventId(bytes.try_into().ok()?))
}

fn hex32(s: &str) -> Option<[u8; 32]> {
    hex::decode(s).ok()?.try_into().ok()
}

/// An event status's wire label.
pub(super) const fn status_label(status: EventStatus) -> &'static str {
    match status {
        EventStatus::Scheduled => "scheduled",
        EventStatus::Active => "active",
        EventStatus::Cancelled => "cancelled",
        EventStatus::Completed => "completed",
    }
}

fn status_of(label: &str) -> Result<EventStatus, GovernanceRuntimeError> {
    match label {
        "scheduled" => Ok(EventStatus::Scheduled),
        "active" => Ok(EventStatus::Active),
        "cancelled" => Ok(EventStatus::Cancelled),
        "completed" => Ok(EventStatus::Completed),
        other => Err(adapter(format!("unknown event status {other}"))),
    }
}

/// Refuse a title or description past the architecture §21 bounds.
fn check_bounds(title: &str, description: &str) -> Result<(), GovernanceRuntimeError> {
    if title.chars().count() > MAX_EVENT_NAME_CHARS {
        return Err(adapter(format!(
            "event name exceeds {MAX_EVENT_NAME_CHARS} characters"
        )));
    }
    if description.chars().count() > MAX_EVENT_DESCRIPTION_CHARS {
        return Err(adapter(format!(
            "event description exceeds {MAX_EVENT_DESCRIPTION_CHARS} characters"
        )));
    }
    Ok(())
}

/// The `EventCreated` entry that carries `event` in full.
fn entry_of(event: &EventInfo) -> Result<GovernanceEntry, GovernanceRuntimeError> {
    let channel_id = event
        .channel_id
        .as_deref()
        .map(|c| {
            hex::decode(c)
                .ok()
                .and_then(|b| <[u8; 16]>::try_from(b).ok())
                .map(ChannelId)
                .ok_or_else(|| adapter("channel id must be 32 hex digits"))
        })
        .transpose()?;
    Ok(GovernanceEntry::EventCreated {
        event_id: event_id_of(&event.id).ok_or_else(|| adapter("malformed event id"))?,
        name: event.title.clone(),
        description: Some(event.description.clone()),
        start_time: event.start_time,
        end_time: event.end_time,
        channel_id,
        cover_image_ref: event.cover_image_ref.clone(),
        creator_pseudonym: hex32(&event.creator_pseudonym).map(PseudonymKey),
        recurrence: event.recurrence.clone(),
        location: event.location.clone(),
        status: Some(status_of(&event.status)?),
    })
}

impl HostGovernance {
    /// Schedule `new` in `community_id`; answers the event's id.
    ///
    /// # Errors
    /// We lack `CREATE_EVENTS` (what readers check of an `EventCreated`),
    /// a bound is exceeded, or a write failed.
    pub(crate) async fn create_event(
        &self,
        community_id: &str,
        new: NewEvent,
    ) -> Result<String, GovernanceRuntimeError> {
        self.require_permission(community_id, permissions::CREATE_EVENTS)?;
        check_bounds(&new.title, &new.description)?;
        let creator = self
            .community_membership_impl(community_id)
            .and_then(|m| m.my_pseudonym_hex)
            .ok_or_else(|| GovernanceRuntimeError::PseudonymKeyMissing(community_id.into()))?;
        let mut id = [0u8; 16];
        self.ctx.rng.fill(&mut id);
        let event = EventInfo {
            id: event_key(&EventId(id)),
            title: new.title,
            description: new.description,
            creator_pseudonym: creator,
            start_time: new.start_time,
            end_time: new.end_time,
            channel_id: new.channel_id,
            max_attendees: new.max_attendees,
            created_at: self.ctx.clock.now_secs(),
            status: status_label(EventStatus::Scheduled).into(),
            rsvps: Vec::new(),
            cover_image_ref: new.cover_image_ref,
            recurrence: new.recurrence,
            location: new.location,
        };
        self.publish_event_change(community_id, event.clone(), true)
            .await?;
        Ok(event.id)
    }

    /// Apply `edit` to the event `event_id`.
    ///
    /// # Errors
    /// We lack `MANAGE_EVENTS`, the event is unknown, a bound is exceeded,
    /// or a write failed.
    pub(crate) async fn edit_event(
        &self,
        community_id: &str,
        event_id: &str,
        edit: EventEdit,
    ) -> Result<(), GovernanceRuntimeError> {
        self.require_permission(community_id, permissions::MANAGE_EVENTS)?;
        let mut event = self.event(community_id, event_id).await?;
        if let Some(title) = edit.title {
            event.title = title;
        }
        if let Some(description) = edit.description {
            event.description = description;
        }
        event.start_time = edit.start_time.unwrap_or(event.start_time);
        event.end_time = edit.end_time.or(event.end_time);
        event.channel_id = edit.channel_id.or(event.channel_id);
        event.max_attendees = edit.max_attendees.or(event.max_attendees);
        check_bounds(&event.title, &event.description)?;
        self.publish_event_change(community_id, event, false).await
    }

    /// Cancel the event `event_id`: kept, marked cancelled.
    ///
    /// # Errors
    /// We lack `MANAGE_EVENTS`, the event is unknown, or a write failed.
    pub(crate) async fn cancel_event(
        &self,
        community_id: &str,
        event_id: &str,
    ) -> Result<(), GovernanceRuntimeError> {
        self.require_permission(community_id, permissions::MANAGE_EVENTS)?;
        let mut event = self.event(community_id, event_id).await?;
        status_label(EventStatus::Cancelled).clone_into(&mut event.status);
        self.publish_event_change(community_id, event, false).await
    }

    /// Delete the event `event_id`: `EventArchived`, the row dropped and
    /// `EventDeleted` sent.
    ///
    /// # Errors
    /// We lack `MANAGE_EVENTS`, the id is malformed, or a write failed.
    pub(crate) async fn delete_event(
        &self,
        community_id: &str,
        event_id: &str,
    ) -> Result<(), GovernanceRuntimeError> {
        self.require_permission(community_id, permissions::MANAGE_EVENTS)?;
        let id = event_id_of(event_id).ok_or_else(|| adapter("malformed event id"))?;
        rekindle_governance_runtime::apply::write_entry(
            self,
            community_id,
            GovernanceEntry::EventArchived { event_id: id },
        )
        .await?;
        self.send_to_mesh(
            community_id,
            &CommunityEnvelope::Control(ControlPayload::EventDeleted {
                event_id: event_id.to_owned(),
            }),
        )?;
        self.remove_event_row(community_id, event_id).await?;
        self.ctx
            .publish_event(SubscriptionEvent::Social(SocialEvent::EventDeleted {
                community: community_id.to_owned(),
                event_id: event_id.to_owned(),
            }));
        Ok(())
    }

    /// Record our answer to `event_id`: our `EventRsvp` entry, our row,
    /// the attendee list, and `EventRsvpChanged` to the mesh and every
    /// frontend.
    ///
    /// # Errors
    /// We hold no pseudonym or lack `VIEW_CHANNELS`, the id is malformed,
    /// or a write failed.
    pub(crate) async fn set_event_rsvp(
        &self,
        community_id: &str,
        event_id: &str,
        status: &str,
    ) -> Result<(), GovernanceRuntimeError> {
        self.require_permission(community_id, permissions::VIEW_CHANNELS)?;
        let id = event_id_of(event_id).ok_or_else(|| adapter("malformed event id"))?;
        let rsvp = RsvpStatus::from_request(status);
        let me = self
            .community_membership_impl(community_id)
            .and_then(|m| m.my_pseudonym_hex)
            .ok_or_else(|| GovernanceRuntimeError::PseudonymKeyMissing(community_id.into()))?;
        rekindle_governance_runtime::apply::write_entry(
            self,
            community_id,
            GovernanceEntry::EventRsvp {
                event_id: id,
                status: rsvp,
            },
        )
        .await?;
        let wire = rsvp.as_wire_str();
        if let Some(cs) = self.ctx.communities.map().write().get_mut(community_id) {
            cs.my_event_rsvps
                .insert(event_id.to_owned(), wire.to_owned());
            super::event_rsvps::apply(cs, event_id, &me, wire);
        }
        self.store_rsvp(community_id, event_id, &me, wire).await?;
        self.send_to_mesh(
            community_id,
            &CommunityEnvelope::Control(ControlPayload::EventRsvpChanged {
                event_id: event_id.to_owned(),
                pseudonym_key: me.clone(),
                status: wire.to_owned(),
            }),
        )?;
        self.ctx
            .publish_event(SubscriptionEvent::Social(SocialEvent::EventRsvpChanged {
                community: community_id.to_owned(),
                event_id: event_id.to_owned(),
                pseudonym: me,
                rsvp_status: wire.to_owned(),
            }));
        Ok(())
    }

    /// The community's events by start time then id, each with its
    /// attendees.
    ///
    /// # Errors
    /// We lack `VIEW_CHANNELS`, or the read failed.
    pub(crate) async fn events(
        &self,
        community_id: &str,
    ) -> Result<Vec<EventInfo>, GovernanceRuntimeError> {
        self.require_permission(community_id, permissions::VIEW_CHANNELS)?;
        let owner = self
            .ctx
            .require_owner()
            .map_err(|_| adapter("no identity is unlocked"))?;
        let db = self.ctx.db.current().map_err(adapter)?;
        let cid = community_id.to_owned();
        let mut events = db
            .call(move |c| rekindle_db::repo::community_events::list(c, &owner, &cid))
            .await
            .map_err(|e| adapter(format!("events not read: {e}")))?;
        let map = self.ctx.communities.map().read();
        let Some(cs) = map.get(community_id) else {
            return Ok(events);
        };
        if let Some(gov) = &cs.governance_state {
            for (id, merged) in &gov.events {
                let key = event_key(id);
                let pos = events.iter().position(|e| e.id == key);
                let stored = pos.map(|i| events.remove(i));
                events.push(EventInfo {
                    id: key,
                    title: merged.name.clone(),
                    description: merged.description.clone().unwrap_or_default(),
                    creator_pseudonym: merged
                        .creator_pseudonym
                        .as_ref()
                        .map(|p| hex::encode(p.0))
                        .unwrap_or_default(),
                    start_time: merged.start_time,
                    end_time: merged.end_time,
                    channel_id: merged.channel_id.map(|c| hex::encode(c.0)),
                    max_attendees: stored.as_ref().and_then(|s| s.max_attendees),
                    created_at: stored.as_ref().map_or(merged.start_time, |s| s.created_at),
                    status: status_label(merged.status).into(),
                    rsvps: Vec::new(),
                    cover_image_ref: merged.cover_image_ref.clone(),
                    recurrence: merged.recurrence.clone(),
                    location: merged.location.clone(),
                });
            }
        }
        for event in &mut events {
            event.rsvps = cs
                .event_rsvps_by_event
                .get(&event.id)
                .cloned()
                .unwrap_or_default();
        }
        events.sort_by(|a, b| (a.start_time, &a.id).cmp(&(b.start_time, &b.id)));
        Ok(events)
    }

    /// The attendees of `event_id`, by pseudonym.
    ///
    /// # Errors
    /// We lack `VIEW_CHANNELS`.
    pub(crate) fn event_attendees(
        &self,
        community_id: &str,
        event_id: &str,
    ) -> Result<Vec<EventRsvp>, GovernanceRuntimeError> {
        self.require_permission(community_id, permissions::VIEW_CHANNELS)?;
        Ok(self
            .ctx
            .communities
            .map()
            .read()
            .get(community_id)
            .and_then(|cs| cs.event_rsvps_by_event.get(event_id).cloned())
            .unwrap_or_default())
    }

    /// The event `event_id` as the list answers it.
    async fn event(
        &self,
        community_id: &str,
        event_id: &str,
    ) -> Result<EventInfo, GovernanceRuntimeError> {
        self.events(community_id)
            .await?
            .into_iter()
            .find(|e| e.id == event_id)
            .ok_or_else(|| adapter("event not found"))
    }

    /// Write `event` in full: its `EventCreated` entry, its row, the
    /// control message to the mesh and the change to every frontend.
    async fn publish_event_change(
        &self,
        community_id: &str,
        event: EventInfo,
        created: bool,
    ) -> Result<(), GovernanceRuntimeError> {
        rekindle_governance_runtime::apply::write_entry(self, community_id, entry_of(&event)?)
            .await?;
        self.store_event_row(community_id, &event).await?;
        let (payload, social) = if created {
            (
                ControlPayload::EventCreated {
                    event: event.clone(),
                },
                SocialEvent::EventCreated {
                    community: community_id.to_owned(),
                    event: Box::new(event),
                },
            )
        } else {
            (
                ControlPayload::EventUpdated {
                    event: event.clone(),
                },
                SocialEvent::EventUpdated {
                    community: community_id.to_owned(),
                    event: Box::new(event),
                },
            )
        };
        self.send_to_mesh(community_id, &CommunityEnvelope::Control(payload))?;
        self.ctx.publish_event(SubscriptionEvent::Social(social));
        Ok(())
    }
}
