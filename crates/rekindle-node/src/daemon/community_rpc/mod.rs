//! Community RPC handlers and DHT inbox processor.
//!
//! Join is fully DHT-based — the owner's daemon polls the join inbox
//! record and processes pending requests by writing to the registry.
//!
//! Leave notification is still best-effort RPC (fire-and-forget from
//! the leaving member to the community route for cleanup + rekey).

use std::sync::Arc;
use std::time::Duration;

use parking_lot::RwLock;

mod inbox;
mod inbox_stages;
mod leave;

pub use inbox::process_inbox;
pub(crate) use leave::handle_leave;

/// Maximum time the leave handler may run before returning.
pub(crate) const HANDLER_DEADLINE: Duration = Duration::from_secs(12);

/// Module-level cache for registry keypairs loaded from the OS keyring.
static REGISTRY_KEYPAIR_CACHE: std::sync::LazyLock<
    parking_lot::Mutex<std::collections::HashMap<String, Vec<u8>>>,
> = std::sync::LazyLock::new(|| parking_lot::Mutex::new(std::collections::HashMap::new()));

// ── Helpers ─────────────────────────────────────────────────────────────

/// Load registry keypair bytes, using module-level cache.
pub(crate) async fn load_registry_keypair(registry_key: &str) -> Option<Vec<u8>> {
    let short = if registry_key.len() > 12 {
        &registry_key[..12]
    } else {
        registry_key
    };
    let label = format!("registry-{short}");
    {
        let cache = REGISTRY_KEYPAIR_CACHE.lock();
        if let Some(bytes) = cache.get(&label) {
            return Some(bytes.clone());
        }
    }
    match crate::state::keystore::load_keypair_bytes(&label).await {
        Ok(Some(bytes)) => {
            REGISTRY_KEYPAIR_CACHE.lock().insert(label, bytes.clone());
            Some(bytes)
        }
        _ => None,
    }
}

/// Open a registry record writable using the cached keypair.
pub(crate) async fn open_registry_writable(
    node: &rekindle_transport::TransportNode,
    registry_key: &str,
) -> bool {
    let Some(kp_bytes) = load_registry_keypair(registry_key).await else {
        tracing::warn!("registry keypair not in keyring — opening readonly");
        let _ = rekindle_transport::broadcast::dht_writes::open_readonly(node, registry_key).await;
        return false;
    };
    let Ok(kp) = rekindle_transport::deserialize_keypair(&kp_bytes) else {
        tracing::warn!("registry keypair deserialize failed — opening readonly");
        let _ = rekindle_transport::broadcast::dht_writes::open_readonly(node, registry_key).await;
        return false;
    };
    match rekindle_transport::broadcast::dht_writes::open_writable(node, registry_key, kp).await {
        Ok(()) => true,
        Err(e) => {
            tracing::warn!(error = %e, "registry open writable failed — falling back to readonly");
            let _ =
                rekindle_transport::broadcast::dht_writes::open_readonly(node, registry_key).await;
            false
        }
    }
}

/// Validate operator status for a community, return its registry key.
pub(crate) fn require_operator_registry(
    session: &RwLock<Option<rekindle_transport::Session>>,
    governance_key: &str,
) -> Option<String> {
    let guard = session.read();
    let sess = guard.as_ref()?;
    let membership = sess.community(governance_key)?;
    if !membership.is_operator {
        return None;
    }
    Some(membership.registry_key.clone())
}

pub(crate) fn get_signing_key(
    signing_key: &RwLock<Option<crate::state::keystore::SigningKeyHandle>>,
) -> Option<[u8; 32]> {
    signing_key.read().as_ref().map(|h| *h.as_bytes())
}

pub(crate) fn get_transport(
    transport: &RwLock<Option<Arc<rekindle_transport::TransportNode>>>,
) -> Option<Arc<rekindle_transport::TransportNode>> {
    transport.read().as_ref().map(Arc::clone)
}
