//! Community resume: the daemon re-borrows its communities' records after
//! an unlock (plan C7.7c). The identity's own records are the Host's.

use std::collections::HashMap;

use rekindle_records::lease::CommunityLeases;
use tracing::{info, warn};

use super::TransportNode;
use crate::error::Result;

impl TransportNode {
    /// Borrow every community's governance record (read-only) and registry
    /// (writable with our slot keypair) from the session's pool, and return
    /// those leases by governance key for the host to hold (plan C7.7c).
    /// The identity's own records are the Host's own-records stage (plan
    /// E1.1). Daemon-only until E3.1 ports community resume into the Host.
    ///
    /// Without this a resumed daemon's communities have no open records:
    /// presence and leave writes would re-open them per call. A community
    /// whose records do not open is logged and resumed without them.
    ///
    /// # Errors
    /// No session is unlocked (no record pool).
    pub async fn resume_communities(
        &self,
        session: &crate::session::Session,
    ) -> Result<HashMap<String, CommunityLeases>> {
        info!(
            "resuming communities for {}",
            &session.identity.display_name
        );
        self.require_records()?;
        // Borrow each community's governance and registry from the
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
                rekindle_crypto::keypair_text::slot_keypair_text(&seed, membership.slot_index).ok()
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
