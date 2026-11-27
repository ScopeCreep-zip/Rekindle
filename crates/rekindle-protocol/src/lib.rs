#![cfg_attr(
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

pub mod dht;
pub mod error;
pub mod node;
pub mod own_routes;
pub mod ports;
pub mod veilid_config;

pub use error::ProtocolError;
pub use node::RekindleNode;
