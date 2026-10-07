//! Phase 23.D.4 — DHT-write/read bodies extracted from `deps_impl.rs`.
//! Each helper takes the session's record pool, fetches the slot keypair +
//! pseudonym credentials, and delegates to the matching
//! `rekindle_protocol::dht::community::channel_record` free fn.

use rekindle_channel::deps::{ChannelEntryItem, ChannelWriteContext, DhtWrite};
use rekindle_channel::error::ChannelError;
use rekindle_codec::community::channel_record::{
    ChannelForward, ChannelHandRaise, ChannelMessage, ChannelPollClose, ChannelPollCreate,
    ChannelPollVote, ChannelReaction,
};
use rekindle_protocol::dht::community::channel_record::AppendOutcome;
use rekindle_protocol::dht::community::channel_record::{
    create_smpl_channel_record, read_all_channel_entries, read_all_channel_messages,
    write_member_forward, write_member_hand_raise, write_member_message, write_member_poll_close,
    write_member_poll_create, write_member_poll_vote, write_member_reaction,
};
use rekindle_protocol::ProtocolError;

use crate::state_helpers;

use super::ChannelAdapter;

pub(super) async fn write_channel_message_smpl_impl(
    adapter: &ChannelAdapter,
    context: &ChannelWriteContext,
    channel_msg: &ChannelMessage,
) -> Result<DhtWrite, ChannelError> {
    let pool = state_helpers::record_pool(&adapter.state).map_err(ChannelError::Adapter)?;
    let writer = context
        .slot_keypair_str
        .parse::<veilid_core::KeyPair>()
        .map_err(|e| ChannelError::Adapter(format!("invalid slot keypair: {e}")))?;
    let (author_pseudo, signing_key) =
        state_helpers::pseudonym_credentials(&adapter.state, &context.community_id)
            .map_err(ChannelError::Adapter)?;
    let written = write_member_message(
        &pool,
        &context.channel_key,
        context.slot_index,
        writer,
        author_pseudo,
        &signing_key,
        channel_msg,
    )
    .await;
    delivery(written, "SMPL channel write failed")
}

pub(super) async fn write_channel_forward_smpl_impl(
    adapter: &ChannelAdapter,
    context: &ChannelWriteContext,
    forward: &ChannelForward,
) -> Result<DhtWrite, ChannelError> {
    let pool = state_helpers::record_pool(&adapter.state).map_err(ChannelError::Adapter)?;
    let writer = context
        .slot_keypair_str
        .parse::<veilid_core::KeyPair>()
        .map_err(|e| ChannelError::Adapter(format!("invalid slot keypair: {e}")))?;
    let (author_pseudo, signing_key) =
        state_helpers::pseudonym_credentials(&adapter.state, &context.community_id)
            .map_err(ChannelError::Adapter)?;
    let written = write_member_forward(
        &pool,
        &context.channel_key,
        context.slot_index,
        writer,
        author_pseudo,
        &signing_key,
        forward,
    )
    .await;
    delivery(written, "SMPL channel forward write failed")
}

pub(super) async fn write_channel_poll_create_smpl_impl(
    adapter: &ChannelAdapter,
    context: &ChannelWriteContext,
    entry: &ChannelPollCreate,
) -> Result<(), ChannelError> {
    let pool = state_helpers::record_pool(&adapter.state).map_err(ChannelError::Adapter)?;
    let writer = context
        .slot_keypair_str
        .parse::<veilid_core::KeyPair>()
        .map_err(|e| ChannelError::Adapter(format!("invalid slot keypair: {e}")))?;
    let (author_pseudo, signing_key) =
        state_helpers::pseudonym_credentials(&adapter.state, &context.community_id)
            .map_err(ChannelError::Adapter)?;
    let written = write_member_poll_create(
        &pool,
        &context.channel_key,
        context.slot_index,
        writer,
        author_pseudo,
        &signing_key,
        entry,
    )
    .await;
    delivery(written, "SMPL poll create write failed").map(|_| ())
}

pub(super) async fn write_channel_poll_vote_smpl_impl(
    adapter: &ChannelAdapter,
    context: &ChannelWriteContext,
    entry: &ChannelPollVote,
) -> Result<(), ChannelError> {
    let pool = state_helpers::record_pool(&adapter.state).map_err(ChannelError::Adapter)?;
    let writer = context
        .slot_keypair_str
        .parse::<veilid_core::KeyPair>()
        .map_err(|e| ChannelError::Adapter(format!("invalid slot keypair: {e}")))?;
    let (author_pseudo, signing_key) =
        state_helpers::pseudonym_credentials(&adapter.state, &context.community_id)
            .map_err(ChannelError::Adapter)?;
    let written = write_member_poll_vote(
        &pool,
        &context.channel_key,
        context.slot_index,
        writer,
        author_pseudo,
        &signing_key,
        entry,
    )
    .await;
    delivery(written, "SMPL poll vote write failed").map(|_| ())
}

pub(super) async fn write_channel_hand_raise_smpl_impl(
    adapter: &ChannelAdapter,
    context: &ChannelWriteContext,
    entry: &ChannelHandRaise,
) -> Result<(), ChannelError> {
    let pool = state_helpers::record_pool(&adapter.state).map_err(ChannelError::Adapter)?;
    let writer = context
        .slot_keypair_str
        .parse::<veilid_core::KeyPair>()
        .map_err(|e| ChannelError::Adapter(format!("invalid slot keypair: {e}")))?;
    let (author_pseudo, signing_key) =
        state_helpers::pseudonym_credentials(&adapter.state, &context.community_id)
            .map_err(ChannelError::Adapter)?;
    let written = write_member_hand_raise(
        &pool,
        &context.channel_key,
        context.slot_index,
        writer,
        author_pseudo,
        &signing_key,
        entry,
    )
    .await;
    delivery(written, "SMPL hand raise write failed").map(|_| ())
}

pub(super) async fn write_channel_poll_close_smpl_impl(
    adapter: &ChannelAdapter,
    context: &ChannelWriteContext,
    entry: &ChannelPollClose,
) -> Result<(), ChannelError> {
    let pool = state_helpers::record_pool(&adapter.state).map_err(ChannelError::Adapter)?;
    let writer = context
        .slot_keypair_str
        .parse::<veilid_core::KeyPair>()
        .map_err(|e| ChannelError::Adapter(format!("invalid slot keypair: {e}")))?;
    let (author_pseudo, signing_key) =
        state_helpers::pseudonym_credentials(&adapter.state, &context.community_id)
            .map_err(ChannelError::Adapter)?;
    let written = write_member_poll_close(
        &pool,
        &context.channel_key,
        context.slot_index,
        writer,
        author_pseudo,
        &signing_key,
        entry,
    )
    .await;
    delivery(written, "SMPL poll close write failed").map(|_| ())
}

pub(super) async fn write_member_reaction_smpl_impl(
    adapter: &ChannelAdapter,
    context: &ChannelWriteContext,
    reaction: &ChannelReaction,
) -> Result<(), ChannelError> {
    let pool = state_helpers::record_pool(&adapter.state).map_err(ChannelError::Adapter)?;
    let writer = context
        .slot_keypair_str
        .parse::<veilid_core::KeyPair>()
        .map_err(|e| ChannelError::Adapter(format!("invalid slot keypair: {e}")))?;
    let (author_pseudo, signing_key) =
        state_helpers::pseudonym_credentials(&adapter.state, &context.community_id)
            .map_err(ChannelError::Adapter)?;
    let written = write_member_reaction(
        &pool,
        &context.channel_key,
        context.slot_index,
        writer,
        author_pseudo,
        &signing_key,
        reaction,
    )
    .await;
    delivery(written, "SMPL reaction write failed").map(|_| ())
}

pub(super) async fn create_smpl_thread_record_impl(
    adapter: &ChannelAdapter,
    slot_seed_bytes: &[u8; 32],
) -> Result<(rekindle_records::lease::LeaseId, String), ChannelError> {
    let pool = state_helpers::record_pool(&adapter.state).map_err(ChannelError::Adapter)?;
    let (lease, record_key, _) = create_smpl_channel_record(&pool, slot_seed_bytes)
        .await
        .map_err(|e| ChannelError::Adapter(format!("create lazy thread record failed: {e}")))?;
    Ok((lease, record_key))
}

pub(super) async fn read_all_channel_entries_impl(
    adapter: &ChannelAdapter,
    community_id: &str,
    record_key: &str,
) -> Result<Vec<ChannelEntryItem>, ChannelError> {
    let pool = state_helpers::record_pool(&adapter.state).map_err(ChannelError::Adapter)?;
    let slots = writer_slot_list(adapter, community_id).await?;
    let items = read_all_channel_entries(&pool, record_key, &slots)
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
    community_id: &str,
    record_key: &str,
) -> Result<Vec<ChannelMessage>, ChannelError> {
    let pool = state_helpers::record_pool(&adapter.state).map_err(ChannelError::Adapter)?;
    let slots = writer_slot_list(adapter, community_id).await?;
    read_all_channel_messages(&pool, record_key, &slots)
        .await
        .map_err(|e| ChannelError::Adapter(format!("read channel messages failed: {e}")))
}

/// The community's writer index, the slots a channel read covers (plan
/// C7.12).
async fn writer_slot_list(
    adapter: &ChannelAdapter,
    community_id: &str,
) -> Result<Vec<u32>, ChannelError> {
    crate::services::community::writers::writer_slot_list(
        &adapter.state,
        &adapter.pool,
        community_id,
    )
    .await
    .map_err(ChannelError::Adapter)
}

/// An append's outcome as the channel crate sees it (plan C7.13). A miss is
/// held by the record pool, and so is a write logout cut short (the pool
/// persists it), so neither is a failure.
fn delivery(
    written: Result<AppendOutcome, ProtocolError>,
    what: &str,
) -> Result<DhtWrite, ChannelError> {
    match written {
        Ok(AppendOutcome::Stored) => Ok(DhtWrite::Stored),
        Ok(AppendOutcome::Held) | Err(ProtocolError::PoolClosed) => Ok(DhtWrite::Held),
        Err(e) => Err(ChannelError::Adapter(format!("{what}: {e}"))),
    }
}
