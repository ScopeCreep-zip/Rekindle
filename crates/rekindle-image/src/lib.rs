//! The one limited image decoder (plan E7.11).
//!
//! Every image Rekindle takes from outside (an avatar, a community avatar
//! or banner, a skin's pictures) is decoded here and only here:
//!
//! * The caller declares the kind ([`ImageKind`]): the file picker's MIME
//!   type, a file's extension, a skin's manifest. The bytes' magic must
//!   match it ([`ImageError::KindMismatch`]), and only the declared kind's
//!   decoder runs. The format is never guessed from the bytes, so a crafted
//!   file cannot pick its own decoder (the FORCEDENTRY path).
//! * The header is read first and its size checked against
//!   [`DecodeLimits`] before any pixel buffer exists
//!   ([`ImageError::TooLarge`]); decoding then runs under `image::Limits`
//!   (`max_image_width`, `max_image_height`, `max_alloc`, which `image`
//!   hands on to the `png` crate as `png::Limits`), and an animated GIF's
//!   frames are counted and their memory summed.
//! * The output is re-encoded ([`encode_png`], [`encode_gif`],
//!   [`encode_webp`]): pixels only, no metadata carried over.
//!
//! Pure Rust (`png`, `zune-jpeg`, `gif`, `image-webp`), no `unsafe`, no
//! Veilid, no Tauri. CPU-bound: async callers run it off the executor.

#![forbid(unsafe_code)]

mod decode;
mod encode;
mod error;
mod kind;

pub use decode::{decode_limited, DecodeLimits, Decoded, Frames};
pub use encode::{encode_gif, encode_png, encode_webp, fit_within, fit_within_box};
pub use error::{ImageError, TooLarge};
pub use image::{DynamicImage, Frame, RgbaImage};
pub use kind::kind_of;
pub use rekindle_types::image_kind::ImageKind;
