//! A community's game servers (plan E3.1 H1), on both hosts.
//!
//! An add or removal is a `GameServerAdded` / `GameServerRemoved` control
//! message on the community mesh, stored by the sender at once and by each
//! peer as it accepts the message. Readers validate: a peer stores an
//! entry only when the sender holds `MANAGE_CHANNELS` in the merged
//! governance state and the entry names the sender as its author, with a
//! well-formed id and a `host:port` address (plan E1.1).

use rekindle_codec::community::envelope::{CommunityEnvelope, ControlPayload};
use rekindle_governance::permissions::{compute_permissions, has_capability};
use rekindle_governance_runtime::{GovernanceRuntimeDeps, GovernanceRuntimeError};
use rekindle_types::game_server::{
    is_server_id, GameServerInfo, ServerAddress, GAME_SERVER_ID_BYTES,
};
use rekindle_types::id::PseudonymKey;
use rekindle_types::permissions;

use super::HostGovernance;

/// A server to add, as the user entered it.
pub(crate) struct NewGameServer {
    pub game_id: String,
    pub label: String,
    pub address: String,
}

/// A change to the stored servers.
enum Change {
    Insert(GameServerInfo),
    Remove(String),
}

impl HostGovernance {
    /// Add `new` to the community: stored here and sent to the mesh.
    /// Answers the new server's id.
    ///
    /// # Errors
    /// We lack `MANAGE_CHANNELS`, the address is not `host:port`, or the
    /// message or the row could not go.
    pub(crate) async fn add_game_server(
        &self,
        community_id: &str,
        new: NewGameServer,
    ) -> Result<String, GovernanceRuntimeError> {
        self.require_permission(community_id, permissions::MANAGE_CHANNELS)?;
        let address = ServerAddress::parse(&new.address)
            .map_err(|e| GovernanceRuntimeError::Adapter(format!("server address: {e}")))?;
        let added_by = self
            .community_membership_impl(community_id)
            .and_then(|m| m.my_pseudonym_hex)
            .ok_or_else(|| GovernanceRuntimeError::PseudonymKeyMissing(community_id.into()))?;
        let mut id = [0u8; GAME_SERVER_ID_BYTES];
        self.ctx.rng.fill(&mut id);
        let server = GameServerInfo {
            id: format!("gs_{}", hex::encode(id)),
            game_id: new.game_id,
            label: new.label,
            address: address.to_string(),
            added_by,
            created_at: self.ctx.clock.now_secs(),
        };
        let envelope = CommunityEnvelope::Control(ControlPayload::GameServerAdded {
            server: server.clone(),
        });
        self.send_to_mesh(community_id, &envelope)?;
        let id = server.id.clone();
        self.store_change(community_id, Change::Insert(server))
            .await?;
        Ok(id)
    }

    /// Remove the server `server_id`: dropped here and sent to the mesh.
    ///
    /// # Errors
    /// We lack `MANAGE_CHANNELS`, or the message or the delete could not go.
    pub(crate) async fn remove_game_server(
        &self,
        community_id: &str,
        server_id: &str,
    ) -> Result<(), GovernanceRuntimeError> {
        self.require_permission(community_id, permissions::MANAGE_CHANNELS)?;
        let envelope = CommunityEnvelope::Control(ControlPayload::GameServerRemoved {
            server_id: server_id.to_owned(),
        });
        self.send_to_mesh(community_id, &envelope)?;
        self.store_change(community_id, Change::Remove(server_id.to_owned()))
            .await
    }

    /// The community's servers, newest first.
    ///
    /// # Errors
    /// We lack `VIEW_CHANNELS`, or the read failed.
    pub(crate) async fn game_servers(
        &self,
        community_id: &str,
    ) -> Result<Vec<GameServerInfo>, GovernanceRuntimeError> {
        self.require_permission(community_id, permissions::VIEW_CHANNELS)?;
        let (owner, db) = self.owner_db()?;
        let community = community_id.to_owned();
        db.call(move |conn| rekindle_db::repo::game_servers::list(conn, &owner, &community))
            .await
            .map_err(|e| GovernanceRuntimeError::Adapter(format!("game servers not read: {e}")))
    }

    /// The stored label of `server_id`, for its removal's review.
    ///
    /// # Errors
    /// The read failed.
    pub(crate) async fn game_server_label(
        &self,
        community_id: &str,
        server_id: &str,
    ) -> Result<Option<String>, GovernanceRuntimeError> {
        let (owner, db) = self.owner_db()?;
        let (community, id) = (community_id.to_owned(), server_id.to_owned());
        db.call(move |conn| rekindle_db::repo::game_servers::label(conn, &owner, &community, &id))
            .await
            .map_err(|e| GovernanceRuntimeError::Adapter(format!("game server not read: {e}")))
    }

    /// Handle one game-server control message from `sender_pseudonym`:
    /// `true` when it was accepted and stored, so the shell shows it.
    pub async fn on_game_server_control(
        &self,
        community_id: &str,
        sender_pseudonym: &str,
        payload: &ControlPayload,
    ) -> bool {
        let change = match payload {
            ControlPayload::GameServerAdded { server } => {
                if !is_server_id(&server.id)
                    || server.added_by != sender_pseudonym
                    || ServerAddress::parse(&server.address).is_err()
                {
                    tracing::warn!(community = %community_id, "game server entry dropped: malformed or not the sender's");
                    return false;
                }
                Change::Insert(server.clone())
            }
            ControlPayload::GameServerRemoved { server_id } if is_server_id(server_id) => {
                Change::Remove(server_id.clone())
            }
            _ => return false,
        };
        if !self.may_manage(community_id, sender_pseudonym) {
            tracing::warn!(community = %community_id, "game server change dropped: the sender lacks MANAGE_CHANNELS");
            return false;
        }
        match self.store_change(community_id, change).await {
            Ok(()) => true,
            Err(e) => {
                tracing::warn!(community = %community_id, error = %e, "game server change not stored");
                false
            }
        }
    }

    /// Whether `sender_pseudonym` holds `MANAGE_CHANNELS` in the merged state.
    fn may_manage(&self, community_id: &str, sender_pseudonym: &str) -> bool {
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
        has_capability(effective, permissions::MANAGE_CHANNELS)
    }

    /// Apply `change` to the stored servers.
    async fn store_change(
        &self,
        community_id: &str,
        change: Change,
    ) -> Result<(), GovernanceRuntimeError> {
        let (owner, db) = self.owner_db()?;
        let community = community_id.to_owned();
        db.call(move |conn| {
            use rekindle_db::repo::game_servers::{insert, remove};
            match change {
                Change::Insert(server) => insert(conn, &owner, &community, &server),
                Change::Remove(id) => remove(conn, &owner, &community, &id),
            }
        })
        .await
        .map_err(|e| GovernanceRuntimeError::Adapter(format!("game server not stored: {e}")))
    }

    /// The unlocked identity's key and database; an error while locked.
    fn owner_db(&self) -> Result<(String, rekindle_db::Db), GovernanceRuntimeError> {
        let not_logged_in = || GovernanceRuntimeError::Adapter(rekindle_db::NotLoggedIn.into());
        let owner = self
            .ctx
            .session
            .read()
            .as_ref()
            .map(|s| s.identity.public_key_hex.clone())
            .ok_or_else(not_logged_in)?;
        let db = self.ctx.db.current().map_err(|_| not_logged_in())?;
        Ok((owner, db))
    }
}
