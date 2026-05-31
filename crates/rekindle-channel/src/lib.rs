//! Phase 19 — community channel messaging.
//!
//! Layered on top of `rekindle-mek-rotation` (per-channel MEK lookup)
//! + `rekindle-gossip` (mesh broadcast primitives) + `rekindle-protocol`
//! (signed envelope encoding).
//!
//! Modules land in subsequent Phase 19 tasks:
//! - `send` (task #161) — encrypt + sign + gossip outbound channel messages
//! - `receive` (task #162) — decrypt + reader-validate + persist inbound
//! - `threads` (task #163) — thread reply chains
//! - `reactions` (task #164) — emoji reactions
//! - `expressions` (task #165) — custom emoji / soundboard / stickers
//! - `mentions` (task #166) — @mention parsing + notification routing

#![forbid(unsafe_code)]

pub mod automod;
pub mod deps;
pub mod error;
pub mod event;
pub mod expressions;
pub mod mentions;
pub mod notifications;
pub mod pipeline;
pub mod polls;
pub mod reactions;
pub mod receive;
pub mod send;
pub mod stage;
pub mod threads;

pub use automod::{
    evaluate_message as evaluate_automod, get_rule as get_automod_rule, list_rules as list_automod_rules,
    AutoModAction, AutoModCompiledCache, AutoModRuleInfo, CompiledAutoModRule,
};
pub use deps::{ChannelInfoSnapshot, ChannelMek, ChannelMessagingDeps, ChannelSendOutcome};
pub use error::ChannelError;
pub use event::ChannelEvent;
pub use receive::{
    decrypt_channel_body, decrypt_channel_body_with_legacy_fallback, extract_mention_signals,
    MentionSignals,
};
pub use expressions::{
    delete_expression, detect_audio_kind, detect_image_media_type, list_expressions,
    normalize_tags, play_soundboard, upload_emoji, upload_soundboard_sound, upload_sticker,
    validate_emoji_bytes, validate_expression_name, validate_sticker_bytes,
    validate_soundboard_bytes, MAX_ANIMATED_EMOJI_BYTES, MAX_ANIMATED_EMOJI_COUNT,
    MAX_SOUNDBOARD_BYTES, MAX_SOUNDBOARD_COUNT, MAX_STATIC_EMOJI_BYTES, MAX_STATIC_EMOJI_COUNT,
    MAX_STICKER_BYTES, MAX_STICKER_COUNT,
};
pub use pipeline::{
    enforce_slowmode_with_bypass, forward_channel_message, process_retry_write,
    send_channel_message, ChannelSendResult,
};
pub use mentions::{
    has_perm, local_member_is_mentioned, matches_from_cleartext, parse_mentions,
    resolve_outbound_mentions, resolve_to_wire, scan_raw_tokens, validate_sender_permissions,
    MentionMatches,
};
pub use polls::{
    get_poll_results, persist_poll_close, persist_poll_create, persist_poll_vote, PollSnapshot,
};
pub use reactions::{build_reaction, build_reaction_envelope, persist_reaction};
pub use send::{build_channel_message, encrypt_channel_body, slowmode_check};
pub use notifications::{
    blake3_hex as notification_content_hash, parse_notification_level,
    verify_message_content_hash, NotificationDecision, NotificationLevel, NotificationThrottle,
};
pub use stage::{list_hand_raises, persist_hand_raise};
pub use threads::{
    archive_thread, create_thread, default_auto_archive_seconds, is_thread_archived,
    list_active_threads, list_threads, load_thread_messages, send_thread_message,
    thread_member_count, validate_auto_archive_seconds, ThreadMessageView,
};
