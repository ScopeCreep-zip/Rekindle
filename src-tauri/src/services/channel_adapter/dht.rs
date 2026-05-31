//! Phase 23.D.4 — DHT-write/read bodies extracted from `deps_impl.rs`.
//! Each helper builds a `DHTManager`, fetches the slot keypair +
//! pseudonym credentials, and delegates to the matching
//! `rekindle_protocol::dht::community::channel_record` free fn.

use rekindle_channel::deps::{ChannelEntryItem, ChannelWriteContext};
use rekindle_channel::error::ChannelError;
use rekindle_protocol::dht::community::channel_record::{
    create_smpl_channel_record, read_all_channel_entries, read_all_channel_messages,
    write_member_forward, write_member_hand_raise, write_member_message, write_member_poll_close,
    write_member_poll_create, write_member_poll_vote, write_member_reaction, ChannelForward,
    ChannelHandRaise, ChannelMessage, ChannelPollClose, ChannelPollCreate, ChannelPollVote,
    ChannelReaction,
};
use rekindle_protocol::dht::DHTManager;

use crate::state_helpers;

use super::ChannelAdapter;

pub(super) async fn write_channel_message_smpl_impl(
    adapter: &ChannelAdapter,
    context: &ChannelWriteContext,
    channel_msg: &ChannelMessage,
) -> Result<(), ChannelError> {
    let rc = state_helpers::safe_routing_context(&adapter.state)
        .ok_or_else(|| ChannelError::Adapter("not attached".into()))?;
    let writer = context
        .slot_keypair_str
        .parse::<veilid_core::KeyPair>()
        .map_err(|e| ChannelError::Adapter(format!("invalid slot keypair: {e}")))?;
    let mgr = DHTManager::new(rc);
    let (author_pseudo, signing_key) =
        state_helpers::pseudonym_credentials(&adapter.state, &context.community_id)
            .map_err(ChannelError::Adapter)?;
    write_member_message(
        &mgr,
        &context.channel_key,
        context.slot_index,
        writer,
        author_pseudo,
        &signing_key,
        channel_msg,
    )
    .await
    .map_err(|e| ChannelError::Adapter(format!("SMPL channel write failed: {e}")))
}

pub(super) async fn write_channel_forward_smpl_impl(
    adapter: &ChannelAdapter,
    context: &ChannelWriteContext,
    forward: &ChannelForward,
) -> Result<(), ChannelError> {
    let rc = state_helpers::safe_routing_context(&adapter.state)
        .ok_or_else(|| ChannelError::Adapter("not attached".into()))?;
    let writer = context
        .slot_keypair_str
        .parse::<veilid_core::KeyPair>()
        .map_err(|e| ChannelError::Adapter(format!("invalid slot keypair: {e}")))?;
    let mgr = DHTManager::new(rc);
    let (author_pseudo, signing_key) =
        state_helpers::pseudonym_credentials(&adapter.state, &context.community_id)
            .map_err(ChannelError::Adapter)?;
    write_member_forward(
        &mgr,
        &context.channel_key,
        context.slot_index,
        writer,
        author_pseudo,
        &signing_key,
        forward,
    )
    .await
    .map_err(|e| ChannelError::Adapter(format!("SMPL channel forward write failed: {e}")))
}

pub(super) async fn write_channel_poll_create_smpl_impl(
    adapter: &ChannelAdapter,
    context: &ChannelWriteContext,
    entry: &ChannelPollCreate,
) -> Result<(), ChannelError> {
    let rc = state_helpers::safe_routing_context(&adapter.state)
        .ok_or_else(|| ChannelError::Adapter("not attached".into()))?;
    let writer = context
        .slot_keypair_str
        .parse::<veilid_core::KeyPair>()
        .map_err(|e| ChannelError::Adapter(format!("invalid slot keypair: {e}")))?;
    let mgr = DHTManager::new(rc);
    let (author_pseudo, signing_key) =
        state_helpers::pseudonym_credentials(&adapter.state, &context.community_id)
            .map_err(ChannelError::Adapter)?;
    write_member_poll_create(
        &mgr,
        &context.channel_key,
        context.slot_index,
        writer,
        author_pseudo,
        &signing_key,
        entry,
    )
    .await
    .map_err(|e| ChannelError::Adapter(format!("SMPL poll create write failed: {e}")))
}

pub(super) async fn write_channel_poll_vote_smpl_impl(
    adapter: &ChannelAdapter,
    context: &ChannelWriteContext,
    entry: &ChannelPollVote,
) -> Result<(), ChannelError> {
    let rc = state_helpers::safe_routing_context(&adapter.state)
        .ok_or_else(|| ChannelError::Adapter("not attached".into()))?;
    let writer = context
        .slot_keypair_str
        .parse::<veilid_core::KeyPair>()
        .map_err(|e| ChannelError::Adapter(format!("invalid slot keypair: {e}")))?;
    let mgr = DHTManager::new(rc);
    let (author_pseudo, signing_key) =
        state_helpers::pseudonym_credentials(&adapter.state, &context.community_id)
            .map_err(ChannelError::Adapter)?;
    write_member_poll_vote(
        &mgr,
        &context.channel_key,
        context.slot_index,
        writer,
        author_pseudo,
        &signing_key,
        entry,
    )
    .await
    .map_err(|e| ChannelError::Adapter(format!("SMPL poll vote write failed: {e}")))
}

pub(super) async fn write_channel_hand_raise_smpl_impl(
    adapter: &ChannelAdapter,
    context: &ChannelWriteContext,
    entry: &ChannelHandRaise,
) -> Result<(), ChannelError> {
    let rc = state_helpers::safe_routing_context(&adapter.state)
        .ok_or_else(|| ChannelError::Adapter("not attached".into()))?;
    let writer = context
        .slot_keypair_str
        .parse::<veilid_core::KeyPair>()
        .map_err(|e| ChannelError::Adapter(format!("invalid slot keypair: {e}")))?;
    let mgr = DHTManager::new(rc);
    let (author_pseudo, signing_key) =
        state_helpers::pseudonym_credentials(&adapter.state, &context.community_id)
            .map_err(ChannelError::Adapter)?;
    write_member_hand_raise(
        &mgr,
        &context.channel_key,
        context.slot_index,
        writer,
        author_pseudo,
        &signing_key,
        entry,
    )
    .await
    .map_err(|e| ChannelError::Adapter(format!("SMPL hand raise write failed: {e}")))
}

pub(super) async fn write_channel_poll_close_smpl_impl(
    adapter: &ChannelAdapter,
    context: &ChannelWriteContext,
    entry: &ChannelPollClose,
) -> Result<(), ChannelError> {
    let rc = state_helpers::safe_routing_context(&adapter.state)
        .ok_or_else(|| ChannelError::Adapter("not attached".into()))?;
    let writer = context
        .slot_keypair_str
        .parse::<veilid_core::KeyPair>()
        .map_err(|e| ChannelError::Adapter(format!("invalid slot keypair: {e}")))?;
    let mgr = DHTManager::new(rc);
    let (author_pseudo, signing_key) =
        state_helpers::pseudonym_credentials(&adapter.state, &context.community_id)
            .map_err(ChannelError::Adapter)?;
    write_member_poll_close(
        &mgr,
        &context.channel_key,
        context.slot_index,
        writer,
        author_pseudo,
        &signing_key,
        entry,
    )
    .await
    .map_err(|e| ChannelError::Adapter(format!("SMPL poll close write failed: {e}")))
}

pub(super) async fn write_member_reaction_smpl_impl(
    adapter: &ChannelAdapter,
    context: &ChannelWriteContext,
    reaction: &ChannelReaction,
) -> Result<(), ChannelError> {
    let rc = state_helpers::safe_routing_context(&adapter.state)
        .ok_or_else(|| ChannelError::Adapter("not attached".into()))?;
    let writer = context
        .slot_keypair_str
        .parse::<veilid_core::KeyPair>()
        .map_err(|e| ChannelError::Adapter(format!("invalid slot keypair: {e}")))?;
    let mgr = DHTManager::new(rc);
    let (author_pseudo, signing_key) =
        state_helpers::pseudonym_credentials(&adapter.state, &context.community_id)
            .map_err(ChannelError::Adapter)?;
    write_member_reaction(
        &mgr,
        &context.channel_key,
        context.slot_index,
        writer,
        author_pseudo,
        &signing_key,
        reaction,
    )
    .await
    .map_err(|e| ChannelError::Adapter(format!("SMPL reaction write failed: {e}")))
}

pub(super) async fn create_smpl_thread_record_impl(
    adapter: &ChannelAdapter,
    slot_seed_bytes: &[u8; 32],
) -> Result<String, ChannelError> {
    let rc = state_helpers::safe_routing_context(&adapter.state)
        .ok_or_else(|| ChannelError::Adapter("not attached".into()))?;
    let mgr = DHTManager::new(rc);
    let (record_key, _) = create_smpl_channel_record(&mgr, slot_seed_bytes)
        .await
        .map_err(|e| ChannelError::Adapter(format!("create lazy thread record failed: {e}")))?;
    Ok(record_key)
}

pub(super) async fn read_all_channel_entries_impl(
    adapter: &ChannelAdapter,
    record_key: &str,
    member_count: u32,
) -> Result<Vec<ChannelEntryItem>, ChannelError> {
    let rc = state_helpers::safe_routing_context(&adapter.state)
        .ok_or_else(|| ChannelError::Adapter("not attached".into()))?;
    let items = read_all_channel_entries(&rc, record_key, member_count)
        .await
        .map_err(|e| ChannelError::Adapter(format!("read channel entries failed: {e}")))?;
    Ok(items
        .into_iter()
        .map(|item| ChannelEntryItem {
            subkey_index: item.subkey_index,
            entry: item.entry,
        })
        .collect())
}

pub(super) async fn read_all_channel_messages_impl(
    adapter: &ChannelAdapter,
    record_key: &str,
    member_count: u32,
) -> Result<Vec<ChannelMessage>, ChannelError> {
    let rc = state_helpers::safe_routing_context(&adapter.state)
        .ok_or_else(|| ChannelError::Adapter("not attached".into()))?;
    read_all_channel_messages(&rc, record_key, member_count)
        .await
        .map_err(|e| ChannelError::Adapter(format!("read channel messages failed: {e}")))
}
