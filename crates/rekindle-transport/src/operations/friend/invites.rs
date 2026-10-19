//! Friend invites we make: a signed invite link (no prekey bundle, plan
//! C7.25) tracked in `outgoing_invites` until it is answered, cancelled or
//! expires, and the invite a link we open carries.

use std::time::Duration;

use rekindle_codec::message::envelope::InviteBlob;
use rekindle_types::dto::friends::{InviteCreated, OutgoingInvite};

use super::error::FriendError;
use super::host::FriendHost;
use super::state;

/// 7-day invite recency window (B11): even a harvested link stops working
/// after it, so a leaked link never grants indefinite reach.
pub const MAX_INVITE_AGE_SECS: u64 = 7 * 24 * 3600;

/// How long making an invite waits for our profile, mailbox and route to be
/// published (right after unlock).
const PUBLISHED_WAIT: Duration = Duration::from_secs(30);
/// How often that wait looks again.
const PUBLISHED_POLL: Duration = Duration::from_millis(500);

/// Our profile and mailbox keys and our route, once all are published.
fn published(host: &dyn FriendHost) -> Option<(state::OwnIdentity, Vec<u8>)> {
    let own = state::own_identity(host).ok()?;
    if own.profile_dht_key.is_empty() || own.mailbox_dht_key.is_empty() {
        return None;
    }
    let route_blob = host.node().ok()?.personal_route_blob()?;
    Some((own, route_blob))
}

/// Make a tracked friend invite.
///
/// # Errors
/// Our records or route are not published within 30 s, our own route does
/// not import, or the invite cannot be stored.
pub async fn create(host: &dyn FriendHost) -> Result<InviteCreated, FriendError> {
    let secret = host.identity_secret()?;
    let (own, route_blob) = tokio::time::timeout(PUBLISHED_WAIT, async {
        loop {
            match published(host) {
                Some(ready) => return ready,
                None => tokio::time::sleep(PUBLISHED_POLL).await,
            }
        }
    })
    .await
    .map_err(|_| {
        FriendError::NotReady("Network not ready — please wait a moment and try again".to_string())
    })?;
    tracing::info!(
        route_blob_len = route_blob.len(),
        route_count = route_blob.first().copied().unwrap_or(0),
        "generate_invite: route blob from state"
    );
    if let Err(e) = host.node()?.routes().import(&route_blob) {
        tracing::error!(error = %e, "generate_invite: OUR OWN route blob fails to import!");
        return Err(FriendError::Failed(format!("route blob is invalid: {e}")));
    }

    let id_bytes = rekindle_utils::random::id_bytes_16_from(host.rng());
    let invite_id = uuid::Builder::from_random_bytes(id_bytes)
        .into_uuid()
        .to_string();
    let issued_at = host.clock().now_ms();
    let blob = rekindle_codec::message::create_invite_blob(
        &secret,
        &own.public_key,
        &own.display_name,
        &own.mailbox_dht_key,
        &own.profile_dht_key,
        &route_blob,
        Some(&invite_id),
        issued_at,
    );
    let url = rekindle_codec::message::encode_invite_url(&blob);
    tracing::info!(url_len = url.len(), "generated friend invite URL");

    let now = host.clock().now_ms_i64();
    let (owner, id, link) = (own.public_key, invite_id.clone(), url.clone());
    state::db_call!(host, "store invite", move |conn| {
        rekindle_db::repo::outgoing_invites::insert(conn, &owner, &id, &link, now)
    })
    .await?;
    tracing::info!(%invite_id, "generated tracked invite");
    Ok(InviteCreated { url, invite_id })
}

/// Our open invites, newest first.
///
/// # Errors
/// They could not be read.
pub async fn list(host: &dyn FriendHost) -> Result<Vec<OutgoingInvite>, FriendError> {
    let owner = state::owner(host)?;
    state::db_call!(host, "read invites", move |conn| {
        rekindle_db::repo::outgoing_invites::open(conn, &owner)
    })
    .await
}

/// Cancel an open invite: a request answering it is rejected.
///
/// # Errors
/// `NotFound` when no open invite has that id.
pub async fn cancel(host: &dyn FriendHost, invite_id: &str) -> Result<(), FriendError> {
    let owner = state::owner(host)?;
    let id = invite_id.to_string();
    let cancelled = state::db_call!(host, "cancel invite", move |conn| {
        rekindle_db::repo::outgoing_invites::cancel(conn, &owner, &id)
    })
    .await?;
    if !cancelled {
        return Err(FriendError::NotFound(format!("no open invite {invite_id}")));
    }
    tracing::info!(%invite_id, "invite cancelled");
    Ok(())
}

/// Expire our invites past their time (the maintenance task).
///
/// # Errors
/// The expiry could not be written.
pub async fn expire_stale(host: &dyn FriendHost) -> Result<usize, FriendError> {
    let owner = state::owner(host)?;
    let now = host.clock().now_ms_i64();
    state::db_call!(host, "expire invites", move |conn| {
        rekindle_db::repo::outgoing_invites::expire(conn, &owner, now)
    })
    .await
}

/// Whether `invite_id` was cancelled (a request answering it is rejected).
///
/// # Errors
/// The invite's state could not be read.
pub(crate) async fn is_cancelled(
    host: &dyn FriendHost,
    invite_id: &str,
) -> Result<bool, FriendError> {
    let owner = state::owner(host)?;
    let id = invite_id.to_string();
    state::db_call!(host, "read invite", move |conn| {
        rekindle_db::repo::outgoing_invites::is_cancelled(conn, &owner, &id)
    })
    .await
}

/// A request from `responder` answered `invite_id`.
pub(crate) fn mark_responded(host: &dyn FriendHost, invite_id: &str, responder: &str) {
    let Ok(owner) = state::owner(host) else {
        return;
    };
    let (id, who) = (invite_id.to_string(), responder.to_string());
    state::db_fire!(host, "mark invite responded", move |conn| {
        rekindle_db::repo::outgoing_invites::mark_responded(conn, &owner, &id, &who)
    });
}

/// We accepted the request that answered `invite_id`.
pub(crate) fn mark_accepted(host: &dyn FriendHost, invite_id: &str) {
    let Ok(owner) = state::owner(host) else {
        return;
    };
    let id = invite_id.to_string();
    state::db_fire!(host, "mark invite accepted", move |conn| {
        rekindle_db::repo::outgoing_invites::mark_accepted(conn, &owner, &id)
    });
}

/// We rejected the request that answered `invite_id`.
pub(crate) fn mark_rejected(host: &dyn FriendHost, invite_id: &str) {
    let Ok(owner) = state::owner(host) else {
        return;
    };
    let id = invite_id.to_string();
    state::db_fire!(host, "mark invite rejected", move |conn| {
        rekindle_db::repo::outgoing_invites::mark_rejected(conn, &owner, &id)
    });
}

/// The invite in a link: decoded, its signature checked, and no older
/// than [`MAX_INVITE_AGE_SECS`].
///
/// # Errors
/// `Invalid` when it does not decode, its signature fails, or it expired.
pub fn open(link: &str, now_ms: u64) -> Result<InviteBlob, FriendError> {
    let blob = rekindle_codec::message::decode_invite_url(link).map_err(FriendError::Invalid)?;
    rekindle_codec::message::verify_invite_blob(&blob).map_err(FriendError::Invalid)?;
    rekindle_codec::message::check_invite_recency(&blob, now_ms, MAX_INVITE_AGE_SECS)
        .map_err(FriendError::Invalid)?;
    Ok(blob)
}
