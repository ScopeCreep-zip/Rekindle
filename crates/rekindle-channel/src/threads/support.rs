//! Shared private helpers for the thread pipeline.

use rekindle_protocol::dht::community::channel_record::ChannelMessage;
use rekindle_types::governance::GovernanceEntry;
use rekindle_types::id::{ChannelId, ThreadId};

use super::policy::thread_member_count;
use crate::deps::{ChannelMessagingDeps, ThreadStateSnapshot};
use crate::error::ChannelError;
use crate::send::{build_channel_message, encrypt_channel_body, BuildChannelMessageParams};

pub(super) fn hex_to_id_16(hex_str: &str) -> [u8; 16] {
    hex::decode(hex_str)
        .ok()
        .and_then(|bytes| bytes.try_into().ok())
        .unwrap_or([0u8; 16])
}

/// Look up parent channel type so create_thread can pick the right
/// auto-archive default. Returns "forum_post" when parent is a forum
/// channel, "public" otherwise.
pub(super) fn parent_thread_type<D: ChannelMessagingDeps>(
    deps: &D,
    community_id: &str,
    channel_id: &str,
) -> Result<String, ChannelError> {
    let gov_state = deps
        .governance_state(community_id)
        .ok_or_else(|| ChannelError::Adapter("governance state not loaded".into()))?;
    let channel = gov_state
        .channels
        .get(&ChannelId(hex_to_id_16(channel_id)))
        .ok_or_else(|| ChannelError::ChannelNotFound(channel_id.into()))?;
    Ok(if channel.channel_type == "forum" {
        "forum_post".to_string()
    } else {
        "public".to_string()
    })
}

pub(super) async fn ensure_thread_record_and_message<D: ChannelMessagingDeps>(
    deps: &D,
    community_id: &str,
    thread_id: &str,
    body: &str,
) -> Result<(String, ChannelMessage), ChannelError> {
    let thread = deps
        .thread_state(community_id, thread_id)
        .ok_or_else(|| ChannelError::Adapter("thread not found".into()))?;
    let record_key = match thread.record_key.clone() {
        Some(key) => key,
        None => create_lazy_thread_record(deps, community_id, thread_id, &thread).await?,
    };
    // Adapter should always provide channel_write_context for the
    // thread's parent channel; if not, fall back to slot 0 so a
    // missing parent doesn't block reply.
    let slot_index = deps
        .channel_write_context(community_id, &thread.parent_channel_id_hex)
        .map_or(0, |c| c.slot_index);
    let message = build_thread_message(deps, community_id, &record_key, slot_index, body)?;
    Ok((record_key, message))
}

pub(super) async fn create_lazy_thread_record<D: ChannelMessagingDeps>(
    deps: &D,
    community_id: &str,
    thread_id: &str,
    thread: &ThreadStateSnapshot,
) -> Result<String, ChannelError> {
    let slot_seed = deps
        .slot_seed_bytes(community_id)
        .ok_or_else(|| ChannelError::Adapter("no slot seed available for community".into()))?;
    let record_key = deps.create_smpl_thread_record(&slot_seed).await?;

    deps.track_open_records(community_id, std::slice::from_ref(&record_key));
    let _ = deps.watch_community_records(community_id).await;

    let lamport = deps.increment_lamport(community_id);
    deps.write_governance_entry(
        community_id,
        GovernanceEntry::ThreadCreated {
            thread_id: ThreadId(hex_to_id_16(thread_id)),
            parent_channel_id: ChannelId(hex_to_id_16(&thread.parent_channel_id_hex)),
            name: thread.name.clone(),
            thread_type: thread.thread_type.clone(),
            record_key: Some(record_key.clone()),
            invited: thread.invited.clone(),
            forum_tag: thread.forum_tag.clone(),
            auto_archive_seconds: thread.auto_archive_seconds,
            lamport,
        },
    )
    .await?;
    Ok(record_key)
}

pub(super) fn build_thread_message<D: ChannelMessagingDeps>(
    deps: &D,
    community_id: &str,
    record_key: &str,
    slot_index: u32,
    body: &str,
) -> Result<ChannelMessage, ChannelError> {
    let lamport_ts = deps.increment_lamport(community_id);
    let mek = deps
        .community_mek(community_id)
        .ok_or_else(|| ChannelError::MekMissing {
            community: community_id.into(),
            channel: "__thread__".into(),
        })?;
    let ciphertext =
        encrypt_channel_body(&mek, record_key, slot_index, lamport_ts, body.as_bytes())?;

    let sender_hex = deps
        .my_pseudonym_hex(community_id)
        .ok_or_else(|| ChannelError::PseudonymKeyMissing(community_id.into()))?;
    let (mentioned_pseudonyms, mentioned_roles, mention_flags) =
        crate::mentions::resolve_outbound_mentions(deps, community_id, &sender_hex, body);
    let mek_generation = deps
        .current_mek_generation(community_id)
        .ok_or_else(|| ChannelError::Adapter("community not found".into()))?;
    let timestamp_ms = rekindle_utils::timestamp_secs() * 1000;

    Ok(build_channel_message(BuildChannelMessageParams {
        sequence: deps.next_thread_sequence(community_id),
        sender_pseudonym: sender_hex,
        ciphertext,
        mek_generation,
        timestamp_ms: i64::try_from(timestamp_ms).unwrap_or(i64::MAX),
        lamport_ts,
        message_id: format!("tmsg_{}", uuid_simple()),
        mention_flag_bits: mention_flags,
        mentioned_pseudonyms,
        mentioned_roles,
    }))
}

pub(super) fn decrypt_thread_body<D: ChannelMessagingDeps>(
    deps: &D,
    community_id: &str,
    record_key: &str,
    subkey_index: u32,
    lamport_ts: u64,
    ciphertext: &[u8],
    mek_generation: u64,
) -> String {
    let Some(mek) = deps.community_mek(community_id) else {
        return String::new();
    };
    if mek.generation != mek_generation {
        return String::new();
    }
    let Ok(plaintext) = crate::receive::decrypt_channel_body_with_legacy_fallback(
        &mek,
        Some(record_key),
        subkey_index,
        lamport_ts,
        ciphertext,
    ) else {
        return String::new();
    };
    String::from_utf8(plaintext).unwrap_or_default()
}

pub(super) async fn thread_activity<D: ChannelMessagingDeps>(
    deps: &D,
    record_key: Option<&str>,
) -> Result<(u64, u64, u32), ChannelError> {
    let Some(record_key) = record_key else {
        return Ok((0, 0, 0));
    };
    let messages = deps
        .read_all_channel_messages(record_key, thread_member_count())
        .await?;
    let last_lamport = messages.iter().map(|m| m.lamport_ts).max().unwrap_or(0);
    let last_activity = messages
        .iter()
        .map(|m| m.timestamp / 1000)
        .max()
        .unwrap_or(0);
    Ok((
        last_lamport,
        last_activity,
        u32::try_from(messages.len()).unwrap_or(u32::MAX),
    ))
}

/// Build a write context that overrides `channel_key` to point at a
/// thread's lazy SMPL record. Other slot fields are inherited from the
/// community membership.
pub(super) fn thread_write_context<D: ChannelMessagingDeps>(
    deps: &D,
    community_id: &str,
    thread_record_key: &str,
) -> Result<crate::deps::ChannelWriteContext, ChannelError> {
    // We don't have a real channel_id to feed channel_write_context, so
    // we build the context from raw membership signals. The adapter
    // sources slot_keypair_str + slot_index from CommunityState.
    let creds_ctx = deps
        .pseudonym_credentials(community_id)
        .map(|_| ())
        .ok()
        .and(
            deps.member_profile(community_id, "")
                .role_ids
                .first()
                .copied(),
        );
    let _ = creds_ctx; // not actually needed — we only use the channel_key override

    // The adapter's channel_write_context impl returns slot fields keyed
    // by the supplied channel_id; here we want those slot fields for a
    // thread, not a channel. The simplest path: call a tiny helper on
    // the adapter side that produces the slot fields without a
    // channel_id. For now, derive from any channel that exists.
    //
    // Stable fallback: use the first channel in governance state, then
    // override the channel_key + channel_id fields. This is unergonomic
    // but works because slot_keypair + slot_index are community-wide,
    // not channel-specific.
    let any_channel_id = deps
        .governance_state(community_id)
        .and_then(|gov| gov.channels.keys().next().map(|c| hex::encode(c.0)))
        .ok_or_else(|| ChannelError::Adapter("no channel in governance state".into()))?;
    let base = deps.channel_write_context(community_id, &any_channel_id)?;
    Ok(crate::deps::ChannelWriteContext {
        community_id: base.community_id,
        channel_id: String::from("__thread__"),
        channel_key: thread_record_key.to_string(),
        slot_keypair_str: base.slot_keypair_str,
        slot_index: base.slot_index,
        segment_index: base.segment_index,
    })
}

pub(super) fn uuid_simple() -> String {
    // Lightweight v4-like 16-byte hex (rand-backed) for thread message
    // IDs. Matches `uuid::Uuid::new_v4().simple()` output length so
    // existing src-tauri callers + DB rows can swap in cleanly.
    let bytes: [u8; 16] = rand::random();
    hex::encode(bytes)
}
