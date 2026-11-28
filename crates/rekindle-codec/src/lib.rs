//! Rekindle's wire types and their encodings, with no Veilid (plan C8,
//! ADR 0014): the Cap'n Proto schemas and codecs, the community and 1:1
//! envelopes, channel record pages, the gossip signed envelope and its
//! dedup. `rekindle-protocol` moves these bytes over Veilid; every tier
//! crate that only needs the types depends on this crate instead, so it
//! never links veilid-core.
//!
//! Tier 3 — depends on `rekindle-types`, `rekindle-secrets` and
//! `rekindle-utils`.
//!
//! Every byte this crate decodes may come from a remote peer, so a panic here
//! is a remote crash (gate hardening 3(a); ANSSI LANG-LIMIT-PANIC,
//! LANG-ARRINDEXING, LANG-ARITH): non-test code may not index, slice, do
//! unchecked arithmetic, unwrap, expect or panic. The lints sit on every
//! hand-written module through [`peer_input_module!`] rather than on the
//! crate, since a crate attribute would also reach the generated Cap'n Proto
//! modules below, which are not ours to change. `cargo xtask
//! check-panic-lints` keeps every root module declared through it.

/// Declare a hand-written module with the panic lints at `deny` for its
/// non-test code.
macro_rules! peer_input_module {
    ($module:ident) => {
        #[cfg_attr(
            not(test),
            deny(
                clippy::indexing_slicing,
                clippy::arithmetic_side_effects,
                clippy::unwrap_used,
                clippy::expect_used,
                clippy::panic,
                clippy::string_slice
            )
        )]
        pub mod $module;
    };
}

peer_input_module!(capnp_codec);
peer_input_module!(capnp_envelope);
peer_input_module!(checkpoint_hint);
peer_input_module!(checkpoint_state);
peer_input_module!(community);
peer_input_module!(dedup);
peer_input_module!(envelope);
peer_input_module!(error);
peer_input_module!(friend);
peer_input_module!(frontier_request);
peer_input_module!(governance_op);
peer_input_module!(ingress);
peer_input_module!(join_locator);
peer_input_module!(member_record);
peer_input_module!(message);
peer_input_module!(presence_row);
peer_input_module!(successor_invites);

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

capnp_module!(direct_capnp);
capnp_module!(presence_capnp);
capnp_module!(identity_capnp);
capnp_module!(chain_capnp);
capnp_module!(friend_capnp);
capnp_module!(voice_capnp);
capnp_module!(voice_packet_capnp);
capnp_module!(media_feedback_capnp);
capnp_module!(account_capnp);

// Typed community-envelope schemas.
capnp_module!(community_game_server_capnp);
capnp_module!(community_event_capnp);
capnp_module!(community_governance_capnp);
capnp_module!(community_envelope_capnp);
capnp_module!(member_record_capnp);
capnp_module!(governance_op_capnp);
