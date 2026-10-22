//! Community lifecycle: create, join, leave.

use crate::daemon::DaemonState;
use rekindle_ipc::protocol::IpcResponse;

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
    let name = name.trim().to_owned();

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

    // Genesis-only, so this is the one chance to set it: the merge
    // honours `AdmissionPolicy` solely as the genesis entry.
    let admission = if approval_required {
        rekindle_types::governance::AdmissionMode::ApprovalRequired
    } else {
        rekindle_types::governance::AdmissionMode::Open
    };
    let created = match adapter(ctx).create(&name, description, admission).await {
        Ok(created) => created,
        Err(e) => return IpcResponse::error(500, e),
    };
    // The shell starts its own community services on `CommunityCreated`.
    if ctx.shell.is_none() {
        let membership = ctx
            .session
            .read()
            .as_ref()
            .and_then(|s| s.community(&created.governance_key).cloned());
        if let Some(membership) = membership {
            activate_community(ctx, &membership).await;
        }
    }
    IpcResponse::ok(&created)
}

/// Bring a community this unlock created or joined up as unlock brings up
/// each resumed one (`dispatch::lifecycle`): its record keepalive, its
/// watches and subscription mesh (the message clock lives there), its
/// gossip mesh, and its presence poll.
pub(crate) async fn activate_community(
    ctx: &DaemonContext,
    membership: &rekindle_transport::session::CommunityMembership,
) {
    let community_id = membership.governance_key.clone();
    crate::daemon::keepalive::start(ctx, &community_id);
    let subscriptions = ctx.subscriptions.read().clone();
    if let Some(subscriptions) = subscriptions {
        subscriptions.setup_community(membership).await;
    }
    if let Some(broadcast) = ctx.broadcast_mgr.read().as_ref() {
        broadcast.register_mesh(&community_id);
    }
    let started = ctx.presence_start_tx.send(
        crate::daemon::presence_adapter::supervisor::PresenceStartRequest {
            community_ids: vec![community_id.clone()],
        },
    );
    if started.is_err() {
        tracing::warn!(community = %community_id, "presence supervisor is gone — no roster");
    }
    tracing::info!(community = %community_id, "community activated");
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
    let link = match rekindle_types::invite::InviteLink::parse(invite) {
        Ok(link) => link,
        Err(e) => {
            return IpcResponse::error(
                400,
                format!(
                    "join requires a full invite link \
                     (rekindle://invite/{{governance_key}}/{{secrets_record_key}}/{{invite_code}}): {e}"
                ),
            )
        }
    };

    if let Err(e) = ctx.require_session(|_| ()) {
        return e;
    }

    let adapter = adapter(ctx);
    let outcome =
        match rekindle_governance_runtime::join_flow::run_join_stages(&adapter, &link).await {
            Ok(o) => o,
            Err(e) => return IpcResponse::error(500, format!("community join failed: {e}")),
        };

    // The claim reports which segment it landed in, so unlike the old
    // flow this is a fact rather than a guess.
    let slot_seed = hex::decode(&outcome.invite.slot_seed_hex)
        .ok()
        .and_then(|b| <[u8; 32]>::try_from(b).ok());
    let membership = rekindle_transport::session::CommunityMembership {
        governance_key: link.governance_key.as_str().to_owned(),
        pseudonym_key: outcome.identity.pseudo_hex.clone(),
        role_ids: Vec::new(),
        slot_index: outcome.claimed.local_subkey,
        registry_key: outcome.claimed.registry_key.clone(),
        community_name: outcome.invite.community_name.clone(),
        slot_seed,
        channel_record_keys: std::collections::HashMap::new(),
        segment_index: Some(outcome.claimed.segment_index),
        lamport_counter: 0,
        mek_generation: 0,
    };
    // A joined community has no owner keypairs of ours: admission is
    // self-sovereign, so there is nothing to petition or be approved from.
    if let Err(e) = crate::host::session::record_membership(
        &ctx.session,
        &ctx.db,
        &ctx.vault,
        &ctx.communities,
        crate::host::session::NewMembership {
            membership,
            dht_owner_keypair: None,
            registry_keypair: None,
            joined_at: ctx.clock.now_ms_i64(),
        },
    ) {
        return IpcResponse::error(500, format!("community joined but not recorded: {e}"));
    }

    // The join's merge input into the warm cache (plan E3.1 C1), after
    // the membership's row, which the cache row references.
    rekindle_governance_runtime::GovernanceRuntimeDeps::persist_governance_entries_cache(
        &adapter,
        link.governance_key.as_str(),
        &outcome.snapshot.all_entries,
    );
    // Seed the runtime cache with the state the join already merged, so
    // the first read afterwards does not go back to the DHT.
    rekindle_governance_runtime::GovernanceRuntimeDeps::set_governance_state(
        &adapter,
        link.governance_key.as_str(),
        outcome.snapshot.gov_state,
    );
    let membership = ctx
        .session
        .read()
        .as_ref()
        .and_then(|s| s.community(link.governance_key.as_str()).cloned());
    if let Some(membership) = membership {
        activate_community(ctx, &membership).await;
    }

    IpcResponse::ok(&serde_json::json!({
        "community_name": outcome.invite.community_name,
        "governance_key": link.governance_key.as_str(),
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
    let membership = match ctx.resolve_community(governance_key) {
        Ok(m) => m,
        Err(e) => return e,
    };
    match adapter(ctx).leave(&membership).await {
        Ok(()) => IpcResponse::ok(&serde_json::json!({ "left": membership.governance_key })),
        Err(e) => IpcResponse::error(500, e),
    }
}

pub(crate) async fn handle_update_info(
    ctx: &DaemonContext,
    state: DaemonState,
    governance_key: &str,
    edit: crate::host::adapters::governance::InfoEdit,
) -> IpcResponse {
    if !state.can_write() {
        return state_error(state, "write");
    }
    if let Err(e) = ctx.require_transport() {
        return e;
    }
    let membership = match ctx.resolve_community(governance_key) {
        Ok(m) => m,
        Err(e) => return e,
    };
    match adapter(ctx)
        .update_info(&membership.governance_key, edit)
        .await
    {
        Ok(()) => IpcResponse::ok(&serde_json::json!({ "updated": membership.governance_key })),
        Err(e) => IpcResponse::error(500, e),
    }
}
