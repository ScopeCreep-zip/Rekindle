//! `rekindle-video-libvpx` — the cross-platform libvpx (VP8/VP9)
//! realtime encoder that implements `rekindle_video::codec::VideoEncoder`
//! (plan `video-media-engine.md` §"Step 2", the "libvpx | new, all
//! platforms | VP8 + VP9, the RFC 7742 floor" row).
//!
//! # Build safety — the C dependency is feature-gated, DEFAULT OFF
//!
//! `rekindle-video` is Tier-7 **pure logic** and must never gain a C
//! dependency. This crate is where the C dependency is allowed to live,
//! and it is quarantined behind the `libvpx` cargo feature:
//!
//! - **without** `libvpx` (the default): the crate compiles a pure-Rust
//!   [`stub::LibvpxEncoder`] whose [`LibvpxEncoder::new`] returns
//!   [`VideoError::Unsupported`]. No `env-libvpx-sys`, no libvpx —
//!   `cargo build`/`cargo test` stay green on a machine without libvpx
//!   installed.
//! - **with** `libvpx`: the crate compiles the real [`imp::LibvpxEncoder`]
//!   backed by libvpx directly (`env-libvpx-sys` → `vpx_sys` FFI).
//!   Requires a system libvpx, libclang (for the `generate` bindgen
//!   path), and `VPX_STATIC=1` to static-link:
//!
//!   ```text
//!   VPX_STATIC=1 cargo build -p rekindle-video-libvpx --features libvpx
//!   ```
//!
//! The public API — `LibvpxEncoder::new(codec) -> Result<Self,
//! VideoError>` plus the `VideoEncoder` impl — is **identical** across
//! both paths, so downstream code compiles regardless of the feature and
//! degrades to a clean `Unsupported` error when libvpx was not built in.

pub use rekindle_types::video::Codec;
pub use rekindle_video::error::VideoError;

#[cfg(feature = "libvpx")]
mod imp;
#[cfg(feature = "libvpx")]
pub use imp::LibvpxEncoder;

#[cfg(not(feature = "libvpx"))]
mod stub;
#[cfg(not(feature = "libvpx"))]
pub use stub::LibvpxEncoder;
