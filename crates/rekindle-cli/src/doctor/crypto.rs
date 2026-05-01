//! Doctor checks: crypto health — identity key, prekeys, MEK cache.

use crate::doctor::{Check, Status};
use crate::transport::TransportHandle;

const PREKEY_THRESHOLD: u32 = 10;

/// Run all crypto health checks.
pub async fn checks(
    handle: &TransportHandle,
    session: &rekindle_transport::Session,
) -> Vec<Check> {
    let mut results = Vec::new();

    // crypto.identity_key — verify the identity key exists and loads from keyring
    let identity_check = match crate::identity::keystore::load_signing_key().await {
        Ok(key) => {
            let fingerprint = hex::encode(&blake3::hash(&key).as_bytes()[..8]);
            Check {
                id: "crypto.identity_key".into(),
                category: "crypto",
                status: Status::Pass,
                value: format!("Ed25519 (BLAKE3:{fingerprint})"),
                description: String::new(),
            }
        }
        Err(e) => Check {
            id: "crypto.identity_key".into(),
            category: "crypto",
            status: Status::Fail,
            value: "not accessible".into(),
            description: format!(
                "identity key not found in keyring: {e}\n\
                 re-initialize: rekindle init"
            ),
        },
    };
    results.push(identity_check);

    // crypto.prekeys.available — check prekey bundle availability
    let prekey_check = match handle.node().dht() {
        Ok(dht) => {
            match dht
                .profile()
                .prekey_count(&session.identity.profile_dht_key)
                .await
            {
                Ok(count) => {
                    let status = if count >= PREKEY_THRESHOLD {
                        Status::Pass
                    } else if count > 0 {
                        Status::Warn
                    } else {
                        Status::Fail
                    };
                    Check {
                        id: "crypto.prekeys.available".into(),
                        category: "crypto",
                        status,
                        value: format!("{count} available (threshold: {PREKEY_THRESHOLD})"),
                        description: if status == Status::Pass {
                            String::new()
                        } else {
                            "prekeys are low — new contacts cannot establish sessions\n\
                             replenish with: rekindle key prekeys replenish"
                                .into()
                        },
                    }
                }
                Err(e) => Check {
                    id: "crypto.prekeys.available".into(),
                    category: "crypto",
                    status: Status::Warn,
                    value: "unreadable".into(),
                    description: format!(
                        "failed to read prekey count: {e}\n\
                         profile DHT record may be inaccessible"
                    ),
                },
            }
        }
        Err(e) => Check {
            id: "crypto.prekeys.available".into(),
            category: "crypto",
            status: Status::Fail,
            value: "DHT unavailable".into(),
            description: format!("cannot access DHT: {e}"),
        },
    };
    results.push(prekey_check);

    // crypto.mek_cache — MEK cache state across all communities
    let cache = handle.mek_cache.read();
    let total_entries = cache.total_entries();
    let channel_count = cache.channel_count();

    results.push(Check {
        id: "crypto.mek_cache".into(),
        category: "crypto",
        status: if total_entries > 0 || session.communities.is_empty() {
            Status::Pass
        } else {
            Status::Warn
        },
        value: format!(
            "{total_entries} entries, {channel_count} channels, {} communities",
            session.communities.len()
        ),
        description: if total_entries == 0 && !session.communities.is_empty() {
            "no MEKs cached but you're a member of communities\n\
             MEKs may need to be re-requested after restart\n\
             request with: rekindle key mek request -c <community> -C <channel>"
                .into()
        } else {
            String::new()
        },
    });

    results
}
