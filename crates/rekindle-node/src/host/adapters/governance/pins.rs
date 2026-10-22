//! A channel's pinned messages and the community's pinned attachments
//! (plan E3.1 E3b), on both hosts.
//!
//! A message pin or unpin is a `MessagePinned` / `MessageUnpinned`
//! control message on the community mesh, stored by the sender at once and
//! by each peer as it accepts the message. Readers validate: a peer stores
//! a change only when the sender holds `MANAGE_MESSAGES` in the merged
//! governance state, and a pin only when it names the sender as pinner.
//! An attachment pin is a `GovernanceEntry::AttachmentPinned` (LWW per
//! attachment), so every peer's merged state agrees on what eviction skips.

use rekindle_codec::community::envelope::{CommunityEnvelope, ControlPayload};
use rekindle_db::repo::channel_pins::PinRow;
use rekindle_governance_runtime::{GovernanceRuntimeDeps, GovernanceRuntimeError};
use rekindle_types::dto::community::PinnedMessageView;
use rekindle_types::governance::GovernanceEntry;
use rekindle_types::permissions;

use super::HostGovernance;

/// A change to the stored pins.
enum PinChange {
    Pin(PinRow),
    Unpin {
        channel_id: String,
        message_id: String,
    },
}

fn attachment_id(hex_id: &str) -> Result<[u8; 16], GovernanceRuntimeError> {
    hex::decode(hex_id)
        .ok()
        .and_then(|bytes| <[u8; 16]>::try_from(bytes).ok())
        .ok_or_else(|| GovernanceRuntimeError::Adapter(format!("invalid attachment id {hex_id:?}")))
}

impl HostGovernance {
    /// Pin `message_id` in `channel_id`: stored here and sent to the mesh.
    ///
    /// # Errors
    /// We lack `MANAGE_MESSAGES` or a pseudonym, or the message or the row
    /// could not go.
    pub(crate) async fn pin_message(
        &self,
        community_id: &str,
        channel_id: &str,
        message_id: &str,
    ) -> Result<(), GovernanceRuntimeError> {
        self.require_permission(community_id, permissions::MANAGE_MESSAGES)?;
        let pinned_by = self
            .community_membership_impl(community_id)
            .and_then(|m| m.my_pseudonym_hex)
            .ok_or_else(|| GovernanceRuntimeError::PseudonymKeyMissing(community_id.into()))?;
        let envelope = CommunityEnvelope::Control(ControlPayload::MessagePinned {
            channel_id: channel_id.to_owned(),
            message_id: message_id.to_owned(),
            pinned_by: pinned_by.clone(),
        });
        self.send_to_mesh(community_id, &envelope)?;
        let row = PinRow {
            message_id: message_id.to_owned(),
            channel_id: channel_id.to_owned(),
            pinned_by,
            pinned_at: self.ctx.clock.now_secs(),
        };
        self.store_pin(community_id, PinChange::Pin(row)).await?;
        Ok(())
    }

    /// Unpin `message_id` in `channel_id`: dropped here and sent to the
    /// mesh.
    ///
    /// # Errors
    /// We lack `MANAGE_MESSAGES`, or the message or the delete could not go.
    pub(crate) async fn unpin_message(
        &self,
        community_id: &str,
        channel_id: &str,
        message_id: &str,
    ) -> Result<(), GovernanceRuntimeError> {
        self.require_permission(community_id, permissions::MANAGE_MESSAGES)?;
        let envelope = CommunityEnvelope::Control(ControlPayload::MessageUnpinned {
            channel_id: channel_id.to_owned(),
            message_id: message_id.to_owned(),
        });
        self.send_to_mesh(community_id, &envelope)?;
        let change = PinChange::Unpin {
            channel_id: channel_id.to_owned(),
            message_id: message_id.to_owned(),
        };
        self.store_pin(community_id, change).await?;
        Ok(())
    }

    /// The channel's pins, oldest first.
    ///
    /// # Errors
    /// We lack `VIEW_CHANNELS`, or the read failed.
    pub(crate) async fn channel_pins(
        &self,
        community_id: &str,
        channel_id: &str,
    ) -> Result<Vec<PinnedMessageView>, GovernanceRuntimeError> {
        self.require_permission(community_id, permissions::VIEW_CHANNELS)?;
        let (owner, db) = self.owner_db()?;
        let (community, channel) = (community_id.to_owned(), channel_id.to_owned());
        let rows = db
            .call(move |conn| {
                rekindle_db::repo::channel_pins::list(conn, &owner, &community, &channel)
            })
            .await
            .map_err(|e| GovernanceRuntimeError::Adapter(format!("pins not read: {e}")))?;
        Ok(rows
            .into_iter()
            .map(|row| PinnedMessageView {
                message_id: row.message_id,
                channel_id: row.channel_id,
                pinned_by: row.pinned_by,
                pinned_at: row.pinned_at,
            })
            .collect())
    }

    /// Pin or unpin the attachment `hex_id`: one `AttachmentPinned` entry.
    ///
    /// # Errors
    /// We lack `MANAGE_COMMUNITY`, the id is not 16 bytes of hex, or the
    /// write failed.
    pub(crate) async fn pin_attachment(
        &self,
        community_id: &str,
        hex_id: &str,
        pinned: bool,
    ) -> Result<(), GovernanceRuntimeError> {
        self.require_permission(community_id, permissions::MANAGE_COMMUNITY)?;
        let entry = GovernanceEntry::AttachmentPinned {
            attachment_id: attachment_id(hex_id)?,
            pinned,
            lamport: self.next_governance_lamport(community_id)?,
        };
        rekindle_governance_runtime::apply::write_entry(self, community_id, entry).await?;
        self.persist_governance_snapshot(community_id).await;
        Ok(())
    }

    /// Handle one pin control message from `sender_pseudonym`: `true` when
    /// it was accepted and stored, so the shell shows it.
    pub async fn on_pin_control(
        &self,
        community_id: &str,
        sender_pseudonym: &str,
        payload: &ControlPayload,
    ) -> bool {
        let change = match payload {
            ControlPayload::MessagePinned {
                channel_id,
                message_id,
                pinned_by,
            } if pinned_by.eq_ignore_ascii_case(sender_pseudonym) => PinChange::Pin(PinRow {
                message_id: message_id.clone(),
                channel_id: channel_id.clone(),
                pinned_by: pinned_by.clone(),
                pinned_at: self.ctx.clock.now_secs(),
            }),
            ControlPayload::MessageUnpinned {
                channel_id,
                message_id,
            } => PinChange::Unpin {
                channel_id: channel_id.clone(),
                message_id: message_id.clone(),
            },
            _ => {
                tracing::warn!(community = %community_id, "pin dropped: not the sender's");
                return false;
            }
        };
        if !self.sender_holds(community_id, sender_pseudonym, permissions::MANAGE_MESSAGES) {
            tracing::warn!(community = %community_id, "pin change dropped: the sender lacks MANAGE_MESSAGES");
            return false;
        }
        match self.store_pin(community_id, change).await {
            Ok(changed) => changed,
            Err(e) => {
                tracing::warn!(community = %community_id, error = %e, "pin change not stored");
                false
            }
        }
    }

    /// Apply `change`; `true` when a row changed.
    async fn store_pin(
        &self,
        community_id: &str,
        change: PinChange,
    ) -> Result<bool, GovernanceRuntimeError> {
        let (owner, db) = self.owner_db()?;
        let community = community_id.to_owned();
        db.call(move |conn| {
            use rekindle_db::repo::channel_pins::{delete, insert};
            match change {
                PinChange::Pin(row) => insert(conn, &owner, &community, &row),
                PinChange::Unpin {
                    channel_id,
                    message_id,
                } => delete(conn, &owner, &community, &channel_id, &message_id),
            }
        })
        .await
        .map_err(|e| GovernanceRuntimeError::Adapter(format!("pin not stored: {e}")))
    }
}
