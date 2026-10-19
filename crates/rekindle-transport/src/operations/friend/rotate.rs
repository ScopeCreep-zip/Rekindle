//! Moving our profile to a new record, so a blocked user's copy of the old
//! key no longer finds us, and telling our remaining friends where it went.

use rekindle_codec::message::envelope::MessagePayload;
use rekindle_types::friend::FriendshipState;

use super::error::FriendError;
use super::host::FriendHost;
use super::state;
use crate::operations::dm;

/// Tell every friend not being removed that our profile is now at
/// `profile_dht_key`; how many the notice reached.
pub async fn notify_profile_key_rotated(host: &dyn FriendHost, profile_dht_key: &str) -> usize {
    let friends: Vec<String> = state::friends(host)
        .into_iter()
        .filter(|f| f.friendship_state != FriendshipState::Removing)
        .map(|f| f.public_key)
        .collect();
    let payload = MessagePayload::ProfileKeyRotated {
        new_profile_dht_key: profile_dht_key.to_string(),
    };
    let mut notified = 0;
    for friend in &friends {
        match dm::send_to_peer(host, friend, &payload).await {
            Ok(()) => notified += 1,
            Err(e) => tracing::warn!(to = %friend, error = %e, "failed to send ProfileKeyRotated"),
        }
    }
    notified
}

/// Move our profile to a new record under a fresh owner key (the Host's
/// own-records stage writes it, E1.1) and tell our friends.
///
/// # Errors
/// The new record could not be created or stored.
pub async fn rotate_profile_key(host: &dyn FriendHost) -> Result<(), FriendError> {
    let new_key = host.rotate_own_profile().await?;
    let notified = notify_profile_key_rotated(host, &new_key).await;
    tracing::info!(new_key = %new_key, notified, "profile DHT key rotated");
    // The action, not the credential: the new key is not logged.
    host.audit(
        rekindle_audit::AuditKind::IdentityRotated,
        serde_json::json!({ "friend_notify_count": notified }),
    )
    .await;
    Ok(())
}
