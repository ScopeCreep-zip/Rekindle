//! Group DMs (plan E2.6): each command is one Host request; the Host
//! validates it, reviews its intent and runs `operations::group_dm`.

use rekindle_ipc::protocol::{GroupDmRequest, IpcRequest, LobbyRequest};
use rekindle_types::dto::dm::{DisappearingTimerView, DmMessage};
use rekindle_types::dto::group_dm::GroupDmView;
use rekindle_types::dto::{CommandError, Done};
use rekindle_types::game_server::ServerAddress;
use tauri::State;

use crate::state::SharedState;

host_command! {
    state;
    /// Create a group of us and `members` (2-7 friends) named `name`
    /// (`GroupDmRequest::Create`); the answer is its id.
    pub async fn group_dm_create(
        name: String,
        members: Vec<String>,
        origin_peer: Option<String>,
        state: State<'_, SharedState>,
    ) -> Result<Done<String>, CommandError> {
        IpcRequest::GroupDm(GroupDmRequest::Create {
            name,
            members,
            origin_peer,
        })
    }
}

host_command! {
    state;
    /// Accept a pending group invite (`GroupDmRequest::Accept`).
    pub async fn group_dm_accept(
        group_id: String,
        state: State<'_, SharedState>,
    ) -> Result<Done<serde_json::Value>, CommandError> {
        IpcRequest::GroupDm(GroupDmRequest::Accept { group_id })
    }
}

host_command! {
    state;
    /// Decline a pending group invite (`GroupDmRequest::Decline`).
    pub async fn group_dm_decline(
        group_id: String,
        state: State<'_, SharedState>,
    ) -> Result<Done<serde_json::Value>, CommandError> {
        IpcRequest::GroupDm(GroupDmRequest::Decline { group_id })
    }
}

host_command! {
    state;
    /// Leave a group (`GroupDmRequest::Leave`).
    pub async fn group_dm_leave(
        group_id: String,
        state: State<'_, SharedState>,
    ) -> Result<Done<serde_json::Value>, CommandError> {
        IpcRequest::GroupDm(GroupDmRequest::Leave { group_id })
    }
}

host_command! {
    state;
    /// Rename a group we created (`GroupDmRequest::Rename`).
    pub async fn group_dm_rename(
        group_id: String,
        name: String,
        state: State<'_, SharedState>,
    ) -> Result<Done<serde_json::Value>, CommandError> {
        IpcRequest::GroupDm(GroupDmRequest::Rename { group_id, name })
    }
}

host_command! {
    state;
    /// Invite friends into a group we created (`GroupDmRequest::Invite`).
    pub async fn group_dm_invite(
        group_id: String,
        members: Vec<String>,
        state: State<'_, SharedState>,
    ) -> Result<Done<serde_json::Value>, CommandError> {
        IpcRequest::GroupDm(GroupDmRequest::Invite { group_id, members })
    }
}

host_command! {
    state;
    /// Set a DM's or group DM's disappearing-message timer
    /// (`IpcRequest::DisappearingSet`, plan E7.1).
    pub async fn disappearing_set(
        conversation: String,
        seconds: u32,
        state: State<'_, SharedState>,
    ) -> Result<Done<serde_json::Value>, CommandError> {
        IpcRequest::DisappearingSet {
            conversation,
            seconds,
        }
    }
}

host_command! {
    state;
    /// The disappearing-message timer a DM or group DM carries now
    /// (`IpcRequest::DisappearingGet`, plan E7.1).
    pub async fn disappearing_get(
        conversation: String,
        state: State<'_, SharedState>,
    ) -> Result<Done<DisappearingTimerView>, CommandError> {
        IpcRequest::DisappearingGet { conversation }
    }
}

host_command! {
    state;
    /// Send `body` to a group (`GroupDmRequest::Send`), a reply to `reply_to`
    /// (the parent's client id) when given; the answer is the
    /// message's client id once its row and echo are committed (`queued`,
    /// then `GroupDmEvent::SendStatus`).
    pub async fn group_dm_send(
        group_id: String,
        body: String,
        reply_to: Option<String>,
        state: State<'_, SharedState>,
    ) -> Result<Done<String>, CommandError> {
        IpcRequest::GroupDm(GroupDmRequest::Send {
            group_id,
            body,
            reply_to,
        })
    }
}

host_command! {
    state;
    /// Report the draft to a group (`GroupDmRequest::Typing`): `typing` is
    /// whether it holds text; the Host decides what goes out, as for a peer.
    pub async fn group_dm_typing(
        group_id: String,
        typing: bool,
        state: State<'_, SharedState>,
    ) -> Result<Done<serde_json::Value>, CommandError> {
        IpcRequest::GroupDm(GroupDmRequest::Typing { group_id, typing })
    }
}

host_command! {
    state;
    /// Every group we are in and every unanswered invite
    /// (`GroupDmRequest::List`).
    pub async fn group_dm_list(
        state: State<'_, SharedState>,
    ) -> Result<Done<Vec<GroupDmView>>, CommandError> {
        IpcRequest::GroupDm(GroupDmRequest::List)
    }
}

host_command! {
    state;
    /// The group's window displayed its messages: received disappearing
    /// timers start (`GroupDmRequest::MarkRead`, plan E7.1).
    pub async fn group_dm_mark_read(
        group_id: String,
        state: State<'_, SharedState>,
    ) -> Result<Done<()>, CommandError> {
        IpcRequest::GroupDm(GroupDmRequest::MarkRead { group_id })
    }
}

host_command! {
    state;
    /// The newest `limit` messages of a group before `before` (ms), oldest
    /// first (`GroupDmRequest::Messages`).
    pub async fn group_dm_messages(
        group_id: String,
        limit: u32,
        before: Option<i64>,
        state: State<'_, SharedState>,
    ) -> Result<Done<Vec<DmMessage>>, CommandError> {
        IpcRequest::GroupDm(GroupDmRequest::Messages {
            group_id,
            limit,
            before,
        })
    }
}

host_command! {
    state;
    /// `IpcRequest::LobbyHost` (plan E7.2 G6).
    pub async fn lobby_host(
        group_id: String,
        state: State<'_, SharedState>,
    ) -> Result<Done<serde_json::Value>, CommandError> {
        IpcRequest::Lobby(LobbyRequest::Host { group_id })
    }
}

host_command! {
    state;
    /// `IpcRequest::LobbyJoin` (plan E7.2 G6).
    pub async fn lobby_join(
        group_id: String,
        lobby_id: String,
        state: State<'_, SharedState>,
    ) -> Result<Done<serde_json::Value>, CommandError> {
        IpcRequest::Lobby(LobbyRequest::Join { group_id, lobby_id })
    }
}

host_command! {
    state;
    /// `IpcRequest::LobbyLeave` (plan E7.2 G6).
    pub async fn lobby_leave(
        group_id: String,
        lobby_id: String,
        state: State<'_, SharedState>,
    ) -> Result<Done<serde_json::Value>, CommandError> {
        IpcRequest::Lobby(LobbyRequest::Leave { group_id, lobby_id })
    }
}

host_command! {
    state;
    /// `IpcRequest::LobbyClose` (plan E7.2 G6).
    pub async fn lobby_close(
        group_id: String,
        lobby_id: String,
        state: State<'_, SharedState>,
    ) -> Result<Done<serde_json::Value>, CommandError> {
        IpcRequest::Lobby(LobbyRequest::Close { group_id, lobby_id })
    }
}

host_command! {
    state;
    /// `IpcRequest::LobbyLaunch` (plan E7.2 G6): `None` launches on our shown
    /// game's server.
    pub async fn lobby_launch(
        group_id: String,
        lobby_id: String,
        server: Option<ServerAddress>,
        state: State<'_, SharedState>,
    ) -> Result<Done<serde_json::Value>, CommandError> {
        IpcRequest::Lobby(LobbyRequest::Launch {
            group_id,
            lobby_id,
            server,
        })
    }
}

host_command! {
    state;
    /// `IpcRequest::LobbyGet` (plan E7.2 G6): the group's lobby, or `null`.
    pub async fn lobby_get(
        group_id: String,
        state: State<'_, SharedState>,
    ) -> Result<Done<serde_json::Value>, CommandError> {
        IpcRequest::Lobby(LobbyRequest::Get { group_id })
    }
}
