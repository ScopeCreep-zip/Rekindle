pub mod capnp_codec;
pub mod capnp_envelope;
pub mod dht;
pub mod error;
pub mod messaging;
pub mod node;
pub mod routing;
pub mod veilid_config;

pub use dht::log::DHTLog;
pub use dht::short_array::DHTShortArray;
pub use error::ProtocolError;
pub use node::RekindleNode;

// Cap'n Proto generated modules — must be at crate root so generated
// `crate::<schema>_capnp` paths resolve correctly. The generated code
// is not hand-written; per the capnpc-rust guidance the lint exemption
// belongs on the wrapping module. Expressed once in this macro instead
// of being copy-pasted onto all 15 modules: the `include!` path derives
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
capnp_module!(account_capnp);
capnp_module!(conversation_capnp);

// Phase 2 of `.claude/plans/community-envelope-capnp-migration.md` —
// typed community-envelope schemas. These replace the JSON wire form
// in Phases 4-5; Phase 2 only emits the generated Rust bindings.
capnp_module!(community_member_capnp);
capnp_module!(community_thread_capnp);
capnp_module!(community_game_server_capnp);
capnp_module!(community_mek_capnp);
capnp_module!(community_message_capnp);
capnp_module!(community_event_capnp);
capnp_module!(community_governance_capnp);
capnp_module!(community_envelope_capnp);
