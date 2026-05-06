//! Invite management operations — create, list, revoke.
//!
//! Typed reads/writes via `dht/governance.rs`.

use tracing::info;

use crate::error::{TransportError, Result};
use crate::broadcast::node::TransportNode;
use crate::payload::dht_types::InviteEntry;

pub async fn create_invite(
    node: &TransportNode, governance_key: &str, created_by: &str,
    max_uses: u32, expires_duration_secs: Option<u64>,
) -> Result<String> {
    let dht = node.dht()?;
    let mut code_bytes = [0u8; 16];
    rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut code_bytes);
    let invite_code = hex::encode(code_bytes);
    let code_hash = hex::encode(blake3::hash(invite_code.as_bytes()).as_bytes());
    let expires_at = expires_duration_secs.map(|dur| rekindle_utils::timestamp_ms() + dur * 1000);
    let entry = InviteEntry {
        code_hash, created_by: created_by.to_string(),
        created_at: rekindle_utils::timestamp_ms(), expires_at,
        max_uses, use_count: 0, encrypted_secrets: None,
    };
    let mut invites = dht.governance().read_invites(governance_key).await?;
    invites.push(entry);
    dht.governance().write_invites(governance_key, &invites).await?;
    info!(governance = governance_key, "invite created");
    Ok(invite_code)
}

pub async fn list_invites(node: &TransportNode, governance_key: &str) -> Result<Vec<InviteEntry>> {
    let invites = node.dht()?.governance().read_invites(governance_key).await?;
    let now = rekindle_utils::timestamp_ms();
    Ok(invites.into_iter().filter(|inv| {
        inv.expires_at.is_none_or(|exp| exp > now) && (inv.max_uses == 0 || inv.use_count < inv.max_uses)
    }).collect())
}

pub async fn revoke_invite(node: &TransportNode, governance_key: &str, invite_code: &str) -> Result<()> {
    let code_hash = hex::encode(blake3::hash(invite_code.as_bytes()).as_bytes());
    let dht = node.dht()?;
    let mut invites = dht.governance().read_invites(governance_key).await?;
    let before = invites.len();
    invites.retain(|inv| inv.code_hash != code_hash);
    if invites.len() == before {
        return Err(TransportError::DhtError { reason: "invite code not found".into() });
    }
    dht.governance().write_invites(governance_key, &invites).await?;
    info!(governance = governance_key, "invite revoked");
    Ok(())
}
