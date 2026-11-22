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
    Community,
    Call,
    Profile,
    /// The skin editor (plan E7.7.5), a Settings sub-window.
    SkinEditor,
    /// A pack popup (plan SK2-A5: Xfire's Infoview, StealthX's main menu)
    /// beside the skin window that launched it.
    SkinPopup,
}

impl WindowKind {
    pub const ALL: [Self; 9] = [
        Self::Login,
        Self::BuddyList,
        Self::Settings,
        Self::Chat,
        Self::Community,
        Self::Call,
        Self::Profile,
        Self::SkinEditor,
        Self::SkinPopup,
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
            Self::Community => "community-*",
            Self::Call => "call-*",
            Self::Profile => "profile-*",
            Self::SkinEditor => SKIN_EDITOR,
            Self::SkinPopup => "popup-*",
        }
    }

    /// Whether this window renders a surface that opens the camera or
    /// microphone through the webview:
    /// - the 1:1 call's controls and video (call);
    /// - voice channels and video (community);
    /// - voice messages (chat, community message input);
    /// - device settings and the pairing QR scanner (settings).
    ///
    /// Login, profile, the skin editor, pack popups and the buddy list never capture (group-call audio
    /// runs on the native pipeline), so the media-capture grant is withheld
    /// from them.
    #[must_use]
    pub const fn captures_media(self) -> bool {
        match self {
            Self::Settings | Self::Chat | Self::Community | Self::Call => true,
            Self::Login | Self::BuddyList | Self::Profile | Self::SkinEditor | Self::SkinPopup => {
                false
            }
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
/// The skin editor: one at a time, opened from Settings → Appearance.
pub const SKIN_EDITOR: &str = "skin-editor";

/// `chat-` + the first 16 hex digits of the peer's identity key.
#[must_use]
pub fn chat_label(peer: &PublicKeyHex) -> String {
    format!("chat-{}", rekindle_utils::text::prefix(peer.as_str(), 16))
}

/// A group DM's id as a window names it: the creator's 16 random bytes
/// as 32 lowercase hex (plan E2.6). Validated here so a label never carries
/// attacker-chosen characters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupIdHex(String);

impl GroupIdHex {
    /// `text` when it is 32 lowercase hex digits.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        (text.len() == 32
            && text
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)))
        .then(|| Self(text.to_owned()))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The chat window in group mode: `chat-g` + the group id. A 1:1 window is
/// `chat-` + 16 hex digits, never a `g`, so the two cannot collide.
#[must_use]
pub fn group_chat_label(group: &GroupIdHex) -> String {
    format!("chat-g{}", group.as_str())
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

/// A pack popup's name (its key in the launching window's `popups`, plan
/// SK2-P1c), validated so a label and route never carry other characters:
/// 1 to 64 ASCII letters and digits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PopupName(String);

impl PopupName {
    /// `text` when it is 1 to 64 ASCII letters and digits.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        ((1..=64).contains(&text.len()) && text.bytes().all(|b| b.is_ascii_alphanumeric()))
            .then(|| Self(text.to_owned()))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// `popup-` + the launching window's label + `-` + the popup's name: one
/// window per popup per launcher (each chat window has its own). A name
/// holds no `-`, so the launcher is everything before the last one.
#[must_use]
pub fn popup_label(opener: &str, name: &PopupName) -> String {
    format!("popup-{opener}-{}", name.as_str())
}

/// The launching window's label of the popup labelled `label`.
#[must_use]
pub fn popup_opener(label: &str) -> Option<&str> {
    let (opener, name) = label.strip_prefix("popup-")?.rsplit_once('-')?;
    PopupName::parse(name).map(|_| opener)
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

/// The group-mode chat window for a group conversation named by an event
/// (its group id).
#[must_use]
pub fn conversation_label(group_id: &str) -> Option<String> {
    GroupIdHex::parse(group_id).map(|g| group_chat_label(&g))
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
        let group = GroupIdHex::parse(rekindle_utils::text::prefix(HEX64, 32)).unwrap();
        let cases = [
            (chat_label(&pk), WindowKind::Chat),
            (group_chat_label(&group), WindowKind::Chat),
            (community_label(Some(&rk)), WindowKind::Community),
            (community_label(None), WindowKind::Community),
            (call_label(&call), WindowKind::Call),
            (profile_label(&pk), WindowKind::Profile),
            (
                popup_label(&chat_label(&pk), &PopupName::parse("Info").unwrap()),
                WindowKind::SkinPopup,
            ),
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
            group_chat_label(&group),
            format!("chat-g{}", rekindle_utils::text::prefix(HEX64, 32))
        );
        assert_ne!(group_chat_label(&group), chat_label(&pk));
        let upper: String = HEX64.chars().take(32).collect::<String>().to_uppercase();
        assert!(GroupIdHex::parse(&upper).is_none());
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
        assert_eq!(
            WindowKind::of_label(SKIN_EDITOR),
            Some(WindowKind::SkinEditor)
        );
        assert_eq!(WindowKind::of_label("unknown"), None);
        let info = PopupName::parse("MainInfoViewPopup").unwrap();
        let popup = popup_label(BUDDY_LIST, &info);
        assert_eq!(popup, "popup-buddy-list-MainInfoViewPopup");
        assert_eq!(WindowKind::of_label(&popup), Some(WindowKind::SkinPopup));
        assert_eq!(popup_opener(&popup), Some(BUDDY_LIST));
        let chat = chat_label(&pk);
        assert_eq!(
            popup_opener(&popup_label(&chat, &info)),
            Some(chat.as_str())
        );
        assert_eq!(popup_opener("popup-buddy-list-"), None);
        assert_eq!(popup_opener(BUDDY_LIST), None);
        assert_eq!(peer_labels("not-a-key"), Vec::<String>::new());
        assert_eq!(peer_labels(HEX64).len(), 2);
        assert!(call_label_for(rekindle_utils::text::prefix(HEX64, 32)).is_some());
        assert!(conversation_label("nope").is_none());
        assert!(conversation_label(rekindle_utils::text::prefix(HEX64, 32)).is_some());
    }

    #[test]
    fn popup_names_are_ascii_alphanumeric_up_to_64() {
        assert!(PopupName::parse("MainMenuPopup").is_some());
        assert!(PopupName::parse(&"a".repeat(64)).is_some());
        for bad in [
            "",
            "Main-Menu",
            "Main Menu",
            "../x",
            "Info\u{e9}",
            &"a".repeat(65),
        ] {
            assert!(PopupName::parse(bad).is_none(), "{bad:?}");
        }
    }
}
