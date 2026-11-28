//! Plan E7.7.7 S2: Classic v3's buddy-list components and both XML
//! layouts' chat window components land in their surface slots
//! (synthetic trees in Classic v3's and Symbiosis's shapes, no Xfire art).

use std::fmt::Write as _;

use rekindle_types::dto::skin::{Insets, RegionFit, RegionState};

use super::surfaces_tests::png;
use super::*;
use crate::{manifest as parse_manifest, package, validate};

/// The theme's `<Bitmap>`s of `images` (name, w, h), each a file of `t`.
pub(super) fn bitmaps(t: &mut Tree, images: &[(&str, u32, u32)]) -> String {
    let mut out = String::new();
    for (i, (name, w, h)) in images.iter().enumerate() {
        let _ = write!(out, "<Bitmap Name=\"{name}\">img/{name}.png</Bitmap>");
        let shade = 40 + u8::try_from(i * 4).unwrap();
        t.insert(format!("img/{name}.png"), png(*w, *h, shade));
    }
    out
}

/// The manifest of `t` imported twice (deterministic) and validated.
pub(super) fn imported(t: &Tree) -> (SkinManifest, Imported) {
    let a = import(t, "fixture", None).unwrap();
    assert_eq!(a, import(t, "fixture", None).unwrap(), "deterministic");
    assert_eq!(
        validate::run(&a.canonical.bytes).unwrap().bytes,
        a.canonical.bytes
    );
    let pkg = package::read(&a.canonical.bytes).unwrap();
    (parse_manifest::parse(&pkg).unwrap(), a)
}

pub(super) fn states(m: &SkinManifest, region: &str) -> Vec<RegionState> {
    m.regions[region].states.keys().copied().collect()
}

/// Classic v3's shape: lower-case files, every component in
/// `xfirecomponents.xml`, `left`/`middle`/`right` bars.
fn v3_tree() -> Tree {
    let mut t = Tree::new();
    t.insert(
        "skin.xml".into(),
        br#"<XfireSkin><Include File="xfirethemes.xml"/><Include File="xfirecomponents.xml"/></XfireSkin>"#.to_vec(),
    );
    let b = bitmaps(
        &mut t,
        &[
            ("SBL", 3, 26),
            ("SBM", 2, 26),
            ("SBR", 5, 26),
            ("TL", 3, 16),
            ("TM", 4, 16),
            ("TR", 6, 16),
            ("TLD", 3, 16),
            ("TMD", 4, 16),
            ("TRD", 6, 16),
            ("VN", 35, 21),
            ("VH", 35, 21),
            ("LN", 35, 21),
            ("HL", 1, 16),
            ("HM", 2, 16),
            ("HR", 1, 16),
        ],
    );
    t.insert(
        "xfirethemes.xml".into(),
        format!("<Themes><Theme Name=\"default\"><Colors><Color Name=\"BuddyViewBG\" red=\"8\" green=\"16\" blue=\"32\" alpha=\"0\"/></Colors><Bitmaps>{b}</Bitmaps></Theme></Themes>").into_bytes(),
    );
    t.insert(
        "xfirecomponents.xml".into(),
        br#"<Components>
<Component name="StatusWrapper" Class="Tile"><Images State="Normal"><Image type="left">SBL</Image><Image type="middle">SBM</Image><Image type="right">SBR</Image></Images></Component>
<Component name="TabNormalButton" Class="PushButton">
<Images State="Normal"><Image type="left">TL</Image><Image type="middle">TM</Image><Image type="right">TR</Image></Images>
<Images State="Down"><Image type="left">TLD</Image><Image type="middle">TMD</Image><Image type="right">TRD</Image></Images></Component>
<Component name="InfoHostChatButton" Class="PushButton"><Images State="Normal"><Image type="middle">VN</Image></Images><Images State="Hover"><Image type="middle">VH</Image></Images></Component>
<Component name="InfoLeaveChatButton" Class="PushButton"><Images State="Normal"><Image type="middle">LN</Image></Images></Component>
<Component name="GroupChatInfoView" Class="GroupChatUserList"><Images State="Normal"><Image type="HeaderLeft">HL</Image><Image type="HeaderMiddle">HM</Image><Image type="HeaderRight">HR</Image></Images></Component>
</Components>"#.to_vec(),
    );
    t
}

#[test]
fn classic_v3_fills_the_buddy_list_and_chat_surfaces() {
    let (m, a) = imported(&v3_tree());
    let bar = &m.regions["statusbar"];
    assert_eq!(
        bar.fit,
        RegionFit::NineSlice(Insets {
            top: 0,
            right: 5,
            bottom: 0,
            left: 3
        })
    );
    // A three-piece sticky tab: Down is also the chosen tab.
    assert_eq!(
        states(&m, "buddylist.tab"),
        [RegionState::Down, RegionState::Selected]
    );
    assert_eq!(states(&m, "chat.call-button"), [RegionState::Hover]);
    assert!(states(&m, "chat.call-button-active").is_empty());
    assert_eq!(
        m.regions["chat.call-button"].text_backing.as_deref(),
        Some("bg.panel")
    );
    assert_eq!(
        m.regions["chat.group-header"].text_backing.as_deref(),
        Some("bg.dark")
    );
    assert!(a
        .notice
        .contains("chat.group-header: the component GroupChatInfoView"));
}

/// Symbiosis's chat windows: `Components/ChatWindow.xml` and
/// `Components/GroupChatWindow.xml`.
fn chat_tree() -> Tree {
    let mut t = Tree::new();
    t.insert(
        "Skin.xml".into(),
        br#"<XfireSkin><Include File="Themes.xml"/><Include File="Components/ChatWindow.xml"/><Include File="Components/GroupChatWindow.xml"/></XfireSkin>"#.to_vec(),
    );
    let b = bitmaps(
        &mut t,
        &[
            ("edit", 30, 30),
            ("bar_l", 8, 24),
            ("bar_m", 1, 24),
            ("bar_r", 8, 24),
            ("call_n", 40, 24),
            ("call_h", 40, 24),
            ("call_d", 40, 24),
            ("leave_n", 40, 24),
            ("hl", 2, 19),
            ("hm", 2, 19),
            ("hr", 2, 19),
        ],
    );
    t.insert(
        "Themes.xml".into(),
        format!("<Themes><Theme Name=\"default\"><Colors><Color Name=\"ListBGCG\">RGBA(22,22,22,0)</Color></Colors><Bitmaps>{b}</Bitmaps></Theme></Themes>").into_bytes(),
    );
    t.insert(
        "Components/ChatWindow.xml".into(),
        br#"<Components>
<Component Name="ChatEditContainer" Class="Tile"><Images State="Normal"><Image Type="Middle">edit</Image></Images></Component>
<Component Name="ChatButtonBar" Class="Tile"><Images State="Normal"><Image Type="TopLeft">bar_l</Image><Image Type="Top">bar_m</Image><Image Type="TopRight">bar_r</Image></Images></Component>
<Component Name="ChatButtonCall" Class="PushButton">
<Images State="Normal"><Image Type="Middle">call_n</Image></Images>
<Images State="Hover"><Image Type="Middle">call_h</Image></Images>
<Images State="Down"><Image Type="Middle">call_d</Image></Images></Component>
<Component Name="ChatButtonLeaveCall" Class="PushButton"><Images State="Normal"><Image Type="Middle">leave_n</Image></Images></Component>
</Components>"#.to_vec(),
    );
    t.insert(
        "Components/GroupChatWindow.xml".into(),
        br#"<Components>
<Component Name="GroupChatUsersList" Class="GroupChatUserList"><Images State="Normal"><Image Type="HeaderLeft">hl</Image><Image Type="HeaderMiddle">hm</Image><Image Type="HeaderRight">hr</Image></Images></Component>
</Components>"#.to_vec(),
    );
    t
}

#[test]
fn symbiosis_chat_components_land_in_the_chat_slots() {
    let (m, a) = imported(&chat_tree());
    assert_eq!(m.regions["chat.input"].fit, RegionFit::Stretch);
    assert_eq!(
        m.regions["chat.input"].text_backing.as_deref(),
        Some("bg.dark")
    );
    assert_eq!(
        m.regions["chat.toolbar"].fit,
        RegionFit::NineSlice(Insets {
            top: 0,
            right: 8,
            bottom: 0,
            left: 8
        })
    );
    assert_eq!(
        m.regions["chat.toolbar"].text_backing.as_deref(),
        Some("bg.panel")
    );
    assert_eq!(
        states(&m, "chat.call-button"),
        [RegionState::Hover, RegionState::Down]
    );
    assert!(m.regions.contains_key("chat.call-button-active"));
    assert!(m.regions.contains_key("chat.group-header"));
    assert!(a
        .notice
        .contains("chat.call-button: the component ChatButtonCall"));
}
