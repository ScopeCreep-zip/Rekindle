use std::sync::Arc;

use rusqlite::params;

use crate::server_state::ServerState;

/// Actions that are recorded in the audit log.
#[derive(Debug, Clone, Copy)]
pub enum AuditAction {
    Kick,
    Ban,
    Unban,
    CreateChannel,
    DeleteChannel,
    RenameChannel,
    RotateMek,
    CreateRole,
    EditRole,
    DeleteRole,
    AssignRole,
    UnassignRole,
    TimeoutMember,
    RemoveTimeout,
    PinMessage,
    UnpinMessage,
    CreateCategory,
    DeleteCategory,
    RenameCategory,
    CreateInvite,
    RevokeInvite,
    UpdateCommunity,
    SetChannelTopic,
    CreateThread,
    ArchiveThread,
    UnarchiveThread,
    AddGameServer,
    RemoveGameServer,
    MoveChannel,
    ReorderCategories,
    ReorderChannels,
    SetSlowmode,
    CreateEvent,
    EditEvent,
    DeleteEvent,
    CancelEvent,
}

impl AuditAction {
    fn as_str(self) -> &'static str {
        match self {
            Self::Kick => "kick",
            Self::Ban => "ban",
            Self::Unban => "unban",
            Self::CreateChannel => "create_channel",
            Self::DeleteChannel => "delete_channel",
            Self::RenameChannel => "rename_channel",
            Self::RotateMek => "rotate_mek",
            Self::CreateRole => "create_role",
            Self::EditRole => "edit_role",
            Self::DeleteRole => "delete_role",
            Self::AssignRole => "assign_role",
            Self::UnassignRole => "unassign_role",
            Self::TimeoutMember => "timeout_member",
            Self::RemoveTimeout => "remove_timeout",
            Self::PinMessage => "pin_message",
            Self::UnpinMessage => "unpin_message",
            Self::CreateCategory => "create_category",
            Self::DeleteCategory => "delete_category",
            Self::RenameCategory => "rename_category",
            Self::CreateInvite => "create_invite",
            Self::RevokeInvite => "revoke_invite",
            Self::UpdateCommunity => "update_community",
            Self::SetChannelTopic => "set_channel_topic",
            Self::CreateThread => "create_thread",
            Self::ArchiveThread => "archive_thread",
            Self::UnarchiveThread => "unarchive_thread",
            Self::AddGameServer => "add_game_server",
            Self::RemoveGameServer => "remove_game_server",
            Self::MoveChannel => "move_channel",
            Self::ReorderCategories => "reorder_categories",
            Self::ReorderChannels => "reorder_channels",
            Self::SetSlowmode => "set_slowmode",
            Self::CreateEvent => "create_event",
            Self::EditEvent => "edit_event",
            Self::DeleteEvent => "delete_event",
            Self::CancelEvent => "cancel_event",
        }
    }
}

/// Record an action in the audit log.
pub fn log_action(
    state: &Arc<ServerState>,
    community_id: &str,
    action: AuditAction,
    actor: &str,
    target: Option<&str>,
    details: Option<&str>,
) {
    let now = rekindle_utils::timestamp_secs_i64();
    let action_str = action.as_str();
    crate::db_helpers::db_fire(&state.db, "write audit log", |conn| {
        conn.execute(
            "INSERT INTO server_audit_log (community_id, action, actor_pseudonym, target, details, timestamp) VALUES (?,?,?,?,?,?)",
            params![community_id, action_str, actor, target, details, now],
        )?;
        Ok(())
    });
}
