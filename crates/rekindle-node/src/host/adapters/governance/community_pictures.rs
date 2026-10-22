//! Community avatars and banners on the Host (plan E3.1 D4), both modes.
//!
//! An upload is decoded as the picker's declared [`ImageKind`] only, under
//! [`SOURCE_LIMITS`] (plan E7.11: PNG, JPEG or GIF in, WebP out), re-encoded
//! as a WebP without metadata, and cached by its BLAKE3 hash in the
//! community's keyed directory under `state/{pk}.files/`
//! (`rekindle_files::CacheScope::CommunityAvatars`), so no name on the disk
//! carries the community id before unlock.

use std::path::PathBuf;

use rekindle_image::{DecodeLimits, Frames};
use rekindle_types::image_kind::ImageKind;

use super::HostGovernance;

/// What a picture is cut to, px: an avatar's sides, or a banner's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PictureSlot {
    /// At most 128 × 128.
    Avatar,
    /// At most 600 × 200.
    Banner,
}

impl PictureSlot {
    /// The widest and tallest the cached WebP is.
    const fn bounds(self) -> (u32, u32) {
        match self {
            Self::Avatar => (128, 128),
            Self::Banner => (600, 200),
        }
    }
}

/// The most bytes an upload may carry (an avatar or a banner, before decode).
pub(crate) const MAX_UPLOAD_BYTES: usize = 8 * 1024 * 1024;

/// What a picture offered as a community avatar or banner may cost to
/// decode (plan E7.11, as our own avatar's): sides up to 8192 px and
/// 40 Mpx, 192 MiB held. An animated GIF gives its first frame.
const SOURCE_LIMITS: DecodeLimits = DecodeLimits {
    max_width: 8192,
    max_height: 8192,
    max_pixels: 40_000_000,
    max_alloc: 192 * 1024 * 1024,
    frames: Frames::First,
};

/// Hex characters in a picture's BLAKE3 hash.
const HASH_HEX_LEN: usize = 64;

/// Whether `hash` is a picture name the cache writes: 64 lowercase hex
/// digits, so it can never leave the community's directory.
#[must_use]
pub(crate) fn is_picture_hash(hash: &str) -> bool {
    hash.len() == HASH_HEX_LEN && hash.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

impl HostGovernance {
    /// Cache `image`, declared to be `kind`, as `community`'s `slot`
    /// picture; answers its hash.
    ///
    /// # Errors
    /// The image is empty, too large, not `kind` or over the decode limits;
    /// no identity is unlocked; or the file could not be written.
    pub(crate) async fn set_community_picture(
        &self,
        community: &str,
        image: Vec<u8>,
        kind: ImageKind,
        slot: PictureSlot,
    ) -> Result<String, String> {
        if image.is_empty() {
            return Err("the picture is empty".into());
        }
        if image.len() > MAX_UPLOAD_BYTES {
            return Err(format!(
                "the picture is {} bytes; at most {MAX_UPLOAD_BYTES} are taken",
                image.len()
            ));
        }
        let dir = self.pictures_dir(community)?;
        let webp = rekindle_lifecycle::run_blocking(move || compress(&image, kind, slot))
            .await
            .map_err(|e| e.to_string())??;
        let hash = blake3::hash(&webp).to_hex().to_string();
        let path = dir.join(format!("{hash}.webp"));
        rekindle_lifecycle::run_blocking(move || {
            std::fs::create_dir_all(&dir)?;
            if path.exists() {
                return Ok(());
            }
            std::fs::write(&path, &webp)
        })
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| format!("picture not cached: {e}"))?;
        Ok(hash)
    }

    /// `community`'s cached picture `hash` as a `data:image/webp` URL;
    /// `None` when it is not cached. The bytes must still be a WebP.
    ///
    /// # Errors
    /// The hash is malformed, no identity is unlocked, the read failed, or
    /// the cached file is not a WebP.
    pub(crate) async fn community_picture_data_url(
        &self,
        community: &str,
        hash: &str,
    ) -> Result<Option<String>, String> {
        use base64::Engine as _;
        if !is_picture_hash(hash) {
            return Err("a picture hash is 64 lowercase hex digits".into());
        }
        let path = self.pictures_dir(community)?.join(format!("{hash}.webp"));
        let read = rekindle_lifecycle::run_blocking(move || std::fs::read(path))
            .await
            .map_err(|e| e.to_string())?;
        let bytes = match read {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(format!("picture not read: {e}")),
        };
        if !is_webp(&bytes) || blake3::hash(&bytes).to_hex().as_str() != hash {
            return Err("the cached picture is not the WebP it names".into());
        }
        let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
        Ok(Some(format!("data:image/webp;base64,{b64}")))
    }

    /// `community`'s keyed picture directory under the identity's files.
    fn pictures_dir(&self, community: &str) -> Result<PathBuf, String> {
        let owner = self
            .ctx
            .require_owner()
            .map_err(|_| "no identity is unlocked")?;
        let secret = zeroize::Zeroizing::new(
            self.ctx
                .require_signing_key()
                .map_err(|_| "no identity is unlocked")?,
        );
        let name_key = rekindle_secrets::derive::derive_file_cache_name_key(&secret);
        Ok(rekindle_files::scope_root(
            &self.ctx.root.identity_files(&owner),
            &name_key,
            rekindle_files::CacheScope::CommunityAvatars(community),
        ))
    }
}

/// Whether `bytes` are a RIFF WebP container (RFC 9649 §2.5).
fn is_webp(bytes: &[u8]) -> bool {
    bytes.len() > 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP"
}

/// Decode `raw` as `kind` only, under [`SOURCE_LIMITS`], and re-encode it
/// as a WebP within `slot`'s bounds. CPU-bound: run off the executor.
fn compress(raw: &[u8], kind: ImageKind, slot: PictureSlot) -> Result<Vec<u8>, String> {
    let picture = rekindle_image::decode_limited(raw, kind, &SOURCE_LIMITS)
        .map_err(|e| e.to_string())?
        .into_first();
    let (max_w, max_h) = slot.bounds();
    rekindle_image::encode_webp(&rekindle_image::fit_within_box(picture, max_w, max_h)).map_err(|e| e.to_string())
}

#[cfg(test)]
#[path = "community_pictures_tests.rs"]
mod tests;
