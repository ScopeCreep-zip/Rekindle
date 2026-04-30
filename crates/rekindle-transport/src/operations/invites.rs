//! Invite management operations — create, list, revoke.

use tracing::info;

use crate::error::{TransportError, Result};
use crate::node::TransportNode;
use crate::payload::dht_types::InviteEntry;

/// Create a new invite for a community.
///
/// Generates a hash of the invite code for storage (the code itself is
/// given to the invitee, the hash is stored in governance for validation).
/// Returns the invite code that the user shares.
pub async fn create_invite(
    node: &TransportNode,
    governance_key: &str,
    created_by: &str,
    max_uses: u32,
    expires_duration_secs: Option<u64>,
) -> Result<String> {
    let dht = node.dht()?;

    // Generate a random invite code
    let mut code_bytes = [0u8; 16];
    rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut code_bytes);
    let invite_code = hex::encode(code_bytes);

    // Hash the code for storage (the code itself is not stored)
    let code_hash = hex::encode(blake3::hash(invite_code.as_bytes()).as_bytes());

    let expires_at = expires_duration_secs.map(|dur| rekindle_utils::timestamp_ms() + dur * 1000);

    let entry = InviteEntry {
        code_hash,
        created_by: created_by.to_string(),
        created_at: rekindle_utils::timestamp_ms(),
        expires_at,
        max_uses,
        use_count: 0,
        encrypted_secrets: None,
    };

    let mut invites = dht.governance().read_invites(governance_key).await?;
    invites.push(entry);
    dht.governance()
        .write_invites(governance_key, &invites)
        .await?;

    info!(governance = governance_key, "invite created");
    Ok(invite_code)
}

/// List all active invites for a community.
///
/// Filters out expired invites and fully-used invites.
pub async fn list_invites(
    node: &TransportNode,
    governance_key: &str,
) -> Result<Vec<InviteEntry>> {
    let dht = node.dht()?;
    let invites = dht.governance().read_invites(governance_key).await?;
    let now = rekindle_utils::timestamp_ms();

    Ok(invites
        .into_iter()
        .filter(|inv| {
            // Not expired
            let not_expired = inv
                .expires_at
                .is_none_or(|exp| exp > now);
            // Not fully used
            let not_exhausted = inv.max_uses == 0 || inv.use_count < inv.max_uses;
            not_expired && not_exhausted
        })
        .collect())
}

/// Revoke an invite by its code.
///
/// Hashes the provided code and removes the matching entry from governance.
pub async fn revoke_invite(
    node: &TransportNode,
    governance_key: &str,
    invite_code: &str,
) -> Result<()> {
    let code_hash = hex::encode(blake3::hash(invite_code.as_bytes()).as_bytes());

    let dht = node.dht()?;
    let mut invites = dht.governance().read_invites(governance_key).await?;
    let before = invites.len();
    invites.retain(|inv| inv.code_hash != code_hash);

    if invites.len() == before {
        return Err(TransportError::DhtError {
            reason: "invite code not found".into(),
        });
    }

    dht.governance()
        .write_invites(governance_key, &invites)
        .await?;

    info!(governance = governance_key, "invite revoked");
    Ok(())
}
