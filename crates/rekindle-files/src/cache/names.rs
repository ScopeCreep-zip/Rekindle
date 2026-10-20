//! Keyed directory names for the per-identity file cache (plan E3.0a).
//!
//! Before unlock anyone with the disk can list `state/{pk}.files/`, so no
//! name below it may carry a community id, a conversation id, a peer key or
//! an attachment id. Every directory name is the first 16 bytes of
//! `BLAKE3-keyed(k_names, label)` in hex (32 characters), where `k_names`
//! is `rekindle_secrets::derive::derive_file_cache_name_key` of the
//! identity secret: stable across restarts, unlinkable without the secret,
//! and different for every identity.
//!
//! Each label is a registered `rekindle_types::domains` prefix, `:` and the
//! value. The registry's tests keep every prefix distinct and none a prefix
//! of another, and none contains `:`, so two labels of different kinds
//! never meet.

use std::path::{Path, PathBuf};

use rekindle_types::domains;

/// Hex characters in every keyed name (128 bits).
pub const NAME_HEX_LEN: usize = 32;

/// What one cache directory under `state/{pk}.files/` holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheScope<'a> {
    /// A community's channel attachments (chunk cache).
    Community(&'a str),
    /// A 1:1 or group conversation's attachments.
    Dm(&'a str),
    /// A community's avatar and banner blobs.
    CommunityAvatars(&'a str),
}

impl CacheScope<'_> {
    fn label(self) -> Vec<u8> {
        let (prefix, id) = match self {
            Self::Community(id) => (domains::FILE_CACHE_COMMUNITY, id),
            Self::Dm(id) => (domains::FILE_CACHE_DM, id),
            Self::CommunityAvatars(id) => (domains::FILE_CACHE_AVATARS, id),
        };
        [prefix.as_bytes(), b":", id.as_bytes()].concat()
    }
}

/// An attachment's directory name inside one scope, as bytes (the hex of
/// it is the directory name). The cache keys its LRU by it, since the
/// attachment id cannot be read back from the disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AttachmentName([u8; NAME_HEX_LEN / 2]);

impl AttachmentName {
    /// The name of `attachment_id` in the scope directory named `scope_dir`.
    /// Binding the scope keeps one file shared in two scopes from showing
    /// as the same directory name in both.
    #[must_use]
    pub fn derive(key: &[u8; 32], scope_dir: &[u8], attachment_id: &[u8; 16]) -> Self {
        let label = [
            domains::FILE_CACHE_ATTACHMENT.as_bytes(),
            b":",
            scope_dir,
            b":",
            attachment_id,
        ]
        .concat();
        Self(truncated(key, &label))
    }

    /// Parse a directory name the cache wrote; anything else is `None`.
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        if name.len() != NAME_HEX_LEN || name.bytes().any(|b| b.is_ascii_uppercase()) {
            return None;
        }
        let bytes = hex::decode(name).ok()?;
        Some(Self(bytes.as_slice().try_into().ok()?))
    }

    /// The directory name.
    #[must_use]
    pub fn to_hex(self) -> String {
        hex::encode(self.0)
    }
}

fn truncated(key: &[u8; 32], label: &[u8]) -> [u8; NAME_HEX_LEN / 2] {
    let hash = blake3::keyed_hash(key, label);
    let mut out = [0u8; NAME_HEX_LEN / 2];
    out.copy_from_slice(&hash.as_bytes()[..NAME_HEX_LEN / 2]);
    out
}

/// The keyed name of `label` under `key`.
#[must_use]
pub fn keyed_name(key: &[u8; 32], label: &[u8]) -> String {
    hex::encode(truncated(key, label))
}

/// The directory of `scope` under an identity's cache root
/// (`DataRoot::identity_files(pk)`): `<identity_files>/<keyed name>`.
#[must_use]
pub fn scope_root(identity_files: &Path, key: &[u8; 32], scope: CacheScope<'_>) -> PathBuf {
    identity_files.join(keyed_name(key, &scope.label()))
}
