#![deny(unsafe_code)]
//! Encrypted IPC transport for the Rekindle platform.
//!
//! v3 wire protocol over Noise-IK-encrypted Unix domain sockets with
//! epoch-rotated AES-256-GCM envelope/header authentication, io_uring
//! read/write tasks, and parallel rayon-dispatched bulk data plane.

pub mod v3;
pub mod calibrate;

pub mod fixture;
