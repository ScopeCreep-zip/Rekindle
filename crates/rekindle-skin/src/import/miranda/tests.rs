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
    let mut tree = Tree::new();
    tree.insert("fixture.tsk".into(), tsk.as_bytes().to_vec());
    tree.insert("frame.png".into(), png(8, 8));
    tree.insert("hover.png".into(), png(4, 4));
    tree.insert("readme.txt".into(), b"unread".to_vec());
    tree
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
    let mut tree = Tree::new();
    tree.insert("skin/fixture.msf".into(), msf.as_bytes().to_vec());
    tree.insert("atlas.png".into(), png(40, 20));
    tree
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
    let tree = tsk_tree(TSK);
    let imp = imported(import(&tree, "fixture").unwrap());
    assert_eq!(imp, imported(import(&tree, "fixture").unwrap()));
    let rd = read(&tree).unwrap();
    assert_eq!(rd.engine, Engine::TabSrmm);
    let frame = &rd.regions["window.frame"];
    let ins = frame.insets;
    assert_eq!((ins.top, ins.right, ins.bottom, ins.left), (3, 2, 2, 2));
    // Alpha=50 (percent) applied after per-pixel alpha.
    assert_eq!(frame.images[&RegionState::Normal].get_pixel(0, 0).0[3], 127);
    let notice = &imp.notice;
    assert!(
        notice.contains("%Container COLOR1=1E1E1E -> token bg.panel"),
        "{notice}"
    );
    assert!(notice.contains("%Frame COLOR2=101010 -> token titlebar.end"));
    assert!(notice.contains("%Button COLOR1=445566 -> unmapped (no registry role)"));
    assert!(notice.contains("[$Frame] Item=Tabitem: no region slot"));
    assert!(notice.contains("titlebar.buttons Hover: no normal image for the slot"));
    assert!(notice.contains("[%Frame] GRADIENT=down: no registry role"));
    assert!(notice.contains("fixture.tsk: [%Frame] COLOR1 (line 9)"));
    assert!(notice.contains("    readme.txt\n"));
}

#[test]
fn clist_modern_imports_and_validates() {
    let tree = msf_tree(MSF);
    let imp = imported(import(&tree, "fixture").unwrap());
    let rd = read(&tree).unwrap();
    assert_eq!(rd.engine, Engine::ClistModern);
    let win = &rd.regions["window.frame"];
    assert_eq!(
        (
            win.insets.left,
            win.insets.top,
            win.insets.right,
            win.insets.bottom
        ),
        (4, 6, 4, 4)
    );
    assert_eq!(win.images[&RegionState::Normal].dimensions(), (20, 20));
    let bar = &rd.regions["statusbar"].images[&RegionState::Normal];
    assert_eq!(bar.get_pixel(0, 0).0[3], 128);
    let notice = &imp.notice;
    assert!(notice.contains("Name: Synthetic"), "{notice}");
    assert!(notice.contains("tProfile0 text object colour=0x80535666 -> unmapped"));
    assert!(notice.contains("$$Contact/Hot: no region slot"));
    assert!(notice.contains("$$Background: no region slot"));
    assert!(notice.contains("[CLUI] TopClientMargin: a contact-list setting"));
}

#[test]
fn insets_larger_than_the_image_are_refused() {
    let tsk = TSK.replace("Left=2\r\nTop=3\r\nRight=2", "Left=5\r\nTop=3\r\nRight=4");
    let err = import(&tsk_tree(&tsk), "x").unwrap_err();
    assert!(
        matches!(&err, ImportError::Image { detail, .. } if detail.contains("insets 5/3/4/2")),
        "{err}"
    );
    let msf = MSF.replace("StretchBoth,4,6,4,4", "StretchBoth,4,12,4,8");
    let err = import(&msf_tree(&msf), "x").unwrap_err();
    assert!(
        matches!(&err, ImportError::Image { detail, .. } if detail.contains("larger than the 20x20")),
        "{err}"
    );
}

#[test]
fn a_path_leaving_the_skin_is_dropped_and_named() {
    let mut tree = tsk_tree(&TSK.replace("Image=frame.png", "Image=../frame.png"));
    tree.insert("../frame.png".into(), png(8, 8));
    let imp = imported(import(&tree, "x").unwrap());
    assert!(!read(&tree).unwrap().regions.contains_key("window.frame"));
    assert!(
        imp.notice
            .contains("[$Frame] Image=../frame.png: ../frame.png: not a relative path"),
        "{}",
        imp.notice
    );
    let msf = MSF.replace("Fragment,atlas.png,0,0", "Fragment,/etc/atlas.png,0,0");
    let imp = imported(import(&msf_tree(&msf), "x").unwrap());
    assert!(imp
        .notice
        .contains("$$Window: /etc/atlas.png: not a relative path"));
}

#[test]
fn a_non_bitmap_image_is_dropped_and_named() {
    let mut tree = tsk_tree(TSK);
    tree.insert(
        "frame.png".into(),
        b"<svg xmlns='http://www.w3.org/2000/svg'/>".to_vec(),
    );
    let imp = imported(import(&tree, "x").unwrap());
    assert!(!read(&tree).unwrap().regions.contains_key("window.frame"));
    assert!(imp
        .notice
        .contains("[$Frame] Image=frame.png: frame.png: not a bitmap the image validator decodes"));
    // TabSRMM takes PNG only (`tabsrmm.tsk:316`).
    let gif = encode_gif(vec![Frame::new(RgbaImage::from_pixel(8, 8, PIXEL))]).unwrap();
    tree.insert("frame.png".into(), gif);
    let imp = imported(import(&tree, "x").unwrap());
    assert!(imp
        .notice
        .contains("frame.png: Gif, but TabSRMM images are PNG only"));
    // A Clist_modern TGA atlas the bounded decoder refuses (no pixels).
    let mut tree = msf_tree(&MSF.replace("atlas.png", "back.tga"));
    tree.insert(
        "back.tga".into(),
        vec![0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 1, 0, 32, 8],
    );
    let imp = imported(import(&tree, "x").unwrap());
    assert!(
        imp.notice
            .contains("$$Window: back.tga: image: TGA: pixel data truncated"),
        "{}",
        imp.notice
    );
    assert!(read(&tree).unwrap().regions.is_empty());
    // A Clist_modern JPEG atlas: refused as the validator refuses JPEG.
    let mut tree = msf_tree(&MSF.replace("atlas.png", "back.jpg"));
    tree.insert(
        "back.jpg".into(),
        vec![0xFF, 0xD8, 0xFF, 0xE0, 0, 16, b'J', b'F', b'I', b'F', 0],
    );
    let imp = imported(import(&tree, "x").unwrap());
    assert!(imp
        .notice
        .contains("$$Window: back.jpg: JPEG, which the image validator refuses"));
    assert!(read(&tree).unwrap().regions.is_empty());
}

/// A TGA: header (a 3-byte image ID, skipped by its length), then `body`.
fn tga(
    kind: u8,
    map: u8,
    (bits, descriptor): (u8, u8),
    (w, h): (u16, u16),
    body: &[u8],
) -> Vec<u8> {
    let mut v = vec![3, map, kind, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    v.extend_from_slice(&w.to_le_bytes());
    v.extend_from_slice(&h.to_le_bytes());
    v.extend_from_slice(&[bits, descriptor]);
    v.extend_from_slice(b"ID!");
    v.extend_from_slice(body);
    v
}

const A: [u8; 3] = [10, 20, 30];
const B: [u8; 3] = [40, 50, 60];
const C: [u8; 3] = [70, 80, 90];
const D: [u8; 3] = [1, 2, 3];

/// Rows `A A B` / `C D D`, stored top-down as 32-bit BGRA.
fn tga32_top_down() -> Vec<u8> {
    let body: Vec<u8> = [A, A, B, C, D, D]
        .iter()
        .flat_map(|[r, g, b]| [*b, *g, *r, 255])
        .collect();
    tga(2, 0, (32, 8 | 0x20), (3, 2), &body)
}

fn bgr([r, g, b]: [u8; 3]) -> [u8; 3] {
    [b, g, r]
}

#[test]
fn uncompressed_and_rle_tga_decode_to_the_same_pixels() {
    let plain = tga::decode("a", &tga32_top_down()).unwrap();
    // Bottom-up rows `C D D` then `A A B`: a raw packet of 1, a run of 2
    // `D`, a run of 2 `A`, a raw packet of 1.
    let mut body = vec![0x00];
    body.extend(bgr(C));
    body.push(0x81);
    body.extend(bgr(D));
    body.push(0x81);
    body.extend(bgr(A));
    body.push(0x00);
    body.extend(bgr(B));
    let rle = tga::decode("b", &tga(10, 0, (24, 0), (3, 2), &body)).unwrap();
    assert_eq!(plain, rle);
    let [r, g, b] = B;
    assert_eq!(plain.get_pixel(2, 0), &Rgba([r, g, b, 255]));
    let [r, g, b] = C;
    assert_eq!(rle.get_pixel(0, 1), &Rgba([r, g, b, 255]));
}

#[test]
fn a_clist_tga_atlas_imports_as_window_frame() {
    let [r, g, b, a] = PIXEL.0;
    let body: Vec<u8> = (0..40 * 20).flat_map(|_| [b, g, r, a]).collect();
    let mut tree = msf_tree(&MSF.replace("atlas.png", "atlas.tga"));
    tree.remove("atlas.png");
    tree.insert("atlas.tga".into(), tga(2, 0, (32, 8), (40, 20), &body));
    let imp = imported(import(&tree, "fixture").unwrap());
    let rd = read(&tree).unwrap();
    let win = &rd.regions["window.frame"].images[&RegionState::Normal];
    assert_eq!(win.dimensions(), (20, 20));
    assert_eq!(win.get_pixel(3, 7), &PIXEL);
    assert!(rd.unread.is_empty(), "{:?}", rd.unread);
    assert!(
        imp.notice
            .contains("Normal: 20x20 px from $$Window atlas.tga"),
        "{}",
        imp.notice
    );
}

#[test]
fn malformed_tga_is_refused() {
    let refused = |bytes: &[u8], why: &str| {
        let err = tga::decode("t", bytes).unwrap_err();
        assert!(err.to_string().contains(why), "{err}");
    };
    // The stream ends inside the second packet's pixel.
    let mut body = vec![0x81];
    body.extend(bgr(A));
    body.push(0x83);
    body.extend(&bgr(B)[..2]);
    refused(&tga(10, 0, (24, 0), (3, 2), &body), "RLE stream truncated");
    // A run of 7 into a 6-pixel image.
    let mut body = vec![0x86];
    body.extend(bgr(A));
    refused(
        &tga(10, 0, (24, 0), (3, 2), &body),
        "RLE packet past the image",
    );
    // A colour map on a true-colour image.
    let mut cm = tga32_top_down();
    cm[1] = 1;
    cm[5] = 2;
    refused(&cm, "a colour map is refused");
    // Wider than the validator's side limit, every pixel present.
    let body = vec![0; 4096 * 4];
    refused(&tga(2, 0, (32, 8), (4096, 1), &body), "4096x1 is outside");
}

#[test]
fn not_exactly_one_skin_file_is_refused() {
    let mut tree = tsk_tree(TSK);
    tree.insert("other.msf".into(), MSF.as_bytes().to_vec());
    assert!(matches!(
        import(&tree, "x"),
        Err(ImportError::Format { .. })
    ));
    assert!(matches!(
        import(&Tree::new(), "x"),
        Err(ImportError::Format { .. })
    ));
}

#[test]
fn a_clist_modern_description_credits_its_author_and_url() {
    let msf = MSF.replace(
        "Author = Fixture\r\n",
        "Author = Fixture\r\nURL = http://example.org/skin\r\n",
    );
    let tree = msf_tree(&msf);
    let m = manifest(&tree, "fixture", &read(&tree).unwrap(), "", SkinBase::Dark);
    let credits = m.credits.expect("credits");
    assert_eq!(credits.authors, vec!["Fixture".to_owned()]);
    assert_eq!(credits.url.as_deref(), Some("http://example.org/skin"));
}
