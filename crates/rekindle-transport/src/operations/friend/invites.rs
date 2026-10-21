//! Friend invites we make: a signed invite link naming its single-use
//! request inbox (no route and no prekey bundle, plans C7.25, E2.5)
//! tracked in `outgoing_invites` until it is answered, cancelled or
//! expires, when its inbox is dropped; and the invite a link we open
//! carries.

use std::time::Duration;

use rekindle_codec::message::envelope::InviteBlob;
use rekindle_types::dto::friends::{InviteCreated, OutgoingInvite};
use rekindle_types::invite::FRIEND_INVITE_TTL_SECS;

use super::error::FriendError;
use super::host::FriendHost;
use super::inbox;
use super::state;

/// How long making an invite or a friend request waits for our profile
/// and mailbox to be published (right after
/// unlock, while the own-records stage writes them).
const PUBLISHED_WAIT: Duration = Duration::from_secs(30);
/// How often that wait looks again.
const PUBLISHED_POLL: Duration = Duration::from_millis(500);

/// Our identity, once the own-records stage has published our profile and
/// mailbox: an invite or a request names them, and one naming none leaves
/// the peer no prekey record to open the session from.
fn records_published(host: &dyn FriendHost) -> Option<state::OwnIdentity> {
    let own = state::own_identity(host).ok()?;
    (!own.profile_dht_key.is_empty() && !own.mailbox_dht_key.is_empty()).then_some(own)
}

/// What `ready` gives, waiting up to [`PUBLISHED_WAIT`] for it.
async fn await_published<T>(
    host: &dyn FriendHost,
    ready: impl Fn(&dyn FriendHost) -> Option<T>,
) -> Result<T, FriendError> {
    tokio::time::timeout(PUBLISHED_WAIT, async {
        loop {
            match ready(host) {
                Some(ready) => return ready,
                None => tokio::time::sleep(PUBLISHED_POLL).await,
            }
        }
    })
    .await
    .map_err(|_| {
        FriendError::NotReady("Network not ready — please wait a moment and try again".to_string())
    })
}

/// Our identity once our profile and mailbox are published, waiting up to
/// 30 s (a friend added right after the first unlock).
///
/// # Errors
/// `NotReady` when they are not published in time.
pub(crate) async fn published_identity(
    host: &dyn FriendHost,
) -> Result<state::OwnIdentity, FriendError> {
    await_published(host, records_published).await
}

/// Make a tracked friend invite.
///
/// # Errors
/// Our records are not published within 30 s, the inbox cannot be
/// created, or the invite cannot be stored.
pub async fn create(host: &dyn FriendHost) -> Result<InviteCreated, FriendError> {
    let secret = host.identity_secret()?;
    let own = await_published(host, records_published).await?;

    let id_bytes = rekindle_utils::random::id_bytes_16_from(host.rng());
    let invite_id = uuid::Builder::from_random_bytes(id_bytes)
        .into_uuid()
        .to_string();
    let issued_at = host.clock().now_ms();
    let inbox = inbox::create(host).await?;
    let blob = rekindle_codec::message::create_invite_blob(
        &secret,
        &own.public_key,
        &own.display_name,
        &own.mailbox_dht_key,
        &own.profile_dht_key,
        &inbox,
        Some(&invite_id),
        issued_at,
    );
    let url = rekindle_codec::message::encode_invite_url(&blob);
    tracing::info!(url_len = url.len(), "generated friend invite URL");

    let now = host.clock().now_ms_i64();
    let (owner, id, link) = (own.public_key, invite_id.clone(), url.clone());
    let inbox_key = inbox.record_key.clone();
    let stored = state::db_call!(host, "store invite", move |conn| {
        rekindle_db::repo::outgoing_invites::insert(conn, &owner, &id, &link, &inbox_key, now)
    })
    .await;
    let expires_at = match stored {
        Ok(expires_at) => expires_at,
        Err(e) => {
            host.drop_request_inbox(&inbox.record_key).await;
            return Err(e);
        }
    };
    tracing::info!(%invite_id, "generated tracked invite");
    Ok(InviteCreated {
        url,
        invite_id,
        created_at: now,
        expires_at,
    })
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
    inbox::drop_for(host, invite_id).await;
    tracing::info!(%invite_id, "invite cancelled");
    Ok(())
}

/// Expire our invites past their time and drop their inboxes (the
/// maintenance task); how many.
///
/// # Errors
/// The expiry could not be written.
pub async fn expire_stale(host: &dyn FriendHost) -> Result<usize, FriendError> {
    let owner = state::owner(host)?;
    let now = host.clock().now_ms_i64();
    let inboxes = state::db_call!(host, "expire invites", move |conn| {
        rekindle_db::repo::outgoing_invites::expire(conn, &owner, now)
    })
    .await?;
    for key in &inboxes {
        host.drop_request_inbox(key).await;
    }
    Ok(inboxes.len())
}

/// Whether a request from `requester` may answer `invite_id` now: the
/// invite is open and unexpired, or this requester already answered it (a
/// retry). Cancelled, expired, used and unknown invites are refused.
///
/// # Errors
/// The invite's state could not be read.
pub(crate) async fn admits_request(
    host: &dyn FriendHost,
    invite_id: &str,
    requester: &str,
) -> Result<bool, FriendError> {
    let owner = state::owner(host)?;
    let (id, who) = (invite_id.to_string(), requester.to_string());
    let now = host.clock().now_ms_i64();
    state::db_call!(host, "read invite", move |conn| {
        rekindle_db::repo::outgoing_invites::admits_request(conn, &owner, &id, &who, now)
    })
    .await
}

/// A request from `responder` answered `invite_id`: the invite leaves
/// `pending` and its inbox is dropped.
pub(crate) fn mark_responded(host: &dyn FriendHost, invite_id: &str, responder: &str) {
    let Ok(owner) = state::owner(host) else {
        return;
    };
    let (id, who) = (invite_id.to_string(), responder.to_string());
    state::db_fire!(host, "mark invite responded", move |conn| {
        rekindle_db::repo::outgoing_invites::mark_responded(conn, &owner, &id, &who)
    });
    let owned = host.owned();
    let id = invite_id.to_string();
    host.scope()
        .spawn_or_drop("drop answered invite inbox", async move {
            inbox::drop_for(owned.as_ref(), &id).await;
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
/// than [`FRIEND_INVITE_TTL_SECS`].
///
/// # Errors
/// `Invalid` when it does not decode, its signature fails, or it expired.
pub fn open(link: &str, now_ms: u64) -> Result<InviteBlob, FriendError> {
    let blob = rekindle_codec::message::decode_invite_url(link).map_err(FriendError::Invalid)?;
    rekindle_codec::message::verify_invite_blob(&blob).map_err(FriendError::Invalid)?;
    rekindle_codec::message::check_invite_recency(&blob, now_ms, FRIEND_INVITE_TTL_SECS)
        .map_err(FriendError::Invalid)?;
    Ok(blob)
}
