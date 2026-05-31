//! Phase 19.e-REDO — full thread pipeline.
//!
//! Ported from src-tauri/services/community/threads.rs. Crate-side
//! functions are parameterised over `D: ChannelMessagingDeps`; the
//! src-tauri facade (after 19.h-REDO) is a thin delegate that
//! constructs the adapter and calls into this module.

use rekindle_protocol::dht::community::channel_record::{ChannelMessage, ChannelRecordEntry};
use rekindle_protocol::dht::community::envelope::{CommunityEnvelope, ControlPayload};
use rekindle_records::schema::MAX_MEMBERS_PER_SEGMENT;
use rekindle_types::governance::GovernanceEntry;
use rekindle_types::id::{ChannelId, ThreadId};

use crate::deps::{ChannelMessagingDeps, ThreadInfoSnapshot, ThreadStateSnapshot};
use crate::error::ChannelError;
use crate::send::{build_channel_message, encrypt_channel_body};

// ---------- Pure validators (kept from prior crate version) ----------

pub fn validate_auto_archive_seconds(secs: u64) -> Result<u64, ChannelError> {
    if matches!(secs, 3600 | 86_400 | 259_200 | 604_800) {
        Ok(secs)
    } else {
        Err(ChannelError::InvalidId(format!(
            "auto_archive_seconds must be 3600, 86400, 259200, or 604800 — got {secs}"
        )))
    }
}

#[must_use]
pub fn default_auto_archive_seconds(thread_type: &str) -> u64 {
    match thread_type {
        "forum_post" => 604_800,
        "announcement" => 259_200,
        _ => 86_400,
    }
}

/// Architecture §14 — a thread is archived when it was manually
/// archived at a lamport >= last activity, OR when it has been idle
/// past its auto-archive window.
#[must_use]
pub fn is_thread_archived(
    archived_lamport: Option<u64>,
    last_lamport: u64,
    last_activity_secs: u64,
    auto_archive_seconds: u64,
    now_secs: u64,
) -> bool {
    let manually_archived = archived_lamport.is_some_and(|archived| last_lamport <= archived);
    let auto_archived = last_activity_secs > 0
        && last_activity_secs.saturating_add(auto_archive_seconds) < now_secs;
    manually_archived || auto_archived
}

#[must_use]
pub fn thread_member_count() -> u32 {
    u32::try_from(MAX_MEMBERS_PER_SEGMENT).unwrap_or(u32::MAX)
}

/// One thread message decrypted and ready for adapter-side display
/// assembly. Crate-side counterpart of src-tauri `Message` — adapter
/// wraps these into the full Message DTO.
#[derive(Debug, Clone)]
pub struct ThreadMessageView {
    pub sender_pseudonym: String,
    pub body: String,
    pub timestamp_ms: u64,
    pub is_own: bool,
    pub server_message_id: Option<String>,
    pub mek_generation: u64,
    pub subkey_index: u32,
    pub lamport_ts: u64,
}

fn hex_to_id_16(hex_str: &str) -> [u8; 16] {
    hex::decode(hex_str)
        .ok()
        .and_then(|bytes| bytes.try_into().ok())
        .unwrap_or([0u8; 16])
}

/// Look up parent channel type so create_thread can pick the right
/// auto-archive default. Returns "forum_post" when parent is a forum
/// channel, "public" otherwise.
fn parent_thread_type<D: ChannelMessagingDeps>(
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

/// Phase 19.e — public create_thread entry. Mirrors src-tauri shape.
pub async fn create_thread<D: ChannelMessagingDeps>(
    deps: &D,
    community_id: &str,
    channel_id: &str,
    name: &str,
    starter_message_id: &str,
    forum_tag: Option<String>,
    auto_archive_override: Option<u64>,
) -> Result<String, ChannelError> {
    let thread_id_bytes: [u8; 16] = rand::random();
    let thread_id = hex::encode(thread_id_bytes);
    let thread_type = parent_thread_type(deps, community_id, channel_id)?;
    let auto_archive_seconds = match auto_archive_override {
        Some(secs) => validate_auto_archive_seconds(secs)?,
        None => default_auto_archive_seconds(&thread_type),
    };
    let lamport = deps.increment_lamport(community_id);
    let forum_tag_for_entry = forum_tag.clone();

    deps.write_governance_entry(
        community_id,
        GovernanceEntry::ThreadCreated {
            thread_id: ThreadId(thread_id_bytes),
            parent_channel_id: ChannelId(hex_to_id_16(channel_id)),
            name: name.to_string(),
            thread_type,
            record_key: None,
            invited: Vec::new(),
            forum_tag: forum_tag_for_entry,
            auto_archive_seconds,
            lamport,
        },
    )
    .await?;

    let creator_pseudonym = deps
        .my_pseudonym_hex(community_id)
        .ok_or_else(|| ChannelError::PseudonymKeyMissing(community_id.into()))?;

    deps.persist_thread_row(
        community_id,
        &ThreadInfoSnapshot {
            id: thread_id.clone(),
            channel_id: channel_id.to_string(),
            name: name.to_string(),
            starter_message_id: starter_message_id.to_string(),
            creator_pseudonym,
            forum_tag,
            created_at: rekindle_utils::timestamp_secs(),
            archived: false,
            auto_archive_seconds: u32::try_from(auto_archive_seconds).unwrap_or(u32::MAX),
            last_message_at: 0,
            message_count: 0,
        },
    )
    .await?;

    Ok(thread_id)
}

/// Phase 19.e — list threads (with archival status computed via
/// `is_thread_archived`).
pub async fn list_threads<D: ChannelMessagingDeps>(
    deps: &D,
    community_id: &str,
    channel_id: &str,
) -> Result<Vec<ThreadInfoSnapshot>, ChannelError> {
    let gov_state = deps
        .governance_state(community_id)
        .ok_or_else(|| ChannelError::Adapter("governance state not loaded".into()))?;
    let mut out = Vec::new();
    let now = rekindle_utils::timestamp_secs();

    for (thread_id, thread) in gov_state
        .threads
        .iter()
        .filter(|(_, thread)| hex::encode(thread.parent_channel_id.0) == channel_id)
    {
        let thread_id_hex = hex::encode(thread_id.0);
        let mut dto = deps
            .load_thread_metadata(community_id, &thread_id_hex)
            .await
            .unwrap_or_else(|| ThreadInfoSnapshot {
                id: thread_id_hex.clone(),
                channel_id: channel_id.to_string(),
                name: thread.name.clone(),
                starter_message_id: String::new(),
                creator_pseudonym: hex::encode(thread.creator.0),
                forum_tag: thread.forum_tag.clone(),
                created_at: thread.created_lamport,
                archived: false,
                auto_archive_seconds: u32::try_from(thread.auto_archive_seconds)
                    .unwrap_or(u32::MAX),
                last_message_at: 0,
                message_count: 0,
            });
        dto.name = thread.name.clone();
        dto.creator_pseudonym = hex::encode(thread.creator.0);
        dto.forum_tag = thread.forum_tag.clone();
        dto.auto_archive_seconds = u32::try_from(thread.auto_archive_seconds).unwrap_or(u32::MAX);

        let (last_lamport, last_activity, message_count) =
            thread_activity(deps, thread.record_key.as_deref()).await?;
        dto.last_message_at = last_activity;
        dto.message_count = message_count;
        dto.archived = is_thread_archived(
            thread.archived_lamport,
            last_lamport,
            last_activity,
            thread.auto_archive_seconds,
            now,
        );
        out.push(dto);
    }

    out.sort_by(|a, b| {
        b.last_message_at
            .cmp(&a.last_message_at)
            .then_with(|| a.name.cmp(&b.name))
    });
    Ok(out)
}

/// Phase 19.e — list only non-archived threads.
pub async fn list_active_threads<D: ChannelMessagingDeps>(
    deps: &D,
    community_id: &str,
    channel_id: &str,
) -> Result<Vec<ThreadInfoSnapshot>, ChannelError> {
    let threads = list_threads(deps, community_id, channel_id).await?;
    Ok(threads.into_iter().filter(|t| !t.archived).collect())
}

/// Phase 19.e — send a thread reply. Encrypts under the community MEK
/// with thread-specific AAD, writes to SMPL, and fan-outs the
/// `ThreadMessageReceived` envelope to the mesh.
pub async fn send_thread_message<D: ChannelMessagingDeps>(
    deps: &D,
    community_id: &str,
    thread_id: &str,
    body: &str,
) -> Result<(), ChannelError> {
    let (record_key, channel_message) =
        ensure_thread_record_and_message(deps, community_id, thread_id, body).await?;

    // Reuse channel_write_context's channel-key plumbing pattern via a
    // temporary context — the slot bits come from community state but
    // the channel_key is the thread's record_key.
    let parent_context = thread_write_context(deps, community_id, &record_key)?;
    deps.write_channel_message_smpl(&parent_context, &channel_message)
        .await?;

    let envelope = CommunityEnvelope::Control(ControlPayload::ThreadMessageReceived {
        thread_id: thread_id.to_string(),
        message_id: channel_message.message_id.clone().unwrap_or_default(),
        sender_pseudonym: channel_message.sender_pseudonym.clone(),
        ciphertext: channel_message.ciphertext.clone(),
        mek_generation: channel_message.mek_generation,
        timestamp: channel_message.timestamp / 1000,
        reply_to_id: None,
    });
    deps.send_to_mesh(community_id, &envelope)?;
    Ok(())
}

/// Phase 19.e — load thread messages from the thread's SMPL record,
/// decrypt with AAD waterfall, and return rendered `ThreadMessageView`s.
pub async fn load_thread_messages<D: ChannelMessagingDeps>(
    deps: &D,
    community_id: &str,
    thread_id: &str,
    limit: u32,
    before_timestamp_secs: Option<u64>,
) -> Result<Vec<ThreadMessageView>, ChannelError> {
    let thread = deps
        .thread_state(community_id, thread_id)
        .ok_or_else(|| ChannelError::Adapter("thread not found".into()))?;
    let Some(record_key) = thread.record_key.clone() else {
        return Ok(Vec::new());
    };
    let entries = deps
        .read_all_channel_entries(&record_key, thread_member_count())
        .await?;
    let mut items: Vec<(u32, ChannelMessage)> = entries
        .into_iter()
        .filter_map(|item| match item.entry {
            ChannelRecordEntry::Message(msg) => Some((item.subkey_index, msg)),
            _ => None,
        })
        .collect();
    items.sort_by(|a, b| {
        a.1.lamport_ts
            .cmp(&b.1.lamport_ts)
            .then_with(|| a.1.sender_pseudonym.cmp(&b.1.sender_pseudonym))
    });
    let before_ms = before_timestamp_secs.map_or(u64::MAX, |ts| ts.saturating_mul(1000));
    let my_pseudonym = deps.my_pseudonym_hex(community_id).unwrap_or_default();

    let mut messages: Vec<ThreadMessageView> = items
        .into_iter()
        .filter(|(_, message)| message.timestamp < before_ms)
        .rev()
        .take(limit.min(200) as usize)
        .map(|(subkey_index, message)| {
            let body = decrypt_thread_body(
                deps,
                community_id,
                &record_key,
                subkey_index,
                message.lamport_ts,
                &message.ciphertext,
                message.mek_generation,
            );
            ThreadMessageView {
                is_own: message.sender_pseudonym == my_pseudonym,
                sender_pseudonym: message.sender_pseudonym.clone(),
                body,
                timestamp_ms: message.timestamp,
                server_message_id: message.message_id.clone(),
                mek_generation: message.mek_generation,
                subkey_index,
                lamport_ts: message.lamport_ts,
            }
        })
        .collect();
    messages.reverse();
    Ok(messages)
}

/// Phase 19.e — archive a thread (writes a `ThreadArchived` governance entry).
pub async fn archive_thread<D: ChannelMessagingDeps>(
    deps: &D,
    community_id: &str,
    thread_id: &str,
) -> Result<(), ChannelError> {
    let lamport = deps.increment_lamport(community_id);
    deps.write_governance_entry(
        community_id,
        GovernanceEntry::ThreadArchived {
            thread_id: ThreadId(hex_to_id_16(thread_id)),
            lamport,
        },
    )
    .await
}

// ---------- private helpers ----------

async fn ensure_thread_record_and_message<D: ChannelMessagingDeps>(
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

async fn create_lazy_thread_record<D: ChannelMessagingDeps>(
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

fn build_thread_message<D: ChannelMessagingDeps>(
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

    Ok(build_channel_message(
        deps.next_thread_sequence(community_id),
        sender_hex,
        ciphertext,
        mek_generation,
        i64::try_from(timestamp_ms).unwrap_or(i64::MAX),
        lamport_ts,
        format!("tmsg_{}", uuid_simple()),
        mention_flags,
        mentioned_pseudonyms,
        mentioned_roles,
    ))
}

fn decrypt_thread_body<D: ChannelMessagingDeps>(
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

async fn thread_activity<D: ChannelMessagingDeps>(
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
fn thread_write_context<D: ChannelMessagingDeps>(
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

fn uuid_simple() -> String {
    // Lightweight v4-like 16-byte hex (rand-backed) for thread message
    // IDs. Matches `uuid::Uuid::new_v4().simple()` output length so
    // existing src-tauri callers + DB rows can swap in cleanly.
    let bytes: [u8; 16] = rand::random();
    hex::encode(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_archive_accepts_spec_values() {
        for secs in [3600, 86_400, 259_200, 604_800] {
            assert_eq!(validate_auto_archive_seconds(secs).unwrap(), secs);
        }
    }

    #[test]
    fn auto_archive_rejects_off_spec() {
        for secs in [0, 1, 60, 7200, 3_600_000, u64::MAX] {
            assert!(validate_auto_archive_seconds(secs).is_err(), "{secs}");
        }
    }

    #[test]
    fn default_auto_archive_forum_post_is_seven_days() {
        assert_eq!(default_auto_archive_seconds("forum_post"), 604_800);
    }

    #[test]
    fn default_auto_archive_announcement_is_three_days() {
        assert_eq!(default_auto_archive_seconds("announcement"), 259_200);
    }

    #[test]
    fn default_auto_archive_text_is_one_day() {
        assert_eq!(default_auto_archive_seconds("text"), 86_400);
        assert_eq!(default_auto_archive_seconds("voice"), 86_400);
        assert_eq!(default_auto_archive_seconds("public"), 86_400);
        assert_eq!(default_auto_archive_seconds("unknown"), 86_400);
    }

    #[test]
    fn is_thread_archived_manual_overrides_activity() {
        // archived_lamport=10, last_lamport=5 → manually archived
        assert!(is_thread_archived(Some(10), 5, 0, 86_400, 0));
    }

    #[test]
    fn is_thread_archived_activity_after_archive_revives() {
        // archived_lamport=5, last_lamport=10 → revived
        assert!(!is_thread_archived(Some(5), 10, 0, 86_400, 0));
    }

    #[test]
    fn is_thread_archived_auto_archives_after_window() {
        // last_activity=1000s, window=86400s, now=88400s → auto-archived
        assert!(is_thread_archived(None, 5, 1_000, 86_400, 88_500));
    }

    #[test]
    fn is_thread_archived_never_archived_when_no_activity_no_archive() {
        // last_activity=0 (no messages yet) + no archived_lamport → live
        assert!(!is_thread_archived(None, 0, 0, 86_400, 1_000_000));
    }

    #[test]
    fn member_count_is_segment_max() {
        assert_eq!(thread_member_count() as usize, MAX_MEMBERS_PER_SEGMENT);
    }
}
