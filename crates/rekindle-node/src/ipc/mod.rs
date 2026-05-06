//! IPC bus subsystem: encrypted Unix domain socket communication.
//!
//! This module contains the complete IPC infrastructure for the rekindle-node
//! daemon. It has **zero knowledge** of Veilid, DHT, communities, or messaging
//! semantics. It is a generic encrypted IPC bus that routes typed messages
//! between agents.
//!
//! Adapted from `open-sesame/core-ipc` (~2,100 lines, 8 modules) and extended
//! for hyperscale agent orchestration (10K+ agents per node group).
//!
//! # Module Layout
//!
//! - `error` — Typed error taxonomy, no catch-all String variants [RC-1]
//! - `framing` — Postcard serialization + length-prefixed wire I/O [RC-2][RC-3]
//! - `transport` — UCred extraction, socket path resolution [RC-6][RC-18]
//! - `message` — `Message<T>` envelope, wire versioning, security levels [RC-16]
//! - `protocol` — `IpcRequest`/`IpcResponse`/`BusPayload` enums
//! - `noise_keys` — Key generation, persistence, tamper detection [RC-4][RC-16]
//! - `registry` — `ClearanceRegistry`: pubkey → identity mapping [RC-14]
//!
//! # Security Properties
//!
//! - `#![forbid(unsafe_code)]` on all modules
//! - No `.unwrap()` on any data received from IPC [RC-2]
//! - Frame length validated before allocation [RC-3]
//! - UCred same-UID enforcement on every connection [RC-6]
//! - Noise IK mutual authentication with UCred prologue binding [RC-4]
//! - Private keys zeroized on drop via `ZeroizingKeypair` [RC-16]
//! - Atomic key file writes (tmp + fsync + rename) [RC-4]
//! - Agent names validated against `[a-zA-Z0-9_-]+` [RC-5]

// [RC-2] Deny unwrap in this module tree for IPC safety.
// Note: this is enforced by workspace clippy config (clippy::unwrap_used = deny).

pub mod error;
pub mod framing;
pub mod transport;
pub mod message;
pub mod protocol;
pub mod noise_keys;
pub mod noise;
pub mod registry;
pub mod server;
pub mod client;

// Re-exports for convenience.
pub use error::{IpcError, Result};
pub use framing::{encode_frame, decode_frame, read_frame, write_frame, MAX_FRAME_SIZE};
pub use message::{Message, MessageContext, SecurityLevel, AgentType, Timestamp, WIRE_VERSION};
pub use protocol::{IpcRequest, IpcResponse, BusPayload, SubscriptionFilter};
pub use noise_keys::{ZeroizingKeypair, generate_keypair, NOISE_PARAMS};
pub use noise::NoiseTransport;
pub use registry::ClearanceRegistry;
pub use server::BusServer;
pub use client::BusClient;
pub use transport::{PeerCredentials, extract_ucred, socket_path, runtime_dir};

