//! Synthetic fixtures only: every archive and bitmap is built here (no
//! third-party skin is committed; plan E7.7.3 *Decided (licence)*).

use super::*;
use crate::validate;

/// An 8-bit BMP, bottom-up; `rle` encodes it as `BI_RLE8` (runs of one
/// index, absolute mode for the rest).
fn bmp8(w: u32, h: u32, palette: &[[u8; 3]], px: impl Fn(u32, u32) -> u8, rle: bool) -> Vec<u8> {
    let mut data = Vec::new();
    for y in (0..h).rev() {
        let row: Vec<u8> = (0..w).map(|x| px(x, y)).collect();
        if rle {
            let mut x = 0;
            while x < row.len() {
                let run = row[x..]
                    .iter()
                    .take(255)
                    .take_while(|v| **v == row[x])
                    .count();
                if run >= 2 {
                    data.extend([u8::try_from(run).unwrap(), row[x]]);
                    x += run;
                } else {
                    let n = (row.len() - x).clamp(1, 255);
                    if n < 3 {
                        data.extend([1, row[x]]);
                        x += 1;
                        continue;
                    }
                    let n = n.min(3);
                    data.extend([0, u8::try_from(n).unwrap()]);
                    data.extend(&row[x..x + n]);
                    if n % 2 == 1 {
                        data.push(0);
                    }
                    x += n;
                }
            }
            data.extend([0, 0]);
        } else {
            data.extend(&row);
            data.resize(data.len() + (4 - row.len() % 4) % 4, 0);
        }
    }
    if rle {
        data.extend([0, 1]);
    }
    let off = 14 + 40 + palette.len() * 4;
    let mut b = Vec::new();
    b.extend(b"BM");
    b.extend(u32::try_from(off + data.len()).unwrap().to_le_bytes());
    b.extend([0; 4]);
    b.extend(u32::try_from(off).unwrap().to_le_bytes());
    b.extend(40u32.to_le_bytes());
    b.extend(w.to_le_bytes());
    b.extend(h.to_le_bytes());
    b.extend(1u16.to_le_bytes());
    b.extend(8u16.to_le_bytes());
    b.extend(u32::from(rle).to_le_bytes());
    b.extend([0; 12]);
    b.extend(u32::try_from(palette.len()).unwrap().to_le_bytes());
    b.extend([0; 4]);
    for [r, g, bl] in palette {
        b.extend([*bl, *g, *r, 0]);
    }
    b.extend(data);
    b
}

/// A zip: (name, data, deflate, claimed CRC override).
fn zip_of(entries: &[(&str, Vec<u8>, bool, Option<u32>)]) -> Vec<u8> {
    let (mut out, mut dir) = (Vec::new(), Vec::new());
    for (name, data, deflate, crc) in entries {
        let packed = if *deflate {
            miniz_oxide::deflate::compress_to_vec(data, 9)
        } else {
            data.clone()
        };
        let crc = crc.unwrap_or_else(|| crc32fast::hash(data));
        let method: u16 = if *deflate { 8 } else { 0 };
        let mut common = Vec::new();
        common.extend(20u16.to_le_bytes());
        common.extend(0u16.to_le_bytes());
        common.extend(method.to_le_bytes());
        common.extend([0; 4]);
        common.extend(crc.to_le_bytes());
        common.extend(u32::try_from(packed.len()).unwrap().to_le_bytes());
        common.extend(u32::try_from(data.len()).unwrap().to_le_bytes());
        common.extend(u16::try_from(name.len()).unwrap().to_le_bytes());
        common.extend(0u16.to_le_bytes());
        let offset = u32::try_from(out.len()).unwrap();
        out.extend(0x0403_4b50u32.to_le_bytes());
        out.extend(&common);
        out.extend(name.as_bytes());
        out.extend(&packed);
        dir.extend(0x0201_4b50u32.to_le_bytes());
        dir.extend(20u16.to_le_bytes());
        dir.extend(&common);
        dir.extend([0; 6]);
        dir.extend([0; 4]);
        dir.extend(offset.to_le_bytes());
        dir.extend(name.as_bytes());
    }
    let (start, n) = (out.len(), u16::try_from(entries.len()).unwrap());
    out.extend(&dir);
    out.extend(0x0605_4b50u32.to_le_bytes());
    out.extend([0; 4]);
    out.extend(n.to_le_bytes());
    out.extend(n.to_le_bytes());
    out.extend(u32::try_from(dir.len()).unwrap().to_le_bytes());
    out.extend(u32::try_from(start).unwrap().to_le_bytes());
    out.extend([0; 2]);
    out
}

/// GENEX with `colours[i]` at x = 48 + 2i on row 0, the rest index 0.
fn genex(colours: &[[u8; 3]; 22]) -> Vec<u8> {
    let mut palette = vec![[0xbd, 0xce, 0xd6]];
    palette.extend(colours);
    bmp8(
        112,
        60,
        &palette,
        |x, y| {
            if y == 0 && (48..=90).contains(&x) && x % 2 == 0 {
                u8::try_from((x - 48) / 2 + 1).unwrap()
            } else {
                0
            }
        },
        true,
    )
}

fn skin(window_text: [u8; 3]) -> Vec<u8> {
    skin_on(window_text, [0x20, 0x20, 0x20])
}

/// A skin with `window_text` on the window background `window_bg`.
fn skin_on(window_text: [u8; 3], window_bg: [u8; 3]) -> Vec<u8> {
    let mut c = [[0x24, 0x24, 0x3c]; 22];
    c[0] = [0, 0, 0];
    c[2] = window_bg;
    c[4] = window_text;
    let gen = bmp8(
        180,
        90,
        &[[0x20, 0x20, 0x40], [0x80, 0x80, 0xa0]],
        |x, y| u8::from((x * x + y * 3) % 5 == 0),
        true,
    );
    let title = bmp8(27, 18, &[[0x60, 0x60, 0x60]], |_, _| 0, false);
    zip_of(&[
        ("MySkin/GEN.BMP", gen, true, None),
        ("MySkin/genex.bmp", genex(&c), true, None),
        ("MySkin/TITLEBAR.BMP", title, false, None),
        (
            "MySkin/PLEDIT.TXT",
            b"[Text]\r\nNormal=#00FF00\r\nCurrent=#FFFFFF\r\nNormalBG=#000000\r\nSelectedBG=#0000C6\r\nFont=Arial\r\n"
                .to_vec(),
            true,
            None,
        ),
        // Never read: a corrupt CRC and garbage data would refuse if inflated.
        ("MySkin/REGION.TXT", b"[Normal]\nNumPoints=4".to_vec(), true, Some(1)),
        ("MySkin/normal.cur", b"RIFF\xff\xff\xff\xffACON".to_vec(), false, Some(2)),
    ])
}

#[test]
fn a_minimal_wsz_imports_and_validates() {
    let Outcome::Passed(i) = import(&skin([0xe7; 3]), "w").unwrap() else {
        panic!("#e7e7e7 window text on #202020 passes")
    };
    let c = validate::run(&i.canonical.bytes).unwrap();
    assert_eq!(c.bytes, i.canonical.bytes);
    let r = read(&skin([0xe7; 3])).unwrap();
    assert_eq!(r.sprites.iter().filter(|(_, p)| *p).count(), 39);
    assert!(r.frame.is_some() && r.title.is_some());
    assert_eq!(
        r.tokens.get("bg.panel"),
        Some(&walk::color([0x20, 0x20, 0x20]))
    );
    assert_eq!(r.tokens.get("text.base"), Some(&walk::color([0, 0xff, 0])));
    assert_eq!(
        r.tokens.get("titlebar.start"),
        Some(&walk::color([0x20, 0x20, 0x40]))
    );
    assert!(i.notice.contains("MySkin/REGION.TXT") && i.notice.contains("MySkin/normal.cur"));
    assert!(i.notice.contains("no registry region scrollbar.thumb yet"));
    // Deterministic.
    let Outcome::Passed(j) = import(&skin([0xe7; 3]), "w").unwrap() else {
        panic!()
    };
    assert_eq!(i, j);
}

#[test]
fn a_failing_wsz_is_a_draft_naming_the_pair() {
    let Outcome::Draft(d) = import(&skin([0x50, 0x50, 0x70]), "w").unwrap() else {
        panic!("#505070 on #202020 fails")
    };
    assert!(validate::run(&d.package).is_err());
    assert!(d.failures.iter().any(|c| c.original == [0x50, 0x50, 0x70]));
    assert!(d.unsettled.is_none());
    validate::run(&d.suggested.unwrap().canonical.bytes).unwrap();
}

#[test]
fn rle8_decodes_like_uncompressed() {
    let pal = [[1, 2, 3], [200, 100, 50], [9, 9, 9]];
    let px = |x: u32, y: u32| u8::try_from((x / 3 + y) % 3).unwrap();
    let plain = bmp::decode("a", &bmp8(13, 5, &pal, px, false)).unwrap();
    let rle = bmp::decode("b", &bmp8(13, 5, &pal, px, true)).unwrap();
    assert_eq!(plain, rle);
    assert_eq!(plain.get_pixel(3, 0).0, [200, 100, 50, 255]);
    assert!(bmp::decode("c", b"BM\0\0").is_err());
}

#[test]
fn a_zip_bomb_and_an_oversize_archive_are_refused() {
    let bomb = zip_of(&[("GEN.BMP", vec![0; 1 << 20], true, None)]);
    let e = archive::unpack(&bomb).unwrap_err();
    assert!(e.to_string().contains("ratio"), "{e}");
    let big = vec![0u8; usize::try_from(archive::MAX_ARCHIVE_BYTES).unwrap() + 1];
    let e = archive::unpack(&big).unwrap_err();
    assert!(e.to_string().contains("past"), "{e}");
}

#[test]
fn cursors_and_region_txt_are_never_read() {
    let (tree, ignored) = archive::unpack(&skin([0xe7; 3])).unwrap();
    assert_eq!(
        tree.keys().collect::<Vec<_>>(),
        ["gen.bmp", "genex.bmp", "pledit.txt", "titlebar.bmp"]
    );
    assert_eq!(ignored, ["MySkin/REGION.TXT", "MySkin/normal.cur"]);
}

#[test]
fn an_unsettled_refusal_is_a_draft_with_no_suggestion() {
    // Stock Winamp's #ffffff window text: on #404040 fixed danger.fg ends
    // up failing on its fill; on a light window no lightness of
    // text.primary passes on every ground. Neither is an error.
    for bg in [[0x40; 3], [0xbd, 0xce, 0xd6]] {
        let Outcome::Draft(d) = import(&skin_on([0xff; 3], bg), "w").unwrap() else {
            panic!("#ffffff on {bg:?} fails")
        };
        let u = d.unsettled.as_ref().expect("an unsettled failure");
        assert!(!u.why.is_empty());
        assert!(d.suggested.is_none());
        assert!(validate::run(&d.package).is_err());
    }
}
