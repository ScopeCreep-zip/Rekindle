//! Our own presence, profile (bio included), game, audit log, links,
//! read-receipt and game-totals sharing settings, and a friend's bio and
//! shared game totals (plan E1.1, E2.5, E7.2): one implementation on both hosts, over
//! `host::{own_presence, own_profile, audit_alert}`.

use rekindle_ipc::protocol::IpcResponse;
use rekindle_types::dto::audit::{AuditEntryView, AuditVerified};
use rekindle_types::game_server::ServerAddress;
use rekindle_types::image_kind::ImageKind;
use rekindle_types::presence::SessionStatus;

use rekindle_ipc::protocol::IpcRequest;

use super::{state_error, DaemonContext};
use crate::daemon::DaemonState;
use crate::host::own_presence::Game;

/// Route one of our own requests (the router's arm for them).
pub(crate) async fn route(
    ctx: &DaemonContext,
    state: DaemonState,
    request: IpcRequest,
) -> IpcResponse {
    match request {
        IpcRequest::PresenceSet { status } => handle_status(ctx, state, status).await,
        IpcRequest::PresenceSetMessage { message } => handle_message(ctx, state, message).await,
        IpcRequest::PresenceGet => handle_get(ctx, state),
        IpcRequest::GameTotalsGet => handle_game_totals(ctx, state).await,
        IpcRequest::FriendGameTotalsGet { public_key } => {
            handle_friend_game_totals(ctx, state, public_key).await
        }
        IpcRequest::ProfileSetNickname { display_name } => {
            handle_nickname(ctx, state, &display_name).await
        }
        IpcRequest::ProfileBioGet => handle_bio_get(ctx, state, None).await,
        IpcRequest::FriendBioGet { public_key } => {
            handle_bio_get(ctx, state, Some(public_key)).await
        }
        IpcRequest::ProfileSetBio { bio } => handle_set_bio(ctx, state, &bio).await,
        IpcRequest::SetReadReceiptsEnabled { enabled } => {
            handle_read_receipts(ctx, state, enabled).await
        }
        IpcRequest::SetShareGameTotals { enabled } => {
            handle_share_game_totals(ctx, state, enabled).await
        }
        IpcRequest::ProfileSetAvatar { image, kind } => {
            handle_avatar(ctx, state, image, kind).await
        }
        IpcRequest::AvatarGet { public_key } => handle_avatar_get(ctx, state, &public_key).await,
        IpcRequest::GamePresenceSet {
            game_name,
            game_id,
            elapsed_seconds,
            server_address,
        } => {
            handle_game_set(
                ctx,
                state,
                game_name,
                game_id,
                elapsed_seconds,
                server_address,
            )
            .await
        }
        IpcRequest::GamePresenceClear => handle_game_clear(ctx, state).await,
        IpcRequest::AuditVerify => handle_audit_verify(ctx, state).await,
        IpcRequest::AuditExport { since } => handle_audit_export(ctx, state, since).await,
        IpcRequest::LinkOpen { url } => handle_link_open(&url),
        other => unreachable!("{} is not one of our own requests", other.name()),
    }
}

/// `PresenceSet`: the status the user picked. Taken while the identity is
/// unlocked, attached or not (Discord and Signal apply your status at once):
/// it is ours to show, and our presence publishes a new beat with it when
/// the network turns ready (`OwnPresence::publish_on_ready`), never a held
/// one (presence beats are never held, `rekindle-record-pool` `durable.rs`).
async fn handle_status(
    ctx: &DaemonContext,
    state: DaemonState,
    status: SessionStatus,
) -> IpcResponse {
    if !state.can_query() {
        return state_error(state, "query");
    }
    let was_invisible = ctx.own_presence.status() == SessionStatus::Invisible;
    ctx.own_presence.set_status(status).await;
    if was_invisible && status != SessionStatus::Invisible {
        // The receipts held while Invisible go now (plan E2.5).
        let host = crate::host::adapters::friends::HostFriends::of(ctx);
        ctx.unlock_scope_or_closed()
            .spawn_or_drop("held dm acks", async move {
                rekindle_transport::operations::dm::ack::flush_held(&host).await;
            });
    }
    IpcResponse::ok(&ctx.own_presence.view())
}

/// `PresenceSetMessage`.
async fn handle_message(ctx: &DaemonContext, state: DaemonState, message: String) -> IpcResponse {
    if !state.can_write() {
        return state_error(state, "write");
    }
    if let Err(e) = ctx.own_presence.set_message(message).await {
        tracing::warn!(error = %e, "status message not published");
    }
    IpcResponse::ok(&ctx.own_presence.view())
}

/// `PresenceGet`.
fn handle_get(ctx: &DaemonContext, state: DaemonState) -> IpcResponse {
    if !state.can_query() {
        return state_error(state, "query");
    }
    IpcResponse::ok(&ctx.own_presence.view())
}

/// `GameTotalsGet` (plan E7.2): `game_sessions` summed per game, each
/// named and given its icon key from the bundled database.
async fn handle_game_totals(ctx: &DaemonContext, state: DaemonState) -> IpcResponse {
    if !state.can_query() {
        return state_error(state, "query");
    }
    let (db, owner) = match (ctx.require_db(), ctx.require_owner()) {
        (Ok(db), Ok(owner)) => (db, owner),
        (Err(e), _) | (_, Err(e)) => return e,
    };
    match db
        .call(move |conn| rekindle_db::repo::game_sessions::totals(conn, &owner))
        .await
    {
        Ok(totals) => IpcResponse::ok(&crate::host::game::total_views(&totals)),
        Err(e) => IpcResponse::error(500, format!("game totals not read: {e}")),
    }
}

/// `FriendGameTotalsGet` (plan E7.2): what `public_key` last shared
/// (`friend_game_totals`), named and given icon keys as ours are.
async fn handle_friend_game_totals(
    ctx: &DaemonContext,
    state: DaemonState,
    public_key: String,
) -> IpcResponse {
    if !state.can_query() {
        return state_error(state, "query");
    }
    let (db, owner) = match (ctx.require_db(), ctx.require_owner()) {
        (Ok(db), Ok(owner)) => (db, owner),
        (Err(e), _) | (_, Err(e)) => return e,
    };
    match db
        .call(move |conn| rekindle_db::repo::friend_game_totals::of(conn, &owner, &public_key))
        .await
    {
        Ok(totals) => IpcResponse::ok(&crate::host::game::total_views(&totals)),
        Err(e) => IpcResponse::error(500, format!("friend's game totals not read: {e}")),
    }
}

/// `SetShareGameTotals`, once its review was committed (plan E7.2): turned
/// on, our totals go to every accepted friend; turned off, nothing more is
/// sent, and what friends already received stays with them (the plan
/// removes a friend's copy only with the friendship). The preferences as
/// they now are.
async fn handle_share_game_totals(
    ctx: &DaemonContext,
    state: DaemonState,
    enabled: bool,
) -> IpcResponse {
    if !state.can_write() {
        return state_error(state, "write");
    }
    let (db, owner) = match (ctx.require_db(), ctx.require_owner()) {
        (Ok(db), Ok(owner)) => (db, owner),
        (Err(e), _) | (_, Err(e)) => return e,
    };
    let saved = db
        .call(move |conn| {
            rekindle_db::repo::preferences::set_share_game_totals(conn, &owner, enabled)?;
            rekindle_db::repo::preferences::load(conn, &owner)
        })
        .await;
    match saved {
        Ok(prefs) => {
            if enabled {
                let friends = crate::host::adapters::friends::HostFriends::of(ctx);
                let reached =
                    rekindle_transport::operations::friend::game_totals::send_to_friends(&friends)
                        .await;
                tracing::debug!(reached, "game totals shared with friends");
            }
            IpcResponse::ok(&prefs)
        }
        Err(e) => IpcResponse::error(500, format!("game totals sharing not saved: {e}")),
    }
}

/// `GamePresenceSet`: a game set by hand, until the detector's next change.
async fn handle_game_set(
    ctx: &DaemonContext,
    state: DaemonState,
    game_name: String,
    game_id: Option<u32>,
    elapsed_seconds: u32,
    server_address: Option<String>,
) -> IpcResponse {
    if !state.can_write() {
        return state_error(state, "write");
    }
    let server_address = match server_address
        .as_deref()
        .map(ServerAddress::parse)
        .transpose()
    {
        Ok(address) => address,
        Err(e) => return IpcResponse::error(400, format!("invalid server address: {e}")),
    };
    let game = Game {
        id: game_id.unwrap_or(0),
        name: game_name,
        server_info: None,
        server_address,
        started_at_ms: ctx
            .clock
            .now_ms()
            .saturating_sub(u64::from(elapsed_seconds) * 1000),
    };
    if let Err(e) = ctx.own_presence.set_game(Some(game)).await {
        tracing::warn!(error = %e, "game not published");
    }
    IpcResponse::ok(&ctx.own_presence.view())
}

/// `GamePresenceClear`.
async fn handle_game_clear(ctx: &DaemonContext, state: DaemonState) -> IpcResponse {
    if !state.can_write() {
        return state_error(state, "write");
    }
    if let Err(e) = ctx.own_presence.set_game(None).await {
        tracing::warn!(error = %e, "game clear not published");
    }
    IpcResponse::ok(&ctx.own_presence.view())
}

/// `ProfileSetNickname`.
async fn handle_nickname(
    ctx: &DaemonContext,
    state: DaemonState,
    display_name: &str,
) -> IpcResponse {
    if !state.can_write() {
        return state_error(state, "write");
    }
    let name = display_name.trim();
    match crate::host::own_profile::set_nickname(ctx, name).await {
        Ok(()) => IpcResponse::ok(&name),
        Err(e) => IpcResponse::error(500, e),
    }
}

/// `ProfileBioGet` (`friend` `None`) or `FriendBioGet` (plan E7.2): our
/// bio, or the one `friend` last sent us; `None` when there is none.
async fn handle_bio_get(
    ctx: &DaemonContext,
    state: DaemonState,
    friend: Option<String>,
) -> IpcResponse {
    if !state.can_query() {
        return state_error(state, "query");
    }
    let (db, owner) = match (ctx.require_db(), ctx.require_owner()) {
        (Ok(db), Ok(owner)) => (db, owner),
        (Err(e), _) | (_, Err(e)) => return e,
    };
    let read = db
        .call(move |conn| match friend {
            None => rekindle_db::repo::bio::own(conn, &owner),
            Some(peer) => rekindle_db::repo::bio::friend(conn, &owner, &peer),
        })
        .await;
    match read {
        Ok(bio) => IpcResponse::ok(&bio),
        Err(e) => IpcResponse::error(500, format!("bio not read: {e}")),
    }
}

/// `ProfileSetBio`, once its review was committed (plan E7.2): stored in
/// `identity.bio` (blank removes it) and sent to every accepted friend as a
/// ratchet-sealed `ProfileBio`. The bio as stored.
async fn handle_set_bio(ctx: &DaemonContext, state: DaemonState, bio: &str) -> IpcResponse {
    if !state.can_write() {
        return state_error(state, "write");
    }
    let (db, owner) = match (ctx.require_db(), ctx.require_owner()) {
        (Ok(db), Ok(owner)) => (db, owner),
        (Err(e), _) | (_, Err(e)) => return e,
    };
    let bio = match rekindle_types::presence::limits::bio(bio) {
        Ok(bio) => bio.map(str::to_string),
        Err(e) => return IpcResponse::error(400, format!("invalid bio: {e}")),
    };
    let stored = bio.clone();
    let saved = db
        .call(move |conn| rekindle_db::repo::bio::set_own(conn, &owner, stored.as_deref()))
        .await;
    if let Err(e) = saved {
        return IpcResponse::error(500, format!("bio not saved: {e}"));
    }
    let friends = crate::host::adapters::friends::HostFriends::of(ctx);
    let reached =
        rekindle_transport::operations::friend::bio::send_to_friends(&friends, bio.clone()).await;
    tracing::debug!(reached, "bio sent to friends");
    IpcResponse::ok(&bio)
}

/// `SetReadReceiptsEnabled`, once its review was committed: the
/// identity's preferences as they now are. The receipt policy (the outbox)
/// reads the setting from the database.
async fn handle_read_receipts(
    ctx: &DaemonContext,
    state: DaemonState,
    enabled: bool,
) -> IpcResponse {
    if !state.can_write() {
        return state_error(state, "write");
    }
    let (db, owner) = match (ctx.require_db(), ctx.require_owner()) {
        (Ok(db), Ok(owner)) => (db, owner),
        (Err(e), _) | (_, Err(e)) => return e,
    };
    let saved = db
        .call(move |conn| {
            rekindle_db::repo::preferences::set_read_receipts_enabled(conn, &owner, enabled)?;
            rekindle_db::repo::preferences::load(conn, &owner)
        })
        .await;
    match saved {
        Ok(prefs) => IpcResponse::ok(&prefs),
        Err(e) => IpcResponse::error(500, format!("read receipts not saved: {e}")),
    }
}

/// `ProfileSetAvatar`.
async fn handle_avatar(
    ctx: &DaemonContext,
    state: DaemonState,
    image: Vec<u8>,
    kind: ImageKind,
) -> IpcResponse {
    if !state.can_write() {
        return state_error(state, "write");
    }
    match crate::host::own_profile::set_avatar(ctx, image, kind).await {
        Ok(()) => IpcResponse::ok(&serde_json::Value::Null),
        Err(e) => IpcResponse::error(400, e),
    }
}

/// `AvatarGet`: WebP bytes, or `None`.
async fn handle_avatar_get(
    ctx: &DaemonContext,
    state: DaemonState,
    public_key: &str,
) -> IpcResponse {
    if !state.can_query() {
        return state_error(state, "query");
    }
    match crate::host::own_profile::avatar(ctx, public_key).await {
        Ok(avatar) => IpcResponse::ok(&avatar),
        Err(e) => IpcResponse::error(500, e),
    }
}

/// `AuditVerify`: a broken chain also raises the tamper signal.
async fn handle_audit_verify(ctx: &DaemonContext, state: DaemonState) -> IpcResponse {
    if !state.can_query() {
        return state_error(state, "query");
    }
    let (db, owner) = match (ctx.require_db(), ctx.require_owner()) {
        (Ok(db), Ok(owner)) => (db, owner),
        (Err(e), _) | (_, Err(e)) => return e,
    };
    let verified = match crate::host::audit_alert::verify(ctx, &db, &owner).await {
        rekindle_audit::Verified::Intact { length } => AuditVerified::Intact { length },
        rekindle_audit::Verified::Broken { length, at, detail } => {
            AuditVerified::Broken { length, at, detail }
        }
    };
    IpcResponse::ok(&verified)
}

/// `AuditExport`: the entries after `since`, MACs as hex.
async fn handle_audit_export(ctx: &DaemonContext, state: DaemonState, since: u64) -> IpcResponse {
    if !state.can_query() {
        return state_error(state, "query");
    }
    let (db, owner) = match (ctx.require_db(), ctx.require_owner()) {
        (Ok(db), Ok(owner)) => (db, owner),
        (Err(e), _) | (_, Err(e)) => return e,
    };
    match crate::host::audit::export(&db, &owner, since).await {
        Ok(entries) => IpcResponse::ok(
            &entries
                .into_iter()
                .map(|entry| AuditEntryView {
                    cursor: entry.cursor,
                    prev_mac: hex::encode(entry.prev_mac),
                    mac: hex::encode(entry.mac),
                    at_ms: entry.record.at_ms,
                    actor_pub: entry.record.actor_pub,
                    kind: format!("{:?}", entry.record.kind),
                    payload: entry.record.payload,
                })
                .collect::<Vec<_>>(),
        ),
        Err(e) => IpcResponse::error(500, e),
    }
}

/// `LinkOpen`, once its review was committed: the normalized URL, which
/// the shell opens in the system browser.
fn handle_link_open(url: &str) -> IpcResponse {
    match rekindle_link_preview::https_url(url) {
        Ok(url) => IpcResponse::ok(&url.as_str()),
        Err(e) => IpcResponse::error(400, format!("invalid link: {e}")),
    }
}
