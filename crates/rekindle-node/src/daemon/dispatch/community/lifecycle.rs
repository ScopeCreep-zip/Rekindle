//! Community lifecycle: create, join, leave.

use crate::daemon::DaemonState;
use crate::ipc::protocol::IpcResponse;
use crate::validation;

use super::ownership::write_encrypted_backup;
use crate::daemon::dispatch::{state_error, DaemonContext};

pub(crate) async fn handle_create(
    ctx: &DaemonContext,
    state: DaemonState,
    name: &str,
    description: &str,
) -> IpcResponse {
    if !state.can_write() {
        return state_error(state, "write");
    }
    let name = match validation::validate_name(name, "Community") {
        Ok(n) => n,
        Err(e) => return e,
    };

    // Refuse if a community with this name already exists in the session
    {
        let guard = ctx.session.read();
        if let Some(ref s) = *guard {
            if s.community_by_name(&name).is_some() {
                return IpcResponse::error(409, format!(
                    "community '{name}' already exists — use a different name or leave the existing one first",
                ));
            }
        }
    }

    let transport = match ctx.require_transport() {
        Ok(t) => t,
        Err(e) => return e,
    };
    let signing_key = match ctx.require_signing_key() {
        Ok(k) => k,
        Err(e) => return e,
    };
    let session = match ctx.require_session(Clone::clone) {
        Ok(s) => s,
        Err(e) => return e,
    };

    let desc = if description.is_empty() {
        None
    } else {
        Some(description)
    };
    match rekindle_transport::operations::community::create_community(
        &transport,
        &session,
        &name,
        desc,
        &ctx.mek_cache,
        &signing_key,
    )
    .await
    {
        Ok(result) => {
            let gov_key_short = if result.governance_key.len() > 12 {
                &result.governance_key[..12]
            } else {
                &result.governance_key
            };
            let membership = rekindle_transport::session::CommunityMembership {
                governance_key: result.governance_key.clone(),
                pseudonym_key: result.our_pseudonym_key.clone(),
                display_name: session.identity.display_name.clone(),
                role_ids: Vec::new(),
                slot_index: 0,
                registry_key: result.registry_key.clone(),
                community_name: name.clone(),
                // The creator must keep the seed: it derives every slot
                // keypair in the registry, so without it neither we nor
                // any joiner we admit can write presence.
                slot_seed: Some(result.slot_seed),
                channel_record_keys: std::collections::HashMap::new(),
                community_mailbox_key: result.community_mailbox_key.clone(),
                join_inbox_key: result.join_inbox_key.clone(),
                is_operator: true,
                governance_keypair_label: Some(format!("community-governance-{gov_key_short}")),
                // The creator is definitionally in the genesis segment.
                // `mek_generation` stays 0 here and is read from
                // `MekCache` by the adapter — the create path does not
                // report a generation, and inventing one would make the
                // persisted value disagree with the cache that actually
                // holds the key.
                segment_index: Some(0),
                lamport_counter: 0,
                mek_generation: 0,
            };
            {
                let mut guard = ctx.session.write();
                if let Some(ref mut s) = *guard {
                    s.join_community(membership);
                }
            }
            if let Err(e) = ctx.save_session() {
                return e;
            }

            // Store governance and registry keypairs. These MUST persist —
            // without them, the community cannot process joins or govern.
            if !result.governance_keypair_bytes.is_empty() {
                if let Err(e) = crate::state::keystore::store_governance_keypair(
                    gov_key_short,
                    &result.governance_keypair_bytes,
                )
                .await
                {
                    return IpcResponse::error(
                        500,
                        format!(
                            "community created but governance keypair storage failed: {e}. \
                         The community will not function. Delete and recreate."
                        ),
                    );
                }
            }
            if !result.registry_keypair_bytes.is_empty() {
                let reg_key_short = if result.registry_key.len() > 12 {
                    &result.registry_key[..12]
                } else {
                    &result.registry_key
                };
                if let Err(e) = crate::state::keystore::store_keypair_bytes(
                    &format!("registry-{reg_key_short}"),
                    &result.registry_keypair_bytes,
                )
                .await
                {
                    return IpcResponse::error(
                        500,
                        format!(
                            "community created but registry keypair storage failed: {e}. \
                         The community will not function. Delete and recreate."
                        ),
                    );
                }
            }

            // Best-effort encrypted backup of governance + registry keypairs.
            // Recovery path if the OS keyring is lost (migration, container rebuild).
            // Encrypted with the signing key so only the identity owner can recover.
            if let Some(ref sk_handle) = *ctx.signing_key.read() {
                let backup_dir = ctx
                    .session_path
                    .parent()
                    .unwrap_or(std::path::Path::new("."));
                let gov_backup = backup_dir.join(format!("governance-backup-{gov_key_short}.enc"));
                let reg_backup = backup_dir.join(format!(
                    "registry-backup-{}.enc",
                    if result.registry_key.len() > 12 {
                        &result.registry_key[..12]
                    } else {
                        &result.registry_key
                    }
                ));
                let key = sk_handle.as_bytes();
                if let Err(e) =
                    write_encrypted_backup(&gov_backup, &result.governance_keypair_bytes, key)
                {
                    tracing::warn!(error = %e, "governance keypair backup failed — keyring is the only copy");
                }
                if let Err(e) =
                    write_encrypted_backup(&reg_backup, &result.registry_keypair_bytes, key)
                {
                    tracing::warn!(error = %e, "registry keypair backup failed — keyring is the only copy");
                }
            }

            // Register gossip mesh for the new community (synchronous, no await needed).
            // The join inbox Veilid-level watch was already established during create_community
            // (transport layer). ValueChange events route through DaemonHandler::on_value_change
            // which checks session.communities for join_inbox_key matches — no SubscriptionManager
            // WatchRegistry registration needed.
            {
                let bcast_guard = ctx.broadcast_mgr.read();
                if let Some(ref bcast_mgr) = *bcast_guard {
                    bcast_mgr.register_mesh(&result.governance_key);
                    tracing::info!(community = %name, "gossip mesh registered for new community");
                }
            }

            IpcResponse::ok(&serde_json::json!({
                "governance_key": result.governance_key,
                "registry_key": result.registry_key,
                "community_mailbox_key": result.community_mailbox_key,
                "name": name,
            }))
        }
        Err(e) => IpcResponse::error(500, format!("community create failed: {e}")),
    }
}

pub(crate) async fn handle_join(
    ctx: &DaemonContext,
    state: DaemonState,
    invite: &str,
) -> IpcResponse {
    if !state.can_write() {
        return state_error(state, "write");
    }

    // v2.0 self-sovereign join. The previous flow submitted a request to
    // an inbox and waited for an operator to *assign* a slot, then
    // derived its slot seed from its own identity key — which can never
    // match the registry's member keys, since those come from the
    // creator's shared seed. The seed rides in the invite, so a full
    // invite link is now required rather than a bare governance key.
    let Some(link) = rekindle_types::invite::InviteLink::parse(invite) else {
        return IpcResponse::error(
            400,
            "join requires a full invite link \
             (rekindle://invite/{governance_key}/{secrets_record_key}/{invite_code}) — \
             a bare governance key no longer works: the slot seed it needs lives in the invite",
        );
    };

    let session_display_name = match ctx.require_session(|s| s.identity.display_name.clone()) {
        Ok(n) => n,
        Err(e) => return e,
    };

    let adapter = crate::daemon::governance_adapter::DaemonGovernanceAdapter::new(ctx);
    let outcome = match rekindle_governance_runtime::join_flow::run_join_stages(
        &adapter,
        &link.governance_key,
        &link.invite_code,
        Some(&link.secrets_record_key),
    )
    .await
    {
        Ok(o) => o,
        Err(e) => return IpcResponse::error(500, format!("community join failed: {e}")),
    };

    // The claim reports which segment it landed in, so unlike the old
    // flow this is a fact rather than a guess.
    let slot_seed = hex::decode(&outcome.invite.slot_seed_hex)
        .ok()
        .and_then(|b| <[u8; 32]>::try_from(b).ok());
    let membership = rekindle_transport::session::CommunityMembership {
        governance_key: link.governance_key.clone(),
        pseudonym_key: outcome.identity.pseudo_hex.clone(),
        display_name: session_display_name,
        role_ids: Vec::new(),
        slot_index: outcome.claimed.local_subkey,
        registry_key: outcome.claimed.registry_key.clone(),
        community_name: outcome.invite.community_name.clone(),
        slot_seed,
        channel_record_keys: std::collections::HashMap::new(),
        // v2.0 join needs neither: admission is self-sovereign, so there
        // is no mailbox to petition and no inbox to be approved from.
        community_mailbox_key: String::new(),
        join_inbox_key: String::new(),
        is_operator: false,
        governance_keypair_label: None,
        segment_index: Some(outcome.claimed.segment_index),
        lamport_counter: 0,
        mek_generation: 0,
    };
    {
        let mut guard = ctx.session.write();
        if let Some(ref mut sess) = *guard {
            sess.join_community(membership);
        }
    }
    if let Err(e) = ctx.save_session() {
        return e;
    }

    // Seed the runtime cache with the state the join already merged, so
    // the first read afterwards does not go back to the DHT.
    ctx.community_runtime
        .set_governance_state(&link.governance_key, outcome.snapshot.gov_state);

    IpcResponse::ok(&serde_json::json!({
        "community_name": outcome.invite.community_name,
        "governance_key": link.governance_key,
        "segment": outcome.claimed.segment_index,
        "slot": outcome.claimed.local_subkey,
        "known_members": outcome.initial_presence.known_members.len(),
    }))
}

pub(crate) async fn handle_leave(
    ctx: &DaemonContext,
    state: DaemonState,
    governance_key: &str,
) -> IpcResponse {
    if !state.can_write() {
        return state_error(state, "write");
    }
    let transport = match ctx.require_transport() {
        Ok(t) => t,
        Err(e) => return e,
    };
    let membership = match ctx.resolve_community(governance_key) {
        Ok(m) => m,
        Err(e) => return e,
    };

    let signing_key = match ctx.require_signing_key() {
        Ok(k) => k,
        Err(e) => return e,
    };
    match rekindle_transport::operations::community::leave_community(
        &transport,
        &membership,
        &ctx.mek_cache,
        &signing_key,
    )
    .await
    {
        Ok(_) => {
            {
                let mut guard = ctx.session.write();
                if let Some(ref mut s) = *guard {
                    s.leave_community(&membership.governance_key);
                }
            }
            if let Err(e) = ctx.save_session() {
                return e;
            }
            IpcResponse::ok(&serde_json::json!({ "left": membership.governance_key }))
        }
        Err(e) => IpcResponse::error(500, format!("community leave failed: {e}")),
    }
}
