//! Rekindle's wire types and their encodings, with no Veilid (plan C8,
//! ADR 0014): the Cap'n Proto schemas and codecs, the community and 1:1
//! envelopes, channel record pages, the gossip signed envelope and its
//! dedup. `rekindle-protocol` moves these bytes over Veilid; every tier
//! crate that only needs the types depends on this crate instead, so it
//! never links veilid-core.
//!
//! Tier 3 — depends on `rekindle-types`, `rekindle-secrets` and
//! `rekindle-utils`.

pub mod capnp_codec;
pub mod capnp_envelope;
pub mod community;
pub mod dedup;
pub mod envelope;
pub mod error;
pub mod friend;
pub mod message;
pub mod presence_row;

pub use error::CodecError;

// Cap'n Proto generated modules — must be at crate root so generated
// `crate::<schema>_capnp` paths resolve correctly. The generated code
// is not hand-written; per the capnpc-rust guidance the lint exemption
// belongs on the wrapping module. Expressed once in this macro instead
// of being copy-pasted onto all 16 modules: the `include!` path derives
// from the module name, so each invocation is a single line.
macro_rules! capnp_module {
    ($module:ident) => {
        #[allow(
            clippy::all,
            clippy::pedantic,
            unused,
            reason = "generated Cap'n Proto bindings"
        )]
        pub mod $module {
            include!(concat!(env!("OUT_DIR"), "/", stringify!($module), ".rs"));
        }
    };
}

capnp_module!(message_capnp);
capnp_module!(presence_capnp);
capnp_module!(identity_capnp);
capnp_module!(friend_capnp);
capnp_module!(voice_capnp);
capnp_module!(voice_packet_capnp);
capnp_module!(media_feedback_capnp);
capnp_module!(account_capnp);
capnp_module!(conversation_capnp);

// Typed community-envelope schemas.
capnp_module!(community_member_capnp);
capnp_module!(community_thread_capnp);
capnp_module!(community_game_server_capnp);
capnp_module!(community_mek_capnp);
capnp_module!(community_message_capnp);
capnp_module!(community_event_capnp);
capnp_module!(community_governance_capnp);
capnp_module!(community_envelope_capnp);
