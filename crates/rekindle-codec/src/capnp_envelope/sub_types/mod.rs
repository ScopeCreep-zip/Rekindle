//! Leaf type encoders / decoders shared across `control.rs` and
//! `governance.rs`. Each pair (`write_<T>` / `read_<T>`) maps a Rust
//! domain type to its Cap'n Proto schema.
//!
//! Helpers here live in one module so adding a new variant in either
//! `ControlPayload` or `GovernanceEntry` only requires *one* place to
//! update if a new sub-type is referenced.

mod events;
mod messages;

pub(super) use events::{
    category_id_from_capnp, channel_id_from_capnp, event_id_from_capnp, pseudonym_key_from_capnp,
    pseudonym_key_to_capnp, read_event_info, read_event_location_via_event_capnp,
    read_member_summary, read_onboarding_answer, read_recurrence_rule_via_event_capnp,
    read_voice_roster_entry, role_id_from_capnp, thread_id_from_capnp, uuid16_from_capnp,
    uuid16_to_capnp, write_event_info, write_event_location_via_event_capnp, write_member_summary,
    write_onboarding_answer, write_recurrence_rule_via_event_capnp, write_voice_roster_entry,
};
pub(super) use messages::{
    read_bootstrap_channel_messages, read_channel_mek_delivery, read_game_server_info,
    read_member_info, read_presence_game_info, read_synced_message, read_thread_info,
    write_bootstrap_channel_messages, write_channel_mek_delivery, write_game_server_info,
    write_member_info, write_presence_game_info, write_synced_message, write_thread_info,
};
