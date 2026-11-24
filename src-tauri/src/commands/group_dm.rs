//! Group DMs (plan E2.6): each command is one Host request; the Host
//! validates it, reviews its intent and runs `operations::group_dm`.

use rekindle_ipc::protocol::{GroupDmRequest, IpcRequest, LobbyRequest};
use rekindle_types::dto::dm::{DisappearingTimerView, DmMessage};
use rekindle_types::dto::group_dm::GroupDmView;
use rekindle_types::dto::{CommandError, Done};
use rekindle_types::game_server::ServerAddress;
use tauri::State;

use crate::host_call;
use crate::state::SharedState;

/// Create a group of us and `members` (2-7 friends) named `name`
/// (`GroupDmRequest::Create`); the answer is its id.
#[tauri::command]
pub async fn group_dm_create(
    name: String,
    members: Vec<String>,
    origin_peer: Option<String>,
    state: State<'_, SharedState>,
) -> Result<Done<String>, CommandError> {
    host_call::call(
        &state,
        IpcRequest::GroupDm(GroupDmRequest::Create {
            name,
            members,
            origin_peer,
        }),
    )
    .await
}

/// Accept a pending group invite (`GroupDmRequest::Accept`).
#[tauri::command]
pub async fn group_dm_accept(
    group_id: String,
    state: State<'_, SharedState>,
) -> Result<Done<serde_json::Value>, CommandError> {
    host_call::call(
        &state,
        IpcRequest::GroupDm(GroupDmRequest::Accept { group_id }),
    )
    .await
}

/// Decline a pending group invite (`GroupDmRequest::Decline`).
#[tauri::command]
pub async fn group_dm_decline(
    group_id: String,
    state: State<'_, SharedState>,
) -> Result<Done<serde_json::Value>, CommandError> {
    host_call::call(
        &state,
        IpcRequest::GroupDm(GroupDmRequest::Decline { group_id }),
    )
    .await
}

/// Leave a group (`GroupDmRequest::Leave`).
#[tauri::command]
pub async fn group_dm_leave(
    group_id: String,
    state: State<'_, SharedState>,
) -> Result<Done<serde_json::Value>, CommandError> {
    host_call::call(
        &state,
        IpcRequest::GroupDm(GroupDmRequest::Leave { group_id }),
    )
    .await
}

/// Rename a group we created (`GroupDmRequest::Rename`).
#[tauri::command]
pub async fn group_dm_rename(
    group_id: String,
    name: String,
    state: State<'_, SharedState>,
) -> Result<Done<serde_json::Value>, CommandError> {
    host_call::call(
        &state,
        IpcRequest::GroupDm(GroupDmRequest::Rename { group_id, name }),
    )
    .await
}

/// Invite friends into a group we created (`GroupDmRequest::Invite`).
#[tauri::command]
pub async fn group_dm_invite(
    group_id: String,
    members: Vec<String>,
    state: State<'_, SharedState>,
) -> Result<Done<serde_json::Value>, CommandError> {
    host_call::call(
        &state,
        IpcRequest::GroupDm(GroupDmRequest::Invite { group_id, members }),
    )
    .await
}

/// Set a DM's or group DM's disappearing-message timer
/// (`IpcRequest::DisappearingSet`, plan E7.1).
#[tauri::command]
pub async fn disappearing_set(
    conversation: String,
    seconds: u32,
    state: State<'_, SharedState>,
) -> Result<Done<serde_json::Value>, CommandError> {
    host_call::call(
        &state,
        IpcRequest::DisappearingSet {
            conversation,
            seconds,
        },
    )
    .await
}

/// The disappearing-message timer a DM or group DM carries now
/// (`IpcRequest::DisappearingGet`, plan E7.1).
#[tauri::command]
pub async fn disappearing_get(
    conversation: String,
    state: State<'_, SharedState>,
) -> Result<Done<DisappearingTimerView>, CommandError> {
    host_call::call(&state, IpcRequest::DisappearingGet { conversation }).await
}

/// Send `body` to a group (`GroupDmRequest::Send`); the answer is the
/// message's client id once its row and echo are committed (`queued`,
/// then `GroupDmEvent::SendStatus`).
#[tauri::command]
pub async fn group_dm_send(
    group_id: String,
    body: String,
    state: State<'_, SharedState>,
) -> Result<Done<String>, CommandError> {
    host_call::call(
        &state,
        IpcRequest::GroupDm(GroupDmRequest::Send { group_id, body }),
    )
    .await
}

/// Report the draft to a group (`GroupDmRequest::Typing`): `typing` is
/// whether it holds text; the Host decides what goes out, as for a peer.
#[tauri::command]
pub async fn group_dm_typing(
    group_id: String,
    typing: bool,
    state: State<'_, SharedState>,
) -> Result<Done<serde_json::Value>, CommandError> {
    host_call::call(
        &state,
        IpcRequest::GroupDm(GroupDmRequest::Typing { group_id, typing }),
    )
    .await
}

/// Every group we are in and every unanswered invite
/// (`GroupDmRequest::List`).
#[tauri::command]
pub async fn group_dm_list(
    state: State<'_, SharedState>,
) -> Result<Done<Vec<GroupDmView>>, CommandError> {
    host_call::call(&state, IpcRequest::GroupDm(GroupDmRequest::List)).await
}

/// The group's window displayed its messages: received disappearing
/// timers start (`GroupDmRequest::MarkRead`, plan E7.1).
#[tauri::command]
pub async fn group_dm_mark_read(
    group_id: String,
    state: State<'_, SharedState>,
) -> Result<Done<()>, CommandError> {
    host_call::call(
        &state,
        IpcRequest::GroupDm(GroupDmRequest::MarkRead { group_id }),
    )
    .await
}

/// The newest `limit` messages of a group before `before` (ms), oldest
/// first (`GroupDmRequest::Messages`).
#[tauri::command]
pub async fn group_dm_messages(
    group_id: String,
    limit: u32,
    before: Option<i64>,
    state: State<'_, SharedState>,
) -> Result<Done<Vec<DmMessage>>, CommandError> {
    let request = IpcRequest::GroupDm(GroupDmRequest::Messages {
        group_id,
        limit,
        before,
    });
    host_call::call(&state, request).await
}

/// `IpcRequest::LobbyHost` (plan E7.2 G6).
#[tauri::command]
pub async fn lobby_host(
    group_id: String,
    state: State<'_, SharedState>,
) -> Result<Done<serde_json::Value>, CommandError> {
    host_call::call(&state, IpcRequest::Lobby(LobbyRequest::Host { group_id })).await
}

/// `IpcRequest::LobbyJoin` (plan E7.2 G6).
#[tauri::command]
pub async fn lobby_join(
    group_id: String,
    lobby_id: String,
    state: State<'_, SharedState>,
) -> Result<Done<serde_json::Value>, CommandError> {
    host_call::call(
        &state,
        IpcRequest::Lobby(LobbyRequest::Join { group_id, lobby_id }),
    )
    .await
}

/// `IpcRequest::LobbyLeave` (plan E7.2 G6).
#[tauri::command]
pub async fn lobby_leave(
    group_id: String,
    lobby_id: String,
    state: State<'_, SharedState>,
) -> Result<Done<serde_json::Value>, CommandError> {
    host_call::call(
        &state,
        IpcRequest::Lobby(LobbyRequest::Leave { group_id, lobby_id }),
    )
    .await
}

/// `IpcRequest::LobbyClose` (plan E7.2 G6).
#[tauri::command]
pub async fn lobby_close(
    group_id: String,
    lobby_id: String,
    state: State<'_, SharedState>,
) -> Result<Done<serde_json::Value>, CommandError> {
    host_call::call(
        &state,
        IpcRequest::Lobby(LobbyRequest::Close { group_id, lobby_id }),
    )
    .await
}

/// `IpcRequest::LobbyLaunch` (plan E7.2 G6): `None` launches on our shown
/// game's server.
#[tauri::command]
pub async fn lobby_launch(
    group_id: String,
    lobby_id: String,
    server: Option<ServerAddress>,
    state: State<'_, SharedState>,
) -> Result<Done<serde_json::Value>, CommandError> {
    host_call::call(
        &state,
        IpcRequest::Lobby(LobbyRequest::Launch {
            group_id,
            lobby_id,
            server,
        }),
    )
    .await
}

/// `IpcRequest::LobbyGet` (plan E7.2 G6): the group's lobby, or `null`.
#[tauri::command]
pub async fn lobby_get(
    group_id: String,
    state: State<'_, SharedState>,
) -> Result<Done<serde_json::Value>, CommandError> {
    host_call::call(&state, IpcRequest::Lobby(LobbyRequest::Get { group_id })).await
}
