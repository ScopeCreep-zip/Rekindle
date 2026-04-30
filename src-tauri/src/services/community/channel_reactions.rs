use std::sync::Arc;

use rekindle_protocol::dht::community::channel_record::ChannelReaction;
use rekindle_protocol::dht::community::envelope::{CommunityEnvelope, ControlPayload};

use crate::state::SharedState;
use crate::state_helpers;

struct ReactionWriteContext {
    channel_key: String,
    reactor_pseudonym: String,
    slot_index: u32,
    slot_keypair: veilid_core::KeyPair,
}

pub async fn persist_reaction(
    state: &Arc<crate::state::AppState>,
    community_id: &str,
    channel_id: &str,
    message_id: &str,
    expression: &str,
    added: bool,
) -> Result<(), String> {
    let context = reaction_write_context(state, community_id, channel_id)?;
    let reaction = ChannelReaction {
        message_id: message_id.to_string(),
        expression: expression.to_string(),
        added,
        lamport: state_helpers::increment_lamport(state, community_id),
    };

    let write_result = write_reaction_once(state, &context, &reaction).await;
    let gossip_result = send_reaction_gossip(
        state,
        community_id,
        channel_id,
        message_id,
        expression,
        &context.reactor_pseudonym,
        added,
    );

    match (write_result, gossip_result) {
        (Ok(()), _) | (_, Ok(())) => Ok(()),
        (Err(write_error), Err(gossip_error)) => Err(format!(
            "reaction delivery failed: SMPL write: {write_error}; gossip notify: {gossip_error}"
        )),
    }
}

fn reaction_write_context(
    state: &SharedState,
    community_id: &str,
    channel_id: &str,
) -> Result<ReactionWriteContext, String> {
    let communities = state.communities.read();
    let community = communities
        .get(community_id)
        .ok_or("community not found for reaction write")?;
    let slot_keypair = community
        .slot_keypair
        .clone()
        .ok_or("slot keypair missing for reaction write")?
        .parse::<veilid_core::KeyPair>()
        .map_err(|e| format!("invalid slot keypair: {e}"))?;
    Ok(ReactionWriteContext {
        channel_key: community
            .channel_log_keys
            .get(channel_id)
            .cloned()
            .ok_or("channel record key missing for reaction write")?,
        reactor_pseudonym: community
            .my_pseudonym_key
            .clone()
            .ok_or("pseudonym key missing for reaction write")?,
        slot_index: community
            .my_subkey_index
            .ok_or("subkey index missing for reaction write")?,
        slot_keypair,
    })
}

async fn write_reaction_once(
    state: &SharedState,
    context: &ReactionWriteContext,
    reaction: &ChannelReaction,
) -> Result<(), String> {
    let rc = state_helpers::safe_routing_context(state).ok_or("not attached")?;
    let mgr = rekindle_protocol::dht::DHTManager::new(rc);
    rekindle_protocol::dht::community::channel_record::write_member_reaction(
        &mgr,
        &context.channel_key,
        context.slot_index,
        context.slot_keypair.clone(),
        reaction,
    )
    .await
    .map_err(|e| format!("SMPL reaction write failed: {e}"))
}

fn send_reaction_gossip(
    state: &SharedState,
    community_id: &str,
    channel_id: &str,
    message_id: &str,
    expression: &str,
    reactor_pseudonym: &str,
    added: bool,
) -> Result<(), String> {
    let payload = if added {
        ControlPayload::ReactionAdded {
            channel_id: channel_id.to_string(),
            message_id: message_id.to_string(),
            emoji: expression.to_string(),
            reactor_pseudonym: reactor_pseudonym.to_string(),
        }
    } else {
        ControlPayload::ReactionRemoved {
            channel_id: channel_id.to_string(),
            message_id: message_id.to_string(),
            emoji: expression.to_string(),
            reactor_pseudonym: reactor_pseudonym.to_string(),
        }
    };
    crate::services::community::send_to_mesh(
        state,
        community_id,
        &CommunityEnvelope::Control(payload),
    )
}
