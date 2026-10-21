//! The pairwise sessions of a group (plan E2.6): a roster member who is
//! not our friend gets one opened from their prekey record, found through
//! the roster's `profile_record_key` (E2.4's B2 pick), with our group
//! control frame as the init's first payload; a refusal of its prekeys
//! resends that frame once from a fresh slot (B6); and the session ends
//! when the member is in no group we hold and is not our friend.

use rekindle_codec::capnp_codec::direct::{DirectBody, DmTypeId};
use rekindle_crypto::signal::PreKeyBundle;

use rekindle_types::friend::FriendshipState;

use crate::operations::direct::admission::friendship_authoritative;
use crate::operations::dm::handshake;
use crate::operations::friend::{state, FriendError, FriendHost};

/// Our group control frame that opened a session with a roster member,
/// until they answer under it.
#[derive(Debug, Clone)]
pub struct GroupFirstContact {
    pub body: DirectBody,
    /// It was resent once from a fresh slot (B6).
    pub retried: bool,
}

/// The profile record of `peer`, a member of a group we hold, as the
/// creator-signed roster names it (the decided 2026-10-21 rule): where a
/// member who is not our friend publishes the prekeys our first contact
/// opens from and the route our frames to them take. `None`: they are on
/// no roster we hold.
pub(crate) async fn member_profile(host: &dyn FriendHost, peer: &str) -> Option<String> {
    let deps = host.group_dm().ok()?;
    let groups = match deps.groups().await {
        Ok(groups) => groups,
        Err(e) => {
            tracing::debug!(error = %e, "groups not read for a member profile");
            return None;
        }
    };
    groups
        .iter()
        .find_map(|g| g.roster.member(peer))
        .map(|m| m.profile_record_key.clone())
}

/// The bundle a session with `peer`, a member of a group we hold, opens
/// from: a slot of the prekey record their roster profile names.
///
/// # Errors
/// `peer` is on no roster we hold, or no bundle can be picked from it.
pub(crate) async fn member_bundle(
    host: &dyn FriendHost,
    peer: &str,
) -> Result<PreKeyBundle, FriendError> {
    let profile = member_profile(host, peer).await.ok_or_else(|| {
        FriendError::Conflict(format!(
            "no secure session with {}: they are on no group we hold",
            state::short_key(peer)
        ))
    })?;
    crate::operations::prekeys::pick_bundle(host, peer, &profile).await
}

/// Our init carrying the group frame `body` to `peer` went out: remember
/// it until they answer, keeping a resend's `retried`.
pub(crate) fn began(host: &dyn FriendHost, peer: &str, body: &DirectBody) {
    host.stores()
        .group_first_contacts
        .lock()
        .entry(peer.to_string())
        .or_insert_with(|| GroupFirstContact {
            body: body.clone(),
            retried: false,
        });
}

/// `peer` answered under the session: the group first contact landed.
pub(crate) fn acknowledged(host: &dyn FriendHost, peer: &str) {
    host.stores().group_first_contacts.lock().remove(peer);
}

/// B6 for a group first contact: `peer` refused the prekeys of our init
/// (its session already discarded). The frame is resent once, opening a
/// session from a fresh slot; a second refusal drops it, logged. Whether
/// a group first contact was pending.
pub(crate) async fn on_rejected(host: &dyn FriendHost, peer: &str) -> bool {
    let contact = host.stores().group_first_contacts.lock().get(peer).cloned();
    let Some(contact) = contact else {
        return false;
    };
    if contact.retried {
        host.stores().group_first_contacts.lock().remove(peer);
        tracing::warn!(peer = %peer, kind = ?contact.body.type_id(),
            "they refused our keys twice; the group frame is not delivered");
        return true;
    }
    if let Some(entry) = host.stores().group_first_contacts.lock().get_mut(peer) {
        entry.retried = true;
    }
    tracing::info!(peer = %peer, kind = ?contact.body.type_id(),
        "our group first contact was refused; resending from a fresh key");
    if let Err(e) = handshake::enqueue(host, peer, &contact.body).await {
        tracing::warn!(peer = %peer, error = %e, "group first contact not resent");
    }
    true
}

/// Whether `kind` opens a session with a group member who is not a
/// friend: a pairwise group control kind on the first-payload allowlist.
#[must_use]
pub(crate) fn opens_member_session(kind: DmTypeId) -> bool {
    rekindle_dm::admission::is_group_kind(kind) && kind.is_first_payload()
}

/// End the pairwise session with each of `members` who is not our
/// accepted friend (a friend's 1:1 session stays) and is on no roster of
/// a group we still hold, after a leave (ours or theirs).
pub(crate) async fn drop_sessions(host: &dyn FriendHost, members: &[String]) {
    let own = state::own_identity(host).map(|o| o.public_key).ok();
    let held = match host.group_dm() {
        Ok(deps) => match deps.groups().await {
            Ok(groups) => groups,
            Err(e) => {
                tracing::warn!(error = %e, "groups unreadable; sessions kept");
                return;
            }
        },
        Err(_) => return,
    };
    for member in members {
        if own.as_deref() == Some(member.as_str()) || held.iter().any(|g| g.roster.contains(member))
        {
            continue;
        }
        // The stored row (the Host's friend authority), as the gate reads it.
        let friendship = friendship_authoritative(host, member).await;
        if friendship == Some(FriendshipState::Accepted) {
            continue;
        }
        host.stores().group_first_contacts.lock().remove(member);
        state::forget_session(host, member).await;
        tracing::debug!(peer = %member, "pairwise session with a former group member ended");
    }
}
