//! The seam every key consumer reads community and channel keys through
//! (plan D6, step B5). Read-only: rotation, distribution and persistence
//! stay with `rekindle-mek-rotation`.
//!
//! A key is named by community, [`KeyScope`] and [`KeyEpoch`] (today's MEK
//! generation), and the provider answers that exact epoch — current or
//! historical — or nothing. Which scope a payload uses is the provider's
//! policy ([`ChannelKeyProvider::scope_for_text`],
//! [`ChannelKeyProvider::scope_for_media`]), stated once, never a chain of
//! "this key, else that one" (RFC 9605 §4.4.1: the key is selected by the
//! id the ciphertext names).

use core::time::Duration;

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

    /// How long the scope's current epoch has been current, or `None`
    /// when no key is held. Bounds how long media under the epoch it
    /// replaced is still accepted ([`MEDIA_PREVIOUS_EPOCH_GRACE`]).
    fn current_epoch_age(&self, community_id: &str, scope: KeyScope) -> Option<Duration>;

    /// The scope channel text (and its attachments and threads) uses.
    fn scope_for_text(&self, community_id: &str, channel: ChannelId) -> KeyScope;

    /// The scope a channel's voice and video use.
    fn scope_for_media(&self, community_id: &str, channel: ChannelId) -> KeyScope;
}

/// How long after a rotation media under the replaced epoch still
/// decrypts, so frames in flight across the rotation are not lost. Past
/// it, the replaced key — which a removed member still holds — opens
/// nothing (RFC 9605 §4.4.1: an old key "may be kept for some time" and
/// then retired; Discord DAVE keeps the previous epoch about ten seconds).
pub const MEDIA_PREVIOUS_EPOCH_GRACE: Duration = Duration::from_secs(10);

/// What a media frame under `epoch` may be opened with.
#[derive(Debug)]
pub enum MediaKey {
    /// The key for the frame's epoch.
    Key(Zeroizing<[u8; 32]>),
    /// The frame is under an epoch we no longer accept: older than the
    /// one the current key replaced, or the replaced one past its grace.
    Stale,
    /// We do not hold the frame's epoch (we are behind, or hold no key):
    /// request exactly `needed`.
    Missing { needed: KeyEpoch },
}

/// Resolve the key for a media frame under `epoch` in `scope`: the
/// current epoch, or the one it replaced within
/// [`MEDIA_PREVIOUS_EPOCH_GRACE`]. Never another epoch's or scope's key.
pub fn media_key(
    provider: &dyn ChannelKeyProvider,
    community_id: &str,
    scope: KeyScope,
    epoch: KeyEpoch,
) -> MediaKey {
    let Some(current) = provider.current_epoch(community_id, scope) else {
        return MediaKey::Missing { needed: epoch };
    };
    if epoch > current {
        return MediaKey::Missing { needed: epoch };
    }
    let in_grace = || {
        provider
            .current_epoch_age(community_id, scope)
            .is_some_and(|age| age < MEDIA_PREVIOUS_EPOCH_GRACE)
    };
    if epoch < current && !(epoch.0 + 1 == current.0 && in_grace()) {
        return MediaKey::Stale;
    }
    provider
        .key(community_id, scope, epoch)
        .map_or(MediaKey::Missing { needed: epoch }, MediaKey::Key)
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

    struct Fixed {
        current: u64,
        age: Duration,
    }

    impl ChannelKeyProvider for Fixed {
        fn current_epoch(&self, _: &str, _: KeyScope) -> Option<KeyEpoch> {
            (self.current > 0).then_some(KeyEpoch(self.current))
        }
        fn key(&self, _: &str, _: KeyScope, epoch: KeyEpoch) -> Option<Zeroizing<[u8; 32]>> {
            (epoch.0 <= self.current).then(|| Zeroizing::new([u8::try_from(epoch.0).unwrap(); 32]))
        }
        fn current_epoch_age(&self, _: &str, _: KeyScope) -> Option<Duration> {
            Some(self.age)
        }
        fn scope_for_text(&self, _: &str, _: ChannelId) -> KeyScope {
            KeyScope::Community
        }
        fn scope_for_media(&self, _: &str, channel: ChannelId) -> KeyScope {
            KeyScope::Channel(channel)
        }
    }

    fn resolve(current: u64, age_secs: u64, epoch: u64) -> MediaKey {
        let provider = Fixed {
            current,
            age: Duration::from_secs(age_secs),
        };
        media_key(&provider, "c", KeyScope::Community, KeyEpoch(epoch))
    }

    #[test]
    fn media_current_and_previous_within_grace() {
        assert!(matches!(resolve(5, 60, 5), MediaKey::Key(k) if k[0] == 5));
        assert!(matches!(resolve(5, 3, 4), MediaKey::Key(k) if k[0] == 4));
    }

    /// The replaced key stops opening media once the grace has passed —
    /// a removed member's frames under it are refused.
    #[test]
    fn media_previous_past_grace_and_older_are_stale() {
        assert!(matches!(resolve(5, 11, 4), MediaKey::Stale));
        assert!(matches!(resolve(5, 3, 3), MediaKey::Stale));
    }

    #[test]
    fn media_ahead_or_unkeyed_is_missing_that_epoch() {
        assert!(matches!(resolve(5, 3, 6), MediaKey::Missing { needed } if needed == KeyEpoch(6)));
        assert!(matches!(resolve(0, 0, 1), MediaKey::Missing { needed } if needed == KeyEpoch(1)));
    }
}
