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
    assert_eq!(popup_parts(&popup), Some((BUDDY_LIST, "MainInfoViewPopup")));
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
