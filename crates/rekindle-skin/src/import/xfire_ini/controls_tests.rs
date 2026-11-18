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
ColorGroup.BuddyViewBG=RGBA(22,22,22,0)\r\n\
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
    let Outcome::Passed(a) = import(&t, FOLDER, "fan", None).unwrap() else {
        panic!("a draft");
    };
    assert_eq!(
        validate::run(&a.canonical.bytes).unwrap().bytes,
        a.canonical.bytes
    );
    assert!(a
        .notice
        .contains("titlebar.btn.close: [CloseBox], 10x10 px"));
    assert!(a
        .notice
        .contains("dropped [CloseBox (titlebar.btn.close)] Down: its size differs"));
    // The logo button, the grip overlay and `[Middle]`'s middle have no slot.
    assert!(a.notice.contains("images with no region slot: 3"));
}
