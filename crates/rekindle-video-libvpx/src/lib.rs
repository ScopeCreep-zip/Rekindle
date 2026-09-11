//! `rekindle-video-libvpx` — the cross-platform libvpx (VP8/VP9)
//! realtime encoder AND decoder that implement
//! `rekindle_video::codec::{VideoEncoder, VideoDecoder}` (plan
//! `video-media-engine.md` §"Step 2", the "libvpx | new, all platforms |
//! VP8 + VP9, the RFC 7742 floor" row). The decoder is the receive-side
//! counterpart of the encoder — same crate, same feature gate.
//!
//! # Build safety — the C dependency is feature-gated, DEFAULT OFF
//!
//! `rekindle-video` is Tier-7 **pure logic** and must never gain a C
//! dependency. This crate is where the C dependency is allowed to live,
//! and it is quarantined behind the `libvpx` cargo feature:
//!
//! - **without** `libvpx` (the default): the crate compiles pure-Rust
//!   [`stub::LibvpxEncoder`] / [`stub::LibvpxDecoder`] whose constructors
//!   return [`VideoError::Unsupported`]. No `env-libvpx-sys`, no libvpx —
//!   `cargo build`/`cargo test` stay green on a machine without libvpx
//!   installed.
//! - **with** `libvpx`: the crate compiles the real
//!   [`imp::LibvpxEncoder`] and [`imp_decoder::LibvpxDecoder`] backed by
//!   libvpx directly (`env-libvpx-sys` → `vpx_sys` FFI). Requires a system
//!   libvpx, libclang (for the `generate` bindgen path), and
//!   `VPX_STATIC=1` to static-link:
//!
//!   ```text
//!   VPX_STATIC=1 cargo build -p rekindle-video-libvpx --features libvpx
//!   ```
//!
//! The public API — `LibvpxEncoder::new(codec)` /
//! `LibvpxDecoder::new(codec)` returning `Result<Self, VideoError>` plus
//! the `VideoEncoder` / `VideoDecoder` impls — is **identical** across
//! both paths, so downstream code compiles regardless of the feature and
//! degrades to a clean `Unsupported` error when libvpx was not built in.

pub use rekindle_types::video::Codec;
pub use rekindle_video::error::VideoError;

#[cfg(feature = "libvpx")]
mod imp;
#[cfg(feature = "libvpx")]
mod imp_decoder;
#[cfg(feature = "libvpx")]
pub use imp::LibvpxEncoder;
#[cfg(feature = "libvpx")]
pub use imp_decoder::LibvpxDecoder;

#[cfg(not(feature = "libvpx"))]
mod stub;
#[cfg(not(feature = "libvpx"))]
pub use stub::{LibvpxDecoder, LibvpxEncoder};
