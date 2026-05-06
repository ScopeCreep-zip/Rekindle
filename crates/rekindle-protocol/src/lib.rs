pub mod capnp_codec;
pub mod capnp_envelope;
pub mod dht;
pub mod error;
pub mod messaging;
pub mod node;
pub mod peer;
pub mod routing;

pub use dht::log::DHTLog;
pub use dht::short_array::DHTShortArray;
pub use error::ProtocolError;
pub use node::RekindleNode;

// Cap'n Proto generated modules — must be at crate root so generated
// `crate::<schema>_capnp` paths resolve correctly.
#[allow(clippy::all, clippy::pedantic, unused)]
pub mod message_capnp {
    include!(concat!(env!("OUT_DIR"), "/message_capnp.rs"));
}

#[allow(clippy::all, clippy::pedantic, unused)]
pub mod presence_capnp {
    include!(concat!(env!("OUT_DIR"), "/presence_capnp.rs"));
}

#[allow(clippy::all, clippy::pedantic, unused)]
pub mod identity_capnp {
    include!(concat!(env!("OUT_DIR"), "/identity_capnp.rs"));
}

#[allow(clippy::all, clippy::pedantic, unused)]
pub mod friend_capnp {
    include!(concat!(env!("OUT_DIR"), "/friend_capnp.rs"));
}

#[allow(clippy::all, clippy::pedantic, unused)]
pub mod voice_capnp {
    include!(concat!(env!("OUT_DIR"), "/voice_capnp.rs"));
}

#[allow(clippy::all, clippy::pedantic, unused)]
pub mod account_capnp {
    include!(concat!(env!("OUT_DIR"), "/account_capnp.rs"));
}

#[allow(clippy::all, clippy::pedantic, unused)]
pub mod conversation_capnp {
    include!(concat!(env!("OUT_DIR"), "/conversation_capnp.rs"));
}

// Phase 2 of `.claude/plans/community-envelope-capnp-migration.md` —
// typed community-envelope schemas. These replace the JSON wire form
// in Phases 4-5; Phase 2 only emits the generated Rust bindings.

#[allow(clippy::all, clippy::pedantic, unused)]
pub mod community_member_capnp {
    include!(concat!(env!("OUT_DIR"), "/community_member_capnp.rs"));
}

#[allow(clippy::all, clippy::pedantic, unused)]
pub mod community_thread_capnp {
    include!(concat!(env!("OUT_DIR"), "/community_thread_capnp.rs"));
}

#[allow(clippy::all, clippy::pedantic, unused)]
pub mod community_game_server_capnp {
    include!(concat!(env!("OUT_DIR"), "/community_game_server_capnp.rs"));
}

#[allow(clippy::all, clippy::pedantic, unused)]
pub mod community_mek_capnp {
    include!(concat!(env!("OUT_DIR"), "/community_mek_capnp.rs"));
}

#[allow(clippy::all, clippy::pedantic, unused)]
pub mod community_message_capnp {
    include!(concat!(env!("OUT_DIR"), "/community_message_capnp.rs"));
}

#[allow(clippy::all, clippy::pedantic, unused)]
pub mod community_event_capnp {
    include!(concat!(env!("OUT_DIR"), "/community_event_capnp.rs"));
}

#[allow(clippy::all, clippy::pedantic, unused)]
pub mod community_governance_capnp {
    include!(concat!(env!("OUT_DIR"), "/community_governance_capnp.rs"));
}

#[allow(clippy::all, clippy::pedantic, unused)]
pub mod community_envelope_capnp {
    include!(concat!(env!("OUT_DIR"), "/community_envelope_capnp.rs"));
}
