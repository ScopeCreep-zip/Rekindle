//! Invite secrets for self-sovereign community join.
//!
//! In v2.0, invites are distributed out-of-band (deep links, QR codes, peer share).
//! The invite blob contains everything a joiner needs: governance key, registry key,
//! slot seed, channel keys, and current MEK. Encrypted with HKDF(invite_code).

use serde::{Deserialize, Serialize};

/// Decrypted invite secrets — everything needed to join a community.
///
/// The invite code (shared out-of-band) is the HKDF key that decrypts this blob.
/// The governance_key in the deep link URL identifies the community.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InviteSecrets {
    /// DHT key of the SMPL governance record (community identifier).
    pub governance_key: String,
    /// DHT key of the SMPL member registry record.
    pub registry_key: String,
    /// Private route blob to the inviting member for bootstrap bundle app_call.
    pub inviter_route_blob: Vec<u8>,
    /// 32-byte slot seed for deriving SMPL member slot keypairs (hex-encoded).
    pub slot_seed: String,
    /// Current MEK wire bytes (generation LE + key, base64-encoded).
    pub mek_wire_bytes: String,
    /// Channel record keys for direct channel access.
    pub channel_keys: Vec<ChannelKeyInfo>,
    /// Community display name (for UI before full state is loaded).
    pub community_name: String,
}

/// A channel's DHT record key bundled in invite secrets.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelKeyInfo {
    /// Channel identifier (hex-encoded 16-byte UUID).
    pub channel_id: String,
    /// DHT key of the channel's SMPL record.
    pub record_key: String,
    /// Channel display name.
    pub name: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invite_secrets_serde_roundtrip() {
        let secrets = InviteSecrets {
            governance_key: "VLD0:gov123".into(),
            registry_key: "VLD0:reg456".into(),
            inviter_route_blob: vec![1, 2, 3, 4],
            slot_seed: "ab".repeat(32),
            mek_wire_bytes: "base64mekdata".into(),
            channel_keys: vec![ChannelKeyInfo {
                channel_id: "ch1".into(),
                record_key: "VLD0:ch789".into(),
                name: "general".into(),
            }],
            community_name: "Test Community".into(),
        };
        let json = serde_json::to_string(&secrets).unwrap();
        let back: InviteSecrets = serde_json::from_str(&json).unwrap();
        assert_eq!(back.governance_key, "VLD0:gov123");
        assert_eq!(back.inviter_route_blob, vec![1, 2, 3, 4]);
        assert_eq!(back.channel_keys.len(), 1);
    }
}

// ── Invite deep link ─────────────────────────────────────────────────

use crate::key_format::{self, Hex16Id, KeyFormatError, RecordKeyStr};

/// Scheme and path prefix of every invite link.
const INVITE_PREFIX: &str = "rekindle://invite/";
/// Longest invite URL accepted. A canonical link with two two-segment
/// record keys is 236 bytes.
pub const MAX_INVITE_URL_LEN: usize = 256;

/// The three components an invite link carries.
///
/// Lives in Tier 1 so every frontend (desktop deep links, the daemon's
/// `CommunityJoin`, the CLI) accepts exactly the same link, and every
/// producer builds it with [`InviteLink::to_url`].
///
/// Format: `rekindle://invite/{governance_key}/{secrets_record_key}/{invite_code}`,
/// where both keys are Veilid record keys and the code is 32 lowercase hex
/// (16 random bytes).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InviteLink {
    pub governance_key: RecordKeyStr,
    pub secrets_record_key: RecordKeyStr,
    pub invite_code: Hex16Id,
}

/// Why a string is not an invite link.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum InviteLinkError {
    #[error("not a rekindle://invite/ link")]
    NotAnInvite,
    #[error("invite link is {got} bytes, max {max}")]
    TooLong { max: usize, got: usize },
    #[error("invite link needs a governance key, a secrets key and a code")]
    MissingPart,
    #[error("invite link has extra path segments")]
    ExtraPart,
    #[error("invite governance key: {0}")]
    GovernanceKey(KeyFormatError),
    #[error("invite secrets key: {0}")]
    SecretsRecordKey(KeyFormatError),
    #[error("invite code: {0}")]
    InviteCode(KeyFormatError),
}

impl InviteLink {
    /// Parse an invite URL. Surrounding whitespace is ignored; anything
    /// else that is not the exact canonical shape is rejected.
    pub fn parse(url: &str) -> Result<Self, InviteLinkError> {
        let url = url.trim();
        if url.len() > MAX_INVITE_URL_LEN {
            return Err(InviteLinkError::TooLong {
                max: MAX_INVITE_URL_LEN,
                got: url.len(),
            });
        }
        let rest = url
            .strip_prefix(INVITE_PREFIX)
            .ok_or(InviteLinkError::NotAnInvite)?;
        let mut parts = rest.split('/');
        let mut next = || {
            parts
                .next()
                .filter(|p| !p.is_empty())
                .ok_or(InviteLinkError::MissingPart)
        };
        let governance_key = next()?;
        let secrets_record_key = next()?;
        let invite_code = next()?;
        if parts.next().is_some() {
            return Err(InviteLinkError::ExtraPart);
        }
        Ok(Self {
            governance_key: key_format::record_key(governance_key)
                .map_err(InviteLinkError::GovernanceKey)?,
            secrets_record_key: key_format::record_key(secrets_record_key)
                .map_err(InviteLinkError::SecretsRecordKey)?,
            invite_code: key_format::hex16_id(invite_code).map_err(InviteLinkError::InviteCode)?,
        })
    }

    /// The canonical link. [`InviteLink::parse`] of the result returns `self`.
    #[must_use]
    pub fn to_url(&self) -> String {
        format!(
            "{INVITE_PREFIX}{}/{}/{}",
            self.governance_key, self.secrets_record_key, self.invite_code
        )
    }
}

/// Pairing URIs (`rekindle://pair?...`), produced by Settings → Devices.
const PAIRING_PREFIX: &str = "rekindle://pair";
/// Every Rekindle URL.
const SCHEME_PREFIX: &str = "rekindle://";
/// Longest friend-invite URL accepted from the OS. A friend invite carries
/// a signed blob with a PQXDH prekey bundle and a route blob.
pub const MAX_FRIEND_INVITE_URL_LEN: usize = 16 * 1024;

/// What an OS deep link asks for, before any network access.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeepLink {
    /// `rekindle://invite/...`: join a community.
    CommunityInvite(InviteLink),
    /// `rekindle://<base64url blob>`: add a friend. The blob is decoded and
    /// its signature checked by `rekindle_protocol::messaging`.
    FriendInvite(String),
    /// `rekindle://pair?...`: link a new device. Never accepted from an OS
    /// deep link, because an attacker-supplied pairing URI would steer this
    /// device into pairing with the attacker; pairing codes are pasted or
    /// scanned in Settings → Devices.
    Pairing,
}

/// Why an OS deep link was rejected.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DeepLinkError {
    #[error("not a rekindle:// link")]
    NotRekindle,
    #[error("friend invite link is {got} bytes, max {max}")]
    TooLong { max: usize, got: usize },
    #[error(transparent)]
    CommunityInvite(#[from] InviteLinkError),
}

impl DeepLink {
    /// Classify a `rekindle://` URL. Community invites are fully validated
    /// here; a friend invite is only length-bounded.
    pub fn parse(url: &str) -> Result<Self, DeepLinkError> {
        let url = url.trim();
        if url.starts_with(INVITE_PREFIX) {
            return Ok(Self::CommunityInvite(InviteLink::parse(url)?));
        }
        if url.starts_with(PAIRING_PREFIX) {
            return Ok(Self::Pairing);
        }
        if !url.starts_with(SCHEME_PREFIX) {
            return Err(DeepLinkError::NotRekindle);
        }
        if url.len() > MAX_FRIEND_INVITE_URL_LEN {
            return Err(DeepLinkError::TooLong {
                max: MAX_FRIEND_INVITE_URL_LEN,
                got: url.len(),
            });
        }
        Ok(Self::FriendInvite(url.to_owned()))
    }
}

#[cfg(test)]
mod invite_link_tests {
    use super::{DeepLink, DeepLinkError, InviteLink, InviteLinkError, MAX_INVITE_URL_LEN};

    const B64: &str = "um7m8HxBluv6XceSaB3dK9Lq0ZpWtYv1NcRe4Gh7Jf2";
    const CODE: &str = "3f1a9c0b7e2d4f6a8b1c3e5d7f9a0b2c";

    fn link(gov: &str, sec: &str, code: &str) -> String {
        format!("rekindle://invite/{gov}/{sec}/{code}")
    }

    #[test]
    fn parses_and_round_trips_a_canonical_link() {
        let gov = format!("VLD0:{B64}:{B64}");
        let sec = format!("VLD0:{B64}");
        let url = link(&gov, &sec, CODE);
        let parsed = InviteLink::parse(&format!("  {url}\n")).unwrap();
        assert_eq!(parsed.governance_key.as_str(), gov);
        assert_eq!(parsed.secrets_record_key.as_str(), sec);
        assert_eq!(parsed.invite_code.as_str(), CODE);
        assert_eq!(parsed.to_url(), url);
        assert!(url.len() <= MAX_INVITE_URL_LEN);
    }

    #[test]
    fn rejects_everything_else() {
        let k = format!("VLD0:{B64}");
        let cases = [
            (
                "https://example.com".to_owned(),
                InviteLinkError::NotAnInvite,
            ),
            (
                format!("rekindle://community/{k}/{k}/{CODE}"),
                InviteLinkError::NotAnInvite,
            ),
            (
                format!("rekindle://invite/{k}/{k}"),
                InviteLinkError::MissingPart,
            ),
            (
                format!("rekindle://invite/{k}//{CODE}"),
                InviteLinkError::MissingPart,
            ),
            (
                format!("{}/", link(&k, &k, CODE)),
                InviteLinkError::ExtraPart,
            ),
            (
                link(&k, &k, &format!("{CODE}/x")),
                InviteLinkError::ExtraPart,
            ),
        ];
        for (url, err) in cases {
            assert_eq!(InviteLink::parse(&url), Err(err), "{url}");
        }
        assert!(matches!(
            InviteLink::parse(&link(&format!("VLD1:{B64}"), &k, CODE)),
            Err(InviteLinkError::GovernanceKey(_))
        ));
        assert!(matches!(
            InviteLink::parse(&link(&k, "VLD0:short", CODE)),
            Err(InviteLinkError::SecretsRecordKey(_))
        ));
        assert!(matches!(
            InviteLink::parse(&link(&k, &k, &format!("{CODE}00"))),
            Err(InviteLinkError::InviteCode(_))
        ));
        assert!(matches!(
            InviteLink::parse(&link(&k, &k, &CODE.to_uppercase())),
            Err(InviteLinkError::InviteCode(_))
        ));
        assert!(matches!(
            InviteLink::parse(&format!("rekindle://invite/{}", "A".repeat(300))),
            Err(InviteLinkError::TooLong { .. })
        ));
    }

    #[test]
    fn classifies_deep_links() {
        let k = format!("VLD0:{B64}");
        let invite = link(&k, &k, CODE);
        assert!(matches!(
            DeepLink::parse(&invite),
            Ok(DeepLink::CommunityInvite(_))
        ));
        assert!(matches!(
            DeepLink::parse("rekindle://invite/bad"),
            Err(DeepLinkError::CommunityInvite(_))
        ));
        assert_eq!(
            DeepLink::parse("rekindle://pair?code=1"),
            Ok(DeepLink::Pairing)
        );
        assert_eq!(
            DeepLink::parse(" rekindle://eyJhIjoxfQ "),
            Ok(DeepLink::FriendInvite("rekindle://eyJhIjoxfQ".into()))
        );
        assert_eq!(
            DeepLink::parse("https://x"),
            Err(DeepLinkError::NotRekindle)
        );
        assert!(matches!(
            DeepLink::parse(&format!("rekindle://{}", "A".repeat(20_000))),
            Err(DeepLinkError::TooLong { .. })
        ));
    }

    /// The hostile corpus the webview spec used to probe. Every entry is
    /// rejected before anything is held for consent.
    #[test]
    fn hostile_invite_links_are_rejected() {
        let corpus = [
            "rekindle://invite/".to_owned(),
            "rekindle://invite/a".to_owned(),
            "rekindle://invite/!!!!#@@@@".to_owned(),
            "rekindle://invite/QUFBQUFBQUFBQQ==#Wm16YnRtbk9hQQ==".to_owned(),
            "rekindle://invite/<script>alert(1)</script>#key".to_owned(),
            "rekindle://invite/../../../etc/passwd#k".to_owned(),
            "rekindle://invite/blob';DROP TABLE friends;--#k".to_owned(),
            format!(
                "rekindle://invite/{}#{}",
                "A".repeat(100_000),
                "B".repeat(10_000)
            ),
            "rekindle://invite/aaa%00bbb#%00".to_owned(),
            "rekindle://invite/aaa\u{0}bbb/x/y".to_owned(),
            "rekindle://invite/aaa%01%02%03#%04".to_owned(),
            "rekindle://invite/blob\u{202E}#key".to_owned(),
            format!("rekindle://invite/VLD0:{B64}/VLD0:{B64}/{CODE}#frag"),
            format!("rekindle://invite/VLD0:{B64}/VLD0:{B64}/{CODE}?q=1"),
        ];
        for url in &corpus {
            assert!(InviteLink::parse(url).is_err(), "{url:.60}");
            assert!(DeepLink::parse(url).is_err(), "{url:.60}");
        }
    }
}
