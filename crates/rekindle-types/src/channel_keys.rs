//! The seam every key consumer reads community and channel keys through
//! (plan D6, step B5). Read-only: rotation, distribution and persistence
//! stay with `rekindle-mek-rotation`.
//!
//! A key is named by community, [`KeyScope`] and [`KeyEpoch`] (today's MEK
//! generation), and the provider answers that exact epoch — current or
//! historical — or nothing. Which scope a payload uses is the provider's
//! policy ([`ChannelKeyProvider::scope_for_text`]), stated once, never a
//! chain of "this key, else that one" (RFC 9605 §4.4.1: the key is
//! selected by the id the ciphertext names).
//!
//! Call media is not keyed here: each participant keys its own media
//! (`rekindle_secrets::media_sender_key`, plan C7.20).

/// Re-exported so implementors of [`ChannelKeyProvider`] name the key type
/// without their own `zeroize` dependency.
pub use zeroize::Zeroizing;

use crate::id::ChannelId;

/// Which key of a community.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KeyScope {
    /// The community-wide key.
    Community,
    /// A channel's own key (rotated with that channel's membership).
    Channel(ChannelId),
}

impl KeyScope {
    /// Parse the scope from a control payload's channel field.
    ///
    /// The MEK control payloads (`MekTransfer`, `MEKRotated`, `RequestMEK`)
    /// name the community scope by an absent or empty channel field and a
    /// channel by its 32-hex id; that format is frozen until D1 (plan D9).
    /// This is the one place it is read. Anything else is not a scope.
    #[must_use]
    pub fn from_wire(channel: Option<&str>) -> Option<Self> {
        match channel {
            None | Some("") => Some(Self::Community),
            Some(hex) => ChannelId::from_hex(hex).map(Self::Channel),
        }
    }

    /// The channel field for an optional-channel payload
    /// (`None` = community).
    #[must_use]
    pub fn wire_channel(self) -> Option<String> {
        match self {
            Self::Community => None,
            Self::Channel(id) => Some(id.to_hex()),
        }
    }

    /// The channel field for a payload whose channel field is a plain
    /// string (`""` = community).
    #[must_use]
    pub fn wire_channel_str(self) -> String {
        self.wire_channel().unwrap_or_default()
    }
}

impl core::fmt::Display for KeyScope {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Community => f.write_str("community"),
            Self::Channel(id) => write!(f, "channel:{}", id.to_hex()),
        }
    }
}

/// A key generation within a scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct KeyEpoch(pub u64);

/// Read access to a host's community and channel keys.
pub trait ChannelKeyProvider: Send + Sync {
    /// The scope's current epoch, or `None` when no key is held.
    fn current_epoch(&self, community_id: &str, scope: KeyScope) -> Option<KeyEpoch>;

    /// The key for exactly `epoch`, from memory or persisted history.
    fn key(
        &self,
        community_id: &str,
        scope: KeyScope,
        epoch: KeyEpoch,
    ) -> Option<Zeroizing<[u8; 32]>>;

    /// The scope channel text (and its attachments and threads) uses.
    fn scope_for_text(&self, community_id: &str, channel: ChannelId) -> KeyScope;
}

/// The scope's current key with its epoch.
pub fn current_key(
    provider: &dyn ChannelKeyProvider,
    community_id: &str,
    scope: KeyScope,
) -> Option<(KeyEpoch, Zeroizing<[u8; 32]>)> {
    let epoch = provider.current_epoch(community_id, scope)?;
    Some((epoch, provider.key(community_id, scope, epoch)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_round_trip() {
        let channel = KeyScope::Channel(ChannelId([0xab; 16]));
        assert_eq!(KeyScope::from_wire(None), Some(KeyScope::Community));
        assert_eq!(KeyScope::from_wire(Some("")), Some(KeyScope::Community));
        assert_eq!(
            KeyScope::from_wire(channel.wire_channel().as_deref()),
            Some(channel)
        );
        assert_eq!(
            KeyScope::from_wire(Some(&channel.wire_channel_str())),
            Some(channel)
        );
        assert_eq!(KeyScope::Community.wire_channel(), None);
        assert_eq!(KeyScope::Community.wire_channel_str(), "");
    }

    /// No other string names a scope: the old `"__community__"` slot and
    /// non-hex channel names are rejected, not mapped somewhere.
    #[test]
    fn non_scopes_are_rejected() {
        for bad in ["__community__", "general", "abcd", &"0".repeat(33)] {
            assert_eq!(KeyScope::from_wire(Some(bad)), None, "{bad}");
        }
    }
}
