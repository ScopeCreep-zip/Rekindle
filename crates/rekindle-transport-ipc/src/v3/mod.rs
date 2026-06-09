//! Rekindle IPC Transport — encrypted, authenticated, multiplexed AF_UNIX protocol.
//!
//! # Architecture
//!
//! Four layers, strict dependency direction (each layer imports only from layers below):
//!
//! ```text
//! handlers → dispatch → codec → wire
//! ```
//!
//! # Quick Start
//!
//! ## Server
//!
//! ```rust,no_run
//! use rekindle_transport_ipc::v3::server::{IpcServer, ConnectionHandle};
//! use rekindle_transport_ipc::v3::router::ReplyRouter;
//! use rekindle_transport_ipc::v3::session::handshake::HandshakeConfig;
//! use rekindle_transport_ipc::v3::context::SessionConfig;
//!
//! // IpcServer::bind(path, keypair, router_factory, config, hs_config).await?;
//! // server.run().await?; // accept loop — spawns task triple per connection
//! ```
//!
//! ## Client
//!
//! ```rust,no_run
//! use rekindle_transport_ipc::v3::client::IpcClient;
//! use std::time::Duration;
//!
//! // let client = IpcClient::connect(path, &server_pub, &keypair, config, hs_config).await?;
//! // client.send_request(b"hello", Duration::from_secs(5)).await?;
//! // client.send_bulk(0, &payload, Duration::from_secs(30)).await?;
//! // client.rotate_keys(Duration::from_secs(5)).await?;
//! // client.shutdown().await;
//! ```
//!
//! # Per-Connection Task Architecture
//!
//! Each connection spawns 3 tasks:
//! 1. **Read task** (std::thread, io_uring) — EMAC verify, replay filter, route to bulk/inline
//! 2. **Control loop** (tokio task) — 11-arm biased select!, heartbeat, audit chain, dispatch
//! 3. **Write task** (std::thread, io_uring) — priority-ordered lane drain, writev coalescing
//!
//! # Key Rotation
//!
//! Epoch-tagged protocol: 1-bit epoch toggle in envelope flags (bit 4). Both epoch=0
//! and epoch=1 decoder keys are active during the transition window. The initiator
//! swaps encoder immediately after COMMIT; the responder defers encoder swap until
//! the initiator's first epoch=1 frame arrives. Forward secrecy achieved via
//! 100ms retirement window + ZeroizeOnDrop on EpochKeys.

pub mod wire;
pub mod codec;
pub mod dispatch;
pub mod crypto;
pub mod session;
pub mod stream;
pub mod audit;
pub mod dedup;
pub mod handoff;
pub mod context;
pub mod handlers;
pub mod conditions;
pub mod router;
#[cfg(unix)]
#[allow(unsafe_code)]
pub mod socket;
pub mod bulk;
pub mod io;
pub mod server;
pub mod client;
