//! The slowmode write, until it moves onto the Host with its command
//! (plan E3.1). The message deletes are the Host's
//! (`rekindle_node::host::adapters::governance::channel_message_ops`, E2d).

pub async fn set_slowmode_inner(
    state: &crate::state::SharedState,
    community_id: String,
    channel_id: String,
    seconds: u32,
) -> Result<(), String> {
    use crate::commands::community::helpers::hex_to_id_16;
    use crate::state_helpers;

    let lamport =
        state_helpers::next_governance_lamport(state, &community_id).map_err(|e| e.to_string())?;
    crate::services::community::write_entry(
        state,
        &community_id,
        rekindle_types::governance::GovernanceEntry::ChannelUpdated {
            channel_id: rekindle_types::id::ChannelId(hex_to_id_16(&channel_id)),
            name: None,
            topic: None,
            forum_tags: None,
            position: None,
            slowmode_seconds: Some(seconds),
            nsfw: None,
            category_id: None,
            lamport,
        },
    )
    .await?;

    let mut communities = state.communities.write();
    if let Some(community) = communities.get_mut(&community_id) {
        if let Some(ch) = community.channels.iter_mut().find(|ch| ch.id == channel_id) {
            ch.slowmode_seconds = Some(seconds);
        }
    }
    Ok(())
}
