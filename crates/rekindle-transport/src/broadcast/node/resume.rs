//! Session resume — reopens DHT records and republishes routes after
//! login, restoring the operational state that `start()` alone doesn't
//! establish.

use std::collections::HashMap;

use rekindle_records::lease::CommunityLeases;
use tracing::{info, warn};

use super::deserialize_keypair;
use super::TransportNode;
use crate::error::{Result, TransportError};

impl TransportNode {
    /// Resume operational state from a persisted session.
    ///
    /// This is the Rekindle bootstrap — the complement to `start()` which
    /// only bootstraps Veilid. Every time the node starts (CLI one-shot,
    /// TUI launch, daemon restart), this method must be called with the
    /// session loaded from disk. It:
    ///
    /// 1. Wants our routes (nothing is allocated while locked; plan C7.9d)
    /// 2. Holds our profile, mailbox and friend list writable in the
    ///    session's record pool, and publishes the personal route blob to
    ///    the profile and mailbox if one is live; one that lands later is
    ///    published by the route publisher
    /// 3. Borrows every community's governance record (read-only) and
    ///    registry (writable with our slot keypair) from the pool, and
    ///    returns those leases by governance key for the host to hold
    ///    (plan C7.7c)
    ///
    /// Without this, the node is a blank Veilid peer with no Rekindle
    /// identity. DHT reads fail with "record not open", friend requests
    /// fail with "no route allocated", and the TUI shows empty data.
    ///
    /// # Errors
    /// One of our own records could not be opened writable (step 2): the
    /// unlock stays locked rather than run unreachable. Community steps
    /// are logged and do not fail the resume.
    pub async fn resume(
        &self,
        session: &crate::session::Session,
        signing_key_bytes: &[u8; 32],
    ) -> Result<HashMap<String, CommunityLeases>> {
        info!("resuming session for {}", &session.identity.display_name);

        // Step 1: want our routes; their owner allocates once the network
        // is ready, off this path.
        self.want_routes();

        // Steps 2-4: hold our own records writable in the session's record
        // pool and publish the route blob. Each opens writable or fails the
        // resume: there is no readonly fallback, since a session that cannot
        // write its profile or mailbox is unreachable (plan C7.4).
        let pool = self.require_records()?;
        let owner_keypair = |bytes: Option<&Vec<u8>>, record: &str| {
            bytes
                .ok_or_else(|| TransportError::Internal(format!("no {record} owner keypair")))
                .and_then(|b| deserialize_keypair(b))
        };
        let identity_keypair = {
            let sk = ed25519_dalek::SigningKey::from_bytes(signing_key_bytes);
            super::ed25519_to_keypair(&sk)
        };
        let id = &session.identity;

        rekindle_protocol::dht::profile::open_profile(
            &pool,
            &id.profile_dht_key,
            owner_keypair(id.profile_keypair_bytes.as_ref(), "profile")?,
        )
        .await?;
        rekindle_protocol::dht::mailbox::open_mailbox_writable(
            &pool,
            &id.mailbox_dht_key,
            identity_keypair,
        )
        .await?;
        rekindle_protocol::dht::friends::open_friend_list(
            &pool,
            &id.friend_list_dht_key,
            owner_keypair(id.friend_list_keypair_bytes.as_ref(), "friend list")?,
        )
        .await?;
        info!("profile, mailbox and friend list reopened writable");

        // Read after the records are held: a blob that landed before this
        // found nothing writable to publish to, so it is published here; one
        // landing after is the route publisher's.
        if let Some(route_blob) = self.personal_route_blob() {
            let profile = rekindle_protocol::dht::profile::set_own_profile_subkey(
                &pool,
                &id.profile_dht_key,
                crate::payload::dht_types::PROFILE_SUBKEY_ROUTE_BLOB,
                route_blob.clone(),
            )
            .await?;
            let mailbox = rekindle_protocol::dht::mailbox::update_mailbox_route(
                &pool,
                &id.mailbox_dht_key,
                &route_blob,
            )
            .await?;
            info!(?profile, ?mailbox, "route blob published");
        }

        // Step 3: borrow each community's governance and registry from the
        // pool. The registry carries our slot keypair as its sticky writer,
        // so the presence heartbeat and a leave write through this lease
        // instead of re-opening the record (V5).
        let mut communities = HashMap::new();
        for membership in session.communities.values() {
            let mut leases = CommunityLeases::default();
            match crate::broadcast::dht_writes::acquire_str(self, &membership.governance_key, None)
                .await
            {
                Ok(lease) => leases.governance = Some(lease),
                Err(e) => {
                    warn!(error = %e, community = membership.community_name.as_str(), "governance reopen failed");
                }
            }
            let slot_writer = membership.slot_seed.and_then(|seed| {
                crate::broadcast::dht_writes::derive_slot_keypair_str(&seed, membership.slot_index)
                    .ok()
            });
            match crate::broadcast::dht_writes::acquire_str(
                self,
                &membership.registry_key,
                slot_writer.as_deref(),
            )
            .await
            {
                Ok(lease) => leases.registry = Some(lease),
                Err(e) => {
                    warn!(error = %e, community = membership.community_name.as_str(), "registry reopen failed");
                }
            }
            communities.insert(membership.governance_key.clone(), leases);
        }

        // Under flat governance there is no per-community route to
        // republish. A member is reached through the route blob in its
        // own registry row, refreshed by the presence heartbeat, and a
        // joiner through `InviteSecrets::inviter_route_blob` — neither
        // needs a shared endpoint that one privileged peer maintains.

        let community_count = session.communities.len();
        info!(communities = community_count, "session resumed");
        Ok(communities)
    }
}
