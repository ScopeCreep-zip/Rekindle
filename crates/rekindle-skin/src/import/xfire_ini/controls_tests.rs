//! The INI importer on a synthetic tree in Alloy's and Simple X's shape
//! (no third-party art): the frame's sides on `[Middle]`, title buttons
//! and a `Class=Scrollbar` section.

use std::fmt::Write as _;

use rekindle_image::{encode_png, DynamicImage, Rgba, RgbaImage};

use super::*;
use crate::validate;

const FOLDER: &str = "Fan";

fn png(side: u32) -> Vec<u8> {
    let img = RgbaImage::from_pixel(side, side, Rgba([40, 100, 160, 255]));
    encode_png(&DynamicImage::ImageRgba8(img)).unwrap()
}

const THEME: &str = "[default]\r\n\
ColorGroup.BuddyViewBG=RGBA(16,16,16,0)\r\n\
ColorGroup.BuddyViewFriendText=RGBA(183,184,184,0)\r\n\
ColorGroup.BorderCG=RGBA(10,0,0,0)\r\n\
Graphics.BTNCLOSE=buttons\\close.png\r\n";

fn skin_ini() -> String {
    let mut s = String::from("[MainWindow]\r\nClass=MainWindow\r\nColorGroupTheme=FanThemes\r\n");
    for (name, keys) in [
        ("Top", &["topleft", "top", "topright"][..]),
        ("Middle", &["left", "middle", "right"][..]),
        ("Bottom", &["bottomleft", "bottom", "bottomright"][..]),
    ] {
        let _ = write!(
            s,
            "[{name}]\r\nClass=Tile\r\nGraphics.ColorGroup=BorderCG\r\n"
        );
        for k in keys {
            let _ = write!(s, "Graphics.{k}=main\\{k}.png\r\n");
        }
    }
    s.push_str(
        "[CloseBox]\r\nClass=PushButton\r\nGraphics.middle=BTNCLOSE\r\n\
         Hover.middle=buttons\\close-hover.png\r\nBtnDown.middle=buttons\\close-big.png\r\n\
         Action.click=AppClose\r\n\
         [Logo]\r\nClass=PushButton\r\nGraphics.middle=buttons\\logo.png\r\n\
         [DefaultScrollbar]\r\nClass=Scrollbar\r\nGraphics.ColorGroup=BorderCG\r\n\
         Graphics.ScrollUp=scroll\\up.png\r\nDisabled.ScrollUp=scroll\\up-dis.png\r\n\
         Graphics.ScrollVertGripperOverlay=scroll\\grips.png\r\n",
    );
    s
}

fn tree() -> Tree {
    let mut t = Tree::new();
    t.insert(
        "version.ini".into(),
        b"[Version]\r\nVersion=1\r\nLongName=Fan\r\n; Fan skin by A. Fan\r\n".to_vec(),
    );
    t.insert(format!("{FOLDER}.ini"), skin_ini().into_bytes());
    t.insert("FanThemes.ini".into(), THEME.as_bytes().to_vec());
    for k in ["topleft", "top", "topright", "left", "right"]
        .into_iter()
        .chain(["bottomleft", "bottom", "bottomright"])
    {
        t.insert(format!("main/{k}.png"), png(4));
    }
    t.insert("main/middle.png".into(), png(9));
    for f in ["buttons/close", "buttons/close-hover", "buttons/logo"]
        .into_iter()
        .chain(["scroll/up", "scroll/up-dis", "scroll/grips"])
    {
        t.insert(format!("{f}.png"), png(10));
    }
    t.insert("buttons/close-big.png".into(), png(12));
    t
}

#[test]
fn middle_sides_title_buttons_and_the_scrollbar_import() {
    let t = tree();
    let r = read(&t, FOLDER).unwrap();
    let f = r.frame.as_ref().unwrap();
    assert_eq!(*f.shape, MIDDLE_FRAME);
    // `[Middle]`'s 9 px `middle` is the content background, not a piece:
    // the centre is the 1 px transparent cell, so the frame is 4 + 4 + 4.
    assert_eq!(f.images[&RegionState::Normal].dimensions(), (12, 12));
    let regions: Vec<&str> = r.controls.iter().map(|c| c.region).collect();
    assert_eq!(regions, ["titlebar.btn.close", "scrollbar.button-up"]);
    let close = &r.controls[0];
    assert!(close.images.contains_key(&RegionState::Hover));
    assert!(!close.images.contains_key(&RegionState::Down));
    assert!(r.controls[1].images.contains_key(&RegionState::Disabled));
    assert!(!r.unread.iter().any(|f| f == "buttons/close-hover.png"));
    assert!(r.unread.iter().any(|f| f == "buttons/logo.png"));
    let a = super::tests::settled(import(&t, FOLDER, "fan", None).unwrap());
    assert_eq!(
        validate::run(&a.canonical.bytes).unwrap().bytes,
        a.canonical.bytes
    );
    // The 10 x 10 button art is centred on the 24 x 24 button box, so it
    // draws at 1:1 (plan E7.7.7, xSLIMFIRE's LED smeared by `stretch`).
    assert!(a
        .notice
        .contains("titlebar.btn.close: [CloseBox], 24x24 px"));
    assert!(a
        .notice
        .contains("dropped [CloseBox (titlebar.btn.close)] Down: its size differs"));
    // The logo button, the grip overlay and `[Middle]`'s middle have no slot.
    assert!(a.notice.contains("images with no region slot: 3"));
}

/// Plan E7.7.7 S2: Combat's shape. The main skin's status arrow, list
/// header, name bar and add button, and each chat window file's
/// `[GCMenuBar]` into the slot of the window that file is for.
#[test]
fn combat_surfaces_import_from_the_skin_and_its_chat_files() {
    let mut t = tree();
    let mut skin = skin_ini().replace(
        "ColorGroupTheme=FanThemes\r\n",
        "ColorGroupTheme=FanThemes\r\nChatSkin=FanChat\r\nGroupChatSkin=FanGroup\r\n",
    );
    skin.push_str(
        "[DownArrow]\r\nClass=Menu\r\nGraphics.middle=s\\arrow.png\r\nHover.middle=s\\arrow-h.png\r\n\
         [BuddyView]\r\nClass=BuddyListView\r\nGraphics.HeaderLeft=s\\hl.png\r\n\
         Graphics.HeaderMiddle=s\\hm.png\r\nGraphics.HeaderRight=s\\hr.png\r\n\
         [CombatDragBar]\r\nClass=Tile\r\nGraphics.left=s\\nl.png\r\nGraphics.middle=s\\nm.png\r\n\
         Graphics.right=s\\nr.png\r\nDisabled.left=s\\nl.png\r\nDisabled.middle=s\\nm.png\r\n\
         Disabled.right=s\\nr.png\r\n\
         [AddFriendButton]\r\nClass=PushButton\r\nGraphics.middle=s\\add.png\r\n\
         [GCMenuBar]\r\nClass=Tile\r\nGraphics.left=s\\ml.png\r\nGraphics.middle=s\\mm.png\r\n\
         Graphics.right=s\\mr.png\r\n",
    );
    t.insert(format!("{FOLDER}.ini"), skin.into_bytes());
    let bar = |p: &str| {
        format!(
            "[MainWindow]\r\nClass=MainWindow\r\n[GCMenuBar]\r\nClass=Tile\r\n\
             Graphics.left=s\\{p}l.png\r\nGraphics.middle=s\\{p}m.png\r\nGraphics.right=s\\{p}r.png\r\n"
        )
    };
    // Plan E7.7.7 S3: the chat window's `[ImButton]` is its Send button.
    let chat = bar("c")
        + "[ImButton]\r\nClass=PushButton\r\nGraphics.middle=s\\send.png\r\n\
           Hover.middle=s\\send.png\r\nBtnDown.middle=s\\send.png\r\n";
    t.insert("FanChat.ini".into(), chat.into_bytes());
    t.insert("FanGroup.ini".into(), bar("g").into_bytes());
    for f in [
        "arrow", "arrow-h", "hl", "hm", "hr", "nl", "nm", "nr", "add",
    ]
    .into_iter()
    .chain(["cl", "cm", "cr", "gl", "gm", "gr", "ml", "mm", "mr", "send"])
    {
        t.insert(format!("s/{f}.png"), png(10));
    }
    let r = read(&t, FOLDER).unwrap();
    let region = |id: &str| r.controls.iter().find(|c| c.region == id);
    let arrow = region("statusbar.arrow").unwrap();
    assert!(arrow.images.contains_key(&RegionState::Hover));
    assert!(region("buddylist.group-header").is_some());
    assert!(region("buddylist.action-button").is_some());
    // The name bar's Disabled set has no state on the info bar: dropped.
    let info = region("buddylist.infobar").unwrap();
    assert_eq!(info.images.len(), 1);
    // The main file's own `[GCMenuBar]` fills nothing: each chat window's
    // bar is read in its own file.
    assert_eq!(r.tally.sources["chat.toolbar"], "GCMenuBar");
    assert!(r.unread.iter().any(|f| f == "s/ml.png"));
    assert!(!r.unread.iter().any(|f| f == "s/cl.png" || f == "s/gl.png"));
    assert!(!r
        .unread
        .iter()
        .any(|f| f == "FanChat.ini" || f == "FanGroup.ini"));
    assert!(region("chat.toolbar").is_some() && region("chat.group-header").is_some());
    let send = region("chat.send-button").unwrap();
    assert_eq!(
        send.images.keys().copied().collect::<Vec<_>>(),
        [RegionState::Normal, RegionState::Hover, RegionState::Down]
    );
    let a = super::tests::settled(import(&t, FOLDER, "fan", None).unwrap());
    assert_eq!(
        validate::run(&a.canonical.bytes).unwrap().bytes,
        a.canonical.bytes
    );
    assert!(a.notice.contains(
        "dropped [CombatDragBar (buddylist.infobar)] Disabled: the slot has no such state"
    ));
}
