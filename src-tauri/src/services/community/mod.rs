pub mod analytics;
pub mod automod;
pub mod background_sync;
pub mod bootstrap;
pub mod channel_messages;
pub mod channel_polls;
pub mod channel_reactions;
pub mod create;
pub mod event_reminders;
pub mod events_hydration;
pub mod expression_assets;
pub mod expressions;
pub mod files;
pub mod gossip;
pub mod governance;
pub mod inspect;
pub mod join;
pub mod keepalive;
pub mod link_previews;
pub mod mek_rotation;
pub mod mek_rotation_orchestrators;
pub(crate) mod mek_rotation_support;
pub mod mentions;
pub mod message_notifications;
pub mod message_notifications_handle;
pub mod notifications;
pub mod presence;
pub mod receiver_limits;
pub mod segments;
pub mod stage;
pub mod threads;
pub mod threads_store;
pub mod video;
pub mod watch;

// Re-export public API (callers use services::community::function_name)
pub use bootstrap::build_bootstrap_response;
pub use channel_messages::{
    emit_local_chat_event, send_message, start_write_retry_worker, PendingChannelMessage,
};
pub use channel_polls::{persist_poll_close, persist_poll_create, persist_poll_vote};
pub use channel_reactions::persist_reaction;
pub use create::create_community;
pub use event_reminders::{start_event_reminders, wake_event_reminders};
pub use expressions::{
    delete_expression, list_expressions, play_soundboard, upload_emoji, upload_soundboard_sound,
    upload_sticker,
};
pub use gossip::{
    flush_peer_reliability, hydrate_peer_reliability, record_peer_reliability, send_to_mesh,
    start_peer_reliability_flush,
};
pub use governance::write_entry;
pub(crate) use join::try_derive_slot_keypair;
pub use join::{join_community, rejoin_community};
pub use keepalive::start_dht_keepalive;
pub use mek_rotation::{handle_incoming_mek_transfer, spawn_mek_request_with_retry};
pub use mek_rotation_orchestrators::{
    handle_request_mek, rotate_text_mek_for_departure, rotate_voice_mek_for_membership,
};
pub use message_notifications_handle::handle_message_notification;
pub use notifications::{emit_message_notification, should_emit_message_notification};
pub use presence::{
    current_presence_status, presence_poll_tick_public, run_initial_sync, start_presence_poll,
    write_our_presence,
};
pub use stage::{list_hand_raises, persist_hand_raise};
pub use watch::{mark_watch_inactive, watch_community_records};
