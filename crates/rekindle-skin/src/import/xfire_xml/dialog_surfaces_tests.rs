//! Plan E7.7.7 S3: the Send button, the titlebar strip, the dialog buttons
//! and Classic v3's wider chosen tab land in their slots (synthetic trees
//! in Symbiosis's and Classic v3's shapes, no Xfire art).

use rekindle_types::dto::skin::{Insets, RegionFit, RegionState};

use super::chat_surfaces_tests::{bitmaps, imported, states};
use super::*;

/// Symbiosis's shape: `ChatButtonSend` in `Components/ChatWindow.xml`, and
/// the dialog buttons only as theme bitmaps (`Themes.xml:386-394`).
fn symbiosis_tree() -> Tree {
    let mut t = Tree::new();
    t.insert(
        "Skin.xml".into(),
        br#"<XfireSkin><Include File="Themes.xml"/><Include File="Components/ChatWindow.xml"/></XfireSkin>"#.to_vec(),
    );
    let b = bitmaps(
        &mut t,
        &[
            ("send_n", 51, 50),
            ("send_h", 51, 50),
            ("send_d", 51, 50),
            ("send_x", 51, 50),
            ("button_ok_norm", 60, 22),
            ("button_ok_hover", 60, 22),
            ("button_ok_down", 60, 22),
            ("button_ok_disabled", 60, 22),
            ("button_cancel_norm", 60, 22),
            ("button_cancel_down", 60, 22),
        ],
    );
    t.insert(
        "Themes.xml".into(),
        format!("<Themes><Theme Name=\"default\"><Colors><Color Name=\"ListBGCG\">RGBA(22,22,22,0)</Color></Colors><Bitmaps>{b}</Bitmaps></Theme></Themes>").into_bytes(),
    );
    t.insert(
        "Components/ChatWindow.xml".into(),
        br#"<Components>
<Component Name="ChatButtonSend" Class="PushButton">
<Images State="Normal"><Image Type="Middle">send_n</Image></Images>
<Images State="Hover"><Image Type="Middle">send_h</Image></Images>
<Images State="Down"><Image Type="Middle">send_d</Image></Images>
<Images State="Disabled"><Image Type="Middle">send_x</Image></Images></Component>
</Components>"#
            .to_vec(),
    );
    t
}

#[test]
fn symbiosis_send_and_dialog_buttons_land_in_their_slots() {
    let (m, a) = imported(&symbiosis_tree());
    let send = &m.regions["chat.send-button"];
    assert_eq!(
        states(&m, "chat.send-button"),
        [RegionState::Hover, RegionState::Down, RegionState::Disabled]
    );
    assert_eq!(send.text_backing.as_deref(), Some("bg.dark"));
    assert!(matches!(send.fit, RegionFit::NineSlice(_)));
    assert_eq!(
        states(&m, "button.primary"),
        [RegionState::Hover, RegionState::Down, RegionState::Disabled]
    );
    assert_eq!(
        m.regions["button.primary"].text_backing.as_deref(),
        Some("accent.fill")
    );
    assert_eq!(states(&m, "button.secondary"), [RegionState::Down]);
    assert!(a
        .notice
        .contains("chat.send-button: the component ChatButtonSend"));
}

/// Classic v3's shape: `MainTitleBar` (Normal, and Disabled while the
/// window is inactive), `SendButton` whose Hover is another size, and the
/// sticky tab whose Down left end is 4 px to Normal's 3 (`tab_control_left`
/// 4 × 16, `tab_control_left_off` 3 × 16).
fn v3_tree() -> Tree {
    let mut t = Tree::new();
    t.insert(
        "skin.xml".into(),
        br#"<XfireSkin><Include File="xfirethemes.xml"/><Include File="xfirecomponents.xml"/></XfireSkin>"#.to_vec(),
    );
    let b = bitmaps(
        &mut t,
        &[
            ("ML", 20, 51),
            ("MM", 4, 51),
            ("MR", 30, 51),
            ("TLO", 3, 16),
            ("TMO", 50, 16),
            ("TRO", 18, 16),
            ("TL", 4, 16),
            ("TM", 50, 16),
            ("TR", 18, 16),
            ("SN", 46, 21),
            ("SH", 48, 22),
        ],
    );
    t.insert(
        "xfirethemes.xml".into(),
        format!("<Themes><Theme Name=\"default\"><Colors><Color Name=\"BuddyViewBG\" red=\"8\" green=\"16\" blue=\"32\" alpha=\"0\"/></Colors><Bitmaps>{b}</Bitmaps></Theme></Themes>").into_bytes(),
    );
    t.insert(
        "xfirecomponents.xml".into(),
        br#"<Components>
<Component Name="MainTitleBar" Class="Tile">
<Images state="Normal"><Image Type="topleft">ML</Image><Image Type="top">MM</Image><Image Type="topright">MR</Image></Images>
<Images state="Disabled"><Image Type="topleft">ML</Image><Image Type="top">MM</Image><Image Type="topright">MR</Image></Images></Component>
<Component name="TabNormalButton" Class="PushButton">
<Images State="Normal"><Image type="left">TLO</Image><Image type="middle">TMO</Image><Image type="right">TRO</Image></Images>
<Images State="Down"><Image type="left">TL</Image><Image type="middle">TM</Image><Image type="right">TR</Image></Images></Component>
<Component Name="SendButton" Class="PushButton">
<Images State="Normal"><Image Type="middle">SN</Image></Images>
<Images State="Hover"><Image Type="middle">SH</Image></Images></Component>
</Components>"#.to_vec(),
    );
    t
}

#[test]
fn classic_v3_fills_the_titlebar_send_and_its_wider_chosen_tab() {
    let (m, _) = imported(&v3_tree());
    let title = &m.regions["titlebar"];
    assert_eq!(
        title.fit,
        RegionFit::NineSlice(Insets {
            top: 0,
            right: 30,
            bottom: 0,
            left: 20
        })
    );
    assert_eq!(states(&m, "titlebar"), [RegionState::WindowInactive]);
    assert_eq!(title.text_backing.as_deref(), Some("titlebar.start"));
    // The chosen tab keeps its wider Down image; the insets are the widest
    // end over both states.
    assert_eq!(
        states(&m, "buddylist.tab"),
        [RegionState::Down, RegionState::Selected]
    );
    assert_eq!(
        m.regions["buddylist.tab"].fit,
        RegionFit::NineSlice(Insets {
            top: 0,
            right: 18,
            bottom: 0,
            left: 4
        })
    );
    assert_eq!(states(&m, "chat.send-button"), [RegionState::Hover]);
}
