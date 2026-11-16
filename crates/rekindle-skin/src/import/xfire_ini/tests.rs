//! The importer on a small synthetic INI-era tree (DoiDD's shape; no
//! third-party art).

use std::fmt::Write as _;

use rekindle_image::{encode_png, DynamicImage, Rgba, RgbaImage};

use super::*;
use crate::import::Imported;
use crate::validate;

const MAGENTA: Rgba<u8> = Rgba([0xff, 0, 0xff, 0xff]);
const PIXEL: Rgba<u8> = Rgba([40, 100, 160, 255]);

fn png() -> Vec<u8> {
    let mut img = RgbaImage::from_pixel(3, 3, PIXEL);
    img.put_pixel(0, 0, MAGENTA);
    encode_png(&DynamicImage::ImageRgba8(img)).unwrap()
}

/// The frame sections with `Graphics`, `Hover` and `BtnDown` pieces (the
/// top middle by a theme alias), `Disabled` only on `[Top]`.
fn skin_ini() -> String {
    let mut s = String::from(
        "; synthetic\r\n[MainWindow]\r\nClass=MainWindow\r\nChatSkin=Fix chatskin\r\n\
         ColorGroupTheme=FixThemes\r\nWindow1.Name=Top\r\nWindow1.Z=1\r\nWindow1.Z=2\r\n",
    );
    let sections: [(&str, &[&str]); 4] = [
        ("Top", &["topleft", "top", "topright"]),
        ("Left", &["middle"]),
        ("Right", &["middle"]),
        ("Bottom", &["bottomleft", "bottom", "bottomright"]),
    ];
    for (name, keys) in sections {
        let _ = write!(
            s,
            "[{name}]\r\nClass=Tile\r\nAction.lbtndown=AppDragWindow\r\n"
        );
        let _ = write!(
            s,
            "Hover.ColorGroup=MenuHoverCG\r\nBtnDown.ColorGroup=MenuDownCG\r\n"
        );
        for k in keys {
            for state in ["Graphics", "Hover", "BtnDown"] {
                let v = if *k == "top" {
                    "TOPMID".to_owned()
                } else {
                    format!("main\\{k}.png")
                };
                let _ = write!(s, "{state}.{k}={v}\r\n");
            }
            if name == "Top" {
                let _ = write!(s, "Disabled.{k}=main\\{k}.png\r\n");
            }
        }
    }
    s
}

const THEME: &str = "[default]\r\n;Sounds.OnlineSound=x.wav\r\n\
ColorGroup.BuddyViewBG=RGBA(22,22,22,0)\r\n\
ColorGroup.BuddyViewFriendText=RGBA(183,184,184,0)\r\n\
ColorGroup.MenuHoverCG=RGBA(30,30,30,0)\r\n\
ColorGroup.MenuDownCG=RGBA(-30,-30,-30,0)\r\n\
ColorGroup.VentCG=RGBA(0,0,0,0)\r\n\
Graphics.TOPMID=main\\top.png\r\n";

fn tree() -> Tree {
    let mut t = Tree::new();
    t.insert(
        "version.ini".into(),
        b"[Version]\r\nVersion=1\r\nLongName=Fixture\r\n".to_vec(),
    );
    t.insert("Fixture.ini".into(), skin_ini().into_bytes());
    t.insert("FixThemes.ini".into(), THEME.as_bytes().to_vec());
    for k in [
        "topleft",
        "top",
        "topright",
        "middle",
        "bottomleft",
        "bottom",
        "bottomright",
    ] {
        t.insert(format!("main/{k}.png"), png());
    }
    // Never read: a malformed chat skin and junk.
    t.insert("Fix chatskin.ini".into(), b"not an ini".to_vec());
    t.insert("Thumbs.db".into(), vec![0; 16]);
    t
}

fn passed(o: Outcome) -> Imported {
    match o {
        Outcome::Passed(i) => i,
        Outcome::Draft(d) => panic!("a draft: {:?}", d.failures),
    }
}

#[test]
fn imports_validates_and_is_byte_identical() {
    let t = tree();
    let a = passed(import(&t, "fixture").unwrap());
    let b = passed(import(&t, "fixture").unwrap());
    assert_eq!(a, b);
    let again = validate::run(&a.canonical.bytes).unwrap();
    assert_eq!(again.bytes, a.canonical.bytes);
    assert!(a.notice.contains("VentCG RGBA(0,0,0,0) -> dropped"));
    assert!(a
        .notice
        .contains("BuddyViewBG RGBA(22,22,22,0) -> token bg.primary"));
    assert!(a.notice.contains("triggers: 4"));
    assert!(a.notice.contains("    Fix chatskin.ini\n"));
    assert!(a.notice.contains("    Thumbs.db\n"));
    assert!(a
        .notice
        .contains("    Fixture.ini: [MainWindow] Window1.Z (line 8)\n"));
    assert!(!a.notice.contains("    main/top.png\n"));
    assert!(a
        .notice
        .contains("WindowInactive not emitted: missing [Left] Disabled.middle"));
}

#[test]
fn a_failing_pair_is_a_draft_naming_it() {
    let mut t = tree();
    let theme = format!("{THEME}ColorGroup.BuddyViewOfflineText=RGBA(12,51,87,0)\r\n");
    t.insert("FixThemes.ini".into(), theme.into_bytes());
    let Outcome::Draft(d) = import(&t, "fixture").unwrap() else {
        panic!("expected a draft");
    };
    let c = d.failures.first().unwrap();
    assert_eq!(
        (c.token.as_str(), c.original),
        ("status.offline-text", [12, 51, 87])
    );
    assert!(validate::run(&d.package).is_err());
    validate::run(&d.suggested.unwrap().canonical.bytes).unwrap();
}

#[test]
fn non_zero_alpha_is_refused() {
    let mut t = tree();
    t.insert(
        "FixThemes.ini".into(),
        THEME
            .replace("RGBA(22,22,22,0)", "RGBA(22,22,22,9)")
            .into_bytes(),
    );
    assert!(matches!(
        import(&t, "fixture"),
        Err(ImportError::NonZeroAlpha { alpha: 9, .. })
    ));
}

#[test]
fn hover_and_pressed_shifts_and_colour_key_are_baked() {
    let f = read(&tree()).unwrap().frame.unwrap();
    assert_eq!((f.insets.top, f.insets.left), (3, 3));
    let at = |s: RegionState, x, y| f.images[&s].get_pixel(x, y).0;
    // The top middle piece (the alias) at its (1, 1).
    assert_eq!(at(RegionState::Normal, 4, 1), [40, 100, 160, 255]);
    assert_eq!(at(RegionState::Hover, 4, 1), [70, 130, 190, 255]);
    assert_eq!(at(RegionState::Down, 4, 1), [10, 70, 130, 255]);
    // Each piece's #FF00FF pixel is keyed out; the middle is transparent.
    for s in [RegionState::Normal, RegionState::Hover, RegionState::Down] {
        assert_eq!(at(s, 0, 0), [0, 0, 0, 0]);
        assert_eq!(at(s, 3, 0), [0, 0, 0, 0]);
        assert_eq!(at(s, 4, 4), [0, 0, 0, 0]);
    }
    assert!(!f.images.contains_key(&RegionState::WindowInactive));
}

#[test]
fn oversize_malformed_and_escaping_inis_are_refused() {
    let refused = |name: &str, bytes: Vec<u8>| {
        let mut t = tree();
        t.insert(name.into(), bytes);
        import(&t, "fixture").is_err()
    };
    let mut big = skin_ini();
    big.push_str(&";\r\n".repeat(ini::MAX_INI_BYTES / 3));
    assert!(refused("Fixture.ini", big.into_bytes()));
    assert!(refused(
        "Fixture.ini",
        format!("{}garbage line\r\n", skin_ini()).into_bytes()
    ));
    assert!(refused(
        "FixThemes.ini",
        format!("{THEME}Graphics.X=a\\\r\n").into_bytes()
    ));
    assert!(refused(
        "version.ini",
        b"[Version]\r\nVersion=2\r\nLongName=Fixture\r\n".to_vec()
    ));
    assert!(refused(
        "version.ini",
        b"[Version]\r\nVersion=1\r\nLongName=..\r\n".to_vec()
    ));
    let escape = skin_ini().replace("main\\topleft.png", "main\\..\\topleft.png");
    assert!(matches!(
        {
            let mut t = tree();
            t.insert("Fixture.ini".into(), escape.into_bytes());
            import(&t, "x")
        },
        Err(ImportError::Format { .. })
    ));
}
