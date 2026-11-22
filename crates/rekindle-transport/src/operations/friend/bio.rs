//! Our bio, shared with friends (plan E7.2): ratchet-sealed `ProfileBio`
//! (0x0171) to each accepted friend when it changes and to a newly accepted
//! friend. Never in the profile record: its key travels in every invite and
//! friend code, so anyone holding one would read it. A received frame
//! replaces what we held from that friend (`friends.bio`, cleared when the
//! friendship ends).

use rekindle_codec::capnp_codec::direct::{DirectBody, ProfileBio};
use rekindle_types::friend::FriendshipState;
use rekindle_types::subscription_events::{FriendEvent, SubscriptionEvent};

use super::{state, FriendHost};
use crate::operations::dm;

/// Our bio as stored; `None` when we have none or it is unreadable.
async fn ours(host: &dyn FriendHost) -> Option<String> {
    let owner = state::owner(host).ok()?;
    let read = state::db_call!(host, "read our bio", move |conn| {
        rekindle_db::repo::bio::own(conn, &owner)
    })
    .await;
    match read {
        Ok(bio) => bio,
        Err(e) => {
            tracing::debug!(error = %e, "bio unread: not sent");
            None
        }
    }
}

async fn enqueue(host: &dyn FriendHost, peer: &str, body: &DirectBody) {
    if let Err(e) = dm::handshake::enqueue(host, peer, body).await {
        tracing::warn!(to = %peer, error = %e, "ProfileBio not committed to the outbox");
    }
}

/// Send our bio to `peer`, just accepted, when we have one.
pub(crate) async fn send(host: &dyn FriendHost, peer: &str) {
    if let Some(bio) = ours(host).await {
        enqueue(
            host,
            peer,
            &DirectBody::ProfileBio(ProfileBio { bio: Some(bio) }),
        )
        .await;
    }
}

/// Our bio changed to `bio` (`None`: removed): send it to every accepted
/// friend. How many it reached.
pub async fn send_to_friends(host: &dyn FriendHost, bio: Option<String>) -> usize {
    let body = DirectBody::ProfileBio(ProfileBio { bio });
    let friends: Vec<String> = state::friends(host)
        .into_iter()
        .filter(|f| f.friendship_state == FriendshipState::Accepted)
        .map(|f| f.public_key)
        .collect();
    for friend in &friends {
        enqueue(host, friend, &body).await;
    }
    friends.len()
}

/// A friend's `ProfileBio`: kept only from an accepted friend, replacing
/// what we held from them. Whether it was handled.
pub(crate) async fn on_profile_bio(host: &dyn FriendHost, sender: &str, body: &ProfileBio) -> bool {
    if state::friend(host, sender).map(|f| f.friendship_state) != Some(FriendshipState::Accepted) {
        tracing::debug!(from = %sender, "ProfileBio from a non-friend dropped");
        return true;
    }
    let Ok(owner) = state::owner(host) else {
        return false;
    };
    let peer = sender.to_string();
    let bio = body.bio.clone();
    let bio_digest = blake3::hash(bio.as_deref().unwrap_or_default().as_bytes())
        .to_hex()
        .to_string();
    let stored = state::db_call!(host, "store a friend's bio", move |conn| {
        rekindle_db::repo::bio::set_friend(conn, &owner, &peer, bio.as_deref())
    })
    .await;
    match stored {
        Ok(_) => {
            // The Host's activity feed folds it (lead decision (51)).
            host.publish(SubscriptionEvent::Friend(FriendEvent::BioChanged {
                peer_key: sender.to_owned(),
                bio_digest,
            }));
            true
        }
        Err(e) => {
            tracing::warn!(from = %sender, error = %e, "friend bio not stored");
            false
        }
    }
}
