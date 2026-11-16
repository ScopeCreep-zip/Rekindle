//! The importer on small synthetic TabSRMM and Clist_modern skins (the
//! engines' documented shapes; no third-party art or skin text).

use rekindle_image::{encode_gif, encode_png, DynamicImage, Frame, Rgba, RgbaImage};

use super::*;
use crate::import::Imported;
use crate::validate;

const PIXEL: Rgba<u8> = Rgba([40, 100, 160, 255]);

fn png(w: u32, h: u32) -> Vec<u8> {
    encode_png(&DynamicImage::ImageRgba8(RgbaImage::from_pixel(
        w, h, PIXEL,
    )))
    .unwrap()
}

const TSK: &str = "; synthetic TabSRMM skin\r\n\
[%Container]\r\nALPHA=100\r\nCOLOR1=1E1E1E\r\nTEXTCOLOR=E8E8E8\r\n\
[%Frame]\r\nCOLOR1=303030\r\nCOLOR2=101010\r\nCOLOR1=AAAAAA\r\nTEXTCOLOR=F0F0F0\r\nGRADIENT=down\r\n\
[%Button]\r\nCOLOR1=445566\r\n\
[%InputArea]\r\nCOLOR1=1E1E1E\r\n\
[$Frame]\r\nImage=frame.png\r\nLeft=2\r\nTop=3\r\nRight=2\r\nBottom=2\r\nAlpha=50\r\n\
Item0=Frame\r\nItem1=Tabitem\r\nPerpixel=1\r\n\
[$Hover]\r\nImage=hover.png\r\nLeft=1\r\nTop=1\r\nRight=1\r\nBottom=1\r\nItem0=Titlebuttonmouseover\r\n";

fn tsk_tree(tsk: &str) -> Tree {
    let mut t = Tree::new();
    t.insert("fixture.tsk".into(), tsk.as_bytes().to_vec());
    t.insert("frame.png".into(), png(8, 8));
    t.insert("hover.png".into(), png(4, 4));
    t.insert("readme.txt".into(), b"unread".to_vec());
    t
}

const MSF: &str = "[Skin_Description_Section]\r\nName = Synthetic\r\nAuthor = Fixture\r\n\
[ModernSkin]\r\nfProfFont = sVerdana,12,---\r\n\
tProfile0 = s$$Window,26,4,-32,19,LTRTCC,fProfFont,0x80535666,0,Fixture\r\n\
$$Window = sGlyph,Fragment,atlas.png,0,0,20,20,StretchBoth,4,6,4,4,255\r\n\
$$StatusBar = sGlyph,Fragment,atlas.png,20,0,10,10,StretchBoth,2,2,2,2,128\r\n\
$$Contact/Hot = sGlyph,Fragment,atlas.png,30,0,4,4,StretchBoth,1,1,1,1,255\r\n\
$$Background = sGlyph,Skip\r\n\
@0 = s$$Window:Main,ID=Background\r\n\
[CLUI]\r\nTopClientMargin = b25\r\n";

fn msf_tree(msf: &str) -> Tree {
    let mut t = Tree::new();
    t.insert("skin/fixture.msf".into(), msf.as_bytes().to_vec());
    t.insert("atlas.png".into(), png(40, 20));
    t
}

/// The passing package (a draft's suggestions accepted), validated.
fn imported(o: Outcome) -> Imported {
    let i = match o {
        Outcome::Passed(i) => i,
        Outcome::Draft(d) => d
            .suggested
            .unwrap_or_else(|| panic!("unsettled: {:?}", d.unsettled)),
    };
    let again = validate::run(&i.canonical.bytes).unwrap();
    assert_eq!(again.bytes, i.canonical.bytes);
    i
}

#[test]
fn tabsrmm_imports_and_validates() {
    let t = tsk_tree(TSK);
    let a = imported(import(&t, "fixture").unwrap());
    assert_eq!(a, imported(import(&t, "fixture").unwrap()));
    let r = read(&t).unwrap();
    assert_eq!(r.engine, Engine::TabSrmm);
    let frame = &r.regions["window.frame"];
    let i = frame.insets;
    assert_eq!((i.top, i.right, i.bottom, i.left), (3, 2, 2, 2));
    // Alpha=50 (percent) applied after per-pixel alpha.
    assert_eq!(frame.images[&RegionState::Normal].get_pixel(0, 0).0[3], 127);
    let n = &a.notice;
    assert!(
        n.contains("%Container COLOR1=1E1E1E -> token bg.panel"),
        "{n}"
    );
    assert!(n.contains("%Frame COLOR2=101010 -> token titlebar.end"));
    assert!(n.contains("%Button COLOR1=445566 -> unmapped (no registry role)"));
    assert!(n.contains("[$Frame] Item=Tabitem: no region slot"));
    assert!(n.contains("titlebar.buttons Hover: no normal image for the slot"));
    assert!(n.contains("[%Frame] GRADIENT=down: no registry role"));
    assert!(n.contains("fixture.tsk: [%Frame] COLOR1 (line 9)"));
    assert!(n.contains("    readme.txt\n"));
}

#[test]
fn clist_modern_imports_and_validates() {
    let t = msf_tree(MSF);
    let a = imported(import(&t, "fixture").unwrap());
    let r = read(&t).unwrap();
    assert_eq!(r.engine, Engine::ClistModern);
    let w = &r.regions["window.frame"];
    assert_eq!(
        (w.insets.left, w.insets.top, w.insets.right, w.insets.bottom),
        (4, 6, 4, 4)
    );
    assert_eq!(w.images[&RegionState::Normal].dimensions(), (20, 20));
    let s = &r.regions["statusbar"].images[&RegionState::Normal];
    assert_eq!(s.get_pixel(0, 0).0[3], 128);
    let n = &a.notice;
    assert!(n.contains("Name: Synthetic"), "{n}");
    assert!(n.contains("tProfile0 text object colour=0x80535666 -> unmapped"));
    assert!(n.contains("$$Contact/Hot: no region slot"));
    assert!(n.contains("$$Background: no region slot"));
    assert!(n.contains("[CLUI] TopClientMargin: a contact-list setting"));
}

#[test]
fn insets_larger_than_the_image_are_refused() {
    let tsk = TSK.replace("Left=2\r\nTop=3\r\nRight=2", "Left=5\r\nTop=3\r\nRight=4");
    let e = import(&tsk_tree(&tsk), "x").unwrap_err();
    assert!(
        matches!(&e, ImportError::Image { detail, .. } if detail.contains("insets 5/3/4/2")),
        "{e}"
    );
    let msf = MSF.replace("StretchBoth,4,6,4,4", "StretchBoth,4,12,4,8");
    let e = import(&msf_tree(&msf), "x").unwrap_err();
    assert!(
        matches!(&e, ImportError::Image { detail, .. } if detail.contains("larger than the 20x20")),
        "{e}"
    );
}

#[test]
fn a_path_leaving_the_skin_is_dropped_and_named() {
    let mut t = tsk_tree(&TSK.replace("Image=frame.png", "Image=../frame.png"));
    t.insert("../frame.png".into(), png(8, 8));
    let a = imported(import(&t, "x").unwrap());
    assert!(!read(&t).unwrap().regions.contains_key("window.frame"));
    assert!(
        a.notice
            .contains("[$Frame] Image=../frame.png: ../frame.png: not a relative path"),
        "{}",
        a.notice
    );
    let msf = MSF.replace("Fragment,atlas.png,0,0", "Fragment,/etc/atlas.png,0,0");
    let a = imported(import(&msf_tree(&msf), "x").unwrap());
    assert!(a
        .notice
        .contains("$$Window: /etc/atlas.png: not a relative path"));
}

#[test]
fn a_non_bitmap_image_is_dropped_and_named() {
    let mut t = tsk_tree(TSK);
    t.insert(
        "frame.png".into(),
        b"<svg xmlns='http://www.w3.org/2000/svg'/>".to_vec(),
    );
    let a = imported(import(&t, "x").unwrap());
    assert!(!read(&t).unwrap().regions.contains_key("window.frame"));
    assert!(a
        .notice
        .contains("[$Frame] Image=frame.png: frame.png: not a bitmap the image validator decodes"));
    // TabSRMM takes PNG only (`tabsrmm.tsk:316`).
    let gif = encode_gif(vec![Frame::new(RgbaImage::from_pixel(8, 8, PIXEL))]).unwrap();
    t.insert("frame.png".into(), gif);
    let a = imported(import(&t, "x").unwrap());
    assert!(a
        .notice
        .contains("frame.png: Gif, but TabSRMM images are PNG only"));
    // A Clist_modern TGA atlas.
    let mut t = msf_tree(&MSF.replace("atlas.png", "back.tga"));
    t.insert(
        "back.tga".into(),
        vec![0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 1, 0, 32, 8],
    );
    let a = imported(import(&t, "x").unwrap());
    assert!(a.notice.contains("$$Window: back.tga: not a bitmap"));
    assert!(read(&t).unwrap().regions.is_empty());
}

#[test]
fn not_exactly_one_skin_file_is_refused() {
    let mut t = tsk_tree(TSK);
    t.insert("other.msf".into(), MSF.as_bytes().to_vec());
    assert!(matches!(import(&t, "x"), Err(ImportError::Format { .. })));
    assert!(matches!(
        import(&Tree::new(), "x"),
        Err(ImportError::Format { .. })
    ));
}
