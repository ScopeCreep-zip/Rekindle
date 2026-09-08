//! Community lifecycle: create, join, leave.

use crate::daemon::DaemonState;
use crate::ipc::protocol::IpcResponse;
use crate::validation;

use rekindle_governance_runtime::deps::GovernanceRuntimeDeps as _;

use crate::daemon::dispatch::{adapter, state_error, DaemonContext};

pub(crate) async fn handle_create(
    ctx: &DaemonContext,
    state: DaemonState,
    name: &str,
    description: &str,
    approval_required: bool,
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

    // v2.0 flat-SMPL creation. The previous path built the registry as
    // a creator-owned record, published the genesis MEK into a registry
    // MEK vault, and wrote the creator into a shared member index —
    // three things `o_cnt: 0` has no writer for, and one
    // (`communities-channels.md`: the MEK is "**never** written to
    // DHT") that must not exist at all. `origin::create_community`
    // creates the three SMPL records from a shared slot seed and keeps
    // the MEK local, which is what the desktop shell has been doing via
    // `services/community/create.rs`.
    //
    // Persistence, keypair storage and the genesis governance state all
    // happen inside the flow via `Deps::insert_community`, so there is
    // no membership to hand-assemble here any more.
    // Genesis-only, so this is the one chance to set it: the merge
    // honours `AdmissionPolicy` solely as the genesis entry, which is
    // what stops anyone holding `MANAGE_COMMUNITY` flipping a gated
    // community open later.
    let admission = if approval_required {
        rekindle_types::governance::AdmissionMode::ApprovalRequired
    } else {
        rekindle_types::governance::AdmissionMode::Open
    };
    let community_id = match rekindle_governance_runtime::create_community(
        &adapter(ctx),
        &name,
        admission,
    )
    .await
    {
        Ok(id) => id,
        Err(e) => return IpcResponse::error(500, format!("community create failed: {e}")),
    };

    if !description.is_empty() {
        // Description is metadata, not part of genesis. A failure here
        // leaves a working community with no description rather than
        // failing a creation that already succeeded.
        if let Err(e) = rekindle_governance_runtime::apply::write_entry(
            &adapter(ctx),
            &community_id,
            rekindle_types::governance::GovernanceEntry::CommunityMeta {
                name: Some(name.clone()),
                description: Some(description.to_string()),
                icon_hash: None,
                banner_hash: None,
                lamport: adapter(ctx).increment_lamport(&community_id),
            },
        )
        .await
        {
            tracing::warn!(error = %e, "community created but description not written");
        }
    }

    let registry_key = ctx
        .session
        .read()
        .as_ref()
        .and_then(|s| s.community(&community_id))
        .map(|m| m.registry_key.clone())
        .unwrap_or_default();

    // Register the gossip mesh for the new community.
    {
        let bcast_guard = ctx.broadcast_mgr.read();
        if let Some(ref bcast_mgr) = *bcast_guard {
            bcast_mgr.register_mesh(&community_id);
            tracing::info!(community = %name, "gossip mesh registered for new community");
        }
    }

    IpcResponse::ok(&serde_json::json!({
        "governance_key": community_id,
        "registry_key": registry_key,
        "name": name,
    }))
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

    let adapter = adapter(ctx);
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
        Ok(result) => {
            // Tell the mesh before forgetting the community — after
            // `leave_community` the gossip overlay is still populated,
            // and the departure is what triggers peers' MEK rotation.
            // These bytes used to be built and dropped on the floor.
            crate::daemon::gossip::send(
                &ctx.gossip_tx,
                &membership.governance_key,
                &result.departure_notice,
            );
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
