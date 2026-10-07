//! The single source of truth for webview window labels.
//!
//! Every window Rekindle creates has a [`WindowKind`]; per-entity windows
//! derive their label from a validated id (`rekindle_types::key_format`),
//! so a label can never carry attacker-chosen characters. Capability files
//! (`capabilities/*.json`) match these patterns, and
//! `tests/capability_policy.rs` includes this file to check they agree.
//!
//! Pure: no tauri imports, so the policy test can `#[path]`-include it.

use rekindle_types::key_format::{self, CallId, PublicKeyHex, RecordKeyStr};

/// Every kind of window the app opens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowKind {
    Login,
    BuddyList,
    Settings,
    Chat,
    Dm,
    Community,
    Call,
    Profile,
}

impl WindowKind {
    pub const ALL: [Self; 8] = [
        Self::Login,
        Self::BuddyList,
        Self::Settings,
        Self::Chat,
        Self::Dm,
        Self::Community,
        Self::Call,
        Self::Profile,
    ];

    /// The label (singletons) or label pattern (per-entity windows) that
    /// capability files use for this kind.
    #[must_use]
    pub const fn pattern(self) -> &'static str {
        match self {
            Self::Login => LOGIN,
            Self::BuddyList => BUDDY_LIST,
            Self::Settings => SETTINGS,
            Self::Chat => "chat-*",
            Self::Dm => "dm-*",
            Self::Community => "community-*",
            Self::Call => "call-*",
            Self::Profile => "profile-*",
        }
    }

    /// Whether this window renders a surface that opens the camera or
    /// microphone through the webview:
    /// - the 1:1 call's controls and video (call);
    /// - voice channels and video (community);
    /// - voice messages (chat, DM, community message input);
    /// - device settings and the pairing QR scanner (settings).
    ///
    /// Login, profile and the buddy list never capture (group-call audio
    /// runs on the native pipeline), so the media-capture grant is withheld
    /// from them.
    #[must_use]
    pub const fn captures_media(self) -> bool {
        match self {
            Self::Settings | Self::Chat | Self::Dm | Self::Community | Self::Call => true,
            Self::Login | Self::BuddyList | Self::Profile => false,
        }
    }
}

impl WindowKind {
    /// The kind of window a label belongs to, if it is one of ours.
    #[must_use]
    pub fn of_label(label: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|kind| match kind.pattern().strip_suffix('*') {
                Some(prefix) => label.starts_with(prefix),
                None => label == kind.pattern(),
            })
    }
}

pub const LOGIN: &str = "login";
pub const BUDDY_LIST: &str = "buddy-list";
pub const SETTINGS: &str = "settings";

/// `chat-` + the first 16 hex digits of the peer's identity key.
#[must_use]
pub fn chat_label(peer: &PublicKeyHex) -> String {
    format!("chat-{}", rekindle_utils::text::prefix(peer.as_str(), 16))
}

/// `dm-` + the first 20 alphanumeric characters of the DM record key.
#[must_use]
pub fn dm_label(record: &RecordKeyStr) -> String {
    let suffix: String = record
        .as_str()
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .take(20)
        .collect();
    format!("dm-{suffix}")
}

/// `community-` + the first 16 characters of the community record key, or
/// `community-browser` for the community browser.
#[must_use]
pub fn community_label(community: Option<&RecordKeyStr>) -> String {
    match community {
        Some(id) => format!(
            "community-{}",
            rekindle_utils::text::prefix(id.as_str(), 16)
        ),
        None => "community-browser".to_owned(),
    }
}

/// `call-` + the first 12 hex digits of the call id.
#[must_use]
pub fn call_label(call: &CallId) -> String {
    format!("call-{}", rekindle_utils::text::prefix(call.as_str(), 12))
}

/// `profile-` + the first 12 hex digits of the identity key.
#[must_use]
pub fn profile_label(peer: &PublicKeyHex) -> String {
    format!(
        "profile-{}",
        rekindle_utils::text::prefix(peer.as_str(), 12)
    )
}

/// The chat and profile windows for a peer named by an event, if the key
/// is well formed. Event payloads are not validated types, so routing
/// goes through the same validators as window creation.
#[must_use]
pub fn peer_labels(peer_key: &str) -> Vec<String> {
    key_format::public_key_hex(peer_key)
        .map(|pk| vec![chat_label(&pk), profile_label(&pk)])
        .unwrap_or_default()
}

/// The DM window for a record-backed conversation named by an event.
#[must_use]
pub fn conversation_label(record_key: &str) -> Option<String> {
    key_format::record_key(record_key)
        .ok()
        .map(|rk| dm_label(&rk))
}

/// The pop-out window for a call named by an event.
#[must_use]
pub fn call_label_for(call_id: &str) -> Option<String> {
    key_format::call_id(call_id).ok().map(|id| call_label(&id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rekindle_types::key_format;

    const B64: &str = "um7m8HxBluv6XceSaB3dK9Lq0ZpWtYv1NcRe4Gh7Jf2";
    const HEX64: &str = "3f1a9c0b7e2d4f6a8b1c3e5d7f9a0b2c4d6e8f0a1b3c5d7e9f1a2b4c6d8e0f1a";

    #[test]
    fn labels_match_their_patterns() {
        let pk = key_format::public_key_hex(HEX64).unwrap();
        let rk = key_format::record_key(&format!("VLD0:{B64}:{B64}")).unwrap();
        let call = key_format::call_id(rekindle_utils::text::prefix(HEX64, 32)).unwrap();
        let cases = [
            (chat_label(&pk), WindowKind::Chat),
            (dm_label(&rk), WindowKind::Dm),
            (community_label(Some(&rk)), WindowKind::Community),
            (community_label(None), WindowKind::Community),
            (call_label(&call), WindowKind::Call),
            (profile_label(&pk), WindowKind::Profile),
        ];
        for (label, kind) in cases {
            let prefix = kind.pattern().trim_end_matches('*');
            assert!(label.starts_with(prefix), "{label} vs {}", kind.pattern());
            assert!(
                label
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | ':' | '_')),
                "{label} has characters outside the tauri label set"
            );
        }
        assert_eq!(
            dm_label(&rk),
            format!("dm-VLD0{}", rekindle_utils::text::prefix(B64, 16))
        );
    }

    #[test]
    fn labels_map_back_to_their_kind() {
        let pk = key_format::public_key_hex(HEX64).unwrap();
        assert_eq!(WindowKind::of_label(LOGIN), Some(WindowKind::Login));
        assert_eq!(
            WindowKind::of_label(&chat_label(&pk)),
            Some(WindowKind::Chat)
        );
        assert_eq!(
            WindowKind::of_label("community-browser"),
            Some(WindowKind::Community)
        );
        assert_eq!(WindowKind::of_label("settingsx"), None);
        assert_eq!(WindowKind::of_label("unknown"), None);
        assert_eq!(peer_labels("not-a-key"), Vec::<String>::new());
        assert_eq!(peer_labels(HEX64).len(), 2);
        assert!(call_label_for(rekindle_utils::text::prefix(HEX64, 32)).is_some());
        assert!(conversation_label("nope").is_none());
    }
}
