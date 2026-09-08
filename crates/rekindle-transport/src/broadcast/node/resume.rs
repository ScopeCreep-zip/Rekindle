//! Session resume — reopens DHT records and republishes routes after
//! login, restoring the operational state that `start()` alone doesn't
//! establish.

use tracing::{info, warn};

use super::deserialize_keypair;
use super::TransportNode;
use crate::error::Result;

impl TransportNode {
    /// Resume operational state from a persisted session.
    ///
    /// This is the Rekindle bootstrap — the complement to `start()` which
    /// only bootstraps Veilid. Every time the node starts (CLI one-shot,
    /// TUI launch, daemon restart), this method must be called with the
    /// session loaded from disk. It:
    ///
    /// 1. Allocates a private route so peers can reach us
    /// 2. Reopens our profile DHT record and publishes the new route blob
    /// 3. Reopens our mailbox DHT record and publishes the new route blob
    /// 4. Reopens our friend list DHT record (readonly)
    /// 5. Reopens all community governance and registry records (readonly)
    ///
    /// Without this, the node is a blank Veilid peer with no Rekindle
    /// identity. DHT reads fail with "record not open", friend requests
    /// fail with "no route allocated", and the TUI shows empty data.
    ///
    /// Errors in individual steps are logged but don't fail the resume —
    /// a partially resumed node is better than no node. The caller can
    /// check `status_snapshot()` to see what succeeded.
    pub async fn resume(
        &self,
        session: &crate::session::Session,
        signing_key_bytes: &[u8; 32],
    ) -> Result<()> {
        info!("resuming session for {}", &session.identity.display_name);

        // Step 1: Allocate private route via broadcast primitive
        let route_blob = match crate::broadcast::route::allocate_personal(self).await {
            Ok((route_id, blob)) => {
                info!(route = route_id, "private route allocated");
                blob
            }
            Err(e) => {
                warn!(error = %e, "route allocation failed — incoming messages will fail");
                Vec::new()
            }
        };

        // Step 2: Reopen profile and publish route blob
        if route_blob.is_empty() {
            let _ = crate::broadcast::dht_writes::open_readonly(
                self,
                &session.identity.profile_dht_key,
            )
            .await;
            let _ = crate::broadcast::dht_writes::open_readonly(
                self,
                &session.identity.mailbox_dht_key,
            )
            .await;
        } else {
            if let Some(ref keypair_bytes) = session.identity.profile_keypair_bytes {
                if let Ok(kp) = deserialize_keypair(keypair_bytes) {
                    match crate::broadcast::dht_writes::open_writable(
                        self,
                        &session.identity.profile_dht_key,
                        kp,
                    )
                    .await
                    {
                        Ok(()) => {
                            let _ = crate::broadcast::dht_writes::set(
                                self,
                                &session.identity.profile_dht_key,
                                crate::payload::dht_types::PROFILE_SUBKEY_ROUTE_BLOB,
                                route_blob.clone(),
                                None,
                            )
                            .await;
                            info!(
                                key = session.identity.profile_dht_key.as_str(),
                                "profile reopened + route published"
                            );
                        }
                        Err(e) => {
                            warn!(error = %e, "profile reopen failed — falling back to readonly");
                            let _ = crate::broadcast::dht_writes::open_readonly(
                                self,
                                &session.identity.profile_dht_key,
                            )
                            .await;
                        }
                    }
                }
            } else {
                let _ = crate::broadcast::dht_writes::open_readonly(
                    self,
                    &session.identity.profile_dht_key,
                )
                .await;
            }

            // Step 3: Reopen mailbox and publish route blob
            let identity_keypair = {
                let sk = ed25519_dalek::SigningKey::from_bytes(signing_key_bytes);
                let pk = sk.verifying_key();
                let bare_pub = veilid_core::BarePublicKey::new(&pk.to_bytes());
                let bare_secret = veilid_core::BareSecretKey::new(signing_key_bytes);
                let veilid_pub =
                    veilid_core::PublicKey::new(veilid_core::CRYPTO_KIND_VLD0, bare_pub);
                veilid_core::KeyPair::new_from_parts(veilid_pub, bare_secret)
            };
            match crate::broadcast::dht_writes::open_writable(
                self,
                &session.identity.mailbox_dht_key,
                identity_keypair,
            )
            .await
            {
                Ok(()) => {
                    let dht = self.dht()?;
                    let _ = dht
                        .mailbox()
                        .update_route(&session.identity.mailbox_dht_key, &route_blob)
                        .await;
                    info!(
                        key = session.identity.mailbox_dht_key.as_str(),
                        "mailbox reopened + route published"
                    );
                }
                Err(e) => {
                    warn!(error = %e, "mailbox reopen failed");
                    let _ = crate::broadcast::dht_writes::open_readonly(
                        self,
                        &session.identity.mailbox_dht_key,
                    )
                    .await;
                }
            }
        }

        // Step 4: Reopen friend list
        if let Some(ref kp_bytes) = session.identity.friend_list_keypair_bytes {
            if let Ok(kp) = deserialize_keypair(kp_bytes) {
                match crate::broadcast::dht_writes::open_writable(
                    self,
                    &session.identity.friend_list_dht_key,
                    kp,
                )
                .await
                {
                    Ok(()) => info!(
                        key = session.identity.friend_list_dht_key.as_str(),
                        "friend list reopened writable"
                    ),
                    Err(e) => {
                        warn!(error = %e, "friend list writable reopen failed, falling back to readonly");
                        let _ = crate::broadcast::dht_writes::open_readonly(
                            self,
                            &session.identity.friend_list_dht_key,
                        )
                        .await;
                    }
                }
            } else {
                let _ = crate::broadcast::dht_writes::open_readonly(
                    self,
                    &session.identity.friend_list_dht_key,
                )
                .await;
            }
        } else if let Err(e) =
            crate::broadcast::dht_writes::open_readonly(self, &session.identity.friend_list_dht_key)
                .await
        {
            warn!(error = %e, "friend list reopen failed");
        } else {
            info!(
                key = session.identity.friend_list_dht_key.as_str(),
                "friend list reopened readonly (no keypair)"
            );
        }

        // Step 5: Reopen community governance + registry records (readonly)
        for membership in session.communities.values() {
            if let Err(e) =
                crate::broadcast::dht_writes::open_readonly(self, &membership.governance_key).await
            {
                warn!(error = %e, community = membership.community_name.as_str(), "governance reopen failed");
            }
            if let Err(e) =
                crate::broadcast::dht_writes::open_readonly(self, &membership.registry_key).await
            {
                warn!(error = %e, community = membership.community_name.as_str(), "registry reopen failed");
            }
        }

        // Under flat governance there is no per-community route to
        // republish. A member is reached through the route blob in its
        // own registry row, refreshed by the presence heartbeat, and a
        // joiner through `InviteSecrets::inviter_route_blob` — neither
        // needs a shared endpoint that one privileged peer maintains.

        let community_count = session.communities.len();
        info!(communities = community_count, "session resumed");
        Ok(())
    }
}
