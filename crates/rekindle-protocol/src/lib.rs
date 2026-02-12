pub mod capnp_codec;
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
#[allow(unused, clippy::all, clippy::pedantic)]
pub mod message_capnp {
    include!(concat!(env!("OUT_DIR"), "/message_capnp.rs"));
}

#[allow(unused, clippy::all, clippy::pedantic)]
pub mod presence_capnp {
    include!(concat!(env!("OUT_DIR"), "/presence_capnp.rs"));
}

#[allow(unused, clippy::all, clippy::pedantic)]
pub mod identity_capnp {
    include!(concat!(env!("OUT_DIR"), "/identity_capnp.rs"));
}

#[allow(unused, clippy::all, clippy::pedantic)]
pub mod community_capnp {
    include!(concat!(env!("OUT_DIR"), "/community_capnp.rs"));
}

#[allow(unused, clippy::all, clippy::pedantic)]
pub mod friend_capnp {
    include!(concat!(env!("OUT_DIR"), "/friend_capnp.rs"));
}

#[allow(unused, clippy::all, clippy::pedantic)]
pub mod voice_capnp {
    include!(concat!(env!("OUT_DIR"), "/voice_capnp.rs"));
}

#[allow(unused, clippy::all, clippy::pedantic)]
pub mod account_capnp {
    include!(concat!(env!("OUT_DIR"), "/account_capnp.rs"));
}

#[allow(unused, clippy::all, clippy::pedantic)]
pub mod conversation_capnp {
    include!(concat!(env!("OUT_DIR"), "/conversation_capnp.rs"));
}
