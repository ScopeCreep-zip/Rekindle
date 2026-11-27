//! The importer on a small synthetic INI-era tree (DoiDD's shape; no
//! third-party art).

use std::fmt::Write as _;

use rekindle_image::{encode_png, DynamicImage, Rgba, RgbaImage};

use super::*;
use crate::import::{walk, Imported};
use crate::validate;

/// The skin's folder: it names `Fixture.ini`; `LongName` differs.
const FOLDER: &str = "Fixture";
const MAGENTA: Rgba<u8> = Rgba([0xff, 0, 0xff, 0xff]);
const PIXEL: Rgba<u8> = Rgba([40, 100, 160, 255]);

/// A 3×3 frame piece; `keyed` puts one #FF00FF pixel at its top left. Only
/// corners are keyed: a keyed pixel in a repeating side
/// would cut the window every 3 px, which no pack's art does (SK0 S1: 1440
/// cuts per side at the cap's 4320 px exceed the 4096-rectangle cap).
fn png(keyed: bool) -> Vec<u8> {
    let mut img = RgbaImage::from_pixel(3, 3, PIXEL);
    if keyed {
        img.put_pixel(0, 0, MAGENTA);
    }
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
ColorGroup.BuddyViewBG=RGBA(16,16,16,0)\r\n\
ColorGroup.BuddyViewFriendText=RGBA(183,184,184,0)\r\n\
ColorGroup.MenuHoverCG=RGBA(30,30,30,0)\r\n\
ColorGroup.MenuDownCG=RGBA(-30,-30,-30,0)\r\n\
ColorGroup.VentCG=RGBA(0,0,0,0)\r\n\
Graphics.TOPMID=main\\top.png\r\n";

fn tree() -> Tree {
    let mut t = Tree::new();
    t.insert(
        "version.ini".into(),
        b"[Version]\r\nVersion=1\r\nLongName=The Fixture\r\n".to_vec(),
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
        // `middle` is the `[Left]`/`[Right]` rail piece.
        let corner = k.ends_with("left") || k.ends_with("right");
        t.insert(format!("main/{k}.png"), png(corner));
    }
    // Never read: a malformed chat skin and junk.
    // The chat skin file is read for its surfaces (plan E7.7.7 S2); this
    // one has none, so it stays named among the files not read.
    t.insert(
        "Fix chatskin.ini".into(),
        b"[MainWindow]\r\nClass=MainWindow\r\n".to_vec(),
    );
    t.insert("Thumbs.db".into(), vec![0; 16]);
    t
}

/// The fixture's list ground, #101010, is also every window's ground
/// (`bg.dark` follows `bg.primary`, plan E7.7.7 e777n). White on the online
/// chip (`status.online` 60% over it) measures 4.44:1 there, under D4's
/// 4.5:1, so the walk settles it by darkening the skin's `bg.primary` to
/// #0d0d0d. A settled change is a draft carrying the suggestion for the
/// author to confirm (`import::conclude`), never a silent pass: this
/// returns that suggestion after checking the repair and that the pack as
/// written fails.
pub(super) fn settled(o: Outcome) -> Imported {
    let Outcome::Draft(d) = o else {
        panic!("#101010 under the online chip passed as written");
    };
    assert!(d.unsettled.is_none(), "{:?}", d.unsettled);
    let c = d.failures.first().unwrap();
    assert_eq!(
        (
            c.token.as_str(),
            c.rule.as_str(),
            c.fg.as_str(),
            c.ground.as_str()
        ),
        (
            "bg.primary",
            "D4",
            "text.on-fill",
            "status.online 60.0% over bg.dark"
        )
    );
    assert_eq!(
        (c.original, c.new, c.before_x100, c.required_x100),
        ([16, 16, 16], [13, 13, 13], 444, 450)
    );
    assert!(validate::run(&d.package).is_err());
    d.suggested.unwrap()
}

#[test]
fn imports_validates_and_is_byte_identical() {
    let t = tree();
    let a = settled(import(&t, FOLDER, "fixture", None).unwrap());
    let b = settled(import(&t, FOLDER, "fixture", None).unwrap());
    assert_eq!(a, b);
    let again = validate::run(&a.canonical.bytes).unwrap();
    assert_eq!(again.bytes, a.canonical.bytes);
    assert!(a.notice.contains("VentCG RGBA(0,0,0,0) -> dropped"));
    assert!(a
        .notice
        .contains("BuddyViewBG RGBA(16,16,16,0) -> token bg.primary"));
    // The triggers are read into the tree now (plan SK2-A2): no NOTICE count.
    assert!(!a.notice.contains("triggers:"));
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
    let Outcome::Draft(d) = import(&t, FOLDER, "fixture", None).unwrap() else {
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
            .replace("RGBA(16,16,16,0)", "RGBA(16,16,16,9)")
            .into_bytes(),
    );
    assert!(matches!(
        import(&t, FOLDER, "fixture", None),
        Err(ImportError::NonZeroAlpha { alpha: 9, .. })
    ));
}

#[test]
fn hover_and_pressed_shifts_and_colour_key_are_baked() {
    let f = read(&tree(), FOLDER).unwrap().frame.unwrap();
    assert_eq!((f.insets.top, f.insets.left), (3, 3));
    let at = |s: RegionState, x, y| f.images[&s].get_pixel(x, y).0;
    // The top middle piece (the alias) at its (1, 1).
    assert_eq!(at(RegionState::Normal, 4, 1), [40, 100, 160, 255]);
    assert_eq!(at(RegionState::Hover, 4, 1), [70, 130, 190, 255]);
    assert_eq!(at(RegionState::Down, 4, 1), [10, 70, 130, 255]);
    // The corners' #FF00FF pixel is keyed out (the top right's at (6, 0));
    // the top side's (3, 0) is kept; the middle is transparent.
    for s in [RegionState::Normal, RegionState::Hover, RegionState::Down] {
        assert_eq!(at(s, 0, 0), [0, 0, 0, 0]);
        assert_eq!(at(s, 6, 0), [0, 0, 0, 0]);
        assert_ne!(at(s, 3, 0)[3], 0);
        assert_eq!(at(s, 4, 4), [0, 0, 0, 0]);
    }
    assert!(!f.images.contains_key(&RegionState::WindowInactive));
}

#[test]
fn oversize_malformed_and_escaping_inis_are_refused() {
    let refused = |name: &str, bytes: Vec<u8>| {
        let mut t = tree();
        t.insert(name.into(), bytes);
        import(&t, FOLDER, "fixture", None).is_err()
    };
    let mut big = skin_ini();
    big.push_str(&";\r\n".repeat(ini::MAX_INI_BYTES / 3));
    assert!(refused("Fixture.ini", big.into_bytes()));
    assert!(refused(
        "Fixture.ini",
        format!("{}garbage]=line\r\n", skin_ini()).into_bytes()
    ));
    assert!(refused(
        "FixThemes.ini",
        format!("{THEME}Graphics.X=a\\\r\n").into_bytes()
    ));
    for folder in ["..", "a/b", "a\\b", ""] {
        assert!(import(&tree(), folder, "x", None).is_err(), "{folder}");
    }
    let escape = skin_ini().replace("main\\topleft.png", "main\\..\\topleft.png");
    assert!(matches!(
        {
            let mut t = tree();
            t.insert("Fixture.ini".into(), escape.into_bytes());
            import(&t, FOLDER, "x", None)
        },
        Err(ImportError::Format { .. })
    ));
}

/// `Version` is the skin's own revision, never a format (plan E7.7.3a):
/// the research index's 43 INI skins carry `1` (37), `2` (3, Xfire's
/// Combat among them), `01.11.2005`, `1.54` and `RC 2`, all one layout.
#[test]
fn a_skin_revision_imports_and_is_recorded() {
    for revision in ["2", "RC 2", "01.11.2005"] {
        let mut t = tree();
        t.insert(
            "version.ini".into(),
            format!("[Version]\r\nVersion={revision}\r\nLongName=Fixture\r\n").into_bytes(),
        );
        let r = read(&t, FOLDER).unwrap();
        assert_eq!(r.info["Version (the skin's own revision)"], revision);
        let i = settled(import(&t, FOLDER, "fixture", None).unwrap());
        assert!(
            i.notice
                .contains(&format!("Version (the skin's own revision): {revision}")),
            "{}",
            i.notice
        );
    }
    let mut t = tree();
    let long = "9".repeat(MAX_REVISION + 10);
    t.insert(
        "version.ini".into(),
        format!("[Version]\r\nVersion={long}\r\nLongName=Fixture\r\n").into_bytes(),
    );
    assert_eq!(
        read(&t, FOLDER).unwrap().info["Version (the skin's own revision)"].len(),
        MAX_REVISION
    );
}

/// Authorship proven outside the files (plan E7.7.3c) replaces the
/// third-party licence in the manifest and the NOTICE.
#[test]
fn a_credit_reaches_the_manifest_and_the_notice() {
    let page = "https://archive.org/details/XFireSkins";
    let quote = "The Combat skin is an official skin created by Xfire.";
    let credit = crate::import::XfireCredit::new(page, quote).unwrap();
    let i = settled(import(&tree(), FOLDER, "fixture", Some(&credit)).unwrap());
    let m = crate::manifest::parse(&crate::package::read(&i.canonical.bytes).unwrap()).unwrap();
    let rekindle_types::dto::skin::SkinLicense::Text(licence) = m.license else {
        panic!("not a licence text");
    };
    for text in [licence.as_str(), i.notice.as_str()] {
        assert!(text.contains(page) && text.contains(quote), "{text}");
        assert!(
            !text.contains("third-party") && !text.contains("Third-party"),
            "{text}"
        );
    }
    let third = settled(import(&tree(), FOLDER, "fixture", None).unwrap());
    assert!(third.notice.contains("third-party work"));
}

/// Combat's frame shape: one `[OuterFrame]` naming all eight pieces, the
/// sides as `left`/`right` (Xfire's own Combat skin, `combat.ini`).
#[test]
fn an_outer_frame_section_is_the_window_frame() {
    let mut s = String::from(
        "[MainWindow]\r\nClass=MainWindow\r\nColorGroupTheme=FixThemes\r\n[OuterFrame]\r\nClass=Tile\r\n",
    );
    for k in [
        "topleft",
        "top",
        "topright",
        "left",
        "right",
        "bottomleft",
        "bottom",
        "bottomright",
    ] {
        let file = if matches!(k, "left" | "right") {
            "middle"
        } else {
            k
        };
        let _ = write!(s, "Graphics.{k}=main\\{file}.png\r\n");
    }
    let mut t = tree();
    t.insert("Fixture.ini".into(), s.into_bytes());
    let f = read(&t, FOLDER).unwrap().frame.unwrap();
    assert_eq!(*f.shape, OUTER_FRAME);
    assert_eq!((f.insets.top, f.insets.left), (3, 3));
    assert_eq!(f.images[&RegionState::Normal].get_pixel(4, 1).0, PIXEL.0);
    assert_eq!(f.images.len(), 1);
    let i = settled(import(&t, FOLDER, "fixture", None).unwrap());
    assert!(
        i.notice
            .contains("window.frame: [OuterFrame], nine-slice 3/3/3/3"),
        "{}",
        i.notice
    );
}

/// Plan E7.7.3d: an INI skin names no one in its files; an outside credit
/// names Xfire, Inc. and its page, and a root licence file is its licence.
#[test]
fn an_ini_skins_credits_come_from_the_credit_and_its_licence_file() {
    use rekindle_types::dto::skin::OriginalLicence;
    let credits = |t: &Tree, credit| {
        let i = settled(import(t, FOLDER, "fixture", credit).unwrap());
        crate::manifest::parse(&crate::package::read(&i.canonical.bytes).unwrap())
            .unwrap()
            .credits
            .expect("credits")
    };
    let page = "https://archive.org/details/XFireSkins";
    let credit = crate::import::XfireCredit::new(page, "Made by Xfire.").unwrap();
    let c = credits(&tree(), Some(&credit));
    assert_eq!(c.authors, vec!["Xfire, Inc.".to_owned()]);
    assert_eq!(
        (c.url.as_deref(), &c.licence),
        (Some(page), &OriginalLicence::NoneStated)
    );
    let mut t = tree();
    t.insert("License.txt".into(), b"(c) Ann. Do not sell.".to_vec());
    let c = credits(&t, None);
    assert!(c.authors.is_empty() && c.url.is_none());
    assert_eq!(
        c.licence,
        OriginalLicence::Text("(c) Ann. Do not sell.".into())
    );
}

/// The skin file is `<folder>.ini`, `LongName` only the name shown (all 45
/// INI archives hold `<folder>.ini`; 13 lack `<LongName>.ini`, Xfire's
/// `Charger/Charger.ini` with `LongName=Dodge Charger` among them).
#[test]
fn the_folder_names_the_skin_file_and_long_name_is_shown() {
    let i = settled(import(&tree(), FOLDER, "fixture", None).unwrap());
    let m = crate::manifest::parse(&crate::package::read(&i.canonical.bytes).unwrap()).unwrap();
    assert_eq!(m.name, "The Fixture");
    assert!(
        i.notice.contains("  skin file: Fixture.ini\n"),
        "{}",
        i.notice
    );
    // Case aside, as on Windows (`Xfireaqua/Xfireaqua.ini`).
    settled(import(&tree(), "FIXTURE", "fixture", None).unwrap());
    // A file named by `LongName` is not read in the folder's place.
    let mut t = tree();
    let skin = t.remove("Fixture.ini").unwrap();
    t.insert("The Fixture.ini".into(), skin);
    assert!(matches!(
        import(&t, FOLDER, "fixture", None),
        Err(ImportError::MissingFile { .. })
    ));
}

/// `version.ini`'s comments name the authors (Alloy, Simple X, Xglow).
#[test]
fn version_ini_comments_credit_the_authors() {
    let comments: Vec<String> = [
        "Compatible up to Xfire Version 1.55",
        "Alloy Xfire Skin by Gunowar",
        "Designed by [S.C.o.D]GrAsMaN",
        "Coded By Avicus|PSK",
        "by Gunowar",
        "www.gunsammocrate.com",
    ]
    .map(str::to_owned)
    .to_vec();
    assert_eq!(
        stated_authors(&comments),
        ["Gunowar", "[S.C.o.D]GrAsMaN", "Avicus|PSK"]
    );
    let mut t = tree();
    t.insert(
        "version.ini".into(),
        b"[Version]\r\nVersion=1\r\nLongName=The Fixture\r\n;Designed by Ann\r\n".to_vec(),
    );
    let i = settled(import(&t, FOLDER, "fixture", None).unwrap());
    let m = crate::manifest::parse(&crate::package::read(&i.canonical.bytes).unwrap()).unwrap();
    assert_eq!(m.credits.unwrap().authors, ["Ann"]);
}

/// Plan E7.7.7 S2: a chat skin file the skin names and ships is read like
/// the skin file, so a malformed one refuses the import.
#[test]
fn a_malformed_chat_skin_file_refuses() {
    let mut t = tree();
    t.insert("Fix chatskin.ini".into(), b"not an ini".to_vec());
    let err = read(&t, FOLDER).unwrap_err();
    assert!(
        matches!(err, ImportError::Format { ref file, .. } if file == "Fix chatskin.ini"),
        "{err:?}"
    );
}

/// Plan E7.7.7 (the Linux survey's item (b)): Xglow names its list ground
/// `Base` and its chat ground `ChatBack`, labels the name table does not
/// hold; the sections that name them (`[BuddyView] ColorGroup=`, the chat
/// skin's `[ChatArea] ColorGroup=`) give the role, and a neutral selection
/// ground hands the accent to the selection text's hue.
#[test]
fn the_sections_naming_a_colour_give_its_role_and_the_accent() {
    let mut t = tree();
    let theme = THEME.replace(
        "ColorGroup.VentCG",
        "ColorGroup.Base=RGBA(73,73,73,0)\r\nColorGroup.ChatBack=RGBA(37,37,37,0)\r\n\
         ColorGroup.Black=RGBA(0,0,0,0)\r\nColorGroup.Glow=RGBA(116,226,248,0)\r\n\
         ColorGroup.VentCG",
    );
    t.insert("FixThemes.ini".into(), theme.into_bytes());
    let skin = skin_ini()
        + "[BuddyView]\r\nColorGroup=Base\r\nText.color_selected=Black\r\n\
           Text.color_selectedtext=Glow\r\n[InfoView]\r\nColorGroup=Black\r\n\
           Text.color_hyperlink=Glow\r\n";
    t.insert("Fixture.ini".into(), skin.into_bytes());
    t.insert(
        "Fix chatskin.ini".into(),
        b"[MainWindow]\r\nClass=MainWindow\r\n[ChatArea]\r\nColorGroup=ChatBack\r\n".to_vec(),
    );
    let r = read(&t, FOLDER).unwrap();
    let glow = walk::color([116, 226, 248]);
    assert_eq!(r.tokens["accent.fill"], glow);
    assert_eq!(r.tokens["accent.text"], glow);
    assert_eq!(r.tokens["link"], glow);
    assert_eq!(r.tokens["bg.panel"], walk::color([37, 37, 37]));
    assert_eq!(r.tokens["bg.secondary"], walk::color([0, 0, 0]));
    // `BuddyViewBG` names the list ground's role itself: it wins over `Base`.
    assert_eq!(r.tokens["bg.primary"], walk::color([16, 16, 16]));
    assert!(r
        .refs
        .iter()
        .any(|a| a.token == "bg.primary" && a.kept.is_some()));
}

/// SKC2 (spec A8): an alias the theme never defines, or a file the tree
/// lacks, is an undefined bitmap.
#[test]
fn resolve_reads_a_missing_alias_or_file_as_an_undefined_bitmap() {
    let t = tree();
    let aliases = BTreeMap::from([("GONE".to_owned(), "main\\gone.png".to_owned())]);
    for v in ["NOALIAS", "GONE", "main\\gone.png"] {
        assert!(
            matches!(
                resolve(&t, v, &aliases),
                Err(ImportError::UndefinedBitmap { .. })
            ),
            "{v}"
        );
    }
    assert!(resolve(&t, "main\\top.png", &aliases).is_ok());
}

/// SKC2 (spec A8): a frame piece naming a file the tree lacks lays
/// nothing (its state is not emitted), and the NOTICE names the file.
#[test]
fn a_named_file_the_tree_lacks_lays_nothing_and_is_recorded() {
    let mut t = tree();
    let skin = skin_ini().replacen(
        "Hover.topleft=main\\topleft.png",
        "Hover.topleft=main\\gone.png",
        1,
    );
    assert_ne!(skin, skin_ini());
    t.insert("Fixture.ini".into(), skin.into_bytes());
    let notice = settled(import(&t, FOLDER, "fixture", None).unwrap()).notice;
    assert!(notice.contains("main/gone.png"), "{notice}");
}

/// SKC2: a DEL byte is dropped and recorded (Xfire, Inc.'s stock
/// `XfireThemes.ini` holds two); every other control character refuses.
#[test]
fn a_del_byte_is_dropped_and_recorded_other_controls_refuse() {
    let mut t = tree();
    let theme = format!(
        "{THEME}\r\n[Ruby Red]\r\nColorGroup.ScrollbarColor=RGBA(88,-76,-76\u{7f}\u{7f})\r\n"
    );
    t.insert("FixThemes.ini".into(), theme.into_bytes());
    let notice = settled(import(&t, FOLDER, "fixture", None).unwrap()).notice;
    assert!(notice.contains("FixThemes.ini: line"), "{notice}");
    assert!(notice.contains("2 DEL (0x7F) byte(s) dropped"), "{notice}");
    let mut t = tree();
    let theme = format!("{THEME}\r\n[Ruby Red]\r\nX=1\u{1}\r\n");
    t.insert("FixThemes.ini".into(), theme.into_bytes());
    assert!(matches!(
        import(&t, FOLDER, "fixture", None),
        Err(ImportError::Format { .. })
    ));
}

/// SKC2: the written RGBA forms with one reading read as that colour, each
/// recorded; a negative absolute channel, whose readings differ, refuses.
#[test]
fn rgba_forms_with_one_reading_are_normalised_and_recorded() {
    for (written, what) in [
        ("RGBA(183,184,184,0,)", "a trailing empty field dropped"),
        ("RGBA(183,184,184.0)", "a zero fraction dropped"),
        ("(183,184,184,0)", "the RGBA name added"),
    ] {
        let mut t = tree();
        let theme = THEME.replace("RGBA(183,184,184,0)", written);
        t.insert("FixThemes.ini".into(), theme.into_bytes());
        let notice = settled(import(&t, FOLDER, "fixture", None).unwrap()).notice;
        let read = format!("ColorGroup.BuddyViewFriendText={written} read as RGBA(183,184,184");
        assert!(notice.contains(&read), "{notice}");
        assert!(notice.contains(what), "{notice}");
    }
    let mut t = tree();
    let theme = THEME.replace("RGBA(16,16,16,0)", "RGBA(-16,16,16,0)");
    t.insert("FixThemes.ini".into(), theme.into_bytes());
    assert!(matches!(
        import(&t, FOLDER, "fixture", None),
        Err(ImportError::BadColour { .. })
    ));
}
